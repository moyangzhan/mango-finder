export function emptyModelPlatform() {
  return {
    id: 0,
    name: '',
    title: '',
    base_url: '',
    api_key: '',
    remark: '',
    is_proxy_enable: false,
  }
}

export function emptyProxyInfo() {
  return {
    protocol: 'http',
    host: '',
    port: 0,
  }
}

export function emptyIndexerSetting(): IndexerSetting {
  return {
    is_private: true,
    parser_mode: 'local',
    image_parser_mode: 'local',
    audio_parser_mode: 'local',
    embedding_parser_mode: 'local',
    file_content_language: 'en',
    ignore_dirs: [] as string[],
    ignore_exts: [] as string[],
    ignore_files: [] as string[],
    ignore_path_prefixes: [] as string[],
    content_storage: {
      document: 'database',
      image: 'database',
      audio: 'database',
    },
  }
}

export function emptyDownloadState() {
  return {
    downloadId: '',
    url: '',
    progress: 0,
    status: 'success',
  }
}

export function emptyWatchSetting() {
  return {
    directories: [] as string[],
    files: [] as string[],
  }
}

export function formatTime(dateStr: string, t: (key: string) => string): string {
  const date = new Date(dateStr)
  const now = new Date()
  const diffMs = now.getTime() - date.getTime()
  const diffMins = Math.floor(diffMs / 60000)

  if (diffMins < 1)
    return t('common.just')
  if (diffMins < 60)
    return `${diffMins}${t('common.minutes')}${t('common.ago')}`

  const diffHours = Math.floor(diffMins / 60)
  if (diffHours < 24)
    return `${diffHours}${t('common.hours')}${t('common.ago')}`

  return `${date.toLocaleDateString()} ${date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`
}
