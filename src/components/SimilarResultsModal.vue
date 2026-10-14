<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { NImage, NModal, NSpin, NTooltip } from 'naive-ui'
import { computed, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import SvgIcon from './SvgIcon.vue'

const emit = defineEmits<{
  'previewFile': [fileId: number, deviceId?: string]
}>()

const { t } = useI18n()

const showModal = ref(false)
const loading = ref(false)
const sourceFile = ref<FileInfo | null>(null)
const results = ref<SearchResult[]>([])

// Track created Blob URLs for cleanup
const createdBlobUrls = new Set<string>()

// Cleanup Blob URLs on unmount
onUnmounted(() => {
  createdBlobUrls.forEach(url => URL.revokeObjectURL(url))
  createdBlobUrls.clear()
})

const similarityTypeLabels = computed<Record<string, string>>(() => ({
  imageHash: t('common.similarityType.imageHash'),
  imageSemantic: t('common.similarityType.imageSemantic'),
  documentSemantic: t('common.similarityType.documentSemantic'),
  audioFingerprint: t('common.similarityType.audioFingerprint'),
  audioTranscription: t('common.similarityType.audioTranscription'),
}))

/**
 * Find similar files by file ID
 * 根据文件 ID 查找相似文件
 * @param fileInfo File info object
 * @param sourceDeviceId Optional source device ID for remote files
 */
async function findSimilars(fileInfo: FileInfo, sourceDeviceId?: string) {
  showModal.value = true
  loading.value = true
  sourceFile.value = fileInfo
  results.value = []

  try {
    // Load source file image data if it's an image (only for local files)
    if (!sourceDeviceId && fileInfo.category === 2 && !fileInfo.file_data) {
      try {
        const resp = await invoke<ArrayBuffer>('read_file_data', { path: fileInfo.path })
        if (resp) {
          const mimeType = fileInfo.file_ext.toLowerCase() === 'png' ? 'image/png' : 'image/jpeg'
          const uint8Array = new Uint8Array(resp)
          const blob = new Blob([uint8Array], { type: mimeType })
          const url = URL.createObjectURL(blob)
          createdBlobUrls.add(url)
          fileInfo.file_data = url
        }
      } catch (e) {
        console.warn('Failed to load source image:', fileInfo.path, e)
      }
    }

    // Load source file image data if it's a remote image
    if (sourceDeviceId && fileInfo.category === 2 && !fileInfo.file_data) {
      try {
        const resp = await invoke<ArrayBuffer>('fetch_remote_file', {
          device_id: sourceDeviceId,
          file_id: fileInfo.id,
        })
        if (resp) {
          const mimeType = fileInfo.file_ext.toLowerCase() === 'png' ? 'image/png' : 'image/jpeg'
          const uint8Array = new Uint8Array(resp)
          const blob = new Blob([uint8Array], { type: mimeType })
          const url = URL.createObjectURL(blob)
          createdBlobUrls.add(url)
          fileInfo.file_data = url
        }
      } catch (e) {
        console.warn('Failed to load remote source image:', fileInfo.path, e)
      }
    }

    const searchResults = await invoke<SearchResult[]>('find_similars_by_file_id', {
      fileId: fileInfo.id,
      sourceDeviceId: sourceDeviceId || null,
      limit: 20,
    })

    // Load image data for image files (category === 2)
    const imagePromises = searchResults
      .filter(item => item.file_info.category === 2)
      .map((item) => {
        // Remote image: use fetch_remote_file
        if (item.source_device) {
          return invoke<ArrayBuffer>('fetch_remote_file', {
            device_id: item.source_device.device_id,
            file_id: item.file_info.id,
          }).then((resp) => {
            if (resp) {
              const mimeType = item.file_info.file_ext.toLowerCase() === 'png' ? 'image/png' : 'image/jpeg'
              const uint8Array = new Uint8Array(resp)
              const blob = new Blob([uint8Array], { type: mimeType })
              item.file_info.file_data = URL.createObjectURL(blob)
              createdBlobUrls.add(item.file_info.file_data)
            }
          }).catch((e) => {
            console.warn('Failed to load remote image:', item.source_device?.device_name, item.file_info.name, e)
          })
        }
        // Local image: use read_file_data
        return invoke<ArrayBuffer>('read_file_data', { path: item.file_info.path }).then((resp) => {
          if (resp) {
            const mimeType = item.file_info.file_ext.toLowerCase() === 'png' ? 'image/png' : 'image/jpeg'
            const uint8Array = new Uint8Array(resp)
            const blob = new Blob([uint8Array], { type: mimeType })
            item.file_info.file_data = URL.createObjectURL(blob)
            createdBlobUrls.add(item.file_info.file_data)
          }
        }).catch((e) => {
          console.warn('Failed to load image:', item.file_info.path, e)
        })
      })

    await Promise.all(imagePromises)
    results.value = searchResults
  } catch (e) {
    console.error('Failed to find similar files:', e)
    window.$message.error(t('common.operationFailed'))
  } finally {
    loading.value = false
  }
}

function previewFile(fileId: number, deviceId?: string) {
  showModal.value = false
  emit('previewFile', fileId, deviceId)
}

// Expose findSimilars for parent component
defineExpose({
  findSimilars,
  showModal,
})
</script>

<template>
  <NModal v-model:show="showModal" preset="card" :title="t('common.similarFiles')" style="width: 80%; max-height: 80vh;" class="select-text">
    <div style="max-height: calc(80vh - 90px); overflow-y: auto;">
      <!-- Source file section -->
      <div v-if="sourceFile" class="mb-4">
        <div class="text-sm font-medium text-gray-600 dark:text-gray-300 mb-2">
          {{ t('common.sourceFile') }}
        </div>
        <div class="p-2 bg-gray-50 dark:bg-gray-800 rounded">
          <div class="flex items-start gap-3">
            <!-- Source image preview -->
            <div v-if="sourceFile.category === 2 && sourceFile.file_data" class="shrink-0">
              <NImage width="100" :src="sourceFile.file_data" />
            </div>
            <div class="flex-1 min-w-0">
              <div class="text-link truncate" @click="previewFile(sourceFile.id)">
                {{ sourceFile.name }}
              </div>
              <div class="text-xs text-gray-400 truncate">
                {{ sourceFile.path }}
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- Loading state -->
      <div v-if="loading" class="flex justify-center p-4">
        <NSpin />
      </div>

      <!-- Similar files section -->
      <div v-else-if="results.length > 0">
        <div class="text-sm font-medium text-gray-600 dark:text-gray-300 mb-2">
          {{ t('common.similarFiles') }} ({{ results.length }})
        </div>
        <div>
          <div
            v-for="item in results" :key="item.file_info.path"
            class="group w-full p-2 border-b border-(--border-color) hover:bg-gray-50 dark:hover:bg-gray-800"
          >
            <div class="flex space-x-2">
              <!-- Image preview -->
              <div
                v-if="item.file_info.file_data && item.file_info.category === 2"
                class="flex justify-center items-start shrink-0 pt-0.5"
              >
                <NImage width="100" :src="item.file_info.file_data" />
              </div>
              <!-- File icon -->
              <div v-else class="flex justify-center items-center shrink-0">
                <SvgIcon
                  :name="item.file_info.file_ext.toLowerCase()" width="40" height="40"
                  style="opacity: 0.7; filter: saturate(0.5)"
                />
              </div>

              <!-- File info -->
              <div class="flex-1 flex flex-col justify-between text-left min-w-0">
                <div>
                  <!-- First row: File name + Source device -->
                  <div class="flex justify-between items-center gap-2">
                    <div class="text-link truncate" @click="previewFile(item.file_info.id, item.source_device?.device_id)">
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
                  <div class="text-xs text-gray-400 truncate">
                    {{ item.file_info.path }}
                  </div>
                </div>
                <!-- Similarity type and score -->
                <div class="flex justify-end items-center text-xs gap-2">
                  <NTooltip v-if="item.similarity_type">
                    <template #trigger>
                      <AppTag>
                        {{ similarityTypeLabels[item.similarity_type] || item.similarity_type }}
                      </AppTag>
                    </template>
                    {{ t('common.similarityTypeTip') }}
                  </NTooltip>
                  <NTooltip>
                    <template #trigger>
                      <AppTag>
                        {{ item.score }}%
                      </AppTag>
                    </template>
                    {{ t('common.similarityScore') }}: {{ item.score }}%
                  </NTooltip>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- Empty state -->
      <div v-else class="p-4 text-gray-400 text-center">
        {{ t('common.noSimilarFiles') }}
      </div>
    </div>
  </NModal>
</template>
