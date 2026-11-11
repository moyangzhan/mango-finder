<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { DocumentTextOutline } from '@vicons/ionicons5'
import cliDocEn from '../../../docs/en/cli.md?raw'
import cliDocZh from '../../../docs/cn/cli.md?raw'
import ProxyEdit from './ProxyEdit.vue'
import MarkdownPreview from '@/components/MarkdownPreview.vue'
import { useAppStore } from '@/stores/app'
import { getLocale, setLocale, t } from '@/locales'

const appStore = useAppStore()
const activePlatform = ref('openai')
const activeTheme = ref(appStore.getTheme)
const activeLanguage = ref('en-US')
const activeTab = ref('openai')
const cliStatus = ref<{ mfd_path: string; exists: boolean; installed: boolean }>({ mfd_path: '', exists: false, installed: false })
const cliBusy = ref(false)
const showCliDoc = ref(false)
const cliDocContent = ref('')

// GUI users already have the app installed; the modal swaps the source-install section for a quick availability check /
// 弹窗面向已装好 GUI 的用户：不显示源码安装章节，换成 CLI 可用性自检
const quickCheckZh = `## 快速自检

新开一个终端（PATH 变更对已打开的终端不生效）并运行：

\`\`\`bash
mfd version
\`\`\`

输出版本号即表示 \`mfd\` 已可用；若提示找不到命令，请重新加入 \`PATH\` 并新开终端重试。`

const quickCheckEn = `## Quick Check

Open a new terminal (PATH changes do not affect terminals that are already open) and run:

\`\`\`bash
mfd version
\`\`\`

A version line means \`mfd\` is ready. If the command is not found, re-add it to \`PATH\` above and reopen the terminal.`

function openCliDoc() {
  // Doc is embedded at build time and follows the UI locale; the install section spans its heading up to the next "## " heading /
  // 文档在构建期内嵌，跟随界面语言；安装章节为其标题行到下一个 "## " 标题之间的内容
  const raw = getLocale() === 'zh-CN' ? cliDocZh : cliDocEn
  const quickCheck = getLocale() === 'zh-CN' ? quickCheckZh : quickCheckEn
  cliDocContent.value = raw.replace(/^## (安装|Installation)[\s\S]*?(?=^## )/m, `${quickCheck}\n\n`)
  showCliDoc.value = true
}

async function loadCliStatus() {
  try {
    cliStatus.value = await invoke('cli_path_status')
  } catch (err) {
    console.error('cli_path_status error', err)
  }
}

async function toggleCliPath() {
  cliBusy.value = true
  try {
    const wasInstalled = cliStatus.value.installed
    await invoke(wasInstalled ? 'cli_path_uninstall' : 'cli_path_install')
    window.$message.success(wasInstalled ? t('cli.uninstallSuccess') : t('cli.installSuccess'))
    await loadCliStatus()
  } catch (err: any) {
    window.$message.error(typeof err === 'string' ? err : t('common.operationFailed'))
  } finally {
    cliBusy.value = false
  }
}

function handleLanguageChanged(newLang: string) {
  console.log('languageChange', newLang)
  activeLanguage.value = newLang
  appStore.setLocale(newLang)
  setLocale(newLang as 'en-US' | 'zh-CN')
  invoke('set_active_locale', { locale: newLang })
}

function handleThemeChanged(selectedTheme: string) {
  console.log('themeChanged', selectedTheme)
  activeTheme.value = selectedTheme
  appStore.changeTheme()
}

onMounted(async () => {
  console.log('common setting onMounted')
  const activePlatformName = await invoke('load_active_platform')
  activePlatform.value = activePlatformName as string
  activeTab.value = activePlatform.value
  const activeLocale = await invoke('load_active_locale')
  console.log('active_locale', activeLocale)
  setLocale(activeLocale as 'en-US' | 'zh-CN')
  activeLanguage.value = activeLocale as string
  await loadCliStatus()
})
</script>

<template>
  <div>
    <NCard :title="t('common.display')" class="mb-4" :bordered="true">
      <div class="flex flex-col">
        <NFormItem :label="t('common.language')">
          <div class="flex flex-col space-y-1">
            <AppTag type="warning">
              {{ t('common.languageSettingWarning') }}
            </AppTag>
            <NRadioGroup :value="activeLanguage" @update:value="handleLanguageChanged">
              <NRadio :label="t('common.english')" value="en-US" class="mr-2" />
              <NRadio :label="t('common.chinese')" value="zh-CN" />
            </NRadioGroup>
          </div>
        </NFormItem>

        <NFormItem :label="t('common.theme')">
          <NRadioGroup v-model:value="activeTheme" @update:value="handleThemeChanged">
            <NRadio :label="`☀️${t('common.light')}`" value="light" class="mr-2" />
            <NRadio :label="`🌙${t('common.dark')}`" value="dark" />
          </NRadioGroup>
        </NFormItem>
      </div>
    </NCard>
    <NCard :title="t('cli.setting')" class="mb-4" size="small" :bordered="true">
      <div class="flex flex-col space-y-2">
        <template v-if="cliStatus.exists">
          <div>{{ cliStatus.mfd_path }}</div>
          <div class="text-xs text-gray-400">
            {{ t('cli.pathDesc') }}
          </div>
          <div>
            <NButton ghost :disabled="cliBusy" :loading="cliBusy" @click="toggleCliPath">
              {{ cliStatus.installed ? t('cli.uninstall') : t('cli.install') }}
            </NButton>
          </div>
        </template>
        <template v-else>
          <div class="text-xs text-gray-400">
            {{ t('cli.notFound') }}
          </div>
        </template>
        <div>
          <NButton text type="primary" class="text-link" @click="openCliDoc">
            <template #icon>
              <NIcon>
                <DocumentTextOutline />
              </NIcon>
            </template>
            <span class="text-xs">{{ t('cli.doc') }}</span>
          </NButton>
        </div>
      </div>
    </NCard>
    <NCard :title="t('proxy.setting')" class="mb-4" size="small" :bordered="true">
      <ProxyEdit />
    </NCard>
    <NModal v-model:show="showCliDoc" preset="card" :title="t('cli.doc')" style="width: 80%; max-height: 80vh;" class="select-text">
      <div class="overflow-y-auto" style="max-height: 65vh">
        <MarkdownPreview :content="cliDocContent" />
      </div>
    </NModal>
  </div>
</template>
