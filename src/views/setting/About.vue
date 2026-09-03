<script setup lang="ts">
import { openPath } from '@tauri-apps/plugin-opener'
import { LogoGithub, DocumentTextOutline } from '@vicons/ionicons5'
import { invoke } from '@tauri-apps/api/core'
import { getVersion } from '@tauri-apps/api/app'
import cliDocEn from '../../../docs/cli.md?raw'
import cliDocZh from '../../../docs/cli_cn.md?raw'
import HowToUse from '../HowToUse.vue'
import MarkdownPreview from '@/components/MarkdownPreview.vue'
import { getLocale, t } from '@/locales'

const appVersion = ref('')
const appClientId = ref('')
const showCliDoc = ref(false)
const cliDocContent = ref('')

function openUrl(path = '') {
  openPath(path).then((res) => {
    console.log('openUrl', res)
  })
}

function openCliDoc() {
  // 文档在构建期内嵌，跟随界面语言选择中英文版本
  cliDocContent.value = getLocale() === 'zh-CN' ? cliDocZh : cliDocEn
  showCliDoc.value = true
}

onMounted(async () => {
  try {
    appVersion.value = await getVersion()
    appClientId.value = await invoke('get_client_id')
  } catch (error) {
    console.error(`getVersion error: ${error}`)
  }
})
</script>

<template>
  <div class="flex flex-col h-full p-4 text-left space-y-8 w-full">
    <div class="flex flex-col w-full items-center justify-center pr-8">
      <NImage src="/mango-desk.png" alt="MangoFinder" width="200" height="200" preview-disabled />
      <p class="text-gray-700 dark:text-gray-300 mb-4">
        {{ t('about.description') }}
      </p>
    </div>
    <HowToUse :show-steps="true">
      <template #tip>
        <div class="font-semibold mb-2">
          {{ t('common.usageGaide') }}
        </div>
      </template>
    </HowToUse>
    <div class="flex-1" />
    <div class="flex flex-col w-full space-y-2 text-left">
      <div class="font-semibold">
        {{ t('about.moreDetail') }}
      </div>
      <div>
        <NButton text type="primary" class="text-link" @click="openUrl('https://github.com/moyangzhan/mango-finder')">
          <template #icon>
            <NIcon>
              <LogoGithub />
            </NIcon>
          </template>
          <span class="text-xs">MangoFinder on GitHub</span>
        </NButton>
      </div>
      <div class="text-xs mt-2 flex flex-col space-y-2">
        <div class="text-sm font-semibold">
          {{ t('about.appInfo') }}
        </div>
        <div>
          {{ t('about.currentVersion') }}: v{{ appVersion }}
        </div>
        <div>
          {{ t('about.appClientId') }}: {{ appClientId }}
        </div>
      </div>
      <div class="mt-4">
        <NButton text type="primary" class="text-link" @click="openCliDoc">
          <template #icon>
            <NIcon>
              <DocumentTextOutline />
            </NIcon>
          </template>
          <span class="text-xs">{{ t('about.cliDoc') }}</span>
        </NButton>
      </div>
    </div>
    <NModal v-model:show="showCliDoc" preset="card" :title="t('about.cliDoc')" style="width: 80%; max-height: 80vh;" class="select-text">
      <div class="overflow-y-auto" style="max-height: 65vh">
        <MarkdownPreview :content="cliDocContent" />
      </div>
    </NModal>
  </div>
</template>

<style scoped></style>
