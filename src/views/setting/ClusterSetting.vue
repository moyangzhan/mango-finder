<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { emit } from '@tauri-apps/api/event'
import { t } from '@/locales'
import { useAppStore } from '@/stores/app'

const appStore = useAppStore()

const setting = ref<ClusterSetting>({
  enabled: false,
  port: 7890,
  device_name: '',
  allow_to_be_discovered: true,
  auto_request_pairing: false,
  auto_accept_pairing: false,
  online_check_interval: 30,
})

const loading = ref(false)

// Read port error from global state (set by App.vue global listener)
const portError = computed(() => {
  const error = appStore.getClusterPortError
  if (error)
    return t('cluster.portBindError', { port: error.port })

  return null
})

async function loadSetting() {
  loading.value = true
  try {
    const result = await invoke<ClusterSetting>('load_cluster_setting')
    if (result)
      setting.value = result
  } catch (e) {
    console.error('Failed to load cluster setting:', e)
  } finally {
    loading.value = false
  }
}

async function saveSetting() {
  try {
    await invoke('update_cluster_setting', { setting: setting.value })
    window.$message.success(t('common.saveSuccess'))
  } catch (e) {
    console.error('Failed to save cluster setting:', e)
    window.$message.error(t('common.operationFailed'))
  }
}

// One toggle at a time: a concurrent second save could hit the stop drain window and skip the start
const toggling = ref(false)

async function toggleEnabled() {
  if (toggling.value)
    return
  toggling.value = true
  try {
    setting.value.enabled = !setting.value.enabled
    // saveSetting's enable-flip branch starts/stops the cluster services
    await saveSetting()
    // Notify device management page to refresh
    await emit('cluster-enabled-changed', { enabled: setting.value.enabled })
  } finally {
    toggling.value = false
  }
}

function dismissPortError() {
  appStore.clearClusterPortError()
}

onMounted(async () => {
  loadSetting()
})
</script>

<template>
  <div class="space-y-4">
    <NSpin :show="loading">
      <div class="space-y-4">
        <!-- Port Error Alert -->
        <NAlert v-if="portError" type="error" closable @close="dismissPortError">
          <template #header>
            {{ t('cluster.portBindErrorTitle') }}
          </template>
          {{ portError }}
        </NAlert>

        <!-- Enable Toggle -->
        <div class="flex items-center justify-between">
          <div class="flex-1 mr-4">
            <div class="font-medium">
              {{ t('cluster.enabled') }}
            </div>
            <div class="text-sm text-gray-500 mt-1">
              {{ t('cluster.enabledDesc') }}
            </div>
          </div>
          <NSwitch :value="setting.enabled" :loading="toggling" :disabled="toggling" @update:value="toggleEnabled" />
        </div>

        <!-- Settings when enabled -->
        <div v-if="setting.enabled" class="space-y-4">
          <!-- Connection Config Card -->
          <NCard :title="t('cluster.connectionConfig')" size="small">
            <!-- Auto Request Pairing -->
            <div class="mt-3">
              <div class="flex items-center justify-between">
                <div class="text-sm">
                  {{ t('cluster.autoRequestPairing') }}
                </div>
                <NSwitch v-model:value="setting.auto_request_pairing" @update:value="saveSetting" />
              </div>
              <div class="text-xs text-gray-500 mt-1">
                {{ t('cluster.autoRequestPairingDesc') }}
              </div>
            </div>

            <hr class="my-4 border-gray-200 dark:border-gray-700">
            <!-- Allow to be Discovered -->
            <div class="mt-3">
              <div class="flex items-center justify-between">
                <div class="text-sm">
                  {{ t('cluster.allowToBeDiscovered') }}
                </div>
                <NSwitch v-model:value="setting.allow_to_be_discovered" @update:value="saveSetting" />
              </div>
              <div class="text-xs text-gray-500 mt-1">
                {{ t('cluster.allowToBeDiscoveredDesc') }}
              </div>
            </div>

            <!-- Auto Accept Pairing -->
            <div class="mt-3">
              <div class="flex items-center justify-between">
                <div class="text-sm font-medium">
                  {{ t('cluster.autoAcceptPairing') }}
                </div>
                <NSwitch v-model:value="setting.auto_accept_pairing" @update:value="saveSetting" />
              </div>
              <div class="text-xs text-gray-500 mt-1">
                {{ t('cluster.autoAcceptPairingDesc') }}
              </div>
            </div>

            <hr class="my-4 border-gray-200 dark:border-gray-700">
            <!-- Status Check Interval -->
            <div class="mt-4 space-y-2">
              <div class="flex items-center justify-between">
                <div class="text-sm font-medium">
                  {{ t('cluster.statusCheckInterval') }}
                </div>
                <NInputNumber
                  v-model:value="setting.online_check_interval" :min="5" :max="300" size="small"
                  style="width: 100px" @update:value="saveSetting"
                />
              </div>
              <div class="text-xs text-gray-500">
                {{ t('cluster.statusCheckIntervalDesc') }}
              </div>
            </div>
          </NCard>
        </div>
      </div>
    </NSpin>
  </div>
</template>

<style scoped></style>
