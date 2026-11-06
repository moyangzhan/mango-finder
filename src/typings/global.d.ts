interface Window {
  $loadingBar: import('naive-ui').LoadingBarProviderInst;
  $dialog: import('naive-ui').DialogProviderInst;
  $message: import('naive-ui').MessageProviderInst;
  $notification: import('naive-ui').NotificationProviderInst;
}

interface AppState {
  theme: string;
  locale: string;
  clusterPortError: { port: number } | null;
}

interface SettingState {
  activeTab: string;
}

interface ModelPlatform {
  id: number;
  name: string;
  title: string;
  base_url: string;
  api_key: string;
  remark: string;
  is_proxy_enable: boolean;
}

interface SelfHostedPlatform {
  id: number;
  name: string;
  title: string;
  host: string;
  port: number;
  remark: string;
}

interface AiModel {
  id: number;
  name: string;
  title: string;
  platform: string;
  model_types: string;
  remark: string;
  setting: string;
  context_window: number;
  max_input_tokens: number;
  max_output_tokens: number;
  input_types: string;
  properties: string;
  is_reasoner: boolean;
  is_thinking_closable: boolean;
  is_free: boolean;
  is_enable: boolean;
}

interface ProxyInfo {
  protocol: string;
  host: string;
  port: number;
}

interface ContentStorage {
  document: string; // "database" | "file" | "none"
  image: string;
  audio: string;
}

interface IndexerSetting {
  is_private: boolean; // Deprecated: kept for backward compatibility
  parser_mode: string; // "local" | "selfhosted" | "remote" | "mixed"
  image_parser_mode: string;
  audio_parser_mode: string;
  embedding_parser_mode: string; // vector category backend: "local" | "selfhosted" | "remote"
  file_content_language: string;
  ignore_dirs: string[];
  ignore_path_prefixes: string[];
  ignore_exts: string[];
  ignore_files: string[];
  content_storage: ContentStorage;
}

interface DownloadState {
  downloadId: string;
  url: string;
  progress: number;
  status: string;
}

interface SelectedItem {
  id: string;
  name: string;
  type: 'file' | 'directory';
  raw: File | null;
  path?: string;
  done: boolean; // for indexing task
}

interface IndexingTask {
  id: number;
  paths: string;
  total_cnt: number;
  content_processed_cnt: number;
  content_indexed_success_cnt: number;
  content_indexed_failed_cnt: number;
  content_indexed_skipped_cnt: number;
  duration: number;
}

interface FileMetadata {
  name: string;
  extension: string;
  category: string;
  size: number; //in bytes
  created: string;
  modified: string;
  author: string;
}

interface FileInfo {
  id: number;
  name: string;
  category: number; //1:document, 2:image, 3:audio, 4:video, 5:other
  path: string;
  content: string;
  content_ref_path?: string;
  metadata: FileMetadata;
  file_ext: string;
  file_size: number;
  md5: string;
  content_index_status: number;
  content_index_status_msg: string;
  meta_index_status: number;
  meta_index_status_msg: string;
  is_invalid: boolean;
  invalid_reason: string;
  file_create_time: string;
  file_update_time: string;
  create_time: string;
  update_time: string;

  // For UI
  file_data?: any // raw file data
  [key: string]: any;
  html_path: string;
}

interface CommandResult {
  success: boolean;
  message: string;
  data?: any;
  code: number;
}

interface WatchSetting {
  directories: string[];
  files: string[];
}

interface SearchResult {
  score: number,
  hit_types: string[],
  file_info: FileInfo,
  matched_keywords: string[],
  matched_chunk_ids: number[],
  similarity_type?: 'imageHash' | 'imageSemantic' | 'documentSemantic' | 'audioFingerprint' | 'audioTranscription',
  source_device?: SourceDevice,
}

// Source device info for search results (only for remote devices)
interface SourceDevice {
  device_id: string,
  device_name: string,
}

// Search device for device filter
interface SearchDevice {
  device_id: string,
  device_name: string,
  is_local: boolean,
  online_status: string,
  index_count: number,
}

// Search status for tracking progress
interface SearchStatus {
  device_id: string,
  device_name: string,
  status: 'Pending' | 'Searching' | 'Completed' | 'Failed',
  result_count: number,
  error?: string,
}

// Local device search result
interface LocalDeviceSearchResult {
  /** One-line degradation notice for semantic searches (backend unavailable / rebuild in progress) */
  semantic_notice?: string | null;
  results: SearchResult[],
  total: number,
}

// Remote device search result
interface RemoteDeviceSearchResult {
  results: SearchResult[],
  statuses: SearchStatus[],
  total: number,
}

//  Start { task_id: i64 },
//  Scan { task_id: i64, msg: String },
//  Stop { task_id: i64, msg: String },
//  Embed { task_id: i64, msg: String },
//  Finish { task_id: i64, msg: String },
interface IndexingEvent {
  event: string
  data: {
    taskId: number
    msg: string
  }
}

// Cluster types
interface Device {
  id: number;
  device_id: string;
  name: string;
  ip_address: string;
  port: number;
  version: string;
  online_status: 'online' | 'offline' | 'unknown';
  pairing_status: 'none' | 'pending_in' | 'pending_out' | 'paired' | 'rejected' | 'blocked';
  last_seen: string;
  first_discovered: string;
  index_count: number;
  capabilities: string;
  discovery_method: 'mdns' | 'manual';
  create_time: string;
  update_time: string;
}

interface PairingRequest {
  id: number;
  device_id: string;
  device_name: string;
  ip_address: string;
  port: number;
  direction: 'in' | 'out';
  status: 'pending' | 'accepted' | 'rejected' | 'expired' | 'auto_rejected';
  remark: string;
  response_time: string | null;
  create_time: string;
  update_time: string;
}

interface ClusterSetting {
  enabled: boolean;
  port: number;
  device_name: string;
  allow_to_be_discovered: boolean;
  auto_request_pairing: boolean;
  auto_accept_pairing: boolean;
  online_check_interval: number;
}