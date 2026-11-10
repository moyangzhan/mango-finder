import type { DataTableColumns } from 'naive-ui'
import { NTooltip } from 'naive-ui'
import { t } from '@/locales'

const INDEX_STATUS_LABEL_KEYS: Record<number, string> = {
  1: 'common.indexStatusWaiting',
  2: 'common.indexStatusIndexing',
  3: 'common.indexStatusIndexed',
  4: 'common.indexStatusFailed',
}

/** Status label only; the detail message lives in its own column, with a
 * tooltip here so hiding that column keeps failure reasons reachable.
 * "success" is the normal receipt of the status itself, not a detail */
function renderIndexStatus(status: number, msg: string) {
  const key = INDEX_STATUS_LABEL_KEYS[status] ?? 'common.indexStatusWaiting'
  const label = h('span', { class: status === 4 ? 'text-red-500' : '' }, t(key))
  if (!msg || msg === 'success')
    return label
  return h(NTooltip, { showDelay: 300, placement: 'top' }, {
    trigger: () => label,
    default: () => msg,
  })
}

/** Detail message of one index status: failure reason, skip note, or the
 * success receipt, shown as a muted truncated line */
function renderIndexStatusMessage(msg: string) {
  const text = msg === 'success' ? t('common.success') : msg
  if (!text)
    return h('span', { class: 'text-gray-400' }, '-')
  return h(NTooltip, { showDelay: 300, placement: 'top' }, {
    trigger: () => h('span', { class: 'text-xs text-gray-400 truncate block' }, text),
    default: () => text,
  })
}

export const getFileColumns = (
  previewFn: (row: FileInfo) => void,
): DataTableColumns<FileInfo> => {
  return [
    {
      type: 'selection' as const,
      fixed: 'left' as const,
    },
    {
      title: 'ID',
      key: 'id',
      width: 70,
      fixed: 'left' as const,
      sorter: true,
    },
    {
      title: () => t('common.name'),
      key: 'name',
      width: 150,
      fixed: 'left' as const,
      sorter: true,
      render(row: FileInfo) {
        return h(NTooltip, { showDelay: 0, placement: 'top' }, {
          trigger: () => h(
            'span',
            {
              class: 'text-link truncate block',
              onClick: () => previewFn(row),
            },
            { default: () => row.name },
          ),
          default: () => t('common.view'),
        })
      },
    },
    {
      title: () => t('common.category'),
      key: 'category',
      width: 100,
      sorter: true,
      render(row: FileInfo) {
        let category = ''
        switch (row.category) {
          case 1:
            category = t('common.document')
            break
          case 2:
            category = t('common.image')
            break
          case 3:
            category = t('common.audio')
            break
          case 4:
            category = t('common.video')
            break
          case 5:
            category = t('common.other')
            break
          default:
        }
        return category
      },
    },
    {
      title: () => t('common.path'),
      key: 'path',
      sorter: true,
    },
    {
      title: () => t('common.extension'),
      key: 'file_ext',
      width: 100,
      sorter: true,
    },
    {
      title: () => t('common.fileSize'),
      key: 'file_size',
      width: 100,
      sorter: true,
      render(row: FileInfo) {
        if (row.file_size > 1024 * 1024 * 1024)
          return `${Math.floor(row.file_size / (1024 * 1024 * 1024))}G`
        else if (row.file_size > 1024 * 1024)
          return `${Math.floor(row.file_size / (1024 * 1024))}M`
        else if (row.file_size > 1024)
          return `${Math.floor(row.file_size / (1024 * 1024))}K`
        else
          return `${row.file_size}B`
      },
    },
    {
      title: () => t('common.contentIndexStatus'),
      key: 'content_index_status',
      width: 130,
      sorter: true,
      render(row: FileInfo) {
        return renderIndexStatus(row.content_index_status, row.content_index_status_msg)
      },
    },
    {
      title: () => t('common.contentIndexStatusMsg'),
      key: 'content_index_status_msg',
      width: 150,
      sorter: true,
      render(row: FileInfo) {
        return renderIndexStatusMessage(row.content_index_status_msg)
      },
    },
    {
      title: () => t('common.metadataIndexStatus'),
      key: 'meta_index_status',
      width: 150,
      sorter: true,
      render(row: FileInfo) {
        return renderIndexStatus(row.meta_index_status, row.meta_index_status_msg)
      },
    },
    {
      title: () => t('common.metadataIndexStatusMsg'),
      key: 'meta_index_status_msg',
      width: 150,
      sorter: true,
      render(row: FileInfo) {
        return renderIndexStatusMessage(row.meta_index_status_msg)
      },
    },
    {
      title: () => t('common.fileCreateTime'),
      key: 'file_create_time',
      width: 130,
      sorter: true,
    },
    {
      title: () => t('common.fileUpdateTime'),
      key: 'file_update_time',
      width: 130,
      sorter: true,
    },
  ]
}
