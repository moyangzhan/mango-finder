import { invoke } from '@tauri-apps/api/core'
import { useDebounceFn } from '@vueuse/core'
import { t } from '@/locales'

const SEMANTIC_SEARCH = 1
const KEYWORD_SEARCH = 2

export function useSearch() {
  // Search state
  const query = ref('')
  const searchResults = ref<SearchResult[]>([])
  const localResults = ref<SearchResult[]>([])
  const remoteResults = ref<SearchResult[]>([])
  const localSearching = ref(false)
  const remoteDeviceSearching = ref(false)
  const searchPhase = ref<'idle' | 'local' | 'cross-device'>('idle')
  const searchType = ref(SEMANTIC_SEARCH)
  const searchGeneration = ref(0)
  // One-line degradation notice for semantic searches (backend unavailable /
  // rebuild in progress); cleared on every new semantic search
  const semanticNotice = ref<string | null>(null)

  // Cluster search state
  const searchDevices = ref<SearchDevice[]>([])
  const selectedDeviceIds = ref<string[]>([])
  const searchStatuses = ref<SearchStatus[]>([])
  const clusterEnabled = ref(false)
  const lastDeviceCheckTime = ref(0) // Last device status check timestamp (ms)

  // Track created Blob URLs for cleanup
  const createdBlobUrls = ref<Set<string>>(new Set())

  // Computed
  const selectedLocalDevice = computed(() => {
    const localDevice = searchDevices.value.find(d => d.is_local)
    return localDevice && selectedDeviceIds.value.includes(localDevice.device_id)
  })

  const hasRemoteDevices = computed(() => {
    return clusterEnabled.value && searchDevices.value.some(d => !d.is_local && d.online_status === 'online')
  })

  const hitTypeLabels = computed<Record<string, string>>(() => ({
    pathKeyword: t('common.hitType.pathKeyword'),
    contentKeyword: t('common.hitType.contentKeyword'),
    contentSemantic: t('common.hitType.contentSemantic'),
    metaSemantic: t('common.hitType.metaSemantic'),
  }))

  // Check if cluster is enabled and has remote devices
  const isKeywordOnlyMatch = (item: SearchResult): boolean => {
    if (!item.hit_types || item.hit_types.length === 0)
      return false
    const keywordTypes = ['pathKeyword', 'contentKeyword']
    return item.hit_types.every(type => keywordTypes.includes(type)) && !item.similarity_type
  }

  // Get selected device IDs for API call
  const getSelectedDeviceIds = (): string[] | undefined => {
    if (selectedDeviceIds.value.length === 0 || selectedDeviceIds.value.length === searchDevices.value.length)
      return undefined

    return selectedDeviceIds.value
  }

  // Highlight helper
  const escapeRegExp = (str: string) => {
    return str.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  }

  const escapeHtml = (str: string) => {
    return str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
  }

  const highlightText = (text: string, keywords: string[]) => {
    if (!text || !keywords || keywords.length === 0)
      return escapeHtml(text)
    const escaped = escapeHtml(text)
    return keywords.reduce((html, keyword) => {
      const safeKeyword = escapeHtml(keyword)
      const regex = new RegExp(`(${escapeRegExp(safeKeyword)})`, 'gi')
      return html.replace(
        regex,
        '<span class="font-bold text-(--match-word-color)">$1</span>',
      )
    }, escaped)
  }

  // Deduplicate results by device + path, keeping higher score
  const dedupeResults = (results: SearchResult[]): SearchResult[] => {
    const map = new Map<string, SearchResult>()
    for (const r of results) {
      // Use device_id + path as unique key (local files have no source_device)
      const deviceKey = r.source_device?.device_id || 'local'
      const uniqueKey = `${deviceKey}:${r.file_info.path}`
      const existing = map.get(uniqueKey)
      if (!existing || r.score > existing.score)
        map.set(uniqueKey, r)
    }
    return Array.from(map.values()).sort((a, b) => b.score - a.score)
  }

  // Merge and dedupe results
  const filterResults = () => {
    const merged = [...localResults.value, ...remoteResults.value]
    searchResults.value = dedupeResults(merged)
  }

  // Toggle device selection
  const toggleDevice = (deviceId: string) => {
    const index = selectedDeviceIds.value.indexOf(deviceId)
    if (index > -1)
      selectedDeviceIds.value.splice(index, 1)
    else
      selectedDeviceIds.value.push(deviceId)

    filterResults()
  }

  // Select all devices
  const selectAllDevices = () => {
    selectedDeviceIds.value = searchDevices.value.map(d => d.device_id)
    filterResults()
  }

  // Get status icon for device
  const getDeviceStatusIcon = (status: string) => {
    switch (status) {
      case 'online': return '●'
      case 'offline': return '○'
      default: return '◐'
    }
  }

  // Get status color for device
  const getDeviceStatusColor = (status: string) => {
    switch (status) {
      case 'online': return 'text-green-500'
      case 'offline': return 'text-gray-400'
      default: return 'text-yellow-500'
    }
  }

  // Cleanup old Blob URLs to prevent memory leaks
  const cleanupBlobUrls = () => {
    createdBlobUrls.value.forEach((url) => {
      URL.revokeObjectURL(url)
    })
    createdBlobUrls.value.clear()
  }

  // Process results for display (highlight, load images)
  const processResults = (results: SearchResult[]) => {
    results.forEach((item) => {
      if (item.hit_types.includes('pathKeyword') && item.matched_keywords.length > 0)
        item.file_info.html_path = highlightText(item.file_info.path, item.matched_keywords)
      else
        item.file_info.html_path = item.file_info.path

      if (item.file_info.category !== 2)
        return

      // Load image data for display
      if (item.source_device) {
        // Remote image: use fetch_remote_file
        invoke('fetch_remote_file', {
          deviceId: item.source_device.device_id,
          fileId: item.file_info.id,
        }).then((resp) => {
          if (!resp)
            throw new Error('No image data received')
          const mimeType = item.file_info.file_ext.toLowerCase() === 'png' ? 'image/png' : 'image/jpeg'
          const uint8Array = new Uint8Array(resp as ArrayBuffer)
          const blob = new Blob([uint8Array], { type: mimeType })
          const imageUrl = URL.createObjectURL(blob)
          createdBlobUrls.value.add(imageUrl)
          item.file_info.file_data = imageUrl
        }).catch((e) => {
          console.warn('Failed to load remote image:', item.source_device?.device_name, item.file_info.name, e)
        })
      } else {
        // Local image: use read_file_data
        invoke('read_file_data', { path: item.file_info.path }).then((resp) => {
          if (!resp)
            throw new Error('No image data received')
          const mimeType = item.file_info.file_ext.toLowerCase() === 'png' ? 'image/png' : 'image/jpeg'
          const uint8Array = new Uint8Array(resp as ArrayBuffer)
          const blob = new Blob([uint8Array], { type: mimeType })
          const imageUrl = URL.createObjectURL(blob)
          createdBlobUrls.value.add(imageUrl)
          item.file_info.file_data = imageUrl
        }).catch((e) => {
          console.warn('Failed to load local image:', item.file_info.name, e)
        })
      }
    })
  }

  // Local search - fast response
  const localSearch = async () => {
    if (!query.value) {
      searchResults.value = []
      localResults.value = []
      semanticNotice.value = null
      return
    }

    const query_txt = query.value.trim()
    if (query_txt.length < 2) {
      window.$message.warning(t('common.queryTooShort'))
      return
    }

    // Revoke old blob URLs before replacing results to prevent memory leak
    cleanupBlobUrls()

    const currentGen = searchGeneration.value

    localSearching.value = true
    searchPhase.value = 'local'
    searchStatuses.value = []

    try {
      const search_type_str = searchType.value === KEYWORD_SEARCH ? 'keyword' : 'semantic'
      const res = await invoke<LocalDeviceSearchResult>('local_device_search', {
        query: query_txt,
        searchType: search_type_str,
      })

      if (currentGen !== searchGeneration.value) {
        console.log('Local search response stale, ignoring')
        return
      }

      localResults.value = res.results
      semanticNotice.value = search_type_str === 'semantic' ? (res.semantic_notice ?? null) : null
      processResults(localResults.value)
      filterResults()

      if (searchResults.value.length === 0 && !hasRemoteDevices.value)
        window.$message.warning(t('common.noData'))
    } catch (e: any) {
      console.log('Local search error:', e)
      if (currentGen === searchGeneration.value) {
        localResults.value = []
        if (typeof e === 'string' && e.length > 0)
          window.$message.error(t('common.searchFailed'))
      }
    } finally {
      // Always reset localSearching, but only update phase if this is the current search
      localSearching.value = false
      if (currentGen === searchGeneration.value && !remoteDeviceSearching.value)
        searchPhase.value = 'idle'
    }
  }

  // Remote device search - slower, progressive enhancement
  const searchRemoteDevices = async () => {
    if (!query.value || !hasRemoteDevices.value)
      return

    const query_txt = query.value.trim()
    if (query_txt.length < 2)
      return

    const currentGen = searchGeneration.value

    remoteDeviceSearching.value = true
    searchPhase.value = 'cross-device'
    searchStatuses.value = []

    try {
      const search_type_str = searchType.value === KEYWORD_SEARCH ? 'keyword' : 'semantic'
      const deviceIds = getSelectedDeviceIds()

      const result = await invoke<RemoteDeviceSearchResult>('remote_device_search', {
        query: query_txt,
        searchType: search_type_str,
        deviceIds,
      })

      if (currentGen !== searchGeneration.value) {
        console.log('Remote device search response stale, ignoring')
        return
      }

      searchStatuses.value = result.statuses
      remoteResults.value = result.results

      processResults(remoteResults.value)
      filterResults()

      if (searchResults.value.length === 0 && localResults.value.length === 0)
        window.$message.warning(t('common.noData'))
    } catch (e: any) {
      console.log('Remote device search error:', e)
      if (currentGen === searchGeneration.value) {
        remoteResults.value = []
        filterResults()
        if (typeof e === 'string' && e.length > 0)
          window.$message.error(t('common.searchFailed'))
      }
    } finally {
      if (currentGen === searchGeneration.value) {
        remoteDeviceSearching.value = false
        searchPhase.value = 'idle'
      }
    }
  }

  // Debounced search functions
  let debounceLocalSearch = useDebounceFn(localSearch, 600)
  const debounceRemoteDeviceSearch = useDebounceFn(searchRemoteDevices, 1000)

  // Load search devices
  const loadSearchDevices = async () => {
    try {
      const devices = await invoke<SearchDevice[]>('list_online_devices')
      searchDevices.value = devices
      // Default to local device only
      if (selectedDeviceIds.value.length === 0) {
        const localDevice = devices.find(d => d.is_local)
        if (localDevice)
          selectedDeviceIds.value = [localDevice.device_id]
      }
    } catch (e) {
      console.log('Failed to load search devices:', e)
    }
  }

  // Trigger search (called from input)
  const triggerSearch = () => {
    // Bump generation so any in-flight local/remote results from previous queries are rejected
    searchGeneration.value++

    // Check if all devices are deselected when cluster is enabled
    if (clusterEnabled.value && searchDevices.value.length > 0 && selectedDeviceIds.value.length === 0) {
      window.$message.warning(t('cluster.selectDeviceRequired'))
      return
    }

    // Refresh device list and check status (min 5s interval) before searching
    // 搜索前刷新设备列表并检查状态（最短间隔5秒）
    if (clusterEnabled.value) {
      const now = Date.now()
      const MIN_CHECK_INTERVAL = 5000 // 5 seconds

      if (now - lastDeviceCheckTime.value >= MIN_CHECK_INTERVAL) {
        lastDeviceCheckTime.value = now
        // Trigger lightweight status check (ping only, no mDNS restart)
        // 触发轻量级状态检查（仅 ping，不重启 mDNS）
        invoke('check_devices_status').catch(e => console.log('Failed to check devices status:', e))
        // Refresh device list alongside status check
        loadSearchDevices()
      }
    }

    // Always search locally unless cluster is enabled and local device is explicitly deselected
    if (!clusterEnabled.value || selectedLocalDevice.value) {
      debounceLocalSearch()
    } else {
      // Local leg skipped: a stale notice from the last local search must not
      // linger over remote-only results
      semanticNotice.value = null
    }
    if (hasRemoteDevices.value)
      debounceRemoteDeviceSearch()
  }

  // Update debounce timing based on search type
  watch(searchType, (newVal) => {
    const debounceTime = newVal === KEYWORD_SEARCH ? 300 : 600
    debounceLocalSearch = useDebounceFn(localSearch, debounceTime)
    triggerSearch()
  })

  // Clear search
  const clearSearch = () => {
    cleanupBlobUrls()
    query.value = ''
    searchResults.value = []
    localResults.value = []
    remoteResults.value = []
    searchStatuses.value = []
    searchPhase.value = 'idle'
    semanticNotice.value = null
  }

  // Load cluster setting
  const loadClusterSetting = async () => {
    try {
      const clusterSetting = await invoke<ClusterSetting>('load_cluster_setting')
      clusterEnabled.value = clusterSetting.enabled
      if (clusterSetting.enabled)
        await loadSearchDevices()
    } catch (e) {
      console.log('Failed to load cluster setting:', e)
    }
  }

  // Get MIME type from file extension
  const getMimeType = (ext: string): string => {
    const extLower = ext.toLowerCase()
    switch (extLower) {
      case 'png': return 'image/png'
      case 'jpg':
      case 'jpeg': return 'image/jpeg'
      case 'gif': return 'image/gif'
      case 'webp': return 'image/webp'
      case 'pdf': return 'application/pdf'
      case 'txt': return 'text/plain'
      case 'html':
      case 'htm': return 'text/html'
      case 'json': return 'application/json'
      case 'mp3': return 'audio/mpeg'
      case 'mp4': return 'video/mp4'
      case 'doc': return 'application/msword'
      case 'docx': return 'application/vnd.openxmlformats-officedocument.wordprocessingml.document'
      case 'xls': return 'application/vnd.ms-excel'
      case 'xlsx': return 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'
      case 'ppt': return 'application/vnd.ms-powerpoint'
      case 'pptx': return 'application/vnd.openxmlformats-officedocument.presentationml.presentation'
      default: return 'application/octet-stream'
    }
  }

  // Open file (local or remote)
  const openFile = async (result: SearchResult) => {
    if (result.source_device) {
      // Remote file - download and open
      try {
        const fileData = await invoke<ArrayBuffer>('fetch_remote_file', {
          deviceId: result.source_device.device_id,
          fileId: result.file_info.id,
        })

        // Create blob and open in new tab
        const uint8Array = new Uint8Array(fileData)
        const mimeType = getMimeType(result.file_info.file_ext)
        const blob = new Blob([uint8Array], { type: mimeType })
        const url = URL.createObjectURL(blob)

        // Open in new tab
        window.open(url, '_blank')

        // Revoke URL after a delay (allow time for browser to load)
        setTimeout(() => URL.revokeObjectURL(url), 30000)

        // Show success message
        window.$message.success(t('common.fileOpened'))
      } catch (e) {
        console.error('Failed to fetch remote file:', e)
        window.$message.error(t('common.openFileFailed'))
      }
    } else {
      // Local file - use Tauri API
      const { openPath } = await import('@tauri-apps/plugin-opener')
      openPath(result.file_info.path)
    }
  }

  // Download remote file to local
  const downloadRemoteFile = async (result: SearchResult) => {
    if (!result.source_device)
      return

    try {
      const fileData = await invoke<ArrayBuffer>('fetch_remote_file', {
        deviceId: result.source_device.device_id,
        fileId: result.file_info.id,
      })

      // Create blob and trigger download
      const uint8Array = new Uint8Array(fileData)
      const mimeType = getMimeType(result.file_info.file_ext)
      const blob = new Blob([uint8Array], { type: mimeType })
      const url = URL.createObjectURL(blob)

      // Create download link and trigger click
      const link = document.createElement('a')
      link.href = url
      link.download = result.file_info.name
      document.body.appendChild(link)
      link.click()
      document.body.removeChild(link)

      // Revoke URL after download
      setTimeout(() => URL.revokeObjectURL(url), 1000)

      window.$message.success(t('common.downloadSuccess'))
    } catch (e) {
      console.error('Failed to download remote file:', e)
      window.$message.error(t('common.downloadFailed'))
    }
  }

  return {
    // Constants
    SEMANTIC_SEARCH,
    KEYWORD_SEARCH,

    // State
    query,
    searchResults,
    localResults,
    remoteResults,
    localSearching,
    remoteDeviceSearching,
    searchPhase,
    searchType,
    searchDevices,
    selectedDeviceIds,
    searchStatuses,
    clusterEnabled,
    semanticNotice,

    // Computed
    hasRemoteDevices,
    hitTypeLabels,
    selectedLocalDevice,

    // Methods
    isKeywordOnlyMatch,
    getSelectedDeviceIds,
    toggleDevice,
    selectAllDevices,
    filterResults,
    getDeviceStatusIcon,
    getDeviceStatusColor,
    triggerSearch,
    clearSearch,
    loadClusterSetting,
    loadSearchDevices,
    highlightText,
    openFile,
    downloadRemoteFile,
    cleanup: cleanupBlobUrls,
  }
}
