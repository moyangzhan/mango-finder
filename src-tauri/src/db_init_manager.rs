use log::{info, warn};
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use tauri::Emitter;

static DB_INIT_MANAGER: OnceLock<DbInitManager> = OnceLock::new();

/// Manages database initialization, path switching, and corruption recovery.
/// Only used during app startup — repository functions open their own connections.
struct DbInitManager {
    /// Current connection (held only during init/migration)
    conn: Mutex<Option<Connection>>,
    /// Current database path
    current_path: Mutex<Option<PathBuf>>,
}

impl DbInitManager {
    fn new() -> Self {
        Self {
            conn: Mutex::new(None),
            current_path: Mutex::new(None),
        }
    }

    /// Initialize database connection
    pub fn init(&self, db_path: &PathBuf) -> Result<(), String> {
        // Check if already initialized with the same path
        {
            let current = self
                .current_path
                .lock()
                .map_err(|e| format!("Failed to acquire lock: {}", e))?;
            if let Some(ref path) = *current {
                if path == db_path {
                    // Same path, check if connection is valid
                    let conn_guard = self
                        .conn
                        .lock()
                        .map_err(|e| format!("Failed to acquire lock: {}", e))?;
                    if let Some(ref conn) = *conn_guard {
                        if check_health(conn) {
                            return Ok(()); // Already initialized and valid
                        }
                    }
                }
            }
        }

        // Need to initialize or reinitialize
        self.do_init(db_path, true)
    }

    /// Execute initialization (internal method)
    fn do_init(&self, db_path: &PathBuf, allow_recover: bool) -> Result<(), String> {
        // Close old connection
        self.close_connection()?;

        // Try to open new connection
        match try_open_connection(db_path) {
            Ok(conn) => {
                let mut conn_guard = self
                    .conn
                    .lock()
                    .map_err(|e| format!("Failed to acquire lock: {}", e))?;
                *conn_guard = Some(conn);

                let mut path_guard = self
                    .current_path
                    .lock()
                    .map_err(|e| format!("Failed to acquire lock: {}", e))?;
                *path_guard = Some(db_path.clone());

                info!("Database connection initialized: {:?}", db_path);
                Ok(())
            }
            Err(e) if allow_recover => {
                warn!("Database open failed, attempting recovery: {}", e);
                recover_database(db_path)?;
                // Retry after recovery (don't allow recover again to prevent infinite loop)
                self.do_init(db_path, false)
            }
            Err(e) => Err(e),
        }
    }

    /// Switch database path (for migration)
    pub fn switch_path(&self, new_path: &PathBuf) -> Result<(), String> {
        info!("Switching database path to: {:?}", new_path);
        self.init(new_path)
    }

    /// Close connection
    fn close_connection(&self) -> Result<(), String> {
        let mut conn_guard = self
            .conn
            .lock()
            .map_err(|e| format!("Failed to acquire lock: {}", e))?;
        if let Some(conn) = conn_guard.take() {
            drop(conn); // Explicitly close
            info!("Database connection closed");
        }
        Ok(())
    }

    /// Get connection (with health check and auto-reconnect)
    pub fn get_connection(&self) -> Result<ConnectionGuard<'_>, DbError> {
        // First try to get existing connection
        {
            let conn_guard = self.conn.lock().map_err(|_| DbError::LockFailed)?;
            if let Some(ref conn) = *conn_guard {
                if check_health(conn) {
                    return Ok(ConnectionGuard::new(conn_guard));
                }
            }
        }

        // Connection invalid, try to reconnect
        self.reconnect()?;

        // Re-acquire connection
        let conn_guard = self.conn.lock().map_err(|_| DbError::LockFailed)?;
        if conn_guard.is_some() {
            Ok(ConnectionGuard::new(conn_guard))
        } else {
            Err(DbError::NotInitialized)
        }
    }

    /// Reconnect to database
    fn reconnect(&self) -> Result<(), DbError> {
        // Get current path
        let path = {
            let path_guard = self.current_path.lock().map_err(|_| DbError::LockFailed)?;
            path_guard.clone().ok_or(DbError::NotInitialized)?
        };

        // Check if file exists
        if !path.exists() {
            // File not found, possibly migrated or deleted
            return Err(DbError::FileNotFound(path));
        }

        // Close old connection
        let mut conn_guard = self.conn.lock().map_err(|_| DbError::LockFailed)?;
        if let Some(conn) = conn_guard.take() {
            drop(conn);
        }

        // Reopen connection
        let conn = try_open_connection(&path).map_err(DbError::ReconnectFailed)?;

        *conn_guard = Some(conn);
        info!("Database reconnected successfully");
        Ok(())
    }

    /// Check if database is available
    pub fn is_available(&self) -> bool {
        self.conn
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
    }
}

/// Connection guard (wraps MutexGuard)
pub struct ConnectionGuard<'a> {
    guard: std::sync::MutexGuard<'a, Option<Connection>>,
}

impl<'a> ConnectionGuard<'a> {
    fn new(guard: std::sync::MutexGuard<'a, Option<Connection>>) -> Self {
        Self { guard }
    }

    pub fn as_conn(&self) -> &Connection {
        self.guard.as_ref().expect("Connection should exist")
    }
}

impl<'a> std::ops::Deref for ConnectionGuard<'a> {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        self.as_conn()
    }
}

// ==================== Helper Functions ====================

/// Open a new DB connection with the per-connection busy_timeout applied.
/// All repositories open their connections through this helper so that
/// concurrent writers (CLI vs GUI/daemon under WAL) wait and retry instead
/// of failing immediately with "database is locked".
/// 打开新的 DB 连接并应用逐连接的 busy_timeout。所有 repo 统一经此打开
/// 连接,使并发写入方(CLI 与 GUI/daemon,WAL 模式)等待重试而非立即
/// 报 "database is locked"。
pub fn open_db() -> Result<Connection, rusqlite::Error> {
    let conn = Connection::open(crate::utils::app_util::get_db_path())?;
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    register_hamming_udf(&conn)?;
    Ok(conn)
}

/// 汉明距离 UDF,视觉回退查询在 SQL 内过滤用
/// Hamming distance UDF for in-DB visual fallback filtering
fn register_hamming_udf(conn: &Connection) -> Result<(), rusqlite::Error> {
    use rusqlite::functions::FunctionFlags;
    conn.create_scalar_function(
        "hamming_dist",
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let a: Vec<u8> = ctx.get(0)?;
            let b: Vec<u8> = ctx.get(1)?;
            // Length mismatch: return a large distance so the entry is filtered out (zip truncation would understate it)
            if a.len() != b.len() {
                return Ok(i64::MAX);
            }
            let dist: i64 = a.iter().zip(b.iter()).map(|(x, y)| (x ^ y).count_ones() as i64).sum();
            Ok(dist)
        },
    )
}

fn try_open_connection(db_path: &PathBuf) -> Result<Connection, String> {
    let conn = Connection::open(db_path).map_err(|e| format!("Failed to open database: {}", e))?;

    // busy_timeout: the CLI and the GUI/daemon are two writers of the same DB;
    // under WAL a write conflict should wait and retry instead of failing
    // immediately with "database is locked".
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000;",
    )
    .map_err(|e| format!("Failed to set PRAGMA: {}", e))?;

    Ok(conn)
}

fn check_health(conn: &Connection) -> bool {
    // Simple health check: execute a simple query
    conn.query_row("SELECT 1", [], |_| Ok(())).is_ok()
}

fn recover_database(db_path: &PathBuf) -> Result<(), String> {
    let backup_path = format!(
        "{}.bak.{}",
        db_path.display(),
        chrono::Local::now().format("%Y%m%d_%H%M%S")
    );

    std::fs::rename(db_path, &backup_path)
        .map_err(|e| format!("Failed to backup database: {}", e))?;

    info!("Backed up corrupted database to {}", backup_path);

    // Create new database
    let conn = try_open_connection(db_path)?;

    // Run migrations to create table structure
    crate::db_migrations::run_all_migrations(&conn)
        .map_err(|e| format!("Failed to run migrations: {}", e))?;

    // Notify UI that re-indexing is needed
    notify_db_recovered(&backup_path);

    Ok(())
}

fn notify_db_recovered(backup_path: &str) {
    // Send event to notify UI
    if let Some(handle) = crate::global::APP_HANDLE.get() {
        let _ = handle.emit("db-recovered", backup_path);
    }
    warn!("Database recovered, backup saved to: {}", backup_path);
}

// ==================== Global Functions ====================

/// Initialize database manager
pub fn init_db_manager(db_path: &PathBuf) -> Result<(), String> {
    let manager = DB_INIT_MANAGER.get_or_init(DbInitManager::new);
    manager.init(db_path)
}

/// Switch database path
pub fn switch_db_path(new_path: &PathBuf) -> Result<(), String> {
    let manager = DB_INIT_MANAGER
        .get()
        .ok_or("Database manager not initialized")?;
    manager.switch_path(new_path)
}

/// Get database connection
pub fn get_connection() -> Result<ConnectionGuard<'static>, DbError> {
    let manager = DB_INIT_MANAGER.get().ok_or(DbError::NotInitialized)?;
    manager.get_connection()
}

/// Check if database is available
pub fn is_db_available() -> bool {
    DB_INIT_MANAGER
        .get()
        .map(|m| m.is_available())
        .unwrap_or(false)
}

// ==================== Error Types ====================

#[derive(Debug)]
pub enum DbError {
    NotInitialized,
    LockFailed,
    ReconnectFailed(String),
    FileNotFound(PathBuf),
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbError::NotInitialized => write!(f, "Database not initialized"),
            DbError::LockFailed => write!(f, "Failed to acquire database lock"),
            DbError::ReconnectFailed(e) => write!(f, "Reconnect failed: {}", e),
            DbError::FileNotFound(path) => write!(f, "Database file not found: {:?}", path),
        }
    }
}

impl std::error::Error for DbError {}

// ==================== Conversions ====================

impl From<DbError> for rusqlite::Error {
    fn from(e: DbError) -> Self {
        rusqlite::Error::InvalidParameterName(e.to_string())
    }
}
