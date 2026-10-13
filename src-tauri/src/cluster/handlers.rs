//! HTTP handlers grouped by route family (/cli local, /cluster remote,
//! /cluster/pairing); routing assembly stays in http_server
//! HTTP handler 按路由族分组(/cli 本地、/cluster 远程、/cluster/pairing);路由装配留在 http_server
pub mod local;
pub mod pairing;
pub mod remote;
