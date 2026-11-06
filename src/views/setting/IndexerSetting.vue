<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { join } from '@tauri-apps/api/path'
import type { DataTableColumns } from 'naive-ui'
import ModelPlatformEdit from './ModelPlatformEdit.vue'
import SelfHostedPlatformEdit from './SelfHostedPlatformEdit.vue'
import { useIndexerStore } from '@/stores/indexer'
import { useAppStore } from '@/stores/app'
import { t } from '@/locales'

interface RowData {
  name: string
  key: string
  status: string
  children?: RowData[]
  render?: (row: RowData) => VNode
}

type ParserMode = 'local' | 'selfhosted' | 'remote' | 'mixed'
type BackendMode = 'local' | 'selfhosted' | 'remote'

const indexerStore = useIndexerStore()
const appStore = useAppStore()
const activePlatform = ref('openai')
const modelPlatformList = ref<ModelPlatform[]>([])
const selfHostedPlatformList = ref<SelfHostedPlatform[]>([])
const activeSelfHostedPlatform = ref('')
const dataRef = ref<RowData[]>([])
const imageParserDesc = ref('')
const audioParserDesc = ref('')
const vectorParserDesc = ref('')

const modelPath = ref('')
const dbPath = ref('')

// Parser mode state
const parserMode = ref<ParserMode>('local')

// Local backend availability (one detection source, shared by greying,
// preset fallback and switch validation)
const localBackends = ref<LocalBackendStatus | null>(null)

// Active embedding identity: { platform, model, dim, has_sparse }
const activeIdentity = ref<EmbeddingIdentity | null>(null)
// Embedding model row of the CURRENT mode's platform (display + switch source)
const activeEmbeddingModel = ref<AiModel | null>(null)

// Self-hosted model state
const selfHostedVisionModel = ref<AiModel | null>(null)
const showModelEditModal = ref(false)
const editingModel = ref<AiModel | null>(null)
const editingModelOldName = ref('')
const showSelfHostedModal = ref(false)
const showCloudModal = ref(false)

// Parser model change light notice (optional category re-index offer)
const parserModelNotice = ref<{ category: 'image' | 'audio'; oldName: string; newName: string } | null>(null)
const reindexingCategory = ref(false)

// Embedding switch dialog state
interface PendingSwitch {
  mode: BackendMode
  platform: string
  model: string
  modelTitle: string
  existingCollection: boolean | null // null = unknown yet (probed at confirm)
  pendingFiles: number
}
const pendingSwitch = ref<PendingSwitch | null>(null)
const showSwitchDialog = ref(false)
const switchRebuildChoice = ref<'fast' | 'full' | 'none'>('fast')
const switchClearOthers = ref(false)
const switchFastAvailable = ref(true)
const switchingBackend = ref(false)

initStatusData()

function vectorAvailable() {
  return localBackends.value ? localBackends.value.vector_missing.length === 0 : true
}
function visionAvailable() {
  return localBackends.value ? localBackends.value.vision_missing.length === 0 : true
}
function audioAvailable() {
  return localBackends.value ? localBackends.value.audio_missing.length === 0 : true
}
/** Only vLLM can serve self-hosted ASR (Ollama has no transcription endpoint) */
function selfHostedAudioAvailable() {
  return activeSelfHostedPlatform.value === 'vllm'
}

function missingFilesTooltip(missing: string[]) {
  return t('indexer.missingLocalFiles', { files: missing.join(', ') })
}

/** Backend display name for an embedding identity: "model · backend" */
function identityBackendTitle(platform: string) {
  if (platform === 'local')
    return t('common.local')
  const selfHosted = selfHostedPlatformList.value.find(p => p.name === platform)
  if (selfHosted)
    return selfHosted.title
  const cloud = modelPlatformList.value.find(p => p.name === platform)
  return cloud?.title || platform
}

async function initStatusData() {
  // Detect parser mode based on image and audio parser modes
  const setting = indexerStore.indexerSetting
  const imageMode = setting.image_parser_mode
  const audioMode = setting.audio_parser_mode

  // Determine mode: local | selfhosted | remote | mixed
  if (imageMode === 'local' && audioMode === 'local')
    parserMode.value = 'local'
  else if (imageMode === 'selfhosted' && audioMode === 'local')
    parserMode.value = 'selfhosted'
  else if (imageMode === 'remote' && audioMode === 'remote')
    parserMode.value = 'remote'
  else
    parserMode.value = 'mixed'

  imageParserDesc.value = t('common.local')
  audioParserDesc.value = t('common.local')
  vectorParserDesc.value = '-'

  if (parserMode.value === 'remote' || imageMode === 'remote') {
    const platformInfo = modelPlatformList.value.find(
      p => p.name === activePlatform.value,
    )
    if (platformInfo) {
      const aiModel = await invoke<AiModel>('load_model_by_type', {
        platform: activePlatform.value,
        oneType: 'vision',
      })
      if (aiModel && aiModel.name)
        imageParserDesc.value = `${platformInfo.title} => ${aiModel.title}`

      const audioModel = await invoke<AiModel>('load_model_by_type', {
        platform: activePlatform.value,
        oneType: 'asr',
      })
      if (audioModel && audioModel.name)
        audioParserDesc.value = `${platformInfo.title} => ${audioModel.title}`
    }
  } else if (imageMode === 'selfhosted') {
    const platformInfo = selfHostedPlatformList.value.find(
      p => p.name === activeSelfHostedPlatform.value,
    )
    if (platformInfo) {
      const aiModel = await invoke<AiModel>('load_model_by_type', {
        platform: activeSelfHostedPlatform.value,
        oneType: 'vision',
      })
      selfHostedVisionModel.value = aiModel
      imageParserDesc.value = `${platformInfo.title} => ${aiModel?.title || aiModel?.name}`
    }
  }

  // Vector backend: display as "model · backend" from the active identity
  if (activeIdentity.value)
    vectorParserDesc.value = `${activeIdentity.value.model} · ${identityBackendTitle(activeIdentity.value.platform)}`

  dataRef.value = [
    {
      name: t('indexer.indexFileMetadata'),
      status: t('common.enable'),
      key: 'index-metadata',
    },
    {
      name: t('indexer.indexFileContent'),
      status: t('common.enable'),
      key: 'index-content',
      children: [
        {
          name: t('indexer.documentParser'),
          status: t('common.local'),
          key: 'index-document-parser',
        },
        {
          name: t('indexer.imageParser'),
          status: imageParserDesc.value,
          key: 'index-image-parser',
        },
        {
          name: t('indexer.audioParser'),
          status: audioParserDesc.value,
          key: 'index-audio-parser',
        },
        {
          name: t('indexer.vectorParser'),
          status: vectorParserDesc.value,
          key: 'index-vector-parser',
        },
      ],
    },
    {
      name: t('indexer.ignoreFilesAndFolders'),
      status: '',
      key: 'ignore-files-and-folders',
      children: [
        {
          name: t('indexer.ignoreFileExtensions'),
          status: indexerStore.indexerSetting.ignore_exts.length === 0 ? t('common.none') : indexerStore.indexerSetting.ignore_exts.join(', '),
          key: 'ignore-file-extensions',
        },
        {
          name: t('indexer.ignoreFiles'),
          status:
            indexerStore.indexerSetting.ignore_files.length === 0
              ? t('common.none')
              : indexerStore.indexerSetting.ignore_files.join(', '),
          key: 'ignore-files',
        },
        {
          name: t('indexer.ignoreHiddenFiles'),
          status: t('common.yes'),
          key: 'ignore-hidden-files',
        },
        {
          name: t('indexer.ignoreFolders'),
          status: indexerStore.indexerSetting.ignore_dirs.length === 0 ? t('common.none') : indexerStore.indexerSetting.ignore_dirs.join(', '),
          key: 'ignore-folders',
        },
        {
          name: t('indexer.ignorePathPrefixes'),
          status: indexerStore.indexerSetting.ignore_path_prefixes?.length === 0 ? t('common.none') : indexerStore.indexerSetting.ignore_path_prefixes?.join(', '),
          key: 'ignore-path-prefixes',
        },
        {
          name: t('indexer.ignoreHiddenFolders'),
          status: t('common.yes'),
          key: 'ignore-hidden-folders',
        },
      ],
    },
  ]
}

const columns = computed<DataTableColumns<RowData>>(() => [
  {
    title: t('common.setting'),
    key: 'name',
    width: '220px',
    render(row) {
      const childCount = row.children?.length || 0
      if (childCount > 0) {
        return h(
          'span',
          {
            class: '',
            onClick: (e: PointerEvent) => {
              const target = e.target as HTMLElement
              if (target.parentElement) {
                const trigger = target.parentElement.querySelector(
                  '.n-data-table-expand-trigger',
                )
                if (trigger)
                  (trigger as HTMLElement).click()
                else console.log('trigger not found')
              } else {
                console.log('no target')
              }
            },
          },
          {
            default: () => row.name,
          },
        )
      } else {
        return row.name
      }
    },
  },
  {
    title: t('common.status'),
    key: 'status',
  },
])

async function doActivePlatformChanged(selectedName: string) {
  activePlatform.value = selectedName
  try {
    await invoke('set_active_platform', {
      platformName: selectedName,
    })
    window.$message.success(t('common.updateSuccess'))
  } catch (error) {
    console.error('set active tab error', error)
  }
  await loadEmbeddingModel()
  initStatusData()
}

async function doActiveSelfHostedPlatformChanged(selectedName: string) {
  activeSelfHostedPlatform.value = selectedName
  try {
    await invoke('set_active_self_hosted_platform', {
      platformName: selectedName,
    })
    window.$message.success(t('common.updateSuccess'))
  } catch (error) {
    console.error('set active self-hosted platform error', error)
  }
  await loadEmbeddingModel()
  initStatusData()
}

/** Load the embedding ai_model row for the CURRENT embedding mode's platform */
async function loadEmbeddingModel() {
  const mode = indexerStore.indexerSetting.embedding_parser_mode
  activeEmbeddingModel.value = null
  if (mode === 'local')
    return
  const platform = mode === 'selfhosted' ? activeSelfHostedPlatform.value : activePlatform.value
  if (!platform)
    return
  try {
    activeEmbeddingModel.value = await invoke<AiModel>('load_model_by_type', {
      platform,
      oneType: 'embedding',
    })
  } catch (e) {
    console.warn('Failed to load embedding model:', e)
  }
}

// Storage change state
const changingType = ref<'document' | 'image' | 'audio' | null>(null)
const changingProgress = ref({ current: 0, total: 0 })

async function onContentStorageChanged(type: 'document' | 'image' | 'audio', value: string) {
  const oldValue = indexerStore.indexerSetting.content_storage[type] || 'database'
  if (oldValue === value)
    return

  // Block if another storage change is already running
  if (changingType.value) {
    window.$message.warning(t('indexer.changing'))
    return
  }

  // Count affected files
  let affectedCount = 0
  try {
    affectedCount = await invoke<number>('count_indexed_files', { category: type })
  } catch (e) {
    console.error('Failed to count indexed files:', e)
    // On error, show dialog anyway so user can decide
  }

  if (affectedCount === 0) {
    // No files to change, just save
    indexerStore.indexerSetting.content_storage[type] = value
    updateIndexerSetting()
    return
  }

  // Build confirmation message
  let confirmMsg = ''
  if (value === 'none')
    confirmMsg = t('indexer.changeConfirmNone', { count: affectedCount })
  else if (oldValue === 'none')
    confirmMsg = t('indexer.changeConfirmReparse', { count: affectedCount })
  else if (value === 'file')
    confirmMsg = t('indexer.changeConfirmFile', { count: affectedCount })
  else
    confirmMsg = t('indexer.changeConfirmDatabase', { count: affectedCount })

  // Show confirmation dialog
  window.$dialog.warning({
    title: t('common.tip'),
    content: confirmMsg,
    positiveText: t('common.confirm'),
    negativeText: t('common.cancel'),
    onPositiveClick: async () => {
      try {
        changingType.value = type
        changingProgress.value = { current: 0, total: affectedCount }
        await invoke('change_content_storage', { category: type, newMode: value })
        // Update local state (backend has spawned the task)
        indexerStore.indexerSetting.content_storage[type] = value
      } catch (e) {
        console.error('Storage change failed:', e)
        window.$message.error(t('common.operationFailed'))
        // Revert UI on failure
        changingType.value = null
        indexerStore.indexerSetting.content_storage[type] = oldValue
      }
    },
    onNegativeClick: () => {
      // Value was never changed, no revert needed (controlled component)
    },
  })
}

// Listen for storage change events
let unlistenStorageChange: (() => void) | null = null

async function doParserModeChanged(mode: ParserMode) {
  // Mixed mode cannot be set by user directly
  if (mode === 'mixed')
    return

  parserMode.value = mode
  indexerStore.indexerSetting.parser_mode = mode

  if (mode === 'local') {
    // 本地模式：image/audio/vector 尽量 local；本地模型文件缺失的类目回落 remote
    indexerStore.indexerSetting.image_parser_mode = visionAvailable() ? 'local' : 'remote'
    indexerStore.indexerSetting.audio_parser_mode = audioAvailable() ? 'local' : 'remote'
    // Vector never silently falls back (identity change = rebuild); keep as-is
  } else if (mode === 'selfhosted') {
    // 自托管模式：image 使用 selfhosted，audio 仅 vLLM 可自托管
    indexerStore.indexerSetting.image_parser_mode = 'selfhosted'
    indexerStore.indexerSetting.audio_parser_mode = selfHostedAudioAvailable() ? 'selfhosted' : (audioAvailable() ? 'local' : 'remote')
  } else if (mode === 'remote') {
    // 云端模式：image 和 audio 都使用 remote
    indexerStore.indexerSetting.image_parser_mode = 'remote'
    indexerStore.indexerSetting.audio_parser_mode = 'remote'
  }

  updateIndexerSetting()
}

// Handle image parser mode change
function onImageParserModeChanged(mode: string) {
  if (mode === 'local' && !visionAvailable()) {
    window.$message.warning(missingFilesTooltip(localBackends.value?.vision_missing || []))
    return
  }
  indexerStore.indexerSetting.image_parser_mode = mode
  updateParserModeFromSettings()
  updateIndexerSetting()
}

// Handle audio parser mode change
function onAudioParserModeChanged(mode: string) {
  if (mode === 'local' && !audioAvailable()) {
    window.$message.warning(missingFilesTooltip(localBackends.value?.audio_missing || []))
    return
  }
  indexerStore.indexerSetting.audio_parser_mode = mode
  updateParserModeFromSettings()
  updateIndexerSetting()
}

// ---------------------------------------------------------------------------
// Embedding (vector) backend switching
// 向量后端切换
// ---------------------------------------------------------------------------

/** Resolve the target platform+model for a vector backend mode */
async function resolveEmbeddingTarget(mode: BackendMode): Promise<{ platform: string, model: string, modelTitle: string } | null> {
  if (mode === 'local')
    return { platform: 'local', model: 'bge-m3', modelTitle: 'BGE-M3' }
  const platform = mode === 'selfhosted' ? activeSelfHostedPlatform.value : activePlatform.value
  if (!platform) {
    window.$message.warning(t('indexer.switchNeedPlatform'))
    return null
  }
  try {
    const model = await invoke<AiModel>('load_model_by_type', {
      platform,
      oneType: 'embedding',
    })
    if (!model || !model.name) {
      window.$message.warning(t('indexer.switchNeedModel'))
      return null
    }
    return { platform, model: model.name, modelTitle: model.title || model.name }
  } catch (e) {
    console.error('load embedding model failed', e)
    window.$message.warning(t('indexer.switchNeedModel'))
    return null
  }
}

/** Vector backend radio change: validate, then open the switch confirmation */
async function onEmbeddingModeChanged(mode: string) {
  const target = mode as BackendMode
  if (target === 'local' && !vectorAvailable()) {
    window.$message.warning(missingFilesTooltip(localBackends.value?.vector_missing || []))
    return
  }
  const resolved = await resolveEmbeddingTarget(target)
  if (!resolved)
    return

  const identity = activeIdentity.value
  const sameBackend = !!identity
    && identity.platform === resolved.platform
    && identity.model === resolved.model
  // Same backend AND same mode = nothing to switch. A mode label that matches
  // but resolves to a different platform (the active platform changed under
  // remote/selfhosted) still opens the dialog.
  if (sameBackend && target === indexerStore.indexerSetting.embedding_parser_mode)
    return
  if (sameBackend) {
    // Same backend, mode realigned: just persist
    indexerStore.indexerSetting.embedding_parser_mode = target
    updateIndexerSetting()
    return
  }

  // Count files for the rebuild estimate
  let pendingFiles = 0
  try {
    pendingFiles = await invoke<number>('count_indexed_files', { category: 'document' })
      + await invoke<number>('count_indexed_files', { category: 'image' })
      + await invoke<number>('count_indexed_files', { category: 'audio' })
  } catch { /* estimate only */ }

  // Collection states: the target's existence, and the ACTIVE library's rows
  // (its stored chunk_text is the fast re-embed channel's raw material)
  let existing = false
  let activeContentRows = 0
  try {
    const collections = await invoke<EmbeddingCollectionRow[]>('load_embedding_collections')
    existing = collections.some(
      c => c.platform === resolved.platform && c.model === resolved.model && (c.content_rows > 0 || c.meta_rows > 0),
    )
    if (identity) {
      activeContentRows = collections.find(
        c => c.platform === identity.platform && c.model === identity.model,
      )?.content_rows ?? 0
    }
  } catch { /* unknown → assume new */ }

  switchFastAvailable.value = activeContentRows > 0
  switchRebuildChoice.value = existing ? 'none' : (switchFastAvailable.value ? 'fast' : 'full')
  switchClearOthers.value = false
  pendingSwitch.value = {
    mode: target,
    platform: resolved.platform,
    model: resolved.model,
    modelTitle: resolved.modelTitle,
    existingCollection: existing,
    pendingFiles,
  }
  showSwitchDialog.value = true
}

async function doConfirmSwitch() {
  if (!pendingSwitch.value)
    return
  const target = pendingSwitch.value
  switchingBackend.value = true
  try {
    const outcome = await invoke<{ identity: EmbeddingIdentity, existing_collection: boolean, rebuild_started: boolean }>('switch_embedding_backend', {
      mode: target.mode,
      platform: target.platform,
      model: target.model,
      rebuild: switchRebuildChoice.value,
      clearOthers: switchClearOthers.value,
    })
    indexerStore.indexerSetting.embedding_parser_mode = target.mode
    showSwitchDialog.value = false
    window.$message.success(
      outcome.rebuild_started
        ? t('indexer.switchDoneRebuilding')
        : t('indexer.switchDoneInstant'),
    )
    activeIdentity.value = await invoke<EmbeddingIdentity>('get_active_embedding_identity')
    await loadEmbeddingModel()
    initStatusData()
  } catch (e) {
    console.error('switch embedding backend failed', e)
    window.$message.error(typeof e === 'string' ? e : t('common.operationFailed'))
  } finally {
    switchingBackend.value = false
  }
}

// Update parserMode based on current settings
function updateParserModeFromSettings() {
  const imageMode = indexerStore.indexerSetting.image_parser_mode
  const audioMode = indexerStore.indexerSetting.audio_parser_mode

  if (imageMode === 'local' && audioMode === 'local') {
    parserMode.value = 'local'
    indexerStore.indexerSetting.parser_mode = 'local'
  } else if (imageMode === 'selfhosted' && audioMode === 'local') {
    parserMode.value = 'selfhosted'
    indexerStore.indexerSetting.parser_mode = 'selfhosted'
  } else if (imageMode === 'remote' && audioMode === 'remote') {
    parserMode.value = 'remote'
    indexerStore.indexerSetting.parser_mode = 'remote'
  } else {
    parserMode.value = 'mixed'
    indexerStore.indexerSetting.parser_mode = 'mixed'
  }
}

async function updateIndexerSetting() {
  try {
    const res = await invoke('update_indexer_setting', {
      indexerSetting: indexerStore.indexerSetting,
    })
    console.log('update_indexer_setting result', res)
    window.$message.success(t('common.updateSuccess'))
  } catch (error) {
    console.error('update indexer setting error', error)
    window.$message.error(t('common.operationFailed'))
  }
  initStatusData()
}

function onModelPlatformSaved(updatedPlatform: ModelPlatform) {
  const index = modelPlatformList.value.findIndex(
    p => p.name === updatedPlatform.name,
  )
  if (index !== -1)
    modelPlatformList.value[index] = { ...updatedPlatform }
}

function onSelfHostedPlatformSaved(updatedPlatform: SelfHostedPlatform) {
  const index = selfHostedPlatformList.value.findIndex(
    p => p.name === updatedPlatform.name,
  )
  if (index !== -1)
    selfHostedPlatformList.value[index] = { ...updatedPlatform }
}

function openModelEditModal() {
  if (selfHostedVisionModel.value) {
    editingModel.value = { ...selfHostedVisionModel.value }
    editingModelOldName.value = selfHostedVisionModel.value.name
    showModelEditModal.value = true
  }
}

async function saveModel() {
  if (!editingModel.value)
    return
  try {
    let modelName = editingModel.value.name
    if (activeSelfHostedPlatform.value === 'ollama' && modelName && !modelName.includes(':')) {
      modelName = `${modelName}:latest`
      editingModel.value.name = modelName
    }
    await invoke('update_ai_model', {
      id: editingModel.value.id,
      name: modelName,
      title: editingModel.value.title,
      remark: editingModel.value.remark,
    })
    // Parser model changed → light notice: existing index & search unaffected,
    // new files use the new model, optional category re-index offered
    if (editingModelOldName.value && editingModelOldName.value !== modelName) {
      parserModelNotice.value = {
        category: 'image',
        oldName: editingModelOldName.value,
        newName: modelName,
      }
    }
    selfHostedVisionModel.value = { ...editingModel.value }
    showModelEditModal.value = false
    window.$message.success(t('common.saveSuccess'))
    initStatusData()
  } catch (error) {
    console.error('save model error', error)
    window.$message.error(t('common.operationFailed'))
  }
}

async function doReindexCategory() {
  if (!parserModelNotice.value || reindexingCategory.value)
    return
  reindexingCategory.value = true
  try {
    const started = await invoke<boolean>('reindex_category', {
      category: parserModelNotice.value.category,
    })
    if (started) {
      window.$message.success(t('indexer.reindexStarted'))
      parserModelNotice.value = null
    } else {
      window.$message.info(t('indexer.reindexNothing'))
    }
  } catch (e) {
    window.$message.error(typeof e === 'string' ? e : t('common.operationFailed'))
  } finally {
    reindexingCategory.value = false
  }
}

watch(() => appStore.locale, () => {
  initStatusData()
})

onMounted(async () => {
  console.log('IndexerSetting onMounted')
  try {
    const indexerSetting = await invoke<IndexerSetting>('load_indexer_setting')
    indexerStore.setIndexerSetting(indexerSetting)

    activePlatform.value = await invoke<string>('load_active_platform')
    modelPlatformList.value = await invoke<ModelPlatform[]>(
      'load_model_platforms',
    )

    // Load self-hosted platforms
    selfHostedPlatformList.value = await invoke<SelfHostedPlatform[]>(
      'load_self_hosted_platforms',
    )
    activeSelfHostedPlatform.value = await invoke<string>(
      'load_active_self_hosted_platform',
    )

    // Local backend detection runs once per settings-page open
    localBackends.value = await invoke<LocalBackendStatus>('check_local_backends')
    activeIdentity.value = await invoke<EmbeddingIdentity>('get_active_embedding_identity')
    await loadEmbeddingModel()

    const userDataPath = await invoke<string>('get_data_path')
    console.log('userDataPath', userDataPath)
    modelPath.value = await join(userDataPath, 'model')
    console.log('modelPath', modelPath.value)
    dbPath.value = await join(userDataPath, 'storage')
    initStatusData()

    // Listen for storage change events
    unlistenStorageChange = await listen<string>('content-storage-change-event', async (event) => {
      let parsed: any
      try {
        parsed = JSON.parse(event.payload)
      } catch {
        console.error('Failed to parse storage change event:', event.payload)
        return
      }
      const { event: eventType, data } = parsed
      if (eventType === 'progress' && data.current !== undefined && data.total !== undefined) {
        changingProgress.value = { current: data.current, total: data.total }
      } else if (eventType === 'complete' || eventType === 'error' || eventType === 'cancelled') {
        changingType.value = null
        if (eventType === 'complete')
          window.$message.success(t('indexer.changeComplete'))
        else if (eventType === 'error')
          window.$message.error(t('common.operationFailed'))
        else if (eventType === 'cancelled')
          window.$message.info(t('indexer.changeCancelled'))

        // Reload setting from backend to sync any revert on failure/cancel
        try {
          const setting = await invoke<IndexerSetting>('load_indexer_setting')
          indexerStore.setIndexerSetting(setting)
        } catch { /* ignore */ }
      }
    })
  } catch (e) {
    console.error('IndexerSetting onMounted error', e)
  }
})

// The settings page is KeepAlive'd: re-run detection on each visit so restored
// model files light the local radios up without an app restart
onActivated(async () => {
  try {
    localBackends.value = await invoke<LocalBackendStatus>('check_local_backends')
    activeIdentity.value = await invoke<EmbeddingIdentity>('get_active_embedding_identity')
    await loadEmbeddingModel()
    initStatusData()
  } catch { /* keep previous snapshot */ }
})

onUnmounted(() => {
  unlistenStorageChange?.()
})
</script>

<template>
  <div>
    <NCard :title="t('common.overview')" class="mb-4 p-0" size="small" :bordered="true" hoverable>
      <div>
        <NDataTable size="small" :columns="columns" :data="dataRef" :default-expanded-row-keys="['index-content']" />
      </div>
    </NCard>
    <NCard :title="t('indexer.parserSetting')" class="mb-4" size="small" :bordered="true" hoverable>
      <div class="flex flex-col">
        <div class="flex flex-col space-y-1 mb-2">
          <NRadioGroup :value="parserMode" @update:value="doParserModeChanged">
            <NSpace>
              <NRadio value="local">
                {{ t('indexer.localMode') }}
                <AppTag v-if="parserMode === 'local'" type="success" size="small" class="ml-1">
                  {{ t('indexer.fullyPrivate') }}
                </AppTag>
              </NRadio>
              <NRadio value="selfhosted">
                {{ t('indexer.selfHostedMode') }}
              </NRadio>
              <NRadio value="remote">
                {{ t('indexer.cloudMode') }}
              </NRadio>
              <NRadio v-if="parserMode === 'mixed'" value="mixed" disabled>
                {{ t('indexer.mixedMode') }}
              </NRadio>
            </NSpace>
          </NRadioGroup>
          <NAlert :show-icon="false" class="text-xs" size="small">
            <div v-if="parserMode === 'local'">
              {{ t('indexer.localModeDesc') }}
            </div>
            <div v-if="parserMode === 'selfhosted'">
              {{
                t('indexer.selfHostedModeDescDynamic', {
                  platform: selfHostedPlatformList.find((p) => p.name === activeSelfHostedPlatform)?.title,
                  model: selfHostedVisionModel?.title || selfHostedVisionModel?.name || 'LLaVA',
                })
              }}
            </div>
            <div v-if="parserMode === 'selfhosted'" class="mt-1">
              <NButton text type="primary" size="tiny" class="text-link" @click="showSelfHostedModal = true">
                {{ t('indexer.selfHostedSetting') }}
              </NButton>
            </div>
            <div v-if="parserMode === 'remote'">
              {{
                t('indexer.cloudModeDescDynamic', {
                  modelPlatform: modelPlatformList.find(
                    (p) => p.name === activePlatform,
                  )?.title,
                })
              }}
            </div>
            <div v-if="parserMode === 'remote'" class="mt-1">
              <NButton text type="primary" size="tiny" class="text-link" @click="showCloudModal = true">
                {{ t('indexer.cloudModeSetting') }}
              </NButton>
            </div>
            <div v-if="parserMode === 'mixed'">
              {{ t('indexer.mixedModeDesc') }}
            </div>
          </NAlert>
        </div>
        <NTable size="small" class="mb-2">
          <tbody class="text-xs">
            <tr>
              <td class="w-32">
                {{ t('indexer.documentParser') }}
              </td>
              <td>{{ t('common.local') }}</td>
            </tr>
            <tr>
              <td>{{ t('indexer.imageParser') }}</td>
              <td>
                <NRadioGroup
                  :value="indexerStore.indexerSetting.image_parser_mode"
                  size="small"
                  @update:value="onImageParserModeChanged"
                >
                  <NTooltip>
                    <template #trigger>
                      <NRadio value="local" :disabled="!visionAvailable()">
                        {{ t('common.local') }}
                      </NRadio>
                    </template>
                    <template v-if="!visionAvailable()">
                      {{ missingFilesTooltip(localBackends?.vision_missing || []) }}
                    </template>
                  </NTooltip>
                  <NRadio value="selfhosted">
                    {{ t('common.selfHosted') }}
                  </NRadio>
                  <NRadio value="remote">
                    {{ t('common.cloud') }}
                  </NRadio>
                </NRadioGroup>
              </td>
            </tr>
            <tr>
              <td>{{ t('indexer.audioParser') }}</td>
              <td>
                <NRadioGroup
                  :value="indexerStore.indexerSetting.audio_parser_mode"
                  size="small"
                  @update:value="onAudioParserModeChanged"
                >
                  <NTooltip>
                    <template #trigger>
                      <NRadio value="local" :disabled="!audioAvailable()">
                        {{ t('common.local') }}
                      </NRadio>
                    </template>
                    <template v-if="!audioAvailable()">
                      {{ missingFilesTooltip(localBackends?.audio_missing || []) }}
                    </template>
                  </NTooltip>
                  <NTooltip>
                    <template #trigger>
                      <NRadio value="selfhosted" :disabled="!selfHostedAudioAvailable()">
                        {{ t('common.selfHosted') }}
                      </NRadio>
                    </template>
                    <template v-if="!selfHostedAudioAvailable()">
                      {{ t('common.selfHostedASRNeedVllm') }}
                    </template>
                  </NTooltip>
                  <NRadio value="remote">
                    {{ t('common.cloud') }}
                  </NRadio>
                </NRadioGroup>
              </td>
            </tr>
            <tr>
              <td>{{ t('indexer.vectorParser') }}</td>
              <td>
                <div class="flex items-center gap-2">
                  <NRadioGroup
                    :value="indexerStore.indexerSetting.embedding_parser_mode"
                    size="small"
                    @update:value="onEmbeddingModeChanged"
                  >
                    <NTooltip>
                      <template #trigger>
                        <NRadio value="local" :disabled="!vectorAvailable()">
                          {{ t('common.local') }}
                        </NRadio>
                      </template>
                      <template v-if="!vectorAvailable()">
                        {{ missingFilesTooltip(localBackends?.vector_missing || []) }}
                      </template>
                    </NTooltip>
                    <NRadio value="selfhosted">
                      {{ t('common.selfHosted') }}
                    </NRadio>
                    <NRadio value="remote">
                      {{ t('common.cloud') }}
                    </NRadio>
                  </NRadioGroup>
                  <span
                    v-if="activeEmbeddingModel && activeIdentity && activeIdentity.platform === (indexerStore.indexerSetting.embedding_parser_mode === 'selfhosted' ? activeSelfHostedPlatform : activePlatform)"
                    class="text-xs text-gray-400"
                  >{{ activeEmbeddingModel.title || activeEmbeddingModel.name }} · {{ activeIdentity.dim }}</span>
                </div>
              </td>
            </tr>
          </tbody>
        </NTable>

        <!-- Parser model change: light notice + optional category re-index -->
        <NAlert v-if="parserModelNotice" type="info" :show-icon="false" class="text-xs mb-2" closable @close="parserModelNotice = null">
          <div>
            {{
              t('indexer.parserModelChanged', {
                category: t('indexer.imageParser'),
                oldName: parserModelNotice.oldName,
                newName: parserModelNotice.newName,
              })
            }}
          </div>
          <div class="mt-1">
            <NButton text type="primary" size="tiny" class="text-link" :loading="reindexingCategory" @click="doReindexCategory">
              {{ t('indexer.reindexCategory') }}
            </NButton>
          </div>
        </NAlert>

        <!-- Self-hosted platform modal -->
        <NModal v-model:show="showSelfHostedModal" preset="card" :title="t('indexer.selfHostedSetting')" style="width: 600px">
          <NScrollbar style="max-height: 70vh">
            <NFormItem :label="t('indexer.selectActivePlatform')">
              <NRadioGroup :value="activeSelfHostedPlatform" @update:value="doActiveSelfHostedPlatformChanged">
                <NRadio
                  v-for="platform in selfHostedPlatformList" :key="platform.id" :label="platform.title"
                  :value="platform.name"
                />
              </NRadioGroup>
            </NFormItem>
            <NCard size="small" :bordered="true">
              <NFormItem :label="t('common.visionModel')">
                <span class="selectable mr-4">{{ selfHostedVisionModel?.title || selfHostedVisionModel?.name || '-' }}</span>
                <NButton text type="primary" size="tiny" class="text-link ml-2" @click="openModelEditModal">
                  {{ t('indexer.editModel') }}
                </NButton>
              </NFormItem>
              <NFormItem :label="t('indexer.embeddingModel')">
                <span class="selectable mr-4">{{ activeEmbeddingModel?.title || activeEmbeddingModel?.name || '-' }}</span>
              </NFormItem>
              <NFormItem :label="`${selfHostedPlatformList.find(p => p.name === activeSelfHostedPlatform)?.title} ${t('common.setting')}`">
                <SelfHostedPlatformEdit
                  v-for="platform in selfHostedPlatformList"
                  v-show="platform.name === activeSelfHostedPlatform"
                  :key="platform.name"
                  :platform="platform"
                  @saved="onSelfHostedPlatformSaved"
                />
              </NFormItem>
            </NCard>
          </NScrollbar>
        </NModal>

        <!-- Cloud platform modal -->
        <NModal v-model:show="showCloudModal" preset="card" :title="t('indexer.cloudModeSetting')" style="width: 600px">
          <NScrollbar style="max-height: 70vh">
            <NFormItem :label="t('model.selectForActivePlatform')">
              <NRadioGroup :value="activePlatform" @update:value="doActivePlatformChanged">
                <NRadio
                  v-for="platform in modelPlatformList" :key="platform.id" :label="platform.title"
                  :value="platform.name"
                />
              </NRadioGroup>
            </NFormItem>
            <NCard size="small" :bordered="true">
              <div>
                <ModelPlatformEdit
                  v-for="platform in modelPlatformList"
                  v-show="platform.name === activePlatform"
                  :key="platform.name"
                  :model-platform="platform"
                  @saved="onModelPlatformSaved"
                />
              </div>
            </NCard>
          </NScrollbar>
        </NModal>

        <!-- Embedding backend switch confirmation -->
        <NModal
          :show="showSwitchDialog"
          preset="card"
          :title="t('indexer.switchTitle')"
          style="width: 560px"
          :mask-closable="!switchingBackend"
          @update:show="(v: boolean) => { if (!switchingBackend) showSwitchDialog = v }"
        >
          <div v-if="pendingSwitch" class="text-xs space-y-2">
            <div>
              {{
                t('indexer.switchTarget', {
                  model: pendingSwitch.modelTitle,
                  backend: identityBackendTitle(pendingSwitch.platform),
                })
              }}
            </div>
            <NAlert v-if="pendingSwitch.existingCollection" type="success" :show-icon="false">
              {{ t('indexer.switchExistingInstant') }}
            </NAlert>
            <NAlert v-else type="warning" :show-icon="false">
              {{
                t('indexer.switchNewRebuild', {
                  count: pendingSwitch.pendingFiles,
                })
              }}
            </NAlert>
            <NFormItem :label="t('indexer.switchRebuildLabel')" label-placement="left">
              <NRadioGroup v-model:value="switchRebuildChoice" size="small">
                <NRadio value="fast" :disabled="!switchFastAvailable">
                  {{ t('indexer.switchRebuildFast') }}
                </NRadio>
                <NRadio value="full">
                  {{ t('indexer.switchRebuildFull') }}
                </NRadio>
                <NRadio value="none">
                  {{ t('indexer.switchRebuildNone') }}
                </NRadio>
              </NRadioGroup>
            </NFormItem>
            <div v-if="switchRebuildChoice === 'fast'" class="text-xs text-gray-400">
              {{ t('indexer.switchFastHint') }}
            </div>
            <NCheckbox v-model:checked="switchClearOthers">
              {{ t('indexer.switchClearOthers') }}
            </NCheckbox>
            <div v-if="switchClearOthers" class="text-xs text-gray-400">
              {{
                pendingSwitch.platform === 'local'
                  ? t('indexer.switchClearCostLocal')
                  : t('indexer.switchClearCostRemote')
              }}
            </div>
          </div>
          <template #footer>
            <NSpace justify="end">
              <NButton ghost :disabled="switchingBackend" @click="showSwitchDialog = false">
                {{ t('common.cancel') }}
              </NButton>
              <NButton type="primary" ghost :loading="switchingBackend" @click="doConfirmSwitch">
                {{ t('common.confirm') }}
              </NButton>
            </NSpace>
          </template>
        </NModal>
      </div>
    </NCard>
    <NCard :title="t('common.storage')" class="mb-4 px-0" size="small" :bordered="true" hoverable>
      <div class="flex flex-col">
        <NAlert :show-icon="false">
          <div class="whitespace-pre-line">
            {{ t('indexer.contentStorageDesc') }}
          </div>
        </NAlert>
        <div class="flex flex-col space-y-3 my-4">
          <div>
            <div>{{ t('indexer.documentStorage') }}</div>
            <NRadioGroup
              size="small"
              :value="indexerStore.indexerSetting.content_storage.document || 'database'"
              @update:value="(v: string) => onContentStorageChanged('document', v)"
            >
              <NSpace>
                <NRadio value="database">
                  {{ t('indexer.storageDatabase') }}
                </NRadio>
                <NRadio value="file">
                  {{ t('indexer.storageFile') }}
                </NRadio>
                <NRadio value="none">
                  {{ t('indexer.storageNone') }}
                </NRadio>
              </NSpace>
            </NRadioGroup>
            <div class="text-xs text-gray-400 mt-1">
              {{ indexerStore.indexerSetting.content_storage.document === 'database' ? t('indexer.storageDatabaseDesc') : indexerStore.indexerSetting.content_storage.document === 'file' ? t('indexer.storageFileDesc') : indexerStore.indexerSetting.content_storage.document === 'none' ? t('indexer.storageNoneDesc') : '' }}
            </div>
          </div>
          <div>
            <div>{{ t('indexer.imageStorage') }}</div>
            <NRadioGroup
              size="small"
              :value="indexerStore.indexerSetting.content_storage.image || 'database'"
              @update:value="(v: string) => onContentStorageChanged('image', v)"
            >
              <NSpace>
                <NRadio value="database">
                  {{ t('indexer.storageDatabase') }}
                </NRadio>
                <NRadio value="file">
                  {{ t('indexer.storageFile') }}
                </NRadio>
              </NSpace>
            </NRadioGroup>
            <div class="text-xs text-gray-400 mt-1">
              {{ indexerStore.indexerSetting.content_storage.image === 'database' ? t('indexer.storageDatabaseDesc') : t('indexer.storageFileDesc') }}
            </div>
          </div>
          <div>
            <div>{{ t('indexer.audioStorage') }}</div>
            <NRadioGroup
              size="small"
              :value="indexerStore.indexerSetting.content_storage.audio || 'database'"
              @update:value="(v: string) => onContentStorageChanged('audio', v)"
            >
              <NSpace>
                <NRadio value="database">
                  {{ t('indexer.storageDatabase') }}
                </NRadio>
                <NRadio value="file">
                  {{ t('indexer.storageFile') }}
                </NRadio>
              </NSpace>
            </NRadioGroup>
            <div class="text-xs text-gray-400 mt-1">
              {{ indexerStore.indexerSetting.content_storage.audio === 'database' ? t('indexer.storageDatabaseDesc') : t('indexer.storageFileDesc') }}
            </div>
          </div>
        </div>
        <!-- Storage change progress -->
        <div v-if="changingType" class="flex items-center gap-2 text-xs text-gray-500">
          <NSpin size="small" />
          <span>{{ t('indexer.changing') }} ({{ changingProgress.current }}/{{ changingProgress.total }}<template v-if="changingProgress.total > 0">, {{ Math.round(changingProgress.current / changingProgress.total * 100) }}%</template>)</span>
        </div>
      </div>
    </NCard>

    <!-- Model Edit Modal -->
    <NModal v-model:show="showModelEditModal" preset="card" :title="t('indexer.editModel')" style="width: 500px">
      <NForm v-if="editingModel" ref="modelEditFormRef" label-placement="left" label-width="80">
        <NFormItem :label="t('common.name')" :rule="{ required: true, message: t('common.nameRequired') }">
          <NInput v-model:value="editingModel.name" />
          <template #feedback>
            <span class="text-xs text-gray-500">{{ t('indexer.modelNameHint', { platform: selfHostedPlatformList.find((p) => p.name === activeSelfHostedPlatform)?.title || 'Ollama' }) }}</span>
          </template>
        </NFormItem>
        <NFormItem :label="t('common.title')" :rule="{ required: true, message: t('common.titleRequired') }">
          <NInput v-model:value="editingModel.title" />
        </NFormItem>
        <NFormItem :label="t('common.description')">
          <NInput v-model:value="editingModel.remark" type="textarea" :rows="3" />
        </NFormItem>
      </NForm>
      <template #footer>
        <NSpace justify="end">
          <NButton ghost @click="showModelEditModal = false">
            {{ t('common.cancel') }}
          </NButton>
          <NButton type="primary" ghost :disabled="!editingModel?.name || !editingModel?.title" @click="saveModel">
            {{ t('common.save') }}
          </NButton>
        </NSpace>
      </template>
    </NModal>
  </div>
</template>
