use crate::document_loaders::cleanup_extracted_images;
use crate::embedding_service_manager::get_manager;
use crate::entities::{FileContentEmbedding, FileInfo, FileMetaEmbedding, IndexingTask};
use crate::enums::{FileCategory, FileIndexStatus, IndexingEvent};
use crate::errors::{AppError, IndexingError};
use crate::global::{INDEXER_SETTING, STOP_INDEX_SIGNAL, STORAGE_PATH};
use crate::indexer_service;
use crate::repositories::{
    file_content_embedding_repo, file_content_fts_repo, file_info_repo,
    file_metadata_embedding_repo,
};
use crate::similarity::image_similarity;
use crate::structs::embed_result::EmbedResult;
use crate::structs::file_metadata::FileMetadata;
use crate::utils::{file_util, frontend_util, indexing_task_util, text_util};
use rust_i18n::t;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;

pub trait IndexingTemplate {
    async fn process(&mut self, task: Arc<IndexingTask>, from: &str) -> Result<(), IndexingError> {
        let event_name = indexer_service::get_event_from(from);

        // Read storage mode once for the entire batch to avoid per-file lock contention
        let storage_mode = {
            INDEXER_SETTING
                .read()
                .await
                .content_storage
                .get_for_category(self.category())
                .to_string()
        };

        let mut min_id = 0i64;
        let mut loop_count = 0;
        let limit = 1000;
        let total = file_info_repo::count_unindexed_files(self.category().value())?;
        log::info!("Total {:?} files to index: {}", self.category(), total);
        if total == 0 {
            return Ok(());
        }
        let max_loop = total / limit + 1; // Max times to avoid infinite loop
        indexing_task_util::set_total_by_category(self.category(), total).await;
        'outer: loop {
            if loop_count > max_loop {
                log::warn!("Max loop reached, exiting...");
                break;
            }
            if STOP_INDEX_SIGNAL.load(Ordering::SeqCst) {
                log::info!("Stopping indexing process");
                frontend_util::send_event(
                    &event_name,
                    &IndexingEvent::Stop {
                        task_id: task.id,
                        msg: "Stop indexing, Stopped by user.".to_string(),
                    },
                );
                break;
            }
            log::debug!(
                "list_unindexed_files by min_id: {}, category: {:?}",
                min_id,
                self.category()
            );

            let file_infos =
                file_info_repo::list_unindexed_files(min_id, limit, self.category().value())?;
            if file_infos.is_empty() {
                break;
            }
            loop_count += 1;
            log::debug!(
                "Found {} files to index in batch {}",
                file_infos.len(),
                loop_count
            );
            min_id = file_infos
                .iter()
                .map(|info| info.id)
                .max()
                .unwrap_or(min_id + 1000);
            let mut removed_any = false;
            for file_info in file_infos {
                if STOP_INDEX_SIGNAL.load(Ordering::SeqCst) {
                    log::info!("Indexing process interrupted by stop signal");
                    break 'outer;
                }
                indexing_task_util::processed_incr(self.category(), 1).await;
                if !Path::new(&file_info.path).exists() {
                    log::debug!("File not exist: {}", file_info.path);
                    indexing_task_util::failed_incr(self.category(), 1).await;
                    // Clean up associated .md file if content was stored on disk
                    if let Some(ref relative_path) = file_info.content_ref_path {
                        if let Some(storage) = STORAGE_PATH.get() {
                            let md_path = Path::new(storage).join(relative_path);
                            let _ = std::fs::remove_file(&md_path);
                        }
                    }
                    cleanup_extracted_images(file_info.id);
                    file_info_repo::delete_by_id(file_info.id)?;
                    file_content_fts_repo::delete_by_file_id(file_info.id)?;
                    file_content_embedding_repo::delete_by_file_id(file_info.id)?;
                    file_metadata_embedding_repo::delete_by_file_id(file_info.id)?;
                    removed_any = true;
                    continue;
                }
                frontend_util::send_event(
                    &event_name,
                    &IndexingEvent::Embed {
                        task_id: task.id,
                        msg: format!("Embedding path: {}", &file_info.path),
                    },
                );
                if let Err(error) = self.embedding_one_file(&file_info, &storage_mode).await {
                    log::warn!("Embedding failed: {}", error);
                    indexing_task_util::failed_incr(self.category(), 1).await;
                }
            }
            // 本批次有删除时通知一次(而非每文件递增)
            // Notify once per batch if any rows were removed
            if removed_any {
                crate::searcher::path_search_engine::notify_paths_removed();
            }
        }
        Ok(())
    }

    async fn embedding_one_file(
        &self,
        file_info: &FileInfo,
        storage_mode: &str,
    ) -> Result<(), IndexingError> {
        let filtered_content = {
            let content = self.load_content(&file_info).await;
            text_util::collapse_newlines(&content)
        };
        let file_id = file_info.id;
        let path_str = file_info.path.as_str();
        let path = Path::new(path_str);
        let mut file_meta = file_util::get_meta_by_record(path, &file_info).await?;

        // Detect audio type for audio files based on transcription content
        // 根据转录内容检测音频类型
        if self.category() == &FileCategory::Audio {
            use crate::structs::file_metadata::AudioType;
            use crate::utils::audio_util::{
                detect_audio_type, extract_music_fingerprint_from_file,
            };
            let audio_type = detect_audio_type(&filtered_content, &file_info.path);
            file_meta.audio_type = Some(audio_type.into());
            let _ = file_info_repo::update_audio_type(file_id, audio_type.into());

            // Extract and store audio fingerprint for music files (for music similarity search)
            // 为音乐文件提取并存储音频指纹（用于音乐相似性搜索）
            if matches!(audio_type, AudioType::Music | AudioType::Mixed) {
                if let Some(fingerprint) = extract_music_fingerprint_from_file(&file_info.path) {
                    let _ =
                        file_info_repo::update_audio_fingerprint(file_id, &fingerprint.to_bytes());
                }
            }
        }

        // Calculate and store image hash for image files (for similarity search)
        if self.category() == &FileCategory::Image {
            if let Some(hash_bytes) = image_similarity::calculate_image_hash(&file_info.path) {
                let _ = file_info_repo::update_image_hash(file_id, &hash_bytes);
            }
        }

        // Per-category storage logic (storage_mode passed from outer loop to avoid per-file lock)
        match storage_mode {
            "file" => {
                // Save as .md file, store relative path in content_ref_path column
                if let Some(storage) = STORAGE_PATH.get() {
                    let relative_path =
                        crate::document_loaders::md_relative_path(file_info.id, &file_info.name);
                    let md_path = Path::new(storage).join(&relative_path);
                    if let Some(parent) = md_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Err(e) = std::fs::write(&md_path, &filtered_content) {
                        log::warn!("Failed to write markdown file {}: {}", md_path.display(), e);
                    }
                    file_info_repo::update_content_meta(
                        file_id,
                        "",
                        Some(&relative_path),
                        &file_meta.to_json(),
                    )?;
                } else {
                    file_info_repo::update_content_meta(file_id, "", None, &file_meta.to_json())?;
                }
            }
            "database" => {
                file_info_repo::update_content_meta(
                    file_id,
                    &filtered_content,
                    None,
                    &file_meta.to_json(),
                )?;
            }
            _ => {
                // "none" or unknown: only store metadata
                file_info_repo::update_content_meta(file_id, "", None, &file_meta.to_json())?;
            }
        }

        //Remove old index
        file_content_fts_repo::delete_by_file_id(file_id)?;
        file_content_embedding_repo::delete_by_file_id(file_id)?;
        file_metadata_embedding_repo::delete_by_file_id(file_id)?;

        embedding_metadata(file_id, &file_meta).await?;
        if filtered_content.is_empty() {
            file_info_repo::update_content_index_status(
                file_id,
                FileIndexStatus::Indexed.value(),
                t!("message.indexing-skip-empty-content").as_ref(),
            )?;
            log::debug!("Skip empty content: {}", path_str);
            indexing_task_util::skipped_incr(self.category(), 1).await;
        } else {
            match embedding_content(file_id, &filtered_content).await {
                Ok(_) => {
                    indexing_task_util::success_incr(self.category(), 1).await;
                }
                Err(error) => {
                    log::warn!("Embedding content error: {}", error);
                    let _ = indexing_task_util::failed_incr(self.category(), 1).await;
                }
            }
        }
        return Ok(());
    }

    async fn load_content(&self, file_info: &FileInfo) -> String;
    fn category(&self) -> &FileCategory;
}

pub async fn embedding_content(file_id: i64, content: &str) -> Result<(), IndexingError> {
    if content.is_empty() {
        return Err(IndexingError::EmptyContent);
    }

    // Embedding content
    let chunks = {
        let mut manager = get_manager().write().await;
        let embedding_service = manager.service().await?;
        text_util::split_text(&content, &embedding_service.tokenizer)
            .map_err(|op| AppError::DocumentSplitterError(op.to_string()))?
    };
    for (chunk_index, chunk_text) in chunks.into_iter().enumerate() {
        log::debug!(
            "Processing chunk {} with {} chars",
            chunk_index,
            chunk_text.len()
        );
        let mut keep_run = true;
        let chunk_embed_result = {
            let mut manager = get_manager().write().await;
            match manager.embed(&chunk_text).await {
                Ok(embedding) => embedding,
                Err(op) => {
                    drop(manager);
                    log::warn!("Embedding chunk error: {}", op);
                    let _ = file_info_repo::update_content_index_status(
                        file_id,
                        FileIndexStatus::IndexFailed.value(),
                        op.to_string().as_str(),
                    );
                    keep_run = false;
                    EmbedResult::default()
                }
            }
        };
        if !keep_run {
            continue;
        }
        if chunk_embed_result.dense.is_empty() {
            let _ = file_info_repo::update_content_index_status(
                file_id,
                FileIndexStatus::IndexFailed.value(),
                "Failed to convert embedding to array",
            );
            keep_run = false;
        }
        if !keep_run {
            continue;
        }
        let new_embedding_data = FileContentEmbedding {
            id: 0,
            file_id,
            embedding: chunk_embed_result.dense.try_into().unwrap_or([0.0; 1024]),
            chunk_index: chunk_index as i64,
            chunk_text,
            sparse_vec: chunk_embed_result.sparse,

            distance: -0.1,
            sparse_score: 0.0,
            score: 0,
        };
        let embedding_result = file_content_embedding_repo::insert(&new_embedding_data)?;
        if let Some(embedding_result) = embedding_result {
            file_content_fts_repo::insert(
                file_id,
                embedding_result.id,
                new_embedding_data.chunk_text.as_str(),
            )
            .await
            .unwrap_or_default();
        }
        let _ = file_info_repo::update_content_index_status(
            file_id,
            FileIndexStatus::Indexed.value(),
            "success",
        )?;
    }
    Ok(())
}

pub async fn embedding_metadata(
    file_id: i64,
    file_meta: &FileMetadata,
) -> Result<(), IndexingError> {
    // File meta embedding
    let mut guard = get_manager().write().await;
    let meta_embed_result = match guard.embed(file_meta.to_text().as_str()).await {
        Ok(embedding) => {
            drop(guard);
            embedding
        }
        Err(op) => {
            drop(guard);
            log::warn!("Embedding meta error: {}", op);
            file_info_repo::update_meta_index_status(
                file_id,
                FileIndexStatus::IndexFailed.value(),
                op.to_string().as_str(),
            )?;
            return Ok(());
        }
    };
    let meta_array: [f32; 256] = meta_embed_result.dense.try_into().unwrap_or([0.0; 256]);
    file_metadata_embedding_repo::insert(
        &(FileMetaEmbedding {
            id: 0,
            file_id,
            embedding: meta_array,
            sparse_vec: meta_embed_result.sparse,

            distance: -0.1,
            sparse_score: 0.0,
            score: 0,
        }),
    )?;
    file_info_repo::update_meta_index_status(file_id, FileIndexStatus::Indexed.value(), "success")?;
    return Ok(());
}
