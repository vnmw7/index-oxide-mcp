/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/config.rs
 * Purpose: Configuration loaded from environment variables, designed for MCP client env setup
 */

use std::env;

/// Root configuration for the oxidized-index-mcp server.
#[derive(Debug, Clone)]
pub struct OxiConfig {
    pub server: ServerConfig,
    pub gemini: GeminiConfig,
    pub qdrant: QdrantConfig,
    pub pipeline: PipelineConfig,
    pub embedding: EmbeddingConfig,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub host_workspace_path: Option<String>,
    pub container_workspace_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GeminiConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
}

#[derive(Debug, Clone)]
pub struct QdrantConfig {
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    pub dimensions: u32,
}

#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub discovery_channel_size: usize,
    pub parser_channel_size: usize,
    pub embedder_channel_size: usize,
    pub indexer_channel_size: usize,
    pub parser_workers: usize,
    pub discovery_workers: usize,
    pub embed_concurrency: usize,
    pub index_concurrency: usize,
    pub embed_batch_max_tokens: usize,
    pub embed_batch_max_items: usize,
    pub index_batch_size: usize,
    pub max_retries: u32,
    pub rate_limit_rpm: u32,
}

impl OxiConfig {
    /// Load configuration from environment variables.
    /// All values have sensible defaults; only `GEMINI_API_KEY` is required.
    pub fn from_env() -> anyhow::Result<Self> {
        let api_key = env::var("GEMINI_API_KEY")
            .map_err(|_| anyhow::anyhow!("GEMINI_API_KEY environment variable is required"))?;

        Ok(Self {
            server: ServerConfig {
                host: env::var("OXI_SERVER_HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
                port: env::var("OXI_SERVER_PORT")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(8754),
                host_workspace_path: env::var("OXI_HOST_WORKSPACE_PATH").ok(),
                container_workspace_path: env::var("OXI_CONTAINER_WORKSPACE_PATH").ok(),
            },
            gemini: GeminiConfig {
                api_key,
                model: env::var("OXI_EMBEDDING_MODEL")
                    .unwrap_or_else(|_| "gemini-embedding-2-preview".to_string()),
                base_url: env::var("OXI_GEMINI_BASE_URL").unwrap_or_else(|_| {
                    "https://generativelanguage.googleapis.com/v1beta".to_string()
                }),
            },
            qdrant: QdrantConfig {
                url: env::var("QDRANT_URL").unwrap_or_else(|_| "http://localhost:6334".to_string()),
            },
            embedding: EmbeddingConfig {
                dimensions: env::var("OXI_EMBEDDING_DIMENSIONS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(3072),
            },
            pipeline: PipelineConfig {
                discovery_channel_size: parse_env_or("OXI_DISCOVERY_CHANNEL", 512),
                parser_channel_size: parse_env_or("OXI_PARSER_CHANNEL", 256),
                embedder_channel_size: parse_env_or("OXI_EMBEDDER_CHANNEL", 128),
                indexer_channel_size: parse_env_or("OXI_INDEXER_CHANNEL", 128),
                parser_workers: parse_env_or("OXI_PARSER_WORKERS", 4),
                discovery_workers: parse_env_or("OXI_DISCOVERY_WORKERS", 4),
                embed_concurrency: parse_env_or("OXI_EMBED_CONCURRENCY", 3),
                index_concurrency: parse_env_or("OXI_INDEX_CONCURRENCY", 4),
                embed_batch_max_tokens: parse_env_or("OXI_EMBED_BATCH_MAX_TOKENS", 8000),
                embed_batch_max_items: parse_env_or("OXI_EMBED_BATCH_MAX_ITEMS", 50),
                index_batch_size: parse_env_or("OXI_INDEX_BATCH_SIZE", 100),
                max_retries: parse_env_or("OXI_MAX_RETRIES", 5),
                rate_limit_rpm: parse_env_or("OXI_RATE_LIMIT_RPM", 15),
            },
        })
    }
}

fn parse_env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
