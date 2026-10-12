use crate::entities::ModelPlatform;
use crate::entities::SelfHostedPlatform;
use crate::errors::AppError;
use crate::global::PROXY;
use crate::repositories::model_platform_repo;
use crate::structs::proxy_setting::ProxyInfo;
use async_openai::Client;
use async_openai::config::OpenAIConfig;

pub async fn init_service(
    platform_name: &str,
    base_url: Option<String>,
) -> (ModelPlatform, ProxyInfo) {
    let platform = model_platform_repo::get_one(platform_name).unwrap_or_else(|e| {
        log::error!("get model platform error: {e}");
        let mut pv = ModelPlatform::default();
        if base_url.is_some() {
            pv.base_url = base_url.unwrap_or("".to_string());
        }
        if pv.base_url.is_empty() {
            log::debug!("base_url is empty");
        }
        pv
    });
    (platform, PROXY.read().await.clone())
}

pub fn create_client(
    platform: &ModelPlatform,
    proxy: &ProxyInfo,
) -> Result<Client<OpenAIConfig>, AppError> {
    if platform.api_key.is_empty() {
        return Err(AppError::InternalError("API key is required".to_string()));
    }

    if platform.is_proxy_enable {
        if proxy.host.is_empty() || proxy.port == 0 {
            return Err(AppError::InternalError(
                "Invalid proxy configuration".to_string(),
            ));
        }
        log::debug!(
            "Using proxy: {}://{}:{}",
            proxy.protocol,
            proxy.host,
            proxy.port
        );
    }

    let open_ai_config = if platform.base_url.is_empty() {
        OpenAIConfig::new().with_api_key(platform.api_key.clone())
    } else {
        OpenAIConfig::new()
            .with_api_key(platform.api_key.clone())
            .with_api_base(platform.base_url.clone())
    };

    let client = if platform.is_proxy_enable && !proxy.host.is_empty() {
        let request_proxy = reqwest::Proxy::http(format!(
            "{}://{}:{}",
            proxy.protocol, proxy.host, proxy.port
        ))?;
        log::debug!("request_proxy: {:?}", request_proxy);
        Client::with_config(open_ai_config)
            .with_http_client(reqwest::Client::builder().proxy(request_proxy).build()?)
    } else {
        Client::with_config(open_ai_config)
    };

    Ok(client)
}

/// Create client for self-hosted platform (Ollama, vLLM, etc.)
/// Self-hosted platforms don't require API key and don't use proxy
pub fn create_client_for_self_hosted(
    platform: &SelfHostedPlatform,
) -> Result<Client<OpenAIConfig>, AppError> {
    // Ollama and vLLM OpenAI-compatible API endpoint requires /v1 suffix
    let base_url = format!("http://{}:{}/v1", platform.host, platform.port);

    let open_ai_config = OpenAIConfig::new()
        .with_api_key("none") // Self-hosted platforms typically don't require API key
        .with_api_base(base_url);

    let client = Client::with_config(open_ai_config);

    Ok(client)
}
