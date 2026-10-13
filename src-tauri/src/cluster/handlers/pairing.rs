//! /cluster/pairing/* endpoints: pairing request/response handshake and
//! reset notification between devices
//! /cluster/pairing/* 端点:设备间的配对请求/应答握手与重置通知

use crate::cluster::get_cluster_setting;
use crate::cluster::http_server::ServerState;
use crate::entities::{Device, PairingRequest};
use crate::enums::{OnlineStatus, PairingRequestStatus, PairingResponseStatus, PairingStatus};
use crate::global::APP_HANDLE;
use crate::repositories::{device_repo, pairing_request_repo};
use crate::structs::cluster_config::{
    PairingRequestPayload, PairingResponsePayload, ResetNotifyPayload,
};
use axum::{extract::State, http::StatusCode, response::Json};
use rust_i18n::t;
use serde_json::{Value, json};
use std::sync::Arc;
use tauri::Emitter;

/// Notify frontend about pairing request handling result
fn notify_pairing_handled(payload: &PairingRequestPayload, status: &str) {
    if let Some(app_handle) = APP_HANDLE.get() {
        let _ = app_handle.emit(
            "pairing-request-handled",
            json!({
                "status": status,
                "device_id": payload.device_id,
                "device_name": payload.device_name,
                "ip_address": payload.ip_address,
                "port": payload.port
            }),
        );
    }
}

/// Check if device already exists and return its status
/// Returns Some(response) if device exists and should be handled early
fn check_existing_device(
    payload: &PairingRequestPayload,
) -> Option<Result<Json<Value>, StatusCode>> {
    let device = device_repo::get_by_device_id(&payload.device_id).ok()??;

    match device.pairing_status {
        PairingStatus::Paired => {
            // Log the request as already_paired
            let request = PairingRequest {
                device_id: payload.device_id.clone(),
                device_name: payload.device_name.clone(),
                ip_address: payload.ip_address.clone(),
                port: payload.port,
                direction: "in".to_string(),
                status: PairingRequestStatus::Accepted,
                remark: t!("pairing.remark.already-paired").to_string(),
                ..Default::default()
            };
            let _ = pairing_request_repo::insert(&request);

            notify_pairing_handled(payload, "already_paired");
            Some(Ok(Json(json!({
                "code": 0,
                "data": {
                    "status": PairingResponseStatus::AlreadyPaired,
                    "message": "Device is already paired"
                }
            }))))
        }
        PairingStatus::Rejected => {
            log::info!(
                "Pairing request from rejected device {}, auto-rejecting",
                payload.device_id
            );

            // Log the request as auto_rejected
            let request = PairingRequest {
                device_id: payload.device_id.clone(),
                device_name: payload.device_name.clone(),
                ip_address: payload.ip_address.clone(),
                port: payload.port,
                direction: "in".to_string(),
                status: PairingRequestStatus::AutoRejected,
                remark: t!("pairing.remark.auto-rejected").to_string(),
                ..Default::default()
            };
            let _ = pairing_request_repo::insert(&request);

            notify_pairing_handled(payload, "auto_rejected");
            Some(Ok(Json(json!({
                "code": 0,
                "data": {
                    "status": PairingResponseStatus::Rejected,
                    "message": "Device is rejected"
                }
            }))))
        }
        _ => None,
    }
}

/// Handle auto-pair scenario when both sides sent requests to each other
/// Returns Some(response) if auto-pair is applicable
fn handle_auto_pair(payload: &PairingRequestPayload) -> Option<Result<Json<Value>, StatusCode>> {
    let out_request =
        pairing_request_repo::get_pending_by_device_id(&payload.device_id, "out").ok()??;

    log::info!(
        "Auto-pairing: both sides sent requests to each other, accepting {}",
        payload.device_id
    );

    // Update device to paired status (automatic process)
    if let Ok(Some(device)) = device_repo::get_by_device_id(&payload.device_id) {
        let remark = t!("pairing.remark.auto-paired");
        let _ = device_repo::update_pairing_status_with_remark(
            device.id,
            PairingStatus::Paired,
            &remark,
            false,
        );
    }

    // Update outgoing request to accepted
    let _ = pairing_request_repo::accept(out_request.id);

    // Log incoming request as accepted
    let request = PairingRequest {
        device_id: payload.device_id.clone(),
        device_name: payload.device_name.clone(),
        ip_address: payload.ip_address.clone(),
        port: payload.port,
        direction: "in".to_string(),
        status: PairingRequestStatus::Accepted,
        remark: t!("pairing.remark.auto-paired").to_string(),
        ..Default::default()
    };
    let _ = pairing_request_repo::insert(&request);

    notify_pairing_handled(payload, "auto_accepted");
    Some(Ok(Json(json!({
        "code": 0,
        "data": {
            "status": PairingResponseStatus::Approved,
            "message": "Auto-paired: both sides requested pairing"
        }
    }))))
}

/// Handle auto-accept scenario when auto_accept_pairing is enabled
/// Returns Some(response) if auto-accept is applicable
async fn handle_auto_accept(
    payload: &PairingRequestPayload,
) -> Option<Result<Json<Value>, StatusCode>> {
    let config = get_cluster_setting().await;
    if !config.auto_accept_pairing {
        return None;
    }

    // Auto-accept
    let device = Device {
        device_id: payload.device_id.clone(),
        name: payload.device_name.clone(),
        ip_address: payload.ip_address.clone(),
        port: payload.port,
        online_status: OnlineStatus::Online,
        pairing_status: PairingStatus::Paired,
        discovery_method: "mdns".to_string(),
        ..Default::default()
    };
    let inserted_device = device_repo::upsert(&device).ok()?;

    // Update pairing status with remark (upsert doesn't set pairing_remark)
    let remark = t!("pairing.remark.auto-accepted");
    let _ = device_repo::update_pairing_status_with_remark(
        inserted_device.id,
        PairingStatus::Paired,
        &remark,
        false,
    );

    // Log the request
    let request = PairingRequest {
        device_id: payload.device_id.clone(),
        device_name: payload.device_name.clone(),
        ip_address: payload.ip_address.clone(),
        port: payload.port,
        direction: "in".to_string(),
        status: PairingRequestStatus::Accepted,
        remark: t!("pairing.remark.auto-accepted").to_string(),
        ..Default::default()
    };
    let _ = pairing_request_repo::insert(&request);

    notify_pairing_handled(payload, "auto_accepted");
    Some(Ok(Json(json!({
        "code": 0,
        "data": {
            "status": PairingResponseStatus::Approved,
            "message": "Pairing auto-approved"
        }
    }))))
}

/// Handle pending request scenario - create pending request and notify frontend
fn handle_pending_request(payload: &PairingRequestPayload) -> Result<Json<Value>, StatusCode> {
    // Log incoming pairing request
    let request = PairingRequest {
        device_id: payload.device_id.clone(),
        device_name: payload.device_name.clone(),
        ip_address: payload.ip_address.clone(),
        port: payload.port,
        direction: "in".to_string(),
        status: PairingRequestStatus::Pending,
        remark: t!("pairing.remark.waiting-approval").to_string(),
        ..Default::default()
    };
    pairing_request_repo::insert(&request).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Create/update device record with pending_in status
    let device = Device {
        device_id: payload.device_id.clone(),
        name: payload.device_name.clone(),
        ip_address: payload.ip_address.clone(),
        port: payload.port,
        online_status: OnlineStatus::Online,
        pairing_status: PairingStatus::PendingIn,
        discovery_method: "mdns".to_string(),
        ..Default::default()
    };
    let upserted_device =
        device_repo::upsert(&device).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Update pairing status with remark (upsert doesn't set pairing_remark)
    let remark = t!("pairing.remark.waiting-approval").to_string();
    let _ = device_repo::update_pairing_status_with_remark(
        upserted_device.id,
        PairingStatus::PendingIn,
        &remark,
        false,
    );

    // Notify frontend about pending request
    notify_pairing_handled(payload, "pending");

    Ok(Json(json!({
        "code": 0,
        "data": {
            "status": PairingResponseStatus::Pending,
            "message": "Pairing request received, waiting for approval"
        }
    })))
}

/// POST /pairing/request - Handle incoming pairing request
pub async fn handle_pairing_request(
    State(_state): State<Arc<ServerState>>,
    Json(payload): Json<PairingRequestPayload>,
) -> Result<Json<Value>, StatusCode> {
    log::info!(
        "Received pairing request from {} ({})",
        payload.device_name,
        payload.ip_address
    );

    // 1. Check if device already exists (paired or rejected)
    if let Some(result) = check_existing_device(&payload) {
        return result;
    }

    // 2. Check for auto-pair scenario (both sides sent requests)
    if let Some(result) = handle_auto_pair(&payload) {
        return result;
    }

    // 3. Check if auto_accept_pairing is enabled
    if let Some(result) = handle_auto_accept(&payload).await {
        return result;
    }

    // 4. Default: create pending request
    handle_pending_request(&payload)
}

/// POST /pairing/respond - Handle pairing response (from remote device)
pub async fn handle_pairing_respond(
    Json(payload): Json<PairingResponsePayload>,
) -> Result<Json<Value>, StatusCode> {
    log::info!(
        "Received pairing response from {}: approved={}",
        payload.responder_id,
        payload.approved
    );

    // Update device pairing status - use responder_id to find the remote device
    if let Ok(Some(device)) = device_repo::get_by_device_id(&payload.responder_id) {
        let (new_status, remark) = if payload.approved {
            (
                PairingStatus::Paired,
                t!("pairing.remark.remote-accepted").to_string(),
            )
        } else {
            // Remote device rejected our request, mark as blocked
            // 对方拒绝了本机的请求，标记为 blocked
            (
                PairingStatus::Blocked,
                t!("pairing.remark.remote-rejected").to_string(),
            )
        };
        log::info!("Updating device pairing status to {:?}", new_status);
        device_repo::update_pairing_status_with_remark(device.id, new_status, &remark, false)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    // Update the latest request status (any direction) - use responder_id to find the request
    // 更新最新的请求状态（不限方向）- 使用 responder_id 查找请求
    if let Ok(Some(latest_request)) =
        pairing_request_repo::get_latest_by_device_id_any(&payload.responder_id)
    {
        let (status, remark) = if payload.approved {
            (
                PairingRequestStatus::Accepted,
                t!("pairing.remark.remote-accepted").to_string(),
            )
        } else {
            (
                PairingRequestStatus::Rejected,
                t!("pairing.remark.remote-rejected").to_string(),
            )
        };
        let _ = pairing_request_repo::update_status_with_remark(latest_request.id, status, &remark);
    }

    // Notify frontend about pairing response
    if let Some(app_handle) = APP_HANDLE.get() {
        let _ = app_handle.emit("pairing-response-received", &payload);
    }

    Ok(Json(json!({
        "code": 0,
        "data": {
            "status": if payload.approved { "approved" } else { "rejected" }
        }
    })))
}

/// POST /pairing/reset_notify - Handle reset notification from remote device
/// POST /pairing/reset_notify - 处理远程设备的重置通知
///
/// When a remote device resets its pairing status, it notifies this device.
/// This device should also reset its pairing status for that device.
/// 当远程设备重置配对状态时，它会通知本设备。
/// 本设备也应该重置该设备的配对状态。
pub async fn handle_reset_notify(
    Json(payload): Json<ResetNotifyPayload>,
) -> Result<Json<Value>, StatusCode> {
    log::info!(
        "Received reset notify from {} (previous status: {})",
        payload.from_device_id,
        payload.previous_status
    );

    // Get local device record for this remote device
    // 获取该远程设备的本机记录
    if let Ok(Some(device)) = device_repo::get_by_device_id(&payload.from_device_id) {
        // Reset pairing status to None with remark
        // 重置配对状态为 None 并附带说明
        // Note: is_manual=true because remote reset is triggered by user action on remote device
        // 注意: is_manual=true 因为远程重置是由对方设备上的用户操作触发的
        let remark = t!("pairing.remark.remote-reset").to_string();
        device_repo::update_pairing_status_with_remark(
            device.id,
            PairingStatus::None,
            &remark,
            true,
        )
        .map_err(|e| {
            log::error!(
                "Failed to update pairing status for device {}: {:?}",
                payload.from_device_id,
                e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        // Update the latest pairing request record for this device
        // 更新该设备最新的配对请求记录
        if let Ok(Some(latest_request)) =
            pairing_request_repo::get_latest_by_device_id_any(&payload.from_device_id)
        {
            let _ = pairing_request_repo::update_status_with_remark(
                latest_request.id,
                PairingRequestStatus::Cancelled,
                &remark,
            );
        }

        // Notify frontend about pairing reset
        if let Some(app_handle) = APP_HANDLE.get() {
            let _ = app_handle.emit("pairing-reset-received", &payload);
        }

        log::info!(
            "Reset pairing status for device {} to None (was notified by remote)",
            payload.from_device_id
        );
    }

    Ok(Json(json!({
        "code": 0,
        "data": {
            "status": "ok"
        }
    })))
}
