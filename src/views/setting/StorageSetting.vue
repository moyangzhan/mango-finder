<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-dialog'
import { relaunch } from '@tauri-apps/plugin-process'
import VectorLibrary from './VectorLibrary.vue'
import { useIndexerStore } from '@/stores/indexer'
import { t } from '@/locales'

const indexerStore = useIndexerStore()

const dataPath = ref('')
const needRestart = ref(false)
const dataCopying = ref(false)

async function openDirDialog() {
  const paths = await open({ directory: true, multiple: false })
  if (!paths || Array.isArray(paths))
    return

  let res: string
  try {
    res = await invoke<string>('set_data_path', { path: paths, force: false })
  } catch (err: any) {
    window.$message.error(typeof err === 'string' ? err : t('common.operationFailed'))
    return
  }
  if (res.indexOf('exist') === 0) {
    const existPath = res.split(':')[1]
    window.$dialog.warning({
      title: t('common.warning'),
      content: `${t('common.existFile')}: ${existPath}${t('common.forceChange')}`,
      positiveText: t('common.confirm'),
      onPositiveClick: async () => {
        dataCopying.value = true
        try {
          res = await invoke<string>('set_data_path', { path: paths, force: true })
          if (res.indexOf('success') === 0) {
            const userDataPath = await invoke<string>('get_data_path')
            dataPath.value = userDataPath
            window.$message.success(t('common.restartAppForChange'))
            needRestart.value = true
          } else {
            window.$message.error(res)
          }
        } catch (err) {
          console.error('set_data_path error', err)
        } finally {
          dataCopying.value = false
        }
      },
    })
    return
  } else if (res.indexOf('same') === 0) {
    window.$message.error(t('common.changeSamePathError'))
    return
  }

  const userDataPath = await invoke<string>('get_data_path')
  dataPath.value = userDataPath
  window.$message.success(t('common.restartAppForChange'))
  needRestart.value = true
}

async function resetDataPath() {
  window.$dialog.warning({
    title: t('common.warning'),
    content: t('common.resetDataPathConfirm'),
    positiveText: t('common.confirm'),
    negativeText: t('common.cancel'),
    onPositiveClick: async () => {
      await doResetDataPath()
    },
  })
}

async function doResetDataPath() {
  try {
    let res = await invoke<string>('reset_data_path', { force: false })
    if (res.indexOf('exist') === 0) {
      const existPath = res.split(':')[1]
      window.$dialog.warning({
        title: t('common.warning'),
        content: `${t('common.existFile')}: ${existPath}${t('common.forceChange')}`,
        positiveText: t('common.confirm'),
        onPositiveClick: async () => {
          dataCopying.value = true
          try {
            res = await invoke<string>('reset_data_path', { force: true })
            if (res.indexOf('success') === 0) {
              const userDataPath = await invoke<string>('get_data_path')
              dataPath.value = userDataPath
              window.$message.success(t('common.restartAppForChange'))
              needRestart.value = true
            } else {
              window.$message.error(res)
            }
          } catch (err) {
            console.log(err)
          } finally {
            dataCopying.value = false
          }
        },
      })
      return
    } else if (res.indexOf('same') === 0) {
      window.$message.error(t('common.changeSamePathError'))
      return
    }
    const userDataPath = await invoke<string>('get_data_path')
    dataPath.value = userDataPath
    window.$message.success(t('common.restartAppForChange'))
    needRestart.value = true
  } catch (err) {
    console.error('reset_data_path error', err)
    window.$message.error(t('common.operationFailed'))
  }
}

async function openDataPath() {
  try {
    await invoke('open_directory', { path: dataPath.value })
  } catch (e) {
    console.error('open_directory failed', e)
    window.$message.error(t('common.operationFailed'))
  }
}

async function restart() {
  try {
    await relaunch()
  } catch (e) {
    console.error('relaunch failed', e)
    window.$message.error(t('common.operationFailed'))
  }
}

// Content storage: parsed-content persistence policy, moved from the indexer tab /
// 内容存储：解析内容的落盘策略，自索引设置页移入
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
    await updateIndexerSetting()
    window.$message.success(t('common.updateSuccess'))
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

async function updateIndexerSetting() {
  try {
    await invoke('update_indexer_setting', {
      indexerSetting: indexerStore.indexerSetting,
    })
  } catch (error) {
    console.error('update indexer setting error', error)
    window.$message.error(t('common.operationFailed'))
  }
}

let unlistenStorageChange: (() => void) | null = null

onMounted(async () => {
  // The radios read the shared store; load it here so the tab works standalone /
  // 单选按钮读取共享 store，这里自加载以保证本 tab 独立可用
  try {
    const setting = await invoke<IndexerSetting>('load_indexer_setting')
    indexerStore.setIndexerSetting(setting)
  } catch (e) {
    console.error('load_indexer_setting error', e)
  }
  try {
    dataPath.value = await invoke<string>('get_data_path')
  } catch (e) {
    console.error('get_data_path error', e)
  }

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
})

onUnmounted(() => {
  unlistenStorageChange?.()
})
</script>

<template>
  <div>
    <NCard :title="t('indexer.dataPath')" class="mb-4" size="small" :bordered="true">
      <div class="flex flex-col space-y-2">
        <div>{{ dataPath }}</div>
        <div class="text-xs text-gray-400">
          {{ t('indexer.dataPathDesc') }}
        </div>
        <NAlert v-if="needRestart" type="warning">
          {{ t('common.restartAppForChange') }}
          <NButton text type="primary" class="text-link" @click="restart">
            {{
              t('common.clickToRestart') }}
          </NButton>
        </NAlert>
        <div class="flex space-x-2">
          <div class="mr-2">
            <NButton ghost :disabled="dataCopying" :loading="dataCopying" @click="openDirDialog">
              <span>{{ t('common.change') }}</span>
            </NButton>
          </div>
          <div class="mr-2">
            <NButton ghost :disabled="dataCopying" :loading="dataCopying" @click="resetDataPath">
              {{ t('common.reset') }}
            </NButton>
          </div>
          <NButton ghost :disabled="dataCopying" :loading="dataCopying" @click="openDataPath">
            {{ t('common.open') }}
          </NButton>
        </div>
      </div>
    </NCard>
    <NCard class="mb-4 px-0" size="small" :bordered="true" hoverable>
      <template #header>
        <CardTitleHelp :title="t('indexer.contentStorage')" :help="t('indexer.contentStorageDesc')" />
      </template>
      <div class="flex flex-col">
        <div class="flex flex-col space-y-3 mt-1 mb-4">
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
    <VectorLibrary />
  </div>
</template>
