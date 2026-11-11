<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useRouter } from 'vue-router'
import { join } from '@tauri-apps/api/path'
import type { DataTableColumns } from 'naive-ui'
import ModelPlatformEdit from './ModelPlatformEdit.vue'
import SelfHostedPlatformEdit from './SelfHostedPlatformEdit.vue'
import ProxyEdit from './ProxyEdit.vue'
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
type FileParserMode = 'local' | 'selfhosted' | 'remote'

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

// Active embedding identity: { platform, model, dim, has_sparse }; null AFTER
// load = no vector library configured yet (fresh lite install)
const activeIdentity = ref<EmbeddingIdentity | null>(null)
// Distinguishes "identity not loaded yet" from "unconfigured" (both null)
const identityLoaded = ref(false)
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
  mode: FileParserMode
  platform: string
  model: string
  modelTitle: string
  existingCollection: boolean | null // null = unknown yet (probed at confirm)
  pendingFiles: number
}
const pendingSwitch = ref<PendingSwitch | null>(null)
/** switch_embedding_backend result */
interface SwitchResult {
  identity: EmbeddingIdentity
  existing_collection: boolean
  rebuild_started: boolean
  service_unreachable: boolean
  indexing_aborted: boolean
}

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

/** Mode label for the overview/status table */
function modeLabel(mode: string) {
  if (mode === 'selfhosted')
    return t('common.selfHosted')
  if (mode === 'remote')
    return t('common.cloud')
  return t('common.local')
}

/** Overview cell for one parser category: the platform and model that serve
 * its own mode, falling back to the mode label when neither is available */
async function resolveCategoryBackend(mode: string, oneType: 'vision' | 'asr') {
  if (mode !== 'selfhosted' && mode !== 'remote')
    return { desc: modeLabel(mode), model: null }
  const selfHosted = mode === 'selfhosted'
  const platform = selfHosted ? activeSelfHostedPlatform.value : activePlatform.value
  const platformInfo = (selfHosted ? selfHostedPlatformList.value : modelPlatformList.value)
    .find(p => p.name === platform)
  if (!platformInfo)
    return { desc: modeLabel(mode), model: null }
  let model: AiModel | null = null
  try {
    model = await invoke<AiModel>('load_model_by_type', { platform, oneType })
  } catch (e) {
    console.warn('Failed to load parser model:', e)
  }
  const modelName = model?.title || model?.name
  return {
    desc: modelName ? `${platformInfo.title} => ${modelName}` : modeLabel(mode),
    model,
  }
}

/** Preset label from all three category modes (vector included) */
function computeParserMode(imageMode: string, audioMode: string, vectorMode: string): ParserMode {
  if (imageMode === 'local' && audioMode === 'local' && vectorMode === 'local')
    return 'local'
  // Self-hosted preset exempts audio: the default tool Ollama has no ASR
  // endpoint, so audio legitimately sits on cloud (or local in Full) while
  // image/vector stay self-hosted
  if (imageMode === 'selfhosted' && vectorMode === 'selfhosted')
    return 'selfhosted'
  if (imageMode === 'remote' && audioMode === 'remote' && vectorMode === 'remote')
    return 'remote'
  return 'mixed'
}

/** Mirror of the backend dispatcher's identity/mode consistency check */
function identityMatchesMode(identity: EmbeddingIdentity, mode: string) {
  if (mode === 'local')
    return identity.platform === 'local'
  if (mode === 'selfhosted')
    return identity.platform !== 'local' && identity.platform === activeSelfHostedPlatform.value
  return identity.platform !== 'local' && identity.platform !== activeSelfHostedPlatform.value
}

/** True while no vector library is configured yet (fresh lite install), or the
 * active one lags behind the selected mode — the switch flow hasn't run.
 * Suppressed while a switch is in flight: the lag is being resolved right
 * now, a transient 当前生效 line (with its retry link) would only blink and
 * a second click would collide with the backend's storage-task lock */
const switchInFlight = ref(false)

const switchPending = computed(() =>
  identityLoaded.value
  && !switchInFlight.value
  && (activeIdentity.value === null
    || !identityMatchesMode(activeIdentity.value, indexerStore.indexerSetting.embedding_parser_mode)))

/** Load the active identity; null after load = unconfigured (fresh lite) */
async function reloadActiveIdentity() {
  activeIdentity.value = await invoke<EmbeddingIdentity | null>('get_active_embedding_identity')
  identityLoaded.value = true
}

/** True while the active vector collection is not the fully indexed one —
 * semantic search stays refused until a (re)build completes. rebuildRunning
 * splits the notice: an indexing task is regenerating vs. waiting for one */
const semanticRebuildPending = ref(false)
const rebuildRunning = ref(false)

async function reloadSemanticPending() {
  try {
    semanticRebuildPending.value = await invoke<boolean>('embedding_rebuild_pending')
    const task = await invoke<{ task_type: string } | null>('get_storage_task')
    rebuildRunning.value = semanticRebuildPending.value && task?.task_type === 'indexing'
  } catch { /* keep previous */ }
}

const router = useRouter()

/** Hand over to the workspace page, where 全量重建 lives next to the file
 * list and the user picks the force option */
function goRebuild() {
  router.push({ name: 'Data' })
}

/** Unconfigured + cloud mode without an API key: the switch cannot succeed
 * yet, guide to the settings modal first (self-hosted seeds count as ready) */
const vectorBackendReady = computed(() => {
  if (indexerStore.indexerSetting.embedding_parser_mode !== 'remote')
    return true
  const platform = modelPlatformList.value.find(p => p.name === activePlatform.value)
  return !!platform && platform.api_key !== ''
})

type ParserCategory = 'image' | 'audio' | 'vector'

const CATEGORY_MODEL_TYPE: Record<ParserCategory, string> = {
  image: 'vision',
  audio: 'asr',
  vector: 'embedding',
}
const CATEGORY_LABEL_KEY: Record<ParserCategory, string> = {
  image: 'indexer.diagCategoryImage',
  audio: 'indexer.diagCategoryAudio',
  vector: 'indexer.diagCategoryVector',
}

interface PlatformIssueAction {
  label: string
  run: () => void
}

/** A problem keeping a used platform from serving its categories, listed
 * under the parser preset radios with the entry that fixes it */
interface PlatformIssue {
  text: string
  /** Platform the issue belongs to (hides the page banner duplicating it) */
  platform: string
  categories: ParserCategory[]
  action?: PlatformIssueAction
}

const platformIssues = ref<PlatformIssue[]>([])
let diagnosing = false
/** Refresh requested while a probe was running; re-runs once it finishes */
let diagnoseQueued = false
/** Run counter: a superseded run's late probe results are discarded */
let diagnoseRun = 0

/** "图片解析、音频解析及向量模型" / "image parsing, audio parsing and the embedding model" */
function categoryList(categories: ParserCategory[]) {
  const labels = categories.map(c => t(CATEGORY_LABEL_KEY[c]))
  if (labels.length < 2)
    return labels.join('')
  const last = labels.pop()!
  // Separator follows the rendered language, not a store flag that startup
  // locale loading never writes back
  return /[\u4E00-\u9FFF]/.test(last)
    ? `${labels.join('、')}及${last}`
    : `${labels.join(', ')} and ${last}`
}

async function loadCategoryModel(platform: string, category: ParserCategory) {
  try {
    return await invoke<AiModel>('load_model_by_type', {
      platform,
      oneType: CATEGORY_MODEL_TYPE[category],
    })
  } catch (e) {
    console.warn('Failed to load parser model:', e)
    return null
  }
}

/** Ollama reports tagged ids (`bge-m3:latest`) for a bare configured name */
function servedIncludes(served: string[], name: string) {
  const bare = (id: string) => id.replace(/:latest$/, '')
  return served.some(id => id === name || bare(id) === bare(name))
}

/** Parser mode per category — the input of one diagnostics run */
type CategoryModes = Record<ParserCategory, string>

/** Availability that needs no network round trip: local model files, the
 * configured model of each category, an API key on the cloud platform.
 * Published before any probe so a mode switch shows its offline problems at
 * once instead of waiting on a backend timeout. */
async function instantPlatformIssues(modes: CategoryModes) {
  const issues: PlatformIssue[] = []
  const localMissing: ParserCategory[] = []
  const selfHostedCats: ParserCategory[] = []
  const cloudCats: ParserCategory[] = []
  for (const cat of ['image', 'audio', 'vector'] as ParserCategory[]) {
    const mode = modes[cat]
    if (mode === 'local') {
      const missingKey = cat === 'image' ? 'vision_missing' : `${cat}_missing`
      if ((localBackends.value?.[missingKey as keyof LocalBackendStatus] || []).length > 0)
        localMissing.push(cat)
    } else if (mode === 'selfhosted') {
      selfHostedCats.push(cat)
    } else {
      cloudCats.push(cat)
    }
  }

  if (localMissing.length > 0) {
    const files = localMissing.flatMap((cat) => {
      const missingKey = cat === 'image' ? 'vision_missing' : `${cat}_missing`
      return localBackends.value?.[missingKey as keyof LocalBackendStatus] || []
    })
    issues.push({
      platform: 'local',
      categories: localMissing,
      text: t('indexer.diagLocalFilesMissing', {
        files: files.join(', '),
        categories: categoryList(localMissing),
      }),
    })
  }

  const selfHostedPlatform = selfHostedPlatformList.value.find(p => p.name === activeSelfHostedPlatform.value)
  const selfHostedAction = { label: t('indexer.diagGoSetting'), run: openSelfHostedSettings }
  if (selfHostedCats.length > 0) {
    if (!selfHostedPlatform) {
      // No active platform selected and the configured name no longer exists
      // are different problems; only the latter reads as unreachable
      const noPlatform = activeSelfHostedPlatform.value === ''
      issues.push({
        platform: activeSelfHostedPlatform.value,
        categories: selfHostedCats,
        text: noPlatform
          ? t('indexer.diagNoSelfHosted', { categories: categoryList(selfHostedCats) })
          : t('indexer.diagUnreachable', {
            platform: activeSelfHostedPlatform.value,
            categories: categoryList(selfHostedCats),
          }),
        action: selfHostedAction,
      })
    } else {
      const noModel: ParserCategory[] = []
      for (const cat of selfHostedCats) {
        const model = await loadCategoryModel(selfHostedPlatform.name, cat)
        if (!model || !model.name)
          noModel.push(cat)
      }
      if (noModel.length > 0) {
        issues.push({
          platform: selfHostedPlatform.name,
          categories: noModel,
          text: t('indexer.diagNoModel', {
            platform: selfHostedPlatform.title,
            categories: categoryList(noModel),
          }),
          action: selfHostedAction,
        })
      }
    }
  }

  const cloudPlatform = modelPlatformList.value.find(p => p.name === activePlatform.value)
  const cloudAction = { label: t('indexer.diagGoSetting'), run: openCloudSettings }
  if (cloudCats.length > 0) {
    if (!cloudPlatform || cloudPlatform.api_key === '') {
      issues.push({
        platform: cloudPlatform?.name || activePlatform.value,
        categories: cloudCats,
        text: t('indexer.diagNoKey', {
          platform: cloudPlatform?.title || activePlatform.value,
          categories: categoryList(cloudCats),
        }),
        action: cloudAction,
      })
    } else {
      const noModel: ParserCategory[] = []
      for (const cat of cloudCats) {
        const model = await loadCategoryModel(cloudPlatform.name, cat)
        if (!model || !model.name)
          noModel.push(cat)
      }
      if (noModel.length > 0) {
        issues.push({
          platform: cloudPlatform.name,
          categories: noModel,
          text: t('indexer.diagNoModel', {
            platform: cloudPlatform.title,
            categories: categoryList(noModel),
          }),
          action: cloudAction,
        })
      }
    }
  }

  return { issues, selfHostedPlatform, selfHostedCats, cloudPlatform, cloudCats }
}

/** Probe every platform the selected parser modes actually use — reachability
 * and per-category model presence — and list each failure with the fix entry */
async function refreshPlatformIssues(modesOverride?: CategoryModes) {
  if (!localBackends.value)
    return
  const modes = modesOverride ?? currentModes()
  if (diagnosing) {
    // Probe in flight: publish the offline verdict for the new modes right
    // away and let the full re-run follow once the current probe settles
    diagnoseQueued = true
    const run = ++diagnoseRun
    instantPlatformIssues(modes).then(({ issues }) => {
      if (run === diagnoseRun)
        platformIssues.value = [...issues]
    }).catch(e => console.warn('platform diagnostics failed', e))
    return
  }
  diagnosing = true
  const run = ++diagnoseRun
  try {
    // Offline results land first: the list follows the click, not the network
    const { issues, selfHostedPlatform, selfHostedCats, cloudPlatform, cloudCats }
      = await instantPlatformIssues(modes)
    const publish = () => {
      if (run === diagnoseRun)
        platformIssues.value = [...issues]
    }
    publish()

    // Both probes run together: a slow self-hosted timeout never holds back
    // the cloud verdict, each one appends as soon as it settles
    const probes: Promise<void>[] = []

    // One probe answers both reachability and which models it serves
    if (selfHostedPlatform && selfHostedCats.length > 0) {
      probes.push((async () => {
        const action = { label: t('indexer.diagGoSetting'), run: openSelfHostedSettings }
        let served: string[] | null = null
        try {
          served = await invoke<string[]>('list_self_hosted_models', { platform: selfHostedPlatform })
        } catch (e) {
          console.warn('self-hosted platform probe failed', e)
        }
        if (served === null) {
          issues.push({
            platform: selfHostedPlatform.name,
            categories: selfHostedCats,
            text: t('indexer.diagUnreachable', {
              platform: selfHostedPlatform.title,
              categories: categoryList(selfHostedCats),
            }),
            action,
          })
        } else {
          const notServed = new Map<string, ParserCategory[]>()
          for (const cat of selfHostedCats) {
            const model = await loadCategoryModel(selfHostedPlatform.name, cat)
            if (!model || !model.name || servedIncludes(served, model.name))
              continue
            const label = model.title || model.name
            notServed.set(label, [...(notServed.get(label) || []), cat])
          }
          for (const [model, cats] of notServed) {
            issues.push({
              platform: selfHostedPlatform.name,
              categories: cats,
              text: t('indexer.diagModelMissing', {
                platform: selfHostedPlatform.title,
                model,
                categories: categoryList(cats),
              }),
              action,
            })
          }
        }
        publish()
      })())
    }

    if (cloudPlatform && cloudPlatform.api_key !== '' && cloudCats.length > 0) {
      probes.push((async () => {
        const action = { label: t('indexer.diagGoSetting'), run: openCloudSettings }
        // Distinguish a rejected key from an unreachable host: the fix differs
        let problem: 'unreachable' | 'unauthorized' | null = null
        try {
          await invoke('check_model_platform', { platform: cloudPlatform })
        } catch (e) {
          const detail = String(e)
          problem = /401|403|API key/i.test(detail) ? 'unauthorized' : 'unreachable'
          console.warn('cloud platform probe failed', detail)
        }
        if (problem === 'unauthorized') {
          issues.push({
            platform: cloudPlatform.name,
            categories: cloudCats,
            text: t('indexer.diagKeyRejected', {
              platform: cloudPlatform.title,
              categories: categoryList(cloudCats),
            }),
            action,
          })
        } else if (problem === 'unreachable') {
          issues.push({
            platform: cloudPlatform.name,
            categories: cloudCats,
            text: t('indexer.diagUnreachable', {
              platform: cloudPlatform.title,
              categories: categoryList(cloudCats),
            }),
            action,
          })
        }
        publish()
      })())
    }

    await Promise.all(probes)
  } catch (e) {
    // Diagnostics must never break the page
    console.warn('platform diagnostics failed', e)
  } finally {
    diagnosing = false
    if (diagnoseQueued) {
      diagnoseQueued = false
      refreshPlatformIssues()
    }
  }
}

function currentModes(): CategoryModes {
  const setting = indexerStore.indexerSetting
  return {
    image: setting.image_parser_mode,
    audio: setting.audio_parser_mode,
    vector: setting.embedding_parser_mode,
  }
}

/** Re-run diagnostics against the stored modes (optimistic writes land here
 * before the save round trip, so the list follows the click) */
function recheckParsers() {
  refreshPlatformIssues(currentModes())
}

/** Categories currently listed as broken (drives redundant notices stepping aside) */
const audioIssueShown = computed(() => platformIssues.value.some(i => i.categories.includes('audio')))

/** Open the settings modal matching the current vector mode */
function openVectorConfig() {
  if (indexerStore.indexerSetting.embedding_parser_mode === 'selfhosted')
    showSelfHostedModal.value = true
  else
    showCloudModal.value = true
}

function openSelfHostedSettings() {
  showSelfHostedModal.value = true
}

function openCloudSettings() {
  showCloudModal.value = true
}

/** First activation: once credentials exist the switch completes itself —
 * an empty library needs no rebuild, so no dialog interrupts the flow.
 * With indexed files present the rebuild choice stays with the user. */
const autoActivating = ref(false)

async function autoActivateVectorBackend() {
  if (autoActivating.value || switchInFlight.value || !identityLoaded.value || activeIdentity.value !== null || !vectorBackendReady.value)
    return
  autoActivating.value = true
  try {
    let pendingFiles = 0
    try {
      pendingFiles = await invoke<number>('count_indexed_files', { category: 'document' })
        + await invoke<number>('count_indexed_files', { category: 'image' })
        + await invoke<number>('count_indexed_files', { category: 'audio' })
    } catch { /* estimate only */ }
    if (pendingFiles > 0)
      return

    const mode = indexerStore.indexerSetting.embedding_parser_mode as FileParserMode
    const resolved = await resolveEmbeddingTarget(mode)
    if (!resolved)
      return
    await switchBackendDirectly(mode, resolved)
  } catch (e) {
    console.error('auto activate vector backend failed', e)
    window.$message.error(typeof e === 'string' ? e : t('common.operationFailed'))
  } finally {
    autoActivating.value = false
  }
}

watch(() => identityLoaded.value && activeIdentity.value === null && vectorBackendReady.value, (ready) => {
  if (ready)
    autoActivateVectorBackend()
})

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
  // Detect parser mode from image, audio and vector modes
  const setting = indexerStore.indexerSetting
  const imageMode = setting.image_parser_mode
  const audioMode = setting.audio_parser_mode

  parserMode.value = computeParserMode(imageMode, audioMode, setting.embedding_parser_mode)

  // Category rows are resolved independently: a self-hosted preset leaves
  // audio on cloud when the self-hosted platform has no ASR endpoint
  const imageBackend = await resolveCategoryBackend(imageMode, 'vision')
  imageParserDesc.value = imageBackend.desc
  if (imageMode === 'selfhosted')
    selfHostedVisionModel.value = imageBackend.model
  const audioBackend = await resolveCategoryBackend(audioMode, 'asr')
  audioParserDesc.value = audioBackend.desc
  vectorParserDesc.value = modeLabel(setting.embedding_parser_mode)

  // Vector backend: display as "backend => model" from the active identity
  // (same shape as the image/audio rows); when the identity lags behind the
  // mode (fresh lite cloud default before the first switch), show the mode
  // label instead of a stale backend name
  if (activeIdentity.value && identityMatchesMode(activeIdentity.value, setting.embedding_parser_mode))
    vectorParserDesc.value = `${identityBackendTitle(activeIdentity.value.platform)} => ${activeIdentity.value.model}`

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

  // Probes run in the background: the table never waits on a slow platform
  refreshPlatformIssues()
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

// Listen for indexing events (the storage tab owns the content-storage listener)
let unlistenIndexing: (() => void) | null = null
let unlistenWatcherIndexing: (() => void) | null = null

async function doParserModeChanged(mode: ParserMode) {
  // Mixed mode cannot be set by user directly; a switch already resolving a
  // previous choice swallows the click instead of interleaving states
  if (mode === 'mixed' || switchInFlight.value)
    return

  if (mode === 'local') {
    // 本地模式：三类目全部 local；本地模型文件缺失的类目回落 remote
    indexerStore.indexerSetting.image_parser_mode = visionAvailable() ? 'local' : 'remote'
    indexerStore.indexerSetting.audio_parser_mode = audioAvailable() ? 'local' : 'remote'
  } else if (mode === 'selfhosted') {
    // 自托管模式：保留当前自托管工具（通常是 Ollama，不强切 vLLM）；
    // Ollama 无转写端点，音频回落云端（Lite 下本地亦不可选），
    // 活动平台为 vLLM 时音频同样可选自托管
    indexerStore.indexerSetting.image_parser_mode = 'selfhosted'
    indexerStore.indexerSetting.audio_parser_mode = selfHostedAudioAvailable() ? 'selfhosted' : 'remote'
  } else if (mode === 'remote') {
    // 云端模式：三类目全部 remote
    indexerStore.indexerSetting.image_parser_mode = 'remote'
    indexerStore.indexerSetting.audio_parser_mode = 'remote'
  }

  // 向量模式与预设同步先行落定：单选立即选中，服务探测随后在后台进行
  // （本地向量文件缺失时不跟随，与 onEmbeddingModeChanged 的守卫一致）
  if (mode !== 'local' || vectorAvailable())
    indexerStore.indexerSetting.embedding_parser_mode = mode

  updateParserModeFromSettings()
  // Availability of the newly selected modes, before the save round trip
  recheckParsers()
  // The flag covers the whole resolve→switch window from the optimistic write
  // on, so the transient pending line never flashes between the two
  switchInFlight.value = true
  try {
    await updateIndexerSetting(true)

    // 向量模型跟随预设，自动模式：同后端直接落库，换后端静默切换，不弹
    // 对话框与 toast——结果由页面自身呈现（向量行即时变化，不可达/重建
    // 中由顶部持久横幅告警）
    await onEmbeddingModeChanged(mode, true)
  } finally {
    switchInFlight.value = false
  }
}

// Handle image parser mode change
function onImageParserModeChanged(mode: string) {
  if (mode === 'local' && !visionAvailable()) {
    window.$message.warning(missingFilesTooltip(localBackends.value?.vision_missing || []))
    return
  }
  indexerStore.indexerSetting.image_parser_mode = mode
  updateParserModeFromSettings()
  recheckParsers()
  updateIndexerSetting(true)
}

// Handle audio parser mode change
function onAudioParserModeChanged(mode: string) {
  if (mode === 'local' && !audioAvailable()) {
    window.$message.warning(missingFilesTooltip(localBackends.value?.audio_missing || []))
    return
  }
  indexerStore.indexerSetting.audio_parser_mode = mode
  updateParserModeFromSettings()
  recheckParsers()
  updateIndexerSetting(true)
}

// ---------------------------------------------------------------------------
// Embedding (vector) backend switching
// 向量后端切换
// ---------------------------------------------------------------------------

/** Resolve the target platform+model for a vector backend mode */
async function resolveEmbeddingTarget(mode: FileParserMode): Promise<{ platform: string; model: string; modelTitle: string } | null> {
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

/** Switch without any dialog and without a rebuild (rebuild "none"): the
 * backend still creates the target vector library, but indexing stays a
 * manual step. No outcome toast: the page itself reports it — the vector row
 * flips instantly, an unreachable backend or a pending rebuild raises the
 * persistent top banner via reloadSemanticPending (errors still toast via
 * reportSwitchError) */
async function switchBackendDirectly(mode: FileParserMode, resolved: { platform: string; model: string; modelTitle: string }) {
  // Select before probing: the radios commit instantly, the service check
  // runs inside switch_embedding_backend afterwards
  indexerStore.indexerSetting.embedding_parser_mode = mode
  updateParserModeFromSettings()
  recheckParsers()
  switchInFlight.value = true
  try {
    const outcome = await invoke<SwitchResult>('switch_embedding_backend', {
      mode,
      platform: resolved.platform,
      model: resolved.model,
      rebuild: 'none',
      clearOthers: false,
    })
    // The switch preempted a running indexing job: name it explicitly, the
    // interrupted run left whatever files it had not reached still Waiting
    if (outcome?.indexing_aborted) {
      window.$dialog.warning({
        title: t('indexer.indexingAbortedTitle'),
        content: t('indexer.indexingAbortedContent', { model: resolved.modelTitle }),
        positiveText: t('common.confirm'),
      })
    }
    await reloadActiveIdentity()
    await loadEmbeddingModel()
    initStatusData()
    await reloadSemanticPending()
  } finally {
    switchInFlight.value = false
  }
}

/** Surface a failed direct switch: the selection stays in effect while the
 * active identity lags behind it, until the user retries — the toast names
 * the concrete cause */
function reportSwitchError(e: unknown) {
  console.error('switch embedding backend failed', e)
  window.$message.error(typeof e === 'string' ? e : t('common.operationFailed'))
}

/** Vector backend switch entry. auto = triggered by a parser preset click:
 * never opens a modal (a compound action must not spawn dialogs) — an
 * existing target collection switches instantly, a new one starts a
 * background rebuild. Outcomes surface on the page, not as toasts. The
 * confirmation dialog stays reserved for explicit vector-row radio clicks
 * with indexed files to protect. */
async function onEmbeddingModeChanged(mode: string, auto = false) {
  // Re-entrancy guard for direct calls; auto calls come from a preset click
  // that already owns the in-flight flag
  if (switchInFlight.value && !auto)
    return
  const target = mode as FileParserMode
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
    updateParserModeFromSettings()
    recheckParsers()
    updateIndexerSetting(true)
    return
  }

  // Count files for the rebuild estimate
  let pendingFiles = 0
  try {
    pendingFiles = await invoke<number>('count_indexed_files', { category: 'document' })
      + await invoke<number>('count_indexed_files', { category: 'image' })
      + await invoke<number>('count_indexed_files', { category: 'audio' })
  } catch { /* estimate only */ }

  // Nothing indexed yet: no rebuild to choose from, switch directly
  if (pendingFiles === 0) {
    try {
      await switchBackendDirectly(target, resolved)
    } catch (e) {
      reportSwitchError(e)
    }
    return
  }

  // Preset click: switch silently, never start a rebuild — the page's pending
  // banner offers 「去重建」 so a heavy re-embed stays the user's call
  if (auto) {
    try {
      await switchBackendDirectly(target, resolved)
    } catch (e) {
      reportSwitchError(e)
    }
    return
  }

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
    const outcome = await invoke<SwitchResult>('switch_embedding_backend', {
      mode: target.mode,
      platform: target.platform,
      model: target.model,
      rebuild: switchRebuildChoice.value,
      clearOthers: switchClearOthers.value,
    })
    indexerStore.indexerSetting.embedding_parser_mode = target.mode
    // Keep the preset label consistent with the flipped mode for the next
    // full-setting persist (backend persist_embedding_mode recomputes too)
    updateParserModeFromSettings()
    showSwitchDialog.value = false
    if (outcome.indexing_aborted) {
      window.$dialog.warning({
        title: t('indexer.indexingAbortedTitle'),
        content: t('indexer.indexingAbortedContent', { model: target.modelTitle }),
        positiveText: t('common.confirm'),
      })
    }
    if (outcome.service_unreachable) {
      window.$message.warning(
        t('indexer.switchDoneDegraded', {
          model: target.modelTitle,
          backend: identityBackendTitle(target.platform),
        }),
      )
    } else {
      window.$message.success(
        outcome.rebuild_started
          ? t('indexer.switchDoneRebuilding')
          : t('indexer.switchDoneInstant'),
      )
    }
    await reloadActiveIdentity()
    await loadEmbeddingModel()
    initStatusData()
    await reloadSemanticPending()
  } catch (e) {
    console.error('switch embedding backend failed', e)
    window.$message.error(typeof e === 'string' ? e : t('common.operationFailed'))
  } finally {
    switchingBackend.value = false
  }
}

// Update parserMode based on current settings (all three category modes)
function updateParserModeFromSettings() {
  const setting = indexerStore.indexerSetting
  parserMode.value = computeParserMode(
    setting.image_parser_mode,
    setting.audio_parser_mode,
    setting.embedding_parser_mode,
  )
  setting.parser_mode = parserMode.value
}

/** Persist the full setting; silent skips the success toast — the parser
 * card's own state change is the feedback, a top-of-page toast is easy to
 * miss next to a mid-page interaction (failures always toast) */
async function updateIndexerSetting(silent = false) {
  try {
    const res = await invoke('update_indexer_setting', {
      indexerSetting: indexerStore.indexerSetting,
    })
    console.log('update_indexer_setting result', res)
    if (!silent)
      window.$message.success(t('common.updateSuccess'))
  } catch (error) {
    console.error('update indexer setting error', error)
    window.$message.error(t('common.operationFailed'))
  }
  initStatusData()
}

async function onModelPlatformSaved(updatedPlatform: ModelPlatform) {
  const index = modelPlatformList.value.findIndex(
    p => p.name === updatedPlatform.name,
  )
  const oldKey = index !== -1 ? modelPlatformList.value[index].api_key : ''
  if (index !== -1)
    modelPlatformList.value[index] = { ...updatedPlatform }
  // An API key added for the active embedding backend clears the
  // unreachable marker diagnosed without credentials
  if (
    oldKey === ''
    && updatedPlatform.api_key !== ''
    && activeIdentity.value?.platform === updatedPlatform.name
  ) {
    try {
      await invoke('clear_embedding_service_unreachable')
      await reloadSemanticPending()
    } catch (e) {
      console.error('clear service unreachable failed', e)
    }
  }
  refreshPlatformIssues()
}

// Modal edits save on a debounce; the close is the point a probe sees them
watch(showCloudModal, (open) => {
  if (!open)
    refreshPlatformIssues()
})
watch(showSelfHostedModal, (open) => {
  if (!open)
    refreshPlatformIssues()
})

// The cloud modal hands over to the proxy editor; closing it returns to the cloud modal
const showProxyEditModal = ref(false)
const proxyEditFromCloud = ref(false)

function onCloudProxySetting() {
  showCloudModal.value = false
  proxyEditFromCloud.value = true
  showProxyEditModal.value = true
}

function onProxyEditClosed() {
  if (proxyEditFromCloud.value) {
    proxyEditFromCloud.value = false
    showCloudModal.value = true
  }
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
    await reloadActiveIdentity()
    await loadEmbeddingModel()
    await reloadSemanticPending()

    const userDataPath = await invoke<string>('get_data_path')
    console.log('userDataPath', userDataPath)
    modelPath.value = await join(userDataPath, 'model')
    console.log('modelPath', modelPath.value)
    dbPath.value = await join(userDataPath, 'storage')
    initStatusData()

    // Background (re)builds flip the semantic-pending state on Finish
    unlistenIndexing = await listen<string>('selector-indexing', onIndexingEvent)
    unlistenWatcherIndexing = await listen<string>('watcher-indexing', onIndexingEvent)
  } catch (e) {
    console.error('IndexerSetting onMounted error', e)
  }
})

// The settings page is KeepAlive'd: re-run detection on each visit so restored
// model files light the local radios up without an app restart
onActivated(async () => {
  try {
    localBackends.value = await invoke<LocalBackendStatus>('check_local_backends')
    await reloadActiveIdentity()
    await loadEmbeddingModel()
    initStatusData()
    await reloadSemanticPending()
  } catch { /* keep previous snapshot */ }
})

/** Indexing Start/Finish events keep the semantic notice live */
async function onIndexingEvent(event: { payload: string }) {
  let parsed: any
  try {
    parsed = JSON.parse(event.payload)
  } catch {
    return
  }
  if (parsed && (parsed.Start !== undefined || parsed.Finish !== undefined))
    await reloadSemanticPending()
}

onUnmounted(() => {
  unlistenIndexing?.()
  unlistenWatcherIndexing?.()
})
</script>

<template>
  <div>
    <NAlert v-if="semanticRebuildPending" type="warning" :show-icon="false" size="small" class="mb-4">
      <div class="flex items-center gap-2">
        <span>{{
          rebuildRunning
            ? t('indexer.semanticRebuilding', { model: activeIdentity?.model || '' })
            : t('indexer.semanticRebuildPending', { model: activeIdentity?.model || '' })
        }}</span>
        <NButton v-if="!rebuildRunning" text type="primary" size="small" class="text-link" @click="goRebuild">
          {{ t('indexer.goRebuild') }}
        </NButton>
      </div>
    </NAlert>
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
              <NTooltip>
                <template #trigger>
                  <NRadio value="mixed" disabled>
                    {{ t('indexer.mixedMode') }}
                  </NRadio>
                </template>
                {{ t('indexer.mixedModeTooltip') }}
              </NTooltip>
            </NSpace>
          </NRadioGroup>
          <NAlert :show-icon="false" class="text-xs" size="small">
            <div v-if="parserMode === 'local'">
              {{ t('indexer.localModeDesc') }}
            </div>
            <div
              v-if="parserMode === 'selfhosted' && !selfHostedAudioAvailable() && indexerStore.indexerSetting.audio_parser_mode !== 'selfhosted' && !audioIssueShown"
              class="mt-1 text-gray-400"
            >
              {{
                t('indexer.selfHostedAudioFallback', {
                  platform: selfHostedPlatformList.find((p) => p.name === activeSelfHostedPlatform)?.title,
                  audio: modeLabel(indexerStore.indexerSetting.audio_parser_mode),
                })
              }}
            </div>
            <!-- Every used backend that cannot serve right now, one red line
                 each with its own fix entry -->
            <div
              v-for="issue in platformIssues"
              :key="`${issue.platform}-${issue.text}`"
              class="mt-1 flex items-center gap-2 text-red-400"
            >
              <span>{{ issue.text }}</span>
              <NButton
                v-if="issue.action"
                text
                type="primary"
                size="tiny"
                class="text-link"
                @click="issue.action.run"
              >
                {{ issue.action.label }}
              </NButton>
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
              <td>{{ t('indexer.imageParser') }}</td>
              <td>
                <NRadioGroup
                  :value="indexerStore.indexerSetting.image_parser_mode"
                  size="small"
                  @update:value="onImageParserModeChanged"
                >
                  <NTooltip :disabled="visionAvailable()">
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
                  <NTooltip :disabled="audioAvailable()">
                    <template #trigger>
                      <NRadio value="local" :disabled="!audioAvailable()">
                        {{ t('common.local') }}
                      </NRadio>
                    </template>
                    <template v-if="!audioAvailable()">
                      {{ missingFilesTooltip(localBackends?.audio_missing || []) }}
                    </template>
                  </NTooltip>
                  <NTooltip :disabled="selfHostedAudioAvailable()">
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
                <div class="flex flex-col gap-1">
                  <div class="flex items-center gap-2">
                    <NRadioGroup
                      :value="indexerStore.indexerSetting.embedding_parser_mode"
                      size="small"
                      @update:value="onEmbeddingModeChanged"
                    >
                      <NTooltip :disabled="vectorAvailable()">
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
                  </div>
                  <!-- No vector library configured yet (fresh lite install), or
                       the active one lags behind the selected mode: a radio
                       already sitting on the target value can never re-fire the
                       switch flow. Without credentials guide to the settings
                       modal first; otherwise offer the switch as "apply now" -->
                  <div v-if="switchPending" class="flex items-center gap-2">
                    <span class="text-xs text-gray-400">
                      <template v-if="activeIdentity">
                        {{ t('indexer.switchPendingCurrent', { model: activeIdentity.model, backend: identityBackendTitle(activeIdentity.platform) }) }}
                      </template>
                      <template v-else-if="!vectorBackendReady">
                        {{ t('indexer.switchNeedConfig') }}
                      </template>
                      <template v-else>
                        {{ t('indexer.switchPendingUnconfigured') }}
                      </template>
                    </span>
                    <NButton
                      v-if="!activeIdentity && !vectorBackendReady"
                      text
                      type="primary"
                      size="tiny"
                      class="text-link"
                      @click="openVectorConfig"
                    >
                      {{ t('indexer.switchGoConfig') }}
                    </NButton>
                    <NButton
                      v-else
                      text
                      type="primary"
                      size="tiny"
                      class="text-link"
                      @click="onEmbeddingModeChanged(indexerStore.indexerSetting.embedding_parser_mode)"
                    >
                      {{ t('indexer.switchPendingAction') }}
                    </NButton>
                  </div>
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
                  @proxy-setting="onCloudProxySetting"
                />
              </div>
            </NCard>
          </NScrollbar>
        </NModal>

        <!-- Proxy editor (handed over from the cloud modal, returns on close) -->
        <NModal
          v-model:show="showProxyEditModal"
          preset="card"
          :title="t('proxy.setting')"
          style="width: 420px"
          @after-leave="onProxyEditClosed"
        >
          <ProxyEdit />
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
