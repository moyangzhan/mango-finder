use crate::enums::{DownloadEvent, Locale};
use crate::global::{
    ACTIVE_LOCALE, DOWNLOADING, EXIT_APP_SIGNAL, HUGGINFACE_MIRROR, HUGGINFACE_WEBSITE,
    MULTI_LANG_MODEL_URL, MULTI_LANG_TOKENIZER_URL, PROXY,
};
use crate::structs::proxy_setting::ProxyInfo;
use crate::utils::app_util;
use crate::utils::app_util::{get_multilingual_embedding_path, get_multilingual_tokenizer_path};
use crate::utils::path_util::check_and_move;
use anyhow::{Context, Result, anyhow};
use futures::StreamExt;
use log::{error, info};
use rand::Rng;
use rand::distr::Alphanumeric;
use reqwest::header;
use reqwest::header::HeaderValue;
use reqwest::{Client, header::RANGE};
use std::fs;
use std::sync::atomic::Ordering;
use std::{
    fs::File,
    io::{Seek, SeekFrom, Write},
    path::Path,
    time::Duration,
};
use tauri::ipc::Channel;
use tokio;

pub async fn download_multilingual_model(
    enable_proxy: bool,
    on_event: &Channel<DownloadEvent>,
) -> Result<()> {
    info!("Downloading multilingual model");
    if DOWNLOADING.load(Ordering::SeqCst) {
        return Err(anyhow!("Downloading is in progress"));
    }
    let tmp_path = app_util::get_assets_tmp_path();
    let mut model_url = MULTI_LANG_MODEL_URL.to_string();
    let mut tokenizer_url = MULTI_LANG_TOKENIZER_URL.to_string();
    // Use mirror first when zh-CN locale is active
    if *crate::read_lock!(ACTIVE_LOCALE) == Locale::ZhCn.text() {
        model_url = model_url.replace(HUGGINFACE_WEBSITE, HUGGINFACE_MIRROR);
        tokenizer_url = tokenizer_url.replace(HUGGINFACE_WEBSITE, HUGGINFACE_MIRROR);
    }
    let file_urls = vec![model_url.clone(), tokenizer_url];
    let tmp_paths = vec![
        format!("{}/model.onnx", tmp_path).to_string(),
        format!("{}/tokenizer.json", tmp_path).to_string(),
    ];
    match download_files(&file_urls, &tmp_paths, enable_proxy, on_event).await {
        Ok(_) => {}
        Err(error) => {
            error!("Failed to download model: {error}");
            if model_url.contains(HUGGINFACE_MIRROR) {
                error!("Failed to download from mirror, trying to download from original source");
                let file_urls = vec![
                    MULTI_LANG_MODEL_URL.to_string(),
                    MULTI_LANG_TOKENIZER_URL.to_string(),
                ];
                download_files(&file_urls, &tmp_paths, enable_proxy, on_event).await?;
            } else {
                error!("Failed to download from original source, trying to download from mirror");
                let file_urls = vec![
                    MULTI_LANG_MODEL_URL
                        .to_string()
                        .replace(HUGGINFACE_WEBSITE, HUGGINFACE_MIRROR),
                    MULTI_LANG_TOKENIZER_URL
                        .to_string()
                        .replace(HUGGINFACE_WEBSITE, HUGGINFACE_MIRROR),
                ];
                download_files(&file_urls, &tmp_paths, enable_proxy, on_event).await?;
            }
        }
    }
    check_and_move(
        tmp_paths[0].as_str(),
        get_multilingual_embedding_path().as_str(),
    )?;
    check_and_move(
        tmp_paths[1].as_str(),
        get_multilingual_tokenizer_path().as_str(),
    )?;
    Ok(())
}

pub async fn download_files(
    file_urls: &Vec<String>,
    local_paths: &Vec<String>,
    enable_proxy: bool,
    on_event: &Channel<DownloadEvent>,
) -> Result<()> {
    info!("Downloading files");
    DOWNLOADING.store(true, Ordering::SeqCst);
    let mut builder = Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(Duration::from_secs(30))
        .default_headers({
            let mut headers = header::HeaderMap::new();
            headers.insert(
                "Accept",
                HeaderValue::from_static("application/octet-stream"),
            );
            headers.insert(
                "Accept-Language",
                HeaderValue::from_static("en-US,en;q=0.9"),
            );
            headers
        });
    if enable_proxy {
        let proxy_info: ProxyInfo = (*PROXY.read().await).clone();
        builder = builder.proxy(reqwest::Proxy::http(format!(
            "{}://{}:{}",
            proxy_info.protocol, proxy_info.host, proxy_info.port
        ))?);
    }
    let client = builder.build()?;
    for (file_url, local_path) in file_urls.iter().zip(local_paths.iter()) {
        info!("Downloading file:{}", file_url);
        if EXIT_APP_SIGNAL.load(Ordering::SeqCst) {
            DOWNLOADING.store(false, Ordering::SeqCst);
            return Err(anyhow!("Download interrupted by exit signal"));
        }
        let file_size = get_remote_file_size(&client, &file_url)
            .await
            .or_else(|error| {
                DOWNLOADING.store(false, Ordering::SeqCst);
                Err(error)
            })?;
        info!("Remote file size: {}", file_size);
        if file_size == 0 {
            DOWNLOADING.store(false, Ordering::SeqCst);
            error!("File size is 0,url:{}", file_url);
            continue;
        }
        let download_id: String = rand::rng()
            .sample_iter(&Alphanumeric)
            .take(20)
            .map(char::from)
            .collect();
        let path = Path::new(local_path);
        on_event
            .send(DownloadEvent::Start {
                url: file_url.to_string(),
                download_id: download_id.clone(),
            })
            .unwrap_or_else(|e| {
                error!("Failed to send started event: {}", e);
            });
        download_file(
            &client,
            &file_url,
            &path,
            file_size,
            5,
            10,
            download_id.as_str(),
            on_event,
        )
        .await
        .or_else(|error| {
            DOWNLOADING.store(false, Ordering::SeqCst);
            let _ = on_event.send(DownloadEvent::Error {
                download_id,
                error: format!("Download file failed:{}", error.to_string()),
            });
            Err(error)
        })?;
    }
    DOWNLOADING.store(false, Ordering::SeqCst);
    Ok(())
}

pub async fn download_file(
    client: &Client,
    url: &str,
    local_path: &Path,
    total_size: u64,
    max_retries: u32,
    retry_delay: u64,
    download_id: &str,
    on_event: &Channel<DownloadEvent>,
) -> Result<()> {
    info!("Downloading file from {} to {}", url, local_path.display());
    // Create the parent directory if it doesn't exist
    if let Some(parent) = local_path.parent() {
        fs::create_dir_all(parent)?;
    }
    // Download with retry
    let mut retries = 0;
    let mut last_start_pos = 0;
    loop {
        // Check local file size to determine starting position
        let start_pos = match File::open(local_path) {
            Ok(file) => {
                let metadata = file.metadata()?;
                if metadata.len() == total_size {
                    info!("✅ Comleted");
                    on_event
                        .send(DownloadEvent::Finish {
                            download_id: download_id.to_string(),
                        })
                        .unwrap_or_else(|e| log::error!("Failed to send finished event: {}", e));
                    return Ok(());
                }
                info!(
                    "⏳ Continue download (downloaded {}/{})",
                    metadata.len(),
                    total_size
                );
                metadata.len()
            }
            Err(_) => 0, // File does not exist, start from the beginning
        };
        // If the starting position has changed, it means progress has been made, reset retries
        if last_start_pos != start_pos {
            retries = 0;
            last_start_pos = start_pos;
        }
        match try_download_range(
            client,
            url,
            local_path,
            start_pos,
            total_size,
            download_id.as_ref(),
            on_event,
        )
        .await
        {
            Ok(_) => {
                info!("✅Download completed");
                on_event
                    .send(DownloadEvent::Finish {
                        download_id: download_id.to_string(),
                    })
                    .unwrap_or_else(|e| log::error!("Failed to send finished event: {}", e));
                return Ok(());
            }
            Err(e) => {
                retries += 1;
                if retries > max_retries {
                    return Err(e).context(format!(
                        "Exceeded max retries times ({}) Exceeded max retries",
                        max_retries
                    ));
                }
                error!(
                    "  ❌ failed( {:?} ), {} restart in seconds ( {}/{} )",
                    e, retry_delay, retries, max_retries
                );
                tokio::time::sleep(Duration::from_secs(retry_delay)).await;
            }
        }
    }
}

// Try to download the specified range of a file
async fn try_download_range(
    client: &Client,
    url: &str,
    local_path: &Path,
    start_pos: u64,
    total_size: u64,
    download_id: &str,
    on_event: &Channel<DownloadEvent>,
) -> Result<()> {
    info!("Downloading file range {} to {}", start_pos, total_size - 1);
    let range_header = format!("bytes={}-{}", start_pos, total_size - 1);
    let response = client
        .get(url)
        .header(RANGE, range_header.clone())
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .with_context(|| format!("request error: {}", url))?;

    // Accept both 200 and 206, but handle them differently:
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow::anyhow!("Error from server: {} ({})", status, url));
    }

    // If we asked for a range (start_pos > 0), we expect 206 Partial Content.
    // If server returned 200 OK, it likely ignored Range and sent full file.
    let is_partial = status.as_u16() == 206;
    if start_pos > 0 && !is_partial {
        // Server didn't honor range. Best approach: restart download from 0.
        info!("Server returned 200 OK for range request. Restarting download from scratch.");
        // Optionally remove the incomplete file or truncate it:
        let mut f = File::options()
            .create(true)
            .write(true)
            .truncate(true)
            .open(local_path)?;
        // Then stream the response body into file from beginning:
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            f.write_all(&chunk)?;
        }
        f.sync_all()?;
        on_event
            .send(DownloadEvent::Finish {
                download_id: download_id.to_string(),
            })
            .ok();
        return Ok(());
    }

    // Open file for random write (not append!), so seek works as expected.
    let mut file = File::options()
        .create(true)
        .write(true) // <- not append
        .open(local_path)
        .with_context(|| format!("Can not open file: {:?}", local_path))?;

    // If server gave a full file (206 with content-range still contains partial), we should seek to start_pos and overwrite from there.
    file.seek(SeekFrom::Start(start_pos))?;

    // Stream the response into the file
    let mut bytes_downloaded = 0u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if EXIT_APP_SIGNAL.load(Ordering::SeqCst) {
            DOWNLOADING.store(false, Ordering::SeqCst);
            return Err(anyhow!("Download interrupted by exit signal"));
        }
        let chunk = chunk?;
        file.write_all(&chunk)?;
        bytes_downloaded += chunk.len() as u64;

        if bytes_downloaded % (1024 * 1024) < chunk.len() as u64 {
            let progress = start_pos + bytes_downloaded;
            info!(
                "process: {}/{} ({:.2}%)\r",
                progress,
                total_size,
                (progress as f64 / total_size as f64) * 100.0
            );
            on_event
                .send(DownloadEvent::Progress {
                    download_id: download_id.to_string(),
                    progress: (progress as f64 / total_size as f64) * 100.0,
                })
                .ok();
        }
    }

    file.sync_all()?; // ensure data flushed

    on_event
        .send(DownloadEvent::Finish {
            download_id: download_id.to_string(),
        })
        .ok();
    Ok(())
}

async fn get_remote_file_size(client: &Client, url: &str) -> Result<u64> {
    let response = match client
        .head(url)
        .header("Accept", "application/octet-stream")
        .header("Accept-Encoding", "identity")
        .timeout(Duration::from_secs(5))
        .send()
        .await
    {
        Ok(response) => {
            if response.status().is_success() {
                response
            } else {
                error!(
                    "HEAD request failed with status: {}, trying GET",
                    response.status()
                );
                client
                    .get(url)
                    .header("Accept", "application/octet-stream")
                    .header("Accept-Encoding", "identity")
                    .timeout(Duration::from_secs(10))
                    .send()
                    .await?
            }
        }
        Err(e) => {
            error!("HEAD request failed: {}, trying GET", e);
            client
                .get(url)
                .header("Accept", "application/octet-stream")
                .header("Accept-Encoding", "identity")
                .timeout(Duration::from_secs(10))
                .send()
                .await?
        }
    };
    info!("Response headers: {:?}", response.headers());
    if !response.status().is_success() {
        return Err(anyhow!(format!(
            "Request failed with status: {}",
            response.status()
        )));
    }
    let content_length = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .ok_or_else(|| anyhow!("Could not determine file size for URL: {}", url))?;

    Ok(content_length)
}
