<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { relaunch } from '@tauri-apps/plugin-process'
import { useAppStore } from '@/stores/app'
import { setLocale, t } from '@/locales'
import { emptyProxyInfo } from '@/utils/functions'

const dataPath = ref('')
const appStore = useAppStore()
const activePlatform = ref('openai')
const activeTheme = ref(appStore.getTheme)
const activeLanguage = ref('en-US')
const activeTab = ref('openai')
const proxy = ref<ProxyInfo>(emptyProxyInfo())
const needRestart = ref(false)
const dataCopying = ref(false)
const cliStatus = ref<{ mfd_path: string, exists: boolean, installed: boolean }>({ mfd_path: '', exists: false, installed: false })
const cliBusy = ref(false)

async function loadCliStatus() {
  try {
    cliStatus.value = await invoke('cli_path_status')
  }
  catch (err) {
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
  }
  catch (err: any) {
    window.$message.error(typeof err === 'string' ? err : t('common.operationFailed'))
  }
  finally {
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

function handleSaveProxy() {
  console.log('onSaveProxy', proxy.value)
  invoke('update_proxy_info', { proxyInfo: proxy.value }).then((res) => {
    console.log('update_proxy_info result', res)
    window.$message.success(t('common.saveSuccess'))
  }).catch((err) => {
    console.error('update_proxy_info error', err)
    window.$message.error(t('common.operationFailed'))
  })
}

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

onMounted(async () => {
  console.log('common setting onMounted')
  const activePlatformName = await invoke('load_active_platform')
  activePlatform.value = activePlatformName as string
  activeTab.value = activePlatform.value
  const activeLocale = await invoke('load_active_locale')
  console.log('active_locale', activeLocale)
  setLocale(activeLocale as 'en-US' | 'zh-CN')
  activeLanguage.value = activeLocale as string
  const proxyInfo = await invoke('load_proxy_info')
  proxy.value = proxyInfo as ProxyInfo
  const userDataPath = await invoke<string>('get_data_path')
  dataPath.value = userDataPath
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
      </div>
    </NCard>
    <NCard :title="t('proxy.setting')" class="mb-4" size="small" :bordered="true">
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
          <NButton ghost @click="handleSaveProxy">
            {{ t('common.save') }}
          </NButton>
        </div>
      </div>
    </NCard>
  </div>
</template>
