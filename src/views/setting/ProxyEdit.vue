<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { emptyProxyInfo } from '@/utils/functions'
import { t } from '@/locales'

const proxy = ref<ProxyInfo>(emptyProxyInfo())

onMounted(async () => {
  try {
    proxy.value = await invoke<ProxyInfo>('load_proxy_info')
  } catch (e) {
    console.error('load_proxy_info error', e)
  }
})

function saveProxy() {
  invoke('update_proxy_info', { proxyInfo: proxy.value }).then(() => {
    window.$message.success(t('common.saveSuccess'))
  }).catch((err) => {
    console.error('update_proxy_info error', err)
    window.$message.error(t('common.operationFailed'))
  })
}
</script>

<template>
  <div class="flex flex-col">
    <NFormItem :label="t('proxy.protocol')">
      <NInput v-model:value="proxy.protocol" />
    </NFormItem>
    <NFormItem :label="t('proxy.host')">
      <NInput v-model:value="proxy.host" />
    </NFormItem>
    <NFormItem :label="t('proxy.port')">
      <NInputNumber v-model:value="proxy.port" />
    </NFormItem>
    <div>
      <NButton ghost @click="saveProxy">
        {{ t('common.save') }}
      </NButton>
    </div>
  </div>
</template>
