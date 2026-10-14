use serde::Serialize;
use tauri::{AppHandle, Manager, command};

// The CLI rides in as an extra cargo bin next to the main executable (auto-collected by tauri-bundler), so installing the GUI delivers mfd.
// CLI 以 cargo bin 身份随主程序一同进包(tauri-bundler 自动收集),安装 GUI 即获得 mfd。
#[cfg(target_os = "windows")]
const MFD_NAME: &str = "mfd.exe";
#[cfg(not(target_os = "windows"))]
const MFD_NAME: &str = "mfd";

#[derive(Serialize)]
pub struct CliPathStatus {
    /// Absolute path of the bundled mfd binary
    pub mfd_path: String,
    /// False in dev mode: mfd is not built into target/debug alongside the GUI binary
    pub exists: bool,
    /// Whether the mfd directory is usable from any terminal (PATH entry / symlink)
    pub installed: bool,
}

/// mfd sits next to the main executable (install root on Windows, Contents/MacOS on macOS, /usr/bin with deb);
/// resource_dir is only a fallback for layouts that ship bins together with the resources.
/// mfd 位于主程序同目录(Windows 安装根目录、macOS Contents/MacOS、deb 的 /usr/bin);resource_dir 仅作兜底。
fn mfd_path(app: &AppHandle) -> Option<std::path::PathBuf> {
    let by_exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(MFD_NAME)));
    if by_exe.as_ref().is_some_and(|p| p.exists()) {
        return by_exe;
    }
    let by_resource = app
        .path()
        .resource_dir()
        .ok()
        .map(|d| d.join(MFD_NAME));
    if by_resource.as_ref().is_some_and(|p| p.exists()) {
        return by_resource;
    }
    // Neither exists (dev mode): report the exe-dir path so the UI can explain it
    by_exe
}

#[command]
pub async fn cli_path_status(app: AppHandle) -> Result<CliPathStatus, String> {
    let Some(path) = mfd_path(&app) else {
        return Err("resource directory unavailable".to_string());
    };
    let exists = path.exists();
    let installed = platform::installed(path.parent().unwrap_or(path.as_path()));
    Ok(CliPathStatus {
        mfd_path: path.to_string_lossy().into_owned(),
        exists,
        installed,
    })
}

#[command]
pub async fn cli_path_install(app: AppHandle) -> Result<(), String> {
    let Some(path) = mfd_path(&app) else {
        return Err("resource directory unavailable".to_string());
    };
    if !path.exists() {
        return Err(format!("mfd binary not found: {}", path.display()));
    }
    platform::install(path.parent().unwrap_or(path.as_path()))
}

#[command]
pub async fn cli_path_uninstall(app: AppHandle) -> Result<(), String> {
    let Some(path) = mfd_path(&app) else {
        return Err("resource directory unavailable".to_string());
    };
    platform::uninstall(path.parent().unwrap_or(path.as_path()))
}

// Windows: user-level (HKCU) Path entry + WM_SETTINGCHANGE so new terminals pick it up without re-login; no admin rights involved.
// macOS: /usr/local/bin symlink created through an osascript administrator prompt (the classic "install command line tool" flow).
// Linux: ~/.local/bin symlink (covered by the default ~/.profile PATH logic on Debian/Ubuntu; a guarded line is appended to ~/.profile when missing).
#[cfg(target_os = "windows")]
mod platform {
    const ENV_SUBKEY: &str = "Environment";
    const PATH_VALUE: &str = "Path";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// (entries, value_type): the current HKCU Path split by ';', plus the registry
    /// type to preserve on write (Path is normally REG_EXPAND_SZ).
    fn read_path() -> Result<(Vec<String>, windows_sys::Win32::System::Registry::REG_VALUE_TYPE), String> {
        use windows_sys::Win32::Foundation::ERROR_SUCCESS;
        use windows_sys::Win32::System::Registry::{
            HKEY_CURRENT_USER, KEY_READ, REG_EXPAND_SZ, REG_VALUE_TYPE, RegCloseKey, RegOpenKeyExW,
            RegQueryValueExW,
        };

        unsafe {
            let mut hkey = std::ptr::null_mut();
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                wide(ENV_SUBKEY).as_ptr(),
                0,
                KEY_READ,
                &mut hkey,
            ) != ERROR_SUCCESS
            {
                return Err("failed to open HKCU\\Environment".to_string());
            }
            let result = (|| {
                let mut value_type: REG_VALUE_TYPE = REG_EXPAND_SZ;
                let mut size_bytes: u32 = 0;
                let status = RegQueryValueExW(
                    hkey,
                    wide(PATH_VALUE).as_ptr(),
                    std::ptr::null(),
                    &mut value_type,
                    std::ptr::null_mut(),
                    &mut size_bytes,
                );
                if status == windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND {
                    return Ok((Vec::new(), value_type));
                }
                // Size-only query: both SUCCESS and MORE_DATA mean "size now known"
                if status != ERROR_SUCCESS
                    && status != windows_sys::Win32::Foundation::ERROR_MORE_DATA
                {
                    return Err(format!("failed to read HKCU Path (win32 error {})", status));
                }
                let mut buf = vec![0u16; size_bytes as usize / 2 + 1];
                let status = RegQueryValueExW(
                    hkey,
                    wide(PATH_VALUE).as_ptr(),
                    std::ptr::null(),
                    &mut value_type,
                    buf.as_mut_ptr() as *mut u8,
                    &mut size_bytes,
                );
                if status != ERROR_SUCCESS {
                    return Err(format!("failed to read HKCU Path (win32 error {})", status));
                }
                let len = size_bytes as usize / 2;
                // Registry string data usually carries a trailing NUL; it would poison the last entry's comparison
                let text = String::from_utf16_lossy(&buf[..len]);
                let text = text.trim_end_matches('\0');
                let entries = text
                    .split(';')
                    .filter(|e| !e.trim().is_empty())
                    .map(|e| e.trim().to_string())
                    .collect();
                Ok((entries, value_type))
            })();
            RegCloseKey(hkey);
            result
        }
    }

    /// Path entry comparison follows Windows semantics (case-insensitive, trailing separators ignored)
    fn same_entry(a: &str, b: &str) -> bool {
        a.trim_end_matches('\\').to_lowercase() == b.trim_end_matches('\\').to_lowercase()
    }

    fn write_path(entries: &[String], value_type: windows_sys::Win32::System::Registry::REG_VALUE_TYPE) -> Result<(), String> {
        use windows_sys::Win32::Foundation::ERROR_SUCCESS;
        use windows_sys::Win32::System::Registry::{
            HKEY_CURRENT_USER, KEY_SET_VALUE, RegCloseKey, RegOpenKeyExW, RegSetValueExW,
        };

        let text = entries.join(";");
        let wide_text = wide(&text);
        unsafe {
            let mut hkey = std::ptr::null_mut();
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                wide(ENV_SUBKEY).as_ptr(),
                0,
                KEY_SET_VALUE,
                &mut hkey,
            ) != ERROR_SUCCESS
            {
                return Err("failed to open HKCU\\Environment for write".to_string());
            }
            let status = RegSetValueExW(
                hkey,
                wide(PATH_VALUE).as_ptr(),
                0,
                value_type,
                wide_text.as_ptr() as *const u8,
                (wide_text.len() * 2) as u32,
            );
            RegCloseKey(hkey);
            if status != ERROR_SUCCESS {
                return Err(format!("failed to write HKCU Path (win32 error {})", status));
            }
            Ok(())
        }
    }

    /// Let running shells know the environment changed; already-open terminals keep their old PATH by design.
    /// 通知系统环境变量已变更;已打开的终端按设计保留旧 PATH。
    fn broadcast_change() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
        };

        unsafe {
            let env = wide(ENV_SUBKEY);
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                env.as_ptr() as isize,
                SMTO_ABORTIFHUNG,
                2000,
                std::ptr::null_mut(),
            );
        }
    }

    pub fn installed(mfd_dir: &std::path::Path) -> bool {
        read_path()
            .map(|(entries, _)| {
                entries
                    .iter()
                    .any(|e| same_entry(e, &mfd_dir.to_string_lossy()))
            })
            .unwrap_or(false)
    }

    pub fn install(mfd_dir: &std::path::Path) -> Result<(), String> {
        let (entries, value_type) = read_path()?;
        let dir = mfd_dir.to_string_lossy().into_owned();
        if entries.iter().any(|e| same_entry(e, &dir)) {
            return Ok(()); // idempotent
        }
        let mut next = entries;
        next.push(dir);
        write_path(&next, value_type)?;
        broadcast_change();
        Ok(())
    }

    pub fn uninstall(mfd_dir: &std::path::Path) -> Result<(), String> {
        let (entries, value_type) = read_path()?;
        let dir = mfd_dir.to_string_lossy().into_owned();
        let next: Vec<String> = entries
            .iter()
            .filter(|e| !same_entry(e, &dir))
            .cloned()
            .collect();
        // Entry not present (or the value never existed): leave the registry untouched
        if next.len() == entries.len() {
            return Ok(());
        }
        write_path(&next, value_type)?;
        broadcast_change();
        Ok(())
    }
}

// macOS: symlink /usr/local/bin/mfd -> <Resources>/mfd; creation/removal needs admin (osascript prompt)
#[cfg(target_os = "macos")]
mod platform {
    use super::MFD_NAME;
    use std::path::Path;

    const LINK_PATH: &str = "/usr/local/bin/mfd";

    /// Escape for an AppleScript string literal
    fn as_escape(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"")
    }

    /// Quote for the sh command embedded in AppleScript (single-quote wrapping)
    fn sh_quote(s: &str) -> String {
        format!("'{}'", s.replace('\'', "'\\''"))
    }

    /// Run a shell command via osascript with an administrator prompt.
    /// osascript 的 User canceled 按原文透传给前端展示。
    fn run_privileged(shell_cmd: &str) -> Result<(), String> {
        let script = format!(
            "do shell script \"{}\" with administrator privileges",
            as_escape(shell_cmd)
        );
        let output = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| format!("failed to run osascript: {}", e))?;
        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if stderr.contains("User canceled") {
                Err("Administrator authorization was canceled".to_string())
            } else {
                Err(stderr)
            }
        }
    }

    /// Only a symlink counts as ours; a regular file at the link path belongs to the user
    /// 只有符号链接才算我们的产物;链接位上的普通文件属于用户
    fn is_our_link() -> bool {
        std::fs::symlink_metadata(LINK_PATH)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
    }

    pub fn installed(_mfd_dir: &Path) -> bool {
        is_our_link()
    }

    pub fn install(mfd_dir: &Path) -> Result<(), String> {
        // Never clobber a regular file the user placed at the link path
        if std::fs::symlink_metadata(LINK_PATH).is_ok() && !is_our_link() {
            return Err(format!(
                "{} exists and is not a symlink; remove it manually first",
                LINK_PATH
            ));
        }
        let target = sh_quote(&mfd_dir.join(MFD_NAME).to_string_lossy());
        run_privileged(&format!(
            "mkdir -p /usr/local/bin && ln -sf {} {}",
            target, LINK_PATH
        ))
    }

    pub fn uninstall(_mfd_dir: &Path) -> Result<(), String> {
        if !is_our_link() {
            return Ok(()); // nothing of ours to remove
        }
        run_privileged(&format!("rm -f {}", LINK_PATH))
    }
}

// Linux: ~/.local/bin symlink; a guarded PATH line is appended to ~/.profile only when absent
#[cfg(target_os = "linux")]
mod platform {
    use super::MFD_NAME;
    use std::path::Path;

    const PROFILE_SNIPPET: &str = "\n# Added by Mango Finder: put ~/.local/bin on PATH\nif [ -d \"$HOME/.local/bin\" ]; then\n    PATH=\"$HOME/.local/bin:$PATH\"\nfi\n";

    fn link_path() -> Option<std::path::PathBuf> {
        dirs::home_dir().map(|h| h.join(".local").join("bin").join("mfd"))
    }

    /// Only a symlink counts as ours; a regular file at the link path belongs to the user
    /// 只有符号链接才算我们的产物;链接位上的普通文件属于用户
    fn is_our_link(link: &Path) -> bool {
        std::fs::symlink_metadata(link)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
    }

    pub fn installed(_mfd_dir: &Path) -> bool {
        link_path()
            .map(|p| is_our_link(&p))
            .unwrap_or(false)
    }

    pub fn install(mfd_dir: &Path) -> Result<(), String> {
        let link = link_path().ok_or("home directory unavailable")?;
        let parent = link
            .parent()
            .ok_or("invalid link directory")?
            .to_path_buf();
        std::fs::create_dir_all(&parent).map_err(|e| format!("failed to create {}: {}", parent.display(), e))?;
        if std::fs::symlink_metadata(&link).is_ok() {
            // Replace a stale link (e.g. pointing into a removed AppImage mount); a regular file here is the user's — refuse to touch it
            if !is_our_link(&link) {
                return Err(format!(
                    "{} exists and is not a symlink; remove it manually first",
                    link.display()
                ));
            }
            std::fs::remove_file(&link).map_err(|e| format!("failed to remove old link: {}", e))?;
        }
        std::os::unix::fs::symlink(mfd_dir.join(MFD_NAME), &link)
            .map_err(|e| format!("failed to create symlink: {}", e))?;

        // Debian/Ubuntu's default ~/.profile already handles ~/.local/bin; append only for other setups.
        // Debian/Ubuntu 的默认 ~/.profile 已含该逻辑;其他环境缺失时补一段。
        if let Some(home) = dirs::home_dir() {
            let profile = home.join(".profile");
            let content = std::fs::read_to_string(&profile).unwrap_or_default();
            if !content.contains(".local/bin") {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&profile)
                    .map_err(|e| format!("failed to open {}: {}", profile.display(), e))?;
                writeln!(file, "{}", PROFILE_SNIPPET.trim_end())
                    .map_err(|e| format!("failed to write {}: {}", profile.display(), e))?;
            }
        }
        Ok(())
    }

    pub fn uninstall(_mfd_dir: &Path) -> Result<(), String> {
        let link = link_path().ok_or("home directory unavailable")?;
        if is_our_link(&link) {
            std::fs::remove_file(&link).map_err(|e| format!("failed to remove link: {}", e))?;
        }
        Ok(())
    }
}
