<script setup lang="ts">
import { useRoute } from 'vue-router'
import CommonSetting from './CommonSetting.vue'
import IndexerSetting from './IndexerSetting.vue'
import StorageSetting from './StorageSetting.vue'
import ClusterSetting from './ClusterSetting.vue'
import About from './About.vue'
import { useSettingStore } from '@/stores/setting'
import { t } from '@/locales'

const route = useRoute()
const settingStore = useSettingStore()

const activeTab = computed(() => {
  return settingStore.activeTab
})

// Watch route query changes to switch tab
watch(() => route.query.tab, (tab) => {
  if (tab)
    settingStore.changeTab(tab as string)
}, { immediate: true })

function onUpdateTab(tabName: string) {
  settingStore.changeTab(tabName)
}
</script>

<template>
  <div class="h-full mx-auto">
    <NTabs
      :value="activeTab" type="line" size="large" :tabs-padding="20"
      pane-style=" height: 100%; overflow-y: auto;padding: 20px;" style="height: 100%;" @update:value="onUpdateTab"
    >
      <NTabPane name="common" display-directive="show" :tab="t('common.commonSetting')">
        <CommonSetting />
      </NTabPane>
      <NTabPane name="indexer" display-directive="show" :tab="t('indexer.setting')">
        <IndexerSetting />
      </NTabPane>
      <NTabPane name="storage" display-directive="show" :tab="t('storage.title')">
        <StorageSetting />
      </NTabPane>
      <NTabPane name="cluster" display-directive="show" :tab="t('cluster.title')">
        <ClusterSetting />
      </NTabPane>
      <NTabPane name="about" :tab="t('menu.about')">
        <About />
      </NTabPane>
    </NTabs>
  </div>
</template>

<style scoped></style>
