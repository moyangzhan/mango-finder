use anyhow::Result;
use log::info;
use rusqlite::Connection;

/// DB_VERSION = 3
/// Add device and pairing_request tables for multi-device networking
/// Add audio_fingerprint column to file_info for music similarity
pub fn exec_ddl_with_conn(conn: &Connection) -> Result<()> {
    info!("dbv3.exec_ddl_with_conn");

    // Add audio_fingerprint column to file_info table (for music similarity)
    // audio_fingerprint contains: spectral_histogram (10 f32) + energy_bands (8 f32) + avg_zcr (f32) + tempo_estimate (f32)
    // Total: 20 f32 values = 80 bytes
    let _ = conn.execute_batch(
        r#"
        ALTER TABLE file_info ADD COLUMN audio_fingerprint BLOB;
        "#,
    ); // Ignore error if column already exists

    // device table - stores discovered remote devices
    //
    // pairing_status values and their behavior when receiving pairing requests:
    //   - none:         No pairing relationship, request will be processed normally
    //   - pending_in:   Received pairing request, waiting for local user to respond
    //   - pending_out:  Sent pairing request, waiting for remote device to respond
    //   - paired:       Successfully paired, will return "already_paired"
    //   - rejected:     Local rejected remote's request (subsequent requests will be auto-rejected)
    //   - blocked:      Remote rejected local's request (blocked by remote)
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS device (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            device_id       TEXT UNIQUE NOT NULL,          -- remote device's client_id (UUID)
            name            TEXT NOT NULL DEFAULT '',       -- device display name
            ip_address      TEXT NOT NULL,                  -- IP address
            port            INTEGER NOT NULL DEFAULT 15678, -- service port
            version         TEXT DEFAULT '',                -- remote software version
            online_status   TEXT NOT NULL DEFAULT 'unknown',-- online/offline/unknown
            pairing_status  TEXT NOT NULL DEFAULT 'none',   -- none/pending_in/pending_out/paired/rejected/blocked
            pairing_remark  TEXT DEFAULT '',                -- remark explaining the pairing status change
            last_seen       TEXT DEFAULT '',                -- last seen timestamp
            first_discovered TEXT DEFAULT '',               -- first discovered timestamp
            index_count     INTEGER DEFAULT 0,              -- number of indexed files
            capabilities    TEXT DEFAULT '{}',              -- JSON: supported search types
            discovery_method TEXT DEFAULT 'mdns',           -- mdns/manual
            create_time     TEXT DEFAULT '',
            update_time     TEXT DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS idx_device_online_status ON device(online_status);
        CREATE INDEX IF NOT EXISTS idx_device_pairing ON device(pairing_status);
        CREATE INDEX IF NOT EXISTS idx_device_device_id ON device(device_id);
        "#,
    )?;

    // pairing_request table - logs all pairing requests for debugging
    //
    // status values:
    //   - pending:       Request is waiting for response
    //   - accepted:      Request was accepted (user approved or auto-accepted)
    //   - rejected:      Request was rejected by user
    //   - expired:       Request timed out (24 hours)
    //   - auto_rejected: Request was auto-rejected (device was previously rejected)
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS pairing_request (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            device_id       TEXT NOT NULL,              -- remote device ID
            device_name     TEXT NOT NULL,              -- remote device name
            ip_address      TEXT NOT NULL,              -- remote IP
            port            INTEGER NOT NULL,           -- remote port
            direction       TEXT NOT NULL,              -- 'in' (received) or 'out' (sent)
            status          TEXT NOT NULL DEFAULT 'pending', -- pending/accepted/rejected/expired/auto_rejected
            remark          TEXT DEFAULT '',            -- remark describing the handling result
            response_time   TEXT DEFAULT '',            -- time when responded
            create_time     TEXT DEFAULT '',
            update_time     TEXT DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS idx_pairing_request_status ON pairing_request(status);
        CREATE INDEX IF NOT EXISTS idx_pairing_request_device ON pairing_request(device_id);
        CREATE INDEX IF NOT EXISTS idx_pairing_request_direction ON pairing_request(direction);
        "#,
    )?;

    // triggers for auto update timestamps
    conn.execute_batch(
        r#"
        CREATE TRIGGER IF NOT EXISTS device_create_time
        AFTER INSERT ON device
        FOR EACH ROW
        BEGIN
            UPDATE device
            SET create_time = datetime('now', 'localtime'),
                update_time = datetime('now', 'localtime')
            WHERE id = NEW.id;
        END;

        CREATE TRIGGER IF NOT EXISTS device_update_time
        AFTER UPDATE ON device
        FOR EACH ROW
        BEGIN
            UPDATE device SET update_time = datetime('now', 'localtime')
            WHERE id = NEW.id;
        END;

        CREATE TRIGGER IF NOT EXISTS pairing_request_create_time
        AFTER INSERT ON pairing_request
        FOR EACH ROW
        BEGIN
            UPDATE pairing_request
            SET create_time = datetime('now', 'localtime'),
                update_time = datetime('now', 'localtime')
            WHERE id = NEW.id;
        END;

        CREATE TRIGGER IF NOT EXISTS pairing_request_update_time
        AFTER UPDATE ON pairing_request
        FOR EACH ROW
        BEGIN
            UPDATE pairing_request
            SET update_time = datetime('now', 'localtime')
            WHERE id = NEW.id;
        END;
        "#,
    )?;

    Ok(())
}

/// Initialize network configuration data
pub fn init_data_with_conn(conn: &Connection) -> Result<()> {
    info!("dbv3.init_data_with_conn");

    // Cluster configuration defaults (stored as JSON)
    conn.execute_batch(
        r#"
        INSERT OR IGNORE INTO config (name, value) VALUES (
            'cluster_setting',
            '{"enabled":false,"port":15678,"device_name":"","allow_to_be_discovered":true,"auto_request_pairing":false,"auto_accept_pairing":false,"online_check_interval":30}'
        );
        "#,
    )?;

    Ok(())
}
