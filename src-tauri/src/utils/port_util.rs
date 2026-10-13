/// Clamp to the valid port range; out-of-range values fall back to `fallback`, so `as u16` truncation can never make callers disagree.
/// 钳制到合法端口区间;超范围值回退 `fallback`,`as u16` 截断不会让各调用方落在不同端口上。
pub fn clamp_port(port: i32, fallback: i32) -> i32 {
    if (1..=65535).contains(&port) {
        port
    } else {
        fallback
    }
}
