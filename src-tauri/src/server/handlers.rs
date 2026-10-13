//! HTTP handlers grouped by route family (cli, remote, pairing); routing assembly stays in http_server
//! HTTP handler 按路由族分组(cli、remote、pairing);路由装配留在 http_server
pub mod cli;
pub mod pairing;
pub mod remote;
