//! The server: the HTTP service answering /cli (and cluster) requests.
//! Resident by design — carried by the GUI while it runs with the cluster on, otherwise by the on-demand daemon (`mfd serve`) — so the hot state behind the routes (path cache, embedding model) survives across commands; the exclusive bind guarantees at most one local instance at any time.
//! The bind IP scope follows the cluster setting: 0.0.0.0 when enabled, loopback otherwise.
//!
//! Module layout:
//! 1. `http_server` — the listener: bind/daemon flags/graceful stop
//! 2. `handlers` — route families
//! 3. `loopback_client` — the loopback HTTP client: POST wrapper plus the probe/shutdown handshake (used by the CLI binary, the GUI's yield logic and `set_data_path`)
//!
//! server:应答 /cli(与 cluster)请求的 HTTP 服务。
//! 设计为常驻——GUI 运行且集群开启时由 GUI 进程承载,否则由按需拉起的 daemon(`mfd serve`)承载——让路由背后的热状态(路径缓存、嵌入模型)跨命令保留,后续命令免于冷启动;端口独占绑定保证任一时刻本地至多一个实例。
//! 绑定 IP 范围跟随集群设置:开启时绑 0.0.0.0,否则仅回环。
//!
//! 模块布局:
//! 1. `http_server` — HTTP 监听器:绑定/daemon 标记/优雅停机
//! 2. `handlers` — 路由族
//! 3. `loopback_client` — 回环 HTTP 客户端:POST 封装与探测/关闭握手(CLI 二进制、GUI 让位逻辑与 `set_data_path` 使用)

pub mod handlers;
pub mod http_server;
pub mod loopback_client;
