<script setup lang="ts">
import { RouterLink, RouterView } from 'vue-router'
import { NBadge, NButton, NConfigProvider, NIcon, NModal, darkTheme, dateEnUS, dateZhCN, enUS, zhCN } from 'naive-ui'
import type { MenuOption } from 'naive-ui'
import { FileTrayStackedOutline, GitNetworkOutline, HomeOutline, SettingsOutline } from '@vicons/ionicons5'
import { DeviceHubOutlined } from '@vicons/material'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import NaiveProvider from '@/components/NaiveProvider.vue'
import router from '@/router'
import { useAppStore } from '@/stores/app'
import { setLocale, t } from '@/locales'

interface StorageTask {
  task_type: string
  category?: string
  new_mode?: string
  old_path?: string
  started_at: number
}

const appStore = useAppStore()
const activeMenu = ref<string>('menu-home')
const showRecoveryDialog = ref(false)
const recoveryTask = ref<StorageTask | null>(null)

// Pending pairing request count for badge
const pendingPairingCount = ref(0)
const clusterEnabled = ref(false)

const menuOptions = computed<MenuOption[]>(() => [
  {
    key: 'menu-home',
    icon: renderIcon(HomeOutline),
    label: () =>
      h(
        RouterLink,
        {
          to: {
            name: 'Home',
          },
        },
        { default: () => t('menu.home') },
      ),
  },
  {
    key: 'menu-index',
    icon: renderIcon(GitNetworkOutline),
    label: () =>
      h(
        RouterLink,
        {
          to: {
            name: 'Indexer',
          },
        },
        { default: () => t('menu.indexer') },
      ),
  },
  {
    key: 'menu-data',
    icon: renderIcon(FileTrayStackedOutline),
    label: () =>
      h(
        RouterLink,
        {
          to: {
            name: 'Data',
          },
        },
        { default: () => t('menu.data') },
      ),
  },
  {
    key: 'menu-device',
    icon: () => (clusterEnabled.value && pendingPairingCount.value > 0)
      ? (
          h(NBadge, { value: pendingPairingCount.value, max: 99, type: 'info' }, {
            default: () => h(NIcon, null, { default: () => h(DeviceHubOutlined) }),
          })
        )
      : h(NIcon, null, { default: () => h(DeviceHubOutlined) }),
    label: () =>
      h(
        RouterLink,
        {
          to: {
            name: 'Device',
          },
        },
        { default: () => t('menu.deviceManagement') },
      ),
  },
])
function renderIcon(icon: Component) {
  return () => h(NIcon, null, { default: () => h(icon) })
}

function gotoSetting() {
  activeMenu.value = 'menu-setting'
  router.push({ name: 'Setting' })
}
const isDark = computed(() => appStore.getTheme === 'dark' ? darkTheme : null)

watch(() => appStore.getTheme, (theme) => {
  if (theme === 'dark')
    document.body.classList.add('dark')
  else
    document.body.classList.remove('dark')
}, { immediate: true })

// Load pending pairing request count
async function loadPendingPairingCount() {
  try {
    const count = await invoke<number>('count_pending_pairing_requests')
    pendingPairingCount.value = count
  } catch (e) {
    console.error('Failed to load pairing requests count:', e)
  }
}

onMounted(() => {
  // Register event listeners first to avoid missing events
  // 优先注册事件监听器，避免错过事件
  listen<{ enabled: boolean }>('cluster-enabled-changed', (event) => {
    clusterEnabled.value = event.payload.enabled
    if (event.payload.enabled)
      loadPendingPairingCount()
    else
      pendingPairingCount.value = 0
  })

  listen('pairing-request-received', () => {
    if (clusterEnabled.value)
      loadPendingPairingCount()
  })

  listen('pairing-response-received', () => {
    if (clusterEnabled.value)
      loadPendingPairingCount()
  })

  listen<string>('offline-sync-status', (event) => {
    const status = event.payload
    if (status === 'started')
      window.$message.info(t('indexer.offlineSyncStarted'), { duration: 3000 })
    else if (status === 'completed')
      window.$message.success(t('indexer.offlineSyncCompleted'), { duration: 3000 })
    else if (status === 'error')
      window.$message.error(t('indexer.offlineSyncFailed'), { duration: 5000 })
  })

  listen<{ port: number }>('cluster-port-error', (event) => {
    console.log('Received cluster-port-error event:', event.payload)
    appStore.setClusterPortError(event.payload)
  })

  listen('cluster-setting-saved', () => {
    appStore.clearClusterPortError()
  })

  // Then invoke commands
  if (import.meta.env.MODE === 'production') {
    window.addEventListener('contextmenu', (e) => {
      e.preventDefault()
    }, false)
  }
  invoke('ui_mounted').then((resp) => {
    console.log('ui_mounted', resp)
  })
  invoke('load_active_locale').then((activeLocale) => {
    setLocale(activeLocale as 'en-US' | 'zh-CN')
  })

  // Load cluster setting
  invoke<{ enabled: boolean }>('load_cluster_setting').then((setting) => {
    clusterEnabled.value = setting?.enabled ?? false
    // Only load pairing count if cluster is enabled
    if (clusterEnabled.value)
      loadPendingPairingCount()
  })

  // Check for unfinished tasks after startup stabilizes
  setTimeout(async () => {
    try {
      const task = await invoke<StorageTask | null>('get_storage_task')
      if (task) {
        // Skip if task started very recently — likely the current session's offline sync
        const ageSeconds = Math.floor(Date.now() / 1000) - task.started_at
        if (ageSeconds < 30)
          return

        recoveryTask.value = task
        showRecoveryDialog.value = true
      }
    } catch (e) {
      console.error('Failed to check storage task:', e)
    }
  }, 3000)
})

function getTaskDescription(task: StorageTask): string {
  switch (task.task_type) {
    case 'indexing':
      return t('common.taskRecovery.taskType.indexing')
    case 'content_storage_change':
      return t('common.taskRecovery.taskType.contentStorageChange', { category: task.category || '' })
    case 'data_copying':
      return t('common.taskRecovery.taskType.dataCopying')
    default:
      return task.task_type
  }
}

async function recoveryAction(action: 'resume' | 'skip' | 'retryCopy' | 'revertPath') {
  showRecoveryDialog.value = false
  if (!recoveryTask.value)
    return

  try {
    switch (action) {
      case 'skip':
        await invoke('clear_storage_task')
        break
      case 'resume':
        if (recoveryTask.value.task_type === 'indexing') {
          await invoke('clear_storage_task')
          window.$message.info(t('common.operationSuccess'))
        } else if (recoveryTask.value.task_type === 'content_storage_change') {
          // Re-trigger storage change using the target mode stored in the task
          const category = recoveryTask.value.category || 'document'
          const targetMode = recoveryTask.value.new_mode || 'database'
          await invoke('clear_storage_task')
          await invoke('change_content_storage', { category, newMode: targetMode })
        }
        break
      case 'retryCopy':
        if (recoveryTask.value.old_path) {
          await invoke('retry_data_copy', { oldPath: recoveryTask.value.old_path })
          window.$message.success(t('common.operationSuccess'))
        }
        break
      case 'revertPath':
        if (recoveryTask.value.old_path) {
          await invoke('revert_data_path', { oldPath: recoveryTask.value.old_path })
          window.$message.success(t('common.restartAppForChange'))
        }
        break
    }
  } catch (e) {
    console.error('Recovery action failed:', e)
    window.$message.error(t('common.operationFailed'))
  } finally {
    recoveryTask.value = null
  }
}
</script>

<template>
  <NConfigProvider
    class="h-full select-text" :locale="appStore.locale === 'en-US' ? enUS : zhCN"
    :date-locale="appStore.locale === 'en-US' ? dateEnUS : dateZhCN" :theme="isDark"
  >
    <NaiveProvider>
      <NLayout class="h-full" has-sider>
        <NLayoutSider bordered :collapsed-width="48" collapse-mode="width" :collapsed="true" class="h-full">
          <div>
            <NMenu v-model:value="activeMenu" :options="menuOptions" />
            <div class="flex flex-col absolute bottom-0 ml-2 mb-2">
              <NTooltip trigger="hover" placement="right" style="margin-left: 1.5rem;">
                <template #trigger>
                  <NButton text type="primary" style="font-size: 26px;" class="text-link" @click="gotoSetting">
                    <NIcon>
                      <SettingsOutline />
                    </NIcon>
                  </NButton>
                </template>
                {{ t('menu.setting') }}
              </NTooltip>
            </div>
          </div>
        </NLayoutSider>
        <NLayout>
          <RouterView v-slot="{ Component }">
            <KeepAlive>
              <component :is="Component" />
            </KeepAlive>
          </RouterView>
        </NLayout>
      </NLayout>

      <!-- Task recovery dialog -->
      <NModal v-model:show="showRecoveryDialog" preset="dialog" :title="t('common.taskRecovery.title')" :mask-closable="false" :close-on-esc="false" :closable="false">
        <div v-if="recoveryTask">
          <p>{{ t('common.taskRecovery.desc', { task: getTaskDescription(recoveryTask) }) }}</p>
        </div>
        <template #action>
          <template v-if="recoveryTask?.task_type === 'data_copying'">
            <NButton @click="recoveryAction('retryCopy')">
              {{ t('common.taskRecovery.retryCopy') }}
            </NButton>
            <NButton type="warning" @click="recoveryAction('revertPath')">
              {{ t('common.taskRecovery.revertPath') }}
            </NButton>
          </template>
          <template v-else>
            <NButton type="primary" @click="recoveryAction('resume')">
              {{ t('common.taskRecovery.resume') }}
            </NButton>
            <NButton @click="recoveryAction('skip')">
              {{ t('common.taskRecovery.skip') }}
            </NButton>
          </template>
        </template>
      </NModal>
    </NaiveProvider>
  </NConfigProvider>
</template>
