<script setup lang='ts'>
import { invoke } from '@tauri-apps/api/core'
import { useDebounceFn } from '@vueuse/core'
import { t } from '@/locales'
import { useAppStore } from '@/stores/app'

const props = defineProps<Props>()
const emit = defineEmits<Emit>()
const appStore = useAppStore()
const labelWidth = computed(() => appStore.locale === 'zh-CN' ? 60 : 100)

interface Props {
  platform: SelfHostedPlatform
}
interface Emit {
  (ev: 'saved', updatedPlatform: SelfHostedPlatform): void
}

const tmpPlatform = ref<SelfHostedPlatform>({
  id: 0,
  name: '',
  title: '',
  host: '127.0.0.1',
  port: 11434,
  remark: '',
})
tmpPlatform.value = { ...props.platform }

const debounceSave = useDebounceFn(async () => {
  await invoke('update_self_hosted_platform', { platform: tmpPlatform.value })
  emit('saved', tmpPlatform.value)
}, 1000)

interface CheckResult {
  ok: boolean
  text: string
}

/** Verdict of the last check, shown inside the modal above the button */
const checkResult = ref<CheckResult | null>(null)

function onEdit() {
  // A past verdict describes the previous values; drop it on the first edit
  checkResult.value = null
  debounceSave()
}

const checking = ref(false)
async function checkConnection() {
  checking.value = true
  checkResult.value = null
  try {
    await invoke('check_self_hosted_platform', { platform: tmpPlatform.value })
    checkResult.value = { ok: true, text: t('common.checkConnectionSuccess') }
  } catch (e: any) {
    checkResult.value = { ok: false, text: `${t('common.checkConnectionFailed')}: ${e}` }
  } finally {
    checking.value = false
  }
}
</script>

<template>
  <div class="w-full">
    <NForm label-placement="left" :label-width="labelWidth" class="w-full">
      <NFormItem :label="t('common.name')">
        <NInput v-model:value="tmpPlatform.name" disabled />
      </NFormItem>
      <NFormItem :label="t('common.title')">
        <NInput v-model:value="tmpPlatform.title" @update:value="onEdit" />
      </NFormItem>
      <NFormItem label="Host">
        <NInput v-model:value="tmpPlatform.host" @update:value="onEdit" />
      </NFormItem>
      <NFormItem label="Port">
        <NInputNumber v-model:value="tmpPlatform.port" :min="1" :max="65535" @update:value="onEdit" />
      </NFormItem>
      <NFormItem :label="t('common.description')">
        <NInput v-model:value="tmpPlatform.remark" @update:value="onEdit" />
      </NFormItem>
    </NForm>
    <div class="w-full">
      <div
        v-if="checkResult"
        class="mb-2 text-xs break-words"
        :class="checkResult.ok ? 'text-green-500' : 'text-red-400'"
      >
        {{ checkResult.text }}
      </div>
      <NButton type="primary" :loading="checking" @click="checkConnection">
        {{ t('common.checkConnection') }}
      </NButton>
    </div>
  </div>
</template>

<style scoped>
:deep(.n-form-item-label) {
  align-items: center !important;
}
</style>
