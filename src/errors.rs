/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/errors.rs
 * Purpose: Domain error types for pipeline, embedding, storage, and parsing failures
 */

use thiserror::Error;

/// Top-level domain errors for the oxidized-index-mcp system.
#[derive(Error, Debug)]
pub enum OxiError {
    #[error("Pipeline error: {0}")]
    Pipeline(#[from] PipelineError),

    #[error("Embedding error: {0}")]
    Embedding(#[from] EmbeddingError),

    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("Parse error: {0}")]
    Parse(#[from] ParseError),

    #[error("Configuration error: {0}")]
    Config(String),
}

#[derive(Error, Debug)]
pub enum PipelineError {
    #[error("Discovery failed: {0}")]
    Discovery(String),

    #[error("Channel closed unexpectedly")]
    ChannelClosed,

    #[error("Job cancelled")]
    Cancelled,

    #[error("File read error for {path}: {source}")]
    FileRead {
        path: String,
        source: std::io::Error,
    },
}

#[derive(Error, Debug)]
pub enum EmbeddingError {
    #[error("API request failed: {0}")]
    ApiRequest(String),

    #[error("Rate limited (429), retry after {retry_after_secs:?}s")]
    RateLimited { retry_after_secs: Option<u64> },

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("Batch too large, shrinking")]
    BatchTooLarge,

    #[error("Max retries exceeded for batch")]
    MaxRetriesExceeded,
}

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Qdrant operation failed: {0}")]
    QdrantOperation(String),

    #[error("Collection creation failed: {0}")]
    CollectionCreation(String),

    #[error("Upsert failed: {0}")]
    UpsertFailed(String),

    #[error("Query failed: {0}")]
    QueryFailed(String),

    #[error("Delete failed: {0}")]
    DeleteFailed(String),
}

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Unsupported language: {0}")]
    UnsupportedLanguage(String),

    #[error("Tree-sitter parse failed for {path}")]
    TreeSitterFailed { path: String },

    #[error("UTF-8 decode error for {path}: {source}")]
    Utf8Decode {
        path: String,
        source: std::string::FromUtf8Error,
    },
}
