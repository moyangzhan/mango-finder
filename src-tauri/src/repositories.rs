pub mod ai_model_repo;
pub mod config_repo;
pub mod device_repo;
pub mod embedding_collection_repo;
pub mod file_content_embedding_repo;
pub mod file_content_fts_repo;
pub mod file_info_repo;
pub mod file_metadata_embedding_repo;
pub mod indexing_task_repo;
pub mod model_platform_repo;
pub mod pairing_request_repo;
pub mod self_hosted_platform_repo;

use crate::errors::AppError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("DateTime parse error: {0}")]
    DateTimeParse(String),
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    #[error("Database error: {0}")]
    Database(rusqlite::Error),
    #[error("Invalid parameter: {0}")]
    InvalidParam(String),
    #[error("Database connection error: {0}")]
    DbConnection(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Invalid operation: {0}")]
    InvalidOperation(String),
}

impl From<chrono::ParseError> for RepositoryError {
    fn from(err: chrono::ParseError) -> Self {
        RepositoryError::DateTimeParse(err.to_string())
    }
}

impl From<AppError> for RepositoryError {
    fn from(err: AppError) -> Self {
        match err {
            AppError::DateTimeParseError(e) => RepositoryError::DateTimeParse(e.to_string()),
            _ => RepositoryError::DateTimeParse(err.to_string()),
        }
    }
}

impl From<rusqlite::Error> for RepositoryError {
    fn from(err: rusqlite::Error) -> Self {
        RepositoryError::Database(err)
    }
}

impl From<RepositoryError> for rusqlite::Error {
    fn from(err: RepositoryError) -> Self {
        match err {
            RepositoryError::Database(e) => e,
            RepositoryError::InvalidInput(msg) => rusqlite::Error::InvalidParameterName(msg),
            RepositoryError::DateTimeParse(msg) => rusqlite::Error::InvalidColumnType(
                0,
                format!("datetime parse error: {}", msg),
                rusqlite::types::Type::Text,
            ),
            RepositoryError::InvalidParam(msg) => rusqlite::Error::InvalidParameterName(msg),
            RepositoryError::DbConnection(msg) => rusqlite::Error::InvalidParameterName(msg),
            RepositoryError::NotFound(msg) => rusqlite::Error::InvalidParameterName(msg),
            RepositoryError::InvalidOperation(msg) => rusqlite::Error::InvalidParameterName(msg),
        }
    }
}
