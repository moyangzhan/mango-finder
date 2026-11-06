<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import type { DataTableColumns } from 'naive-ui'
import { useAppStore } from '@/stores/app'
import { t } from '@/locales'

const appStore = useAppStore()
const collections = ref<EmbeddingCollectionRow[]>([])
const loading = ref(false)
const clearing = ref(false)
const modelPlatformList = ref<ModelPlatform[]>([])
const selfHostedPlatformList = ref<SelfHostedPlatform[]>([])

function backendTitle(platform: string) {
  if (platform === 'local')
    return t('common.local')
  const selfHosted = selfHostedPlatformList.value.find(p => p.name === platform)
  if (selfHosted)
    return selfHosted.title
  const cloud = modelPlatformList.value.find(p => p.name === platform)
  return cloud?.title || platform
}

function formatBytes(bytes: number) {
  if (bytes < 1024)
    return `${bytes} B`
  if (bytes < 1024 * 1024)
    return `${(bytes / 1024).toFixed(1)} KB`
  if (bytes < 1024 * 1024 * 1024)
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`
}

async function loadCollections() {
  loading.value = true
  try {
    collections.value = await invoke<EmbeddingCollectionRow[]>('load_embedding_collections')
  } catch (e) {
    console.error('load embedding collections failed', e)
    window.$message.error(t('common.operationFailed'))
  } finally {
    loading.value = false
  }
}

function confirmClear(row: EmbeddingCollectionRow) {
  window.$dialog.warning({
    title: t('common.tip'),
    content: t('vectorLibrary.clearConfirm', {
      model: row.model,
      backend: backendTitle(row.platform),
      cost: row.is_local ? t('vectorLibrary.costLocal') : t('vectorLibrary.costRemote'),
    }),
    positiveText: t('common.confirm'),
    negativeText: t('common.cancel'),
    onPositiveClick: async () => {
      clearing.value = true
      try {
        await invoke('clear_embedding_collection', { platform: row.platform, model: row.model })
        window.$message.success(t('common.updateSuccess'))
        await loadCollections()
      } catch (e) {
        window.$message.error(typeof e === 'string' ? e : t('common.operationFailed'))
      } finally {
        clearing.value = false
      }
    },
  })
}

function confirmClearAll() {
  const inactive = collections.value.filter(c => !c.is_active)
  if (inactive.length === 0)
    return
  window.$dialog.warning({
    title: t('common.tip'),
    content: t('vectorLibrary.clearAllConfirm', { count: inactive.length }),
    positiveText: t('common.confirm'),
    negativeText: t('common.cancel'),
    onPositiveClick: async () => {
      clearing.value = true
      try {
        await invoke('clear_inactive_embedding_collections')
        window.$message.success(t('common.updateSuccess'))
        await loadCollections()
      } catch (e) {
        window.$message.error(typeof e === 'string' ? e : t('common.operationFailed'))
      } finally {
        clearing.value = false
      }
    },
  })
}

const columns = computed<DataTableColumns<EmbeddingCollectionRow>>(() => [
  {
    title: t('vectorLibrary.modelBackend'),
    key: 'model',
    render(row) {
      return `${row.model} · ${backendTitle(row.platform)}`
    },
  },
  {
    title: t('vectorLibrary.dimension'),
    key: 'dim',
    width: 90,
    render: row => `${row.dim}`,
  },
  {
    title: t('vectorLibrary.dataVolume'),
    key: 'data',
    width: 180,
    render: row =>
      t('vectorLibrary.dataVolumeValue', {
        chunks: row.content_rows,
        files: row.file_count,
      }),
  },
  {
    title: t('vectorLibrary.diskUsage'),
    key: 'estimated_bytes',
    width: 110,
    render: row => `≈ ${formatBytes(row.estimated_bytes)}`,
  },
  {
    title: t('common.status'),
    key: 'status',
    width: 100,
    render(row) {
      return row.is_active
        ? h('span', { class: 'text-green-500' }, t('vectorLibrary.inUse'))
        : h('span', { class: 'text-gray-400' }, t('vectorLibrary.retained'))
    },
  },
  {
    title: t('common.operation'),
    key: 'actions',
    width: 100,
    render(row) {
      if (row.is_active)
        return h('span', { class: 'text-xs text-gray-300' }, t('vectorLibrary.switchAwayFirst'))
      return h(
        'span' as any,
        { class: 'text-xs text-red-400 cursor-pointer hover:text-red-500', onClick: () => confirmClear(row) },
        { default: () => t('vectorLibrary.clear') },
      )
    },
  },
])

watch(() => appStore.locale, () => {
  // columns are computed on t() so they refresh automatically
})

// The settings page is KeepAlive'd: re-sync when the user comes back
// (the active flag changes after backend switches on the indexer tab)
onActivated(async () => {
  await loadCollections()
})

onMounted(async () => {
  try {
    modelPlatformList.value = await invoke<ModelPlatform[]>('load_model_platforms')
    selfHostedPlatformList.value = await invoke<SelfHostedPlatform[]>('load_self_hosted_platforms')
  } catch (e) {
    console.error('load platforms failed', e)
  }
  await loadCollections()
})
</script>

<template>
  <div>
    <NCard :title="t('vectorLibrary.title')" size="small" :bordered="true" hoverable>
      <NAlert :show-icon="false" class="text-xs mb-2">
        {{ t('vectorLibrary.desc') }}
      </NAlert>
      <NDataTable
        size="small"
        :columns="columns"
        :data="collections"
        :loading="loading"
        :row-key="(row: EmbeddingCollectionRow) => `${row.platform}|${row.model}`"
      />
      <div class="mt-3 flex items-center gap-2">
        <NButton size="small" ghost :loading="loading" @click="loadCollections">
          {{ t('common.refresh') }}
        </NButton>
        <NButton
          size="small" ghost type="error" :loading="clearing"
          :disabled="collections.filter(c => !c.is_active).length === 0"
          @click="confirmClearAll"
        >
          {{ t('vectorLibrary.clearAllInactive') }}
        </NButton>
      </div>
    </NCard>
  </div>
</template>
