use crate::entities::{ModelPlatform, SelfHostedPlatform};
use crate::enums::ModelPlatformName;
use crate::errors::AppError;
use crate::global::{
    ACTIVE_MODEL_PLATFORM, ACTIVE_SELF_HOSTED_PLATFORM, CONFIG_NAME_ACTIVE_SELF_HOSTED_PLATFORM,
    CONFIG_NAME_INDEXER_SETTING, CONFIG_NAME_PROXY,
};
use crate::indexer_service;
use crate::repositories::{
    ai_model_repo, config_repo, file_info_repo, model_platform_repo, self_hosted_platform_repo,
};
use crate::structs::proxy_setting::ProxyInfo;
use crate::utils::app_util;
use crate::utils::locale_util;
use tauri::{AppHandle, command};

// ========== Locale commands ==========

#[command]
pub async fn load_active_locale() -> Result<String, String> {
    Ok(locale_util::current_locale())
}

#[command]
pub async fn set_active_locale(app: AppHandle, locale: &str) -> Result<usize, String> {
    // no tray rebuild (preserves the original GUI behavior)
    if locale.is_empty() {
        return Ok(0);
    }
    // GUI soft contract: invalid input warns and returns 0, never errors
    let rows = match locale_util::set_locale_core(locale) {
        Ok(rows) => rows,
        Err(e) => {
            log::warn!("{}", e);
            return Ok(0);
        }
    };
    let _ = app_util::rebuild_tray_menu(&app);
    log::debug!("update db result: {}", rows);
    Ok(rows)
}

// ========== Model platform commands ==========

#[command]
pub async fn load_model_platforms() -> Vec<ModelPlatform> {
    model_platform_repo::list(&vec![
        ModelPlatformName::OpenAi.text().to_string(),
        ModelPlatformName::SiliconFlow.text().to_string(),
    ])
    .unwrap_or_else(|e| {
        log::error!("Failed to load model platforms: {}", e);
        vec![]
    })
}

#[command]
pub async fn load_model_by_type(
    platform: &str,
    one_type: &str,
) -> Result<Option<crate::entities::AiModel>, String> {
    let result = ai_model_repo::get_one_by_type(platform, one_type)?;
    Ok(result)
}

#[command]
pub async fn update_ai_model(
    id: i64,
    name: String,
    title: String,
    remark: String,
) -> Result<usize, String> {
    let result = ai_model_repo::update_basic(id, &name, &title, &remark)?;
    Ok(result)
}

#[command]
pub async fn update_model_platform(platform: ModelPlatform) -> Result<usize, AppError> {
    let result = model_platform_repo::update_by_name(&platform.name, &platform)?;
    if platform.name == crate::read_lock!(ACTIVE_MODEL_PLATFORM).name {
        match ACTIVE_MODEL_PLATFORM.try_write() {
            Ok(mut guard) => {
                let one = model_platform_repo::get_one(&platform.name)?;
                *guard = one;
            }
            Err(e) => {
                log::error!(
                    "Failed to acquire write lock for ACTIVE_MODEL_PLATFORM: {}",
                    e
                );
            }
        }
    }
    Ok(result)
}

#[command]
pub async fn check_self_hosted_platform(platform: SelfHostedPlatform) -> Result<String, String> {
    use crate::repositories::ai_model_repo;
    use crate::utils::llm_client_util::create_client_for_self_hosted;

    if platform.host.is_empty() {
        return Err("Host is required".to_string());
    }

    // 1. Check platform availability
    let base_url = format!("http://{}:{}", platform.host, platform.port);
    // Bounded so a blackholed network cannot leave the caller hanging
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&base_url)
        .send()
        .await
        .map_err(|_| format!("Unable to connect to {}:{}", platform.host, platform.port))?;
    if !resp.status().is_success() {
        return Err(format!("Platform returned HTTP {}", resp.status()));
    }

    // 2. Check if configured models are available
    let openai_client = create_client_for_self_hosted(&platform).map_err(|e| e.to_string())?;
    let models = openai_client
        .models()
        .list()
        .await
        .map_err(|e| format!("Failed to list models: {}", e))?;
    let remote_model_ids: Vec<&str> = models.data.iter().map(|m| m.id.as_str()).collect();

    let configured = ai_model_repo::list_by_platform(&platform.name).map_err(|e| e.to_string())?;
    let mut missing = vec![];
    for model in &configured {
        if !remote_model_ids.iter().any(|id| id == &model.name.as_str()) {
            missing.push(model.name.clone());
        }
    }

    if !missing.is_empty() {
        return Err(format!(
            "Configured models not found on platform: {}",
            missing.join(", ")
        ));
    }

    Ok(format!("ok ({} models)", configured.len()))
}

/// Model ids served by a self-hosted platform. One call doubles as the
/// reachability probe and the per-category model presence check (settings
/// diagnostics list every used backend that cannot serve right now).
#[command]
pub async fn list_self_hosted_models(platform: SelfHostedPlatform) -> Result<Vec<String>, String> {
    use crate::utils::llm_client_util::create_client_for_self_hosted;

    if platform.host.is_empty() {
        return Err("Host is required".to_string());
    }
    let client = create_client_for_self_hosted(&platform).map_err(|e| e.to_string())?;
    let models = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        client.models().list(),
    )
    .await
    .map_err(|_| "Request timed out".to_string())?
    .map_err(|e| e.to_string())?;
    Ok(models.data.into_iter().map(|m| m.id).collect())
}

#[command]
pub async fn load_active_platform() -> String {
    let platform = crate::read_lock!(ACTIVE_MODEL_PLATFORM);
    platform.name.clone()
}

#[command]
pub async fn set_active_platform(platform_name: &str) -> Result<usize, String> {
    let Ok(platform) = model_platform_repo::get_one(platform_name) else {
        log::error!("Failed to get platform: {}", platform_name);
        return Ok(0);
    };

    let result = config_repo::update_by_name("active_model_platform", platform_name)
        .unwrap_or_else(|e| {
            log::error!("update config error: {}", e);
            0
        });
    *crate::write_lock!(ACTIVE_MODEL_PLATFORM) = platform;
    return Ok(result);
}

// ========== Self-hosted platform commands ==========

#[command]
pub async fn load_self_hosted_platforms() -> Vec<SelfHostedPlatform> {
    self_hosted_platform_repo::list().unwrap_or_else(|e| {
        log::error!("Failed to load self-hosted platforms: {}", e);
        vec![]
    })
}

#[command]
pub async fn load_active_self_hosted_platform() -> String {
    let platform = crate::read_lock!(ACTIVE_SELF_HOSTED_PLATFORM);
    platform.name.clone()
}

#[command]
pub async fn set_active_self_hosted_platform(platform_name: &str) -> Result<usize, String> {
    let Ok(platform) = self_hosted_platform_repo::get_one(platform_name) else {
        log::error!("Failed to get self-hosted platform: {}", platform_name);
        return Ok(0);
    };

    let result =
        config_repo::update_by_name(CONFIG_NAME_ACTIVE_SELF_HOSTED_PLATFORM, platform_name)
            .unwrap_or_else(|e| {
                log::error!("update config error: {}", e);
                0
            });
    *crate::write_lock!(ACTIVE_SELF_HOSTED_PLATFORM) = platform;
    return Ok(result);
}

#[command]
pub async fn update_self_hosted_platform(platform: SelfHostedPlatform) -> Result<usize, AppError> {
    let result = self_hosted_platform_repo::update_by_name(&platform.name, &platform)?;
    if platform.name == crate::read_lock!(ACTIVE_SELF_HOSTED_PLATFORM).name {
        match ACTIVE_SELF_HOSTED_PLATFORM.try_write() {
            Ok(mut guard) => {
                let one = self_hosted_platform_repo::get_one(&platform.name)?;
                *guard = one;
            }
            Err(e) => {
                log::error!(
                    "Failed to acquire write lock for ACTIVE_SELF_HOSTED_PLATFORM: {}",
                    e
                );
            }
        }
    }
    Ok(result)
}

#[command]
pub async fn check_model_platform(platform: ModelPlatform) -> Result<String, String> {
    use crate::global::PROXY;

    if platform.base_url.is_empty() {
        return Err("Base URL is required".to_string());
    }
    if platform.api_key.is_empty() {
        return Err("API key is required".to_string());
    }

    let url = format!("{}/models", platform.base_url.trim_end_matches('/'));
    let proxy = PROXY.read().await.clone();

    // Bounded so a blackholed network cannot leave the caller hanging
    let builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(8));
    let client = if platform.is_proxy_enable && !proxy.host.is_empty() {
        let proxy_url = format!("{}://{}:{}", proxy.protocol, proxy.host, proxy.port);
        builder
            .proxy(reqwest::Proxy::http(&proxy_url).map_err(|e| e.to_string())?)
            .build()
            .map_err(|e| e.to_string())?
    } else {
        builder.build().map_err(|e| e.to_string())?
    };

    let resp = client
        .get(&url)
        .bearer_auth(&platform.api_key)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.status().is_success() {
        Ok("ok".to_string())
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(format!("HTTP {}: {}", status, body))
    }
}

// ========== Proxy commands ==========

#[command]
pub async fn load_proxy_info() -> Result<ProxyInfo, String> {
    let result = config_repo::get_one(CONFIG_NAME_PROXY)?
        .map(|config| serde_json::from_str(&config.value).map_err(|e| e.to_string()))
        .unwrap_or_else(|| Ok(ProxyInfo::default()))?;
    Ok(result)
}

#[command]
pub async fn update_proxy_info(proxy_info: ProxyInfo) -> Result<usize, String> {
    let proxy_json = serde_json::to_string(&proxy_info).map_err(|e| AppError::SerializeError(e))?;
    Ok(config_repo::update_by_name("proxy", &proxy_json)?)
}

// ========== Indexer setting commands ==========

#[command]
pub async fn load_indexer_setting()
-> Result<crate::structs::indexer_setting::IndexerSetting, String> {
    let mut result = config_repo::get_one(CONFIG_NAME_INDEXER_SETTING)?
        .map(|config| {
            serde_json::from_str(&config.value).map_err(|e| {
                log::error!("Failed to parse indexer setting: {}", e);
                e.to_string()
            })
        })
        .unwrap_or_else(|| Ok(crate::structs::indexer_setting::IndexerSetting::default()))?;

    // Add HOME_PATH to ignore_path_prefixes if not already present
    if let Some(home_path) = crate::global::HOME_PATH.get() {
        if !result.ignore_path_prefixes.contains(&home_path.to_string()) {
            result.ignore_path_prefixes.push(home_path.to_string());
        }
    }

    Ok(result)
}

#[command]
pub async fn update_indexer_setting(
    indexer_setting: crate::structs::indexer_setting::IndexerSetting,
) -> Result<usize, String> {
    indexer_service::update_indexer_setting(indexer_setting).await
}

#[command]
pub async fn is_embedding_model_changed() -> Result<bool, String> {
    return indexer_service::is_embedding_model_changed();
}

#[command]
pub async fn change_content_storage(category: &str, new_mode: &str) -> Result<(), String> {
    indexer_service::start_content_storage_change(category, new_mode).await
}

#[command]
pub async fn count_indexed_files(category: &str) -> Result<i64, String> {
    let cat = match category {
        "document" => 1,
        "image" => 2,
        "audio" => 3,
        _ => return Err(format!("Unknown category: {}", category)),
    };
    file_info_repo::count_indexed_by_category(cat)
        .map_err(|e: crate::repositories::RepositoryError| e.to_string())
}

// ========== Embedding backend & vector collection commands ==========

/// Per-category availability of the bundled local model backends (settings-page
/// greying, preset fallback and first-run default all share this one source).
#[command]
pub async fn check_local_backends()
-> Result<crate::utils::resource_util::LocalBackendStatus, String> {
    Ok(crate::utils::resource_util::check_local_backends())
}

/// Startup diagnostic: diff the resource snapshot against the previous startup.
/// Non-empty vanished list = install damage / antivirus quarantine / deletion →
/// the GUI shows the diagnostic modal. The baseline is scoped to the installed
/// edition via the app identifier (editions have separate data directories;
/// scoping also covers a `.config` override converging them on one).
#[derive(serde::Serialize)]
pub struct ResourceDiagnostic {
    pub vanished: Vec<String>,
    pub affected_categories: Vec<String>,
}

#[command]
pub async fn get_resource_diagnostic(app: AppHandle) -> Result<ResourceDiagnostic, String> {
    let vanished =
        crate::utils::resource_util::startup_resource_check(&app.config().identifier);
    let affected_categories = crate::utils::resource_util::affected_categories(&vanished);
    Ok(ResourceDiagnostic {
        vanished,
        affected_categories,
    })
}

/// Active embedding identity (platform+model+dim+has_sparse) for the settings
/// UI. None = no vector library configured yet (fresh lite install).
#[command]
pub async fn get_active_embedding_identity()
-> Result<Option<crate::structs::embedding_identity::EmbeddingIdentity>, String> {
    Ok(crate::repositories::embedding_collection_repo::active_identity())
}

/// True while the active vector collection is not the fully indexed one —
/// semantic search stays refused until a (re)build completes.
#[command]
pub async fn embedding_rebuild_pending() -> bool {
    match crate::repositories::embedding_collection_repo::active_identity() {
        Some(active) => !matches!(
            crate::repositories::embedding_collection_repo::indexed_identity(),
            Some(ref indexed) if indexed.same_backend(&active)
        ),
        None => false,
    }
}

/// True while the active backend is known unreachable (declared-dim switch
/// fallback); the first successful embed clears it.
#[command]
pub async fn embedding_service_unreachable() -> bool {
    crate::repositories::embedding_collection_repo::service_unreachable()
}

/// Clear the unreachable marker once an API key is added for the platform the
/// active collection runs on — the marker was set under missing credentials
/// and that diagnosis no longer holds.
#[command]
pub async fn clear_embedding_service_unreachable() {
    if let Err(e) = crate::repositories::embedding_collection_repo::set_service_unreachable(false)
    {
        log::warn!("Failed to clear service_unreachable: {e}");
    }
}

/// All registered vector collections with live stats + active flag, for the
/// management tab.
#[derive(serde::Serialize)]
pub struct EmbeddingCollectionRow {
    #[serde(flatten)]
    pub info: crate::repositories::embedding_collection_repo::EmbeddingCollectionInfo,
    pub is_active: bool,
}

#[command]
pub async fn load_embedding_collections() -> Result<Vec<EmbeddingCollectionRow>, String> {
    let active = crate::repositories::embedding_collection_repo::active_identity();
    let rows = crate::repositories::embedding_collection_repo::list_collections()
        .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|info| {
            let is_active = active
                .as_ref()
                .map(|a| a.platform == info.platform && a.model == info.model)
                .unwrap_or(false);
            EmbeddingCollectionRow { info, is_active }
        })
        .collect())
}

/// Clear one non-active collection (drop its tables + registry row). The active
/// collection must be switched away from first - the switch flow itself guards it.
#[command]
pub async fn clear_embedding_collection(platform: String, model: String) -> Result<(), String> {
    if crate::global::INDEXING.load(std::sync::atomic::Ordering::SeqCst)
        || crate::global::SCANNING.load(std::sync::atomic::Ordering::SeqCst)
    {
        return Err("Indexing is in progress; try again after it finishes".to_string());
    }
    let active = crate::repositories::embedding_collection_repo::active_identity();
    if active
        .as_ref()
        .map(|a| a.platform == platform && a.model == model)
        .unwrap_or(false)
    {
        return Err("The active collection cannot be cleared; switch to another backend first".to_string());
    }
    // Dim only matters for the derived table names; look it up from the registry
    let identity = crate::repositories::embedding_collection_repo::list_collections()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|c| c.platform == platform && c.model == model)
        .map(|c| crate::structs::embedding_identity::EmbeddingIdentity {
            platform: c.platform,
            model: c.model,
            dim: c.dim,
            has_sparse: c.has_sparse,
        })
        .ok_or_else(|| format!("Collection not found: {} · {}", model, platform))?;
    crate::repositories::embedding_collection_repo::drop_collection(&identity)
        .map_err(|e| e.to_string())
}

/// Clear every non-active collection in one go.
#[command]
pub async fn clear_inactive_embedding_collections() -> Result<usize, String> {
    if crate::global::INDEXING.load(std::sync::atomic::Ordering::SeqCst)
        || crate::global::SCANNING.load(std::sync::atomic::Ordering::SeqCst)
    {
        return Err("Indexing is in progress; try again after it finishes".to_string());
    }
    let active = crate::repositories::embedding_collection_repo::active_identity();
    let collections = crate::repositories::embedding_collection_repo::list_collections()
        .map_err(|e| e.to_string())?;
    let mut cleared = 0;
    for collection in collections {
        if active
            .as_ref()
            .map(|a| a.platform == collection.platform && a.model == collection.model)
            .unwrap_or(false)
        {
            continue
        }
        let identity = crate::structs::embedding_identity::EmbeddingIdentity {
            platform: collection.platform,
            model: collection.model,
            dim: collection.dim,
            has_sparse: collection.has_sparse,
        };
        crate::repositories::embedding_collection_repo::drop_collection(&identity)
            .map_err(|e| e.to_string())?;
        cleared += 1;
    }
    Ok(cleared)
}

/// Switch the vector backend. The settings page confirms the rebuild (and its
/// cost) BEFORE calling this - the command itself is the single switch entry.
#[command]
pub async fn switch_embedding_backend(
    mode: String,
    platform: String,
    model: String,
    rebuild: String,
    clear_others: bool,
) -> Result<crate::indexer_service::SwitchOutcome, String> {
    let mode = match mode.as_str() {
        "local" => crate::enums::FileParserMode::Local,
        "selfhosted" => crate::enums::FileParserMode::SelfHosted,
        "remote" => crate::enums::FileParserMode::Remote,
        _ => return Err(format!("Unknown backend mode: {}", mode)),
    };
    crate::indexer_service::switch_embedding_backend(mode, platform, model, rebuild, clear_others)
        .await
}

/// Category-level optional re-index after changing a parser model
/// (re-transcribe audio / re-caption images with the new model).
#[command]
pub async fn reindex_category(category: String) -> Result<bool, String> {
    crate::indexer_service::start_category_reindex(&category).await
}

// ========== Config commands ==========

#[command]
pub async fn load_config_value(config_name: &str) -> Result<String, String> {
    let config = config_repo::get_one(config_name)?;
    if let Some(config) = config {
        return Ok(config.value);
    }
    Ok(String::new())
}

#[command]
pub async fn update_config(name: &str, value: &str) -> Result<(), String> {
    config_repo::update_by_name(name, value)?;
    Ok(())
}
