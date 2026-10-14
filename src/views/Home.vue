<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { openPath } from '@tauri-apps/plugin-opener'
import HowToUse from './HowToUse.vue'
import SimilarResultsModal from '@/components/SimilarResultsModal.vue'
import MarkdownPreview from '@/components/MarkdownPreview.vue'
import { t } from '@/locales'
import SvgIcon from '@/components/SvgIcon.vue'
import { useIndexerStore } from '@/stores/indexer'
import { useSearch } from '@/composables/useSearch'

// Use search composable
const {
  SEMANTIC_SEARCH,
  KEYWORD_SEARCH,
  query,
  searchResults,
  localSearching,
  remoteDeviceSearching,
  searchType,
  searchDevices,
  selectedDeviceIds,
  searchStatuses,
  hasRemoteDevices,
  hitTypeLabels,
  isKeywordOnlyMatch,
  toggleDevice,
  selectAllDevices,
  getDeviceStatusIcon,
  getDeviceStatusColor,
  triggerSearch,
  clearSearch,
  loadClusterSetting,
  highlightText,
  downloadRemoteFile,
  cleanup: cleanupSearch,
} = useSearch()

// UI state
const extIcons = ['csv', 'doc', 'docx', 'html', 'json', 'mp3', 'mp4', 'pdf', 'ppt', 'pptx', 'psd', 'rar', 'txt', 'xls', 'xlsx']
const selectedIndex = ref(-1)
const inputRef = ref<HTMLInputElement | null>(null)
const isFocused = ref(false)
const indexerStore = useIndexerStore()
const parsedContent = ref('')
const showContentModal = ref(false)
const currentFilePath = ref('')
const showChunksModal = ref(false)
const matchChunks = ref<string[]>([])
const showSimilarOnPreviewClose = ref(false)

// Similar results modal ref
const similarModalRef = ref<InstanceType<typeof SimilarResultsModal> | null>(null)

const focusInput = () => {
  inputRef.value?.focus()
}

const blurInput = () => {
  inputRef.value?.blur()
}

function openFile(path = '') {
  openPath(path).catch((e: any) => {
    console.error('Failed to open file:', e)
    const msg = String(e).toLowerCase()
    if (msg.includes('not found') || msg.includes('does not exist') || msg.includes('找不到') || msg.includes('ENOENT'))
      window.$message.error(t('common.fileNotFound'))
    else
      window.$message.error(t('common.openFileFailed'))
  })
}

function handleContentModalClose() {
  if (showSimilarOnPreviewClose.value && similarModalRef.value) {
    similarModalRef.value.showModal = true
    showSimilarOnPreviewClose.value = false
  }
}

async function loadFileDetail(id = 0, deviceId?: string, restoreSimilar = false) {
  showContentModal.value = true
  showSimilarOnPreviewClose.value = restoreSimilar
  parsedContent.value = ''
  try {
    const fileInfo = await invoke<FileInfo>('load_file_detail', { fileId: id, deviceId })
    if (fileInfo) {
      parsedContent.value = fileInfo.content || ''
      currentFilePath.value = fileInfo.path
    }
  } catch (e) {
    console.log(e)
    window.$message.error(t('common.operationFailed'))
  }
}

async function loadChunks(ids: number[], keywords: string[], deviceId?: string) {
  showChunksModal.value = true
  matchChunks.value = []
  try {
    const chunks = await invoke<string[]>('load_chunks', { ids, deviceId })
    if (chunks) {
      if (keywords && keywords.length > 0) {
        matchChunks.value = chunks.map((chunk) => {
          return highlightText(chunk, keywords)
        })
      } else {
        matchChunks.value = chunks
      }
    }
  } catch (e) {
    console.log(e)
    window.$message.error(t('common.operationFailed'))
  }
}

function onClear() {
  clearSearch()
  selectedIndex.value = -1
  focusInput()
}

const keyDown = (e: any) => {
  if (e.ctrlKey && e.key === 'Tab') {
    searchType.value = searchType.value === SEMANTIC_SEARCH ? KEYWORD_SEARCH : SEMANTIC_SEARCH
    query.value = query.value.trim()
    focusInput()
    triggerSearch()
  } else if (e.key === 'Enter') {
    if (!isFocused.value && selectedIndex.value > -1) {
      const result = searchResults.value[selectedIndex.value]
      loadFileDetail(result.file_info.id, result.source_device?.device_id)
    } else { triggerSearch() }
  } else if (e.key === 'ArrowUp') {
    if (selectedIndex.value === 0) {
      focusInput()
      selectedIndex.value = -1
      return
    } else if (selectedIndex.value === -1) {
      selectedIndex.value = searchResults.value.length - 1
      return
    }
    blurInput()
    selectedIndex.value = Math.max(0, selectedIndex.value - 1)
  } else if (e.key === 'ArrowDown') {
    if (selectedIndex.value === searchResults.value.length - 1) {
      focusInput()
      selectedIndex.value = -1
      return
    } else if (selectedIndex.value === -1) {
      blurInput()
      selectedIndex.value = 0
      return
    }
    blurInput()
    selectedIndex.value = Math.min(searchResults.value.length - 1, selectedIndex.value + 1)
  } else if (e.key === 'Escape') {
    if (isFocused.value) {
      onClear()
    } else if (selectedIndex.value > -1) {
      selectedIndex.value = -1
      focusInput()
    }
  }
}

// Device event listener cleanup
let unlistenOnline: (() => void) | null = null
let unlistenOffline: (() => void) | null = null

onMounted(async () => {
  const indexerSetting = await invoke<IndexerSetting>('load_indexer_setting')
  indexerStore.setIndexerSetting(indexerSetting)

  // Load cluster setting and devices
  await loadClusterSetting()

  // Listen for device online/offline events
  unlistenOnline = await listen<string>('device-online', (event) => {
    const deviceId = event.payload
    const device = searchDevices.value.find(d => d.device_id === deviceId)
    if (device)
      device.online_status = 'online'
  })

  unlistenOffline = await listen<string>('device-offline', (event) => {
    const deviceId = event.payload
    const device = searchDevices.value.find(d => d.device_id === deviceId)
    if (device)
      device.online_status = 'offline'

    // Remove offline device from selected
    if (!selectedDeviceIds.value.includes(deviceId))
      return

    selectedDeviceIds.value = selectedDeviceIds.value.filter(id => id !== deviceId)
  })
})

onUnmounted(() => {
  unlistenOnline?.()
  unlistenOffline?.()
  cleanupSearch()
})

onActivated(() => {
  window.addEventListener('keydown', keyDown)
})

onDeactivated(() => {
  window.removeEventListener('keydown', keyDown)
})
</script>

<template>
  <div class="h-full flex flex-col items-center p-4 text-center">
    <div v-if="searchResults.length === 0" class="mb-4 flex items-center">
      <NImage
        src="/mango-desk.png" alt="MangoFinder" width="100" height="100"
        class="transition-all duration-300 hover:scale-105" style="opacity: 0.8; filter: saturate(0.9)"
        preview-disabled
      />
      <div class="text-sm text-gray-400 mt-2">
        {{ t('common.tagline') }}
      </div>
    </div>
    <div class="flex flex-col w-full justify-center space-x-2 max-w-[80%]">
      <div class="flex flex-row items-center justify-center space-x-2">
        <NInput
          ref="inputRef" v-model:value="query" class="flex-1 min-w-[100px] text-left" clearable
          :placeholder="searchType === KEYWORD_SEARCH ? t('common.keywordSearchTip.description') : t('common.semanticSearchTip.description')"
          @input="triggerSearch" @focus="isFocused = true" @blur="isFocused = false" @clear="onClear"
        >
          <template #prefix>
            <span
              class="text-link font-medium"
              @click="searchType = searchType === SEMANTIC_SEARCH ? KEYWORD_SEARCH : SEMANTIC_SEARCH"
            >
              <span v-if="searchType === SEMANTIC_SEARCH" class="pr-2">
                {{ t('common.semantic') }}
              </span>
              <span v-else class="pr-2">
                {{ t('common.keyword') }}
              </span>
            </span>
          </template>
        </NInput>
      </div>

      <!-- Device filter (only show when cluster is enabled and has remote devices) -->
      <div v-if="hasRemoteDevices && searchDevices.length > 0" class="mt-2 flex items-center gap-2 flex-wrap">
        <span class="text-xs text-gray-500">{{ t('cluster.deviceFilter') }}:</span>
        <div class="flex items-center gap-1 flex-wrap">
          <NTag
            v-for="device in searchDevices" :key="device.device_id"
            :type="selectedDeviceIds.includes(device.device_id) ? 'primary' : 'default'"
            :bordered="selectedDeviceIds.includes(device.device_id)" round size="small" class="cursor-pointer"
            @click="toggleDevice(device.device_id)"
          >
            <span :class="getDeviceStatusColor(device.online_status)" class="mr-1">
              {{ getDeviceStatusIcon(device.online_status) }}
            </span>
            <span v-if="device.is_local">{{ t('cluster.localDevice') }}</span>
            <span v-else>{{ device.device_name }}</span>
            <span class="text-xs text-gray-400 ml-1">({{ device.index_count }})</span>
          </NTag>
          <NButton
            v-if="selectedDeviceIds.length !== searchDevices.length" size="tiny" quaternary
            @click="selectAllDevices"
          >
            {{ t('cluster.selectAll') }}
          </NButton>
        </div>
      </div>

      <!-- Search status indicator -->
      <div v-if="localSearching || remoteDeviceSearching" class="mt-2 text-xs text-gray-500">
        <div class="flex items-center gap-4">
          <div v-if="localSearching" class="flex items-center gap-1">
            <NSpin size="small" />
            <span>{{ t('cluster.localSearching') }}</span>
          </div>
          <div v-if="remoteDeviceSearching" class="flex items-center gap-1">
            <NSpin size="small" />
            <span>{{ t('cluster.remoteSearching') }}</span>
          </div>
        </div>
      </div>

      <!-- Device search statuses (show during cross-device search) -->
      <div v-if="searchStatuses.length > 0 && remoteDeviceSearching" class="mt-2 text-xs text-gray-500">
        <div class="flex items-center gap-4">
          <span v-for="status in searchStatuses" :key="status.device_id" class="flex items-center gap-1">
            <NSpin v-if="status.status === 'Searching' || status.status === 'Pending'" size="small" />
            <span v-else-if="status.status === 'Completed'" class="text-green-500">✓</span>
            <span v-else-if="status.status === 'Failed'" class="text-red-500">✗</span>
            <span>{{ status.device_name }}</span>
            <span v-if="status.status === 'Completed'" class="text-gray-400">({{ status.result_count }})</span>
            <span v-if="status.status === 'Failed'" class="text-red-400 text-xs">{{ status.error }}</span>
          </span>
        </div>
      </div>

      <div v-if="searchResults.length === 0" class="mt-2 text-xs text-gray-400 w-full text-left">
        <div>{{ t('common.semanticSearchTip.title') }}：{{ t('common.semanticSearchTip.description') }}</div>
        <div>
          {{ t('common.keywordSearchTip.title') }}：{{ t('common.keywordSearchTip.description') }}, {{
            t('common.keywordSearchTip.example') }}
        </div>
      </div>
    </div>
    <div
      class="flex-1 flex flex-col w-full items-center justify-start mt-4"
      :class="searchResults.length > 0 ? 'border-t border-(--border-color)' : ''"
    >
      <div v-if="searchResults.length === 0" class="flex flex-col mt-8 h-full space-y-4">
        <!-- Empty state when search returned no results -->
        <div v-if="query.trim().length > 0 && !localSearching && !remoteDeviceSearching" class="flex flex-col items-center space-y-2">
          <div class="text-base text-gray-400">
            {{ t('common.searchEmpty') }}
          </div>
          <div class="text-xs text-gray-400">
            {{ t('common.searchEmptyTip') }}
          </div>
        </div>
        <!-- Keyborad Shortcuts -->
        <div v-if="query.trim().length === 0" class="text text-sm text-gray-400 text-left">
          {{ t('common.searchKeyboradShortcuts') }}
        </div>
        <div v-if="query.trim().length === 0" class="flex space-x-2 text-sm text-gray-500">
          <div class="w-[65px] text-left">
            <kbd class="px-2 py-1 text-xs bg-gray-100 dark:bg-gray-700 dark:text-gray-200 rounded">Ctrl+Tab</kbd>
          </div>
          <span>{{ t('common.switchSearchMode') }}</span>
        </div>
        <div v-if="query.trim().length === 0" class="flex space-x-2 text-sm text-gray-500">
          <div class="w-[65px] text-left">
            <kbd class="px-2 py-1 text-xs bg-gray-100 dark:bg-gray-700 dark:text-gray-200 rounded">Esc</kbd>
          </div>
          <span>{{ t('common.searchClearTip') }}</span>
        </div>
        <div v-if="query.trim().length === 0" class="text text-sm text-gray-400 text-left">
          {{ t('common.resultKeyboradShortcuts') }}
        </div>
        <div v-if="query.trim().length === 0" class="flex space-x-2 text-sm text-gray-500">
          <div class="w-[65px] text-left">
            <kbd class="px-2 py-1 text-xs bg-gray-100 dark:bg-gray-700 dark:text-gray-200 rounded">↑↓</kbd>
          </div>
          <span>{{ t('common.navigateTip') }}</span>
        </div>
        <div v-if="query.trim().length === 0" class="flex space-x-2 text-sm text-gray-500">
          <div class="w-[65px] text-left">
            <kbd class="px-2 py-1 text-xs bg-gray-100 dark:bg-gray-700 dark:text-gray-200 rounded">Enter</kbd>
          </div>
          <span>{{ t('common.openTip') }}</span>
        </div>
        <div v-if="query.trim().length === 0" class="flex space-x-2 text-sm text-gray-500">
          <div class="w-[65px] text-left">
            <kbd class="px-2 py-1 text-xs bg-gray-100 dark:bg-gray-700 dark:text-gray-200 rounded">Esc</kbd>
          </div>
          <span>{{ t('common.clearTip') }}</span>
        </div>
      </div>

      <NImageGroup v-else>
        <div
          v-for="(item, idx) in searchResults" :key="`${item.file_info.path}-${idx}`"
          class="group w-full p-2 border-b border-(--border-color)"
          :style="selectedIndex === idx ? 'background-color: var(--secondary-bg-color);border: 1px solid var(--primary-color); box-sizing: border-box;border-radius: 0.25rem;' : ''"
        >
          <!-- Icon + File info -->
          <div class="flex space-x-2">
            <!-- Large image: top aligned -->
            <div
              v-if="item.file_info.file_data && item.file_info.category === 2"
              class="flex justify-center items-start shrink-0 pt-0.5"
            >
              <NImage width="100" :src="item.file_info.file_data" />
            </div>
            <!-- Small icon: vertically centered -->
            <div v-else class="flex justify-center items-center shrink-0">
              <NImage v-if="item.file_info.file_data" width="40" height="40" :src="item.file_info.file_data" />
              <div
                v-else-if="!item.file_info.file_data && !extIcons.includes(item.file_info.file_ext.toLowerCase())"
                class="w-10 h-10 flex justify-center items-center text-sm font-bold"
                style="opacity: 0.7;filter: saturate(0.5)"
              >
                {{ item.file_info.file_ext.toUpperCase() }}
              </div>
              <SvgIcon
                v-else :name="item.file_info.file_ext.toLowerCase()" width="40" height="40"
                style="opacity: 0.7;filter: saturate(0.5)"
              />
            </div>
            <div class="flex-1 flex flex-col justify-between text-left min-w-0 min-h-14">
              <div class="min-h-11">
                <!-- First row: File name + Source device -->
                <div class="flex justify-between items-center gap-2">
                  <div class="text-link truncate" @click="loadFileDetail(item.file_info.id, item.source_device?.device_id)">
                    {{ item.file_info.name }}
                  </div>
                  <NTooltip v-if="item.source_device">
                    <template #trigger>
                      <div class="flex items-center gap-1 shrink-0 text-xs">
                        <span>🖥️</span>
                        <span class="max-w-24 truncate">{{ item.source_device.device_name }}</span>
                      </div>
                    </template>
                    {{ t('cluster.sourceDevice') }}: {{ item.source_device.device_name }}
                  </NTooltip>
                </div>
                <div class="text-xs truncate">
                  <div v-html="item.file_info.html_path" />
                </div>
              </div>
              <!-- Third row: Actions + Metadata -->
              <div class="flex justify-between items-center text-xs text-gray-400">
                <div class="flex items-center gap-2">
                  <NButton size="tiny" ghost @click="similarModalRef?.findSimilars(item.file_info, item.source_device?.device_id)">
                    {{ t('common.findSimilar') }}
                  </NButton>
                  <NButton
                    v-if="item.matched_chunk_ids && item.matched_chunk_ids.length > 0" size="tiny" ghost
                    @click="loadChunks(item.matched_chunk_ids, item.matched_keywords, item.source_device?.device_id)"
                  >
                    {{ t('common.matchedSegments', { count: item.matched_chunk_ids.length }) }}
                  </NButton>
                  <NButton v-if="item.source_device" size="tiny" ghost @click="downloadRemoteFile(item)">
                    {{ t('common.download') }}
                  </NButton>
                </div>
                <div class="flex items-center gap-2">
                  <NTooltip v-if="item.hit_types && item.hit_types.length > 0">
                    <template #trigger>
                      <span class="flex gap-1">
                        <AppTag v-for="hitType in item.hit_types" :key="hitType">
                          {{ hitTypeLabels[hitType] || hitType }}
                        </AppTag>
                      </span>
                    </template>
                    {{ t('common.hitTypeTip') }}
                  </NTooltip>
                  <NTooltip v-if="item.score && !isKeywordOnlyMatch(item)">
                    <template #trigger>
                      <AppTag>
                        {{ item.score }}%
                      </AppTag>
                    </template>
                    {{ t('common.score') }}: {{ item.score }}%
                  </NTooltip>
                </div>
              </div>
            </div>
          </div>
        </div>
      </NImageGroup>
    </div>
    <HowToUse v-if="searchResults.length === 0 && query.trim().length === 0" />
    <NModal
      v-model:show="showContentModal" preset="card" :title="t('indexer.parsedContent')"
      style="width: 80%; height:80%;"
      @after-leave="handleContentModalClose"
    >
      <div style="max-height: 600px;overflow-y: auto;" class="select-text">
        <MarkdownPreview :content="parsedContent" />
      </div>
      <template #footer>
        <div style="display: flex; justify-content: flex-end;">
          <NButton ghost size="small" @click="openFile(currentFilePath)">
            {{ t('common.openFile') }}
          </NButton>
        </div>
      </template>
    </NModal>
    <NModal
      v-model:show="showChunksModal" preset="card" :title="t('common.matchedSegments')"
      style="width: 80%; height:80%;"
    >
      <div style="max-height: 600px;overflow-y: auto;" class="select-text">
        <div v-if="matchChunks.length > 0">
          <div v-for="(chunk, index) in matchChunks" :key="index" class="mb-4">
            <div class="mb-2">
              <strong>{{ t('common.segment') }} {{ index + 1 }}:</strong>
            </div>
            <div v-html="chunk" />
          </div>
        </div>
        <div v-else>
          {{ t('common.noData') }}
        </div>
      </div>
    </NModal>
    <SimilarResultsModal ref="similarModalRef" :file-id="null" @preview-file="(id, deviceId) => loadFileDetail(id, deviceId, true)" />
  </div>
</template>

<style scoped></style>
