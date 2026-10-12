//! Locale helpers shared by the GUI commands and the CLI.
//! locale 共享 core:GUI 命令与 CLI 共用。

use crate::enums::Locale;
use crate::global::{ACTIVE_LOCALE, CONFIG_NAME_ACTIVE_LOCALE};
use crate::repositories::config_repo;

/// Whether the locale is a supported value / 是否为受支持的 locale
pub fn is_valid_locale(locale: &str) -> bool {
    locale == Locale::EnUs.text() || locale == Locale::ZhCn.text()
}

/// Apply a locale: rust_i18n → persist (best-effort) → global, in GUI order.
/// Err means invalid input only; empty returns Ok(0), matching the GUI's
/// soft no-op contract. Persist failure logs and continues (GUI semantics).
/// 应用 locale:rust_i18n → 持久化(尽力而为) → 全局,GUI 顺序。
/// Err 仅表示非法输入;空串返回 Ok(0)(GUI 软契约);持久化失败记日志继续。
pub fn set_locale_core(locale: &str) -> Result<usize, String> {
    if locale.is_empty() {
        return Ok(0);
    }
    if !is_valid_locale(locale) {
        return Err(format!("Unsupported locale: {}", locale));
    }
    rust_i18n::set_locale(locale);
    let rows = config_repo::update_by_name(CONFIG_NAME_ACTIVE_LOCALE, locale).unwrap_or_else(|e| {
        log::error!("update config error: {}", e);
        0
    });
    *crate::write_lock!(ACTIVE_LOCALE) = locale.to_string();
    Ok(rows)
}

/// Current locale; en-US fallback when the lock is poisoned
/// 当前 locale;锁毒化时回落 en-US
pub fn current_locale() -> String {
    crate::read_lock!(ACTIVE_LOCALE).clone()
}

/// Apply the locale held in the global to rust-i18n at startup
/// 启动时把全局中的 locale 应用到 rust-i18n
pub fn apply_persisted_locale() {
    rust_i18n::set_locale(current_locale().as_str());
}
