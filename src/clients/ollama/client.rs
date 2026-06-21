/*
 * System: Index Oxide MCP
 * Module: Ollama Client
 * File URL: index-oxide-mcp/src/clients/ollama/client.rs
 * Purpose: Ollama embedding API client with fast batching via /api/embed
 */

use crate::clients::{BatchEmbedResult, EmbedInput};
use crate::config::OllamaConfig;
use crate::errors::EmbeddingError;
use rand::RngExt;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tracing::{debug, error, info, warn};

pub struct OllamaClient {
    http: reqwest::Client,
    config: OllamaConfig,
    current_batch_max: AtomicU32,
    consecutive_rate_limits: AtomicU32,
}

#[derive(Debug, Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

impl OllamaClient {
    pub fn new(config: OllamaConfig) -> Self {
        let http = reqwest::Client::builder()
            .pool_max_idle_per_host(10)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(300))
            .tcp_keepalive(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            http,
            config,
            current_batch_max: AtomicU32::new(16),
            consecutive_rate_limits: AtomicU32::new(0),
        }
    }

    pub fn get_current_batch_max(&self) -> u32 {
        self.current_batch_max.load(Ordering::Relaxed)
    }

    pub fn build_batches(
        &self,
        inputs: Vec<EmbedInput>,
        max_tokens_per_batch: usize,
    ) -> Vec<Vec<EmbedInput>> {
        let batch_max = self.get_current_batch_max() as usize;
        let mut batches = Vec::new();
        let mut current_batch = Vec::new();
        let mut current_tokens = 0usize;

        for input in inputs {
            let tokens = match &input {
                EmbedInput::Text(t) => t.len() / 4,
                EmbedInput::Multimodal { text, .. } => {
                    text.as_ref().map(|t| t.len() / 4).unwrap_or(10)
                }
            };

            if !current_batch.is_empty()
                && (current_tokens + tokens > max_tokens_per_batch
                    || current_batch.len() >= batch_max)
            {
                batches.push(std::mem::take(&mut current_batch));
                current_tokens = 0;
            }

            current_tokens += tokens;
            current_batch.push(input);
        }

        if !current_batch.is_empty() {
            batches.push(current_batch);
        }

        batches
    }

    pub async fn embed_batch(
        &self,
        inputs: &[EmbedInput],
        _task_type: &str, // Ollama doesn't use task_type
        max_retries: u32,
    ) -> Result<BatchEmbedResult, EmbeddingError> {
        let consecutive = self.consecutive_rate_limits.load(Ordering::Relaxed);
        if consecutive >= 3 {
            warn!(
                "Circuit breaker: {} consecutive rate limits, pausing 30s",
                consecutive
            );
            tokio::time::sleep(Duration::from_secs(30)).await;
            self.consecutive_rate_limits.store(0, Ordering::Relaxed);
        }

        let url = format!("{}/api/embed", self.config.base_url.trim_end_matches('/'));

        let extracted_inputs: Vec<String> = inputs
            .iter()
            .map(|i| match i {
                EmbedInput::Text(t) => t.clone(),
                EmbedInput::Multimodal {
                    text, mime_type, ..
                } => {
                    if let Some(t) = text {
                        format!("[Image: {}] {}", mime_type, t)
                    } else {
                        format!("[Uncaptioned Image: {}]", mime_type)
                    }
                }
            })
            .collect();

        let req = EmbedRequest {
            model: &self.config.model,
            input: extracted_inputs,
        };

        let mut attempt = 0u32;
        let mut base_delay = Duration::from_secs(1);

        loop {
            attempt += 1;
            debug!(
                attempt,
                batch_size = inputs.len(),
                "Sending embedding batch to Ollama"
            );

            let response = self
                .http
                .post(&url)
                .json(&req)
                .send()
                .await
                .map_err(|e| EmbeddingError::ApiRequest(e.to_string()))?;

            let status = response.status();

            if status.is_success() {
                self.consecutive_rate_limits.store(0, Ordering::Relaxed);

                let embed_response: EmbedResponse = response
                    .json()
                    .await
                    .map_err(|e| EmbeddingError::InvalidResponse(e.to_string()))?;

                info!(
                    batch_size = inputs.len(),
                    embedding_count = embed_response.embeddings.len(),
                    "Embedding batch succeeded"
                );
                return Ok(BatchEmbedResult {
                    embeddings: embed_response.embeddings,
                });
            }

            if status == StatusCode::TOO_MANY_REQUESTS {
                self.consecutive_rate_limits.fetch_add(1, Ordering::Relaxed);
                if attempt > max_retries {
                    return Err(EmbeddingError::MaxRetriesExceeded);
                }
                let wait_time = if let Some(header) = response.headers().get("retry-after") {
                    header
                        .to_str()
                        .ok()
                        .and_then(|v| v.parse::<u64>().ok())
                        .map(Duration::from_secs)
                        .unwrap_or(base_delay)
                } else {
                    let jitter_ms = rand::rng().random_range(0..500);
                    let backoff = base_delay
                        .checked_mul(2u32.pow(attempt - 1))
                        .unwrap_or(Duration::from_secs(60))
                        .min(Duration::from_secs(60));
                    backoff + Duration::from_millis(jitter_ms)
                };
                warn!(
                    attempt,
                    wait_ms = wait_time.as_millis(),
                    "Rate limited, backing off"
                );
                let current_max = self.current_batch_max.load(Ordering::Relaxed);
                if current_max > 5 {
                    self.current_batch_max
                        .store((current_max / 2).max(5), Ordering::Relaxed);
                }
                tokio::time::sleep(wait_time).await;
                continue;
            }

            if status.is_client_error() {
                let error_body = response.text().await.unwrap_or_default();
                error!(status = %status, body = %error_body, "Ollama API client error");
                if attempt > max_retries {
                    return Err(EmbeddingError::ApiRequest(format!(
                        "Client error {}: {}",
                        status, error_body
                    )));
                }
                let current_max = self.current_batch_max.load(Ordering::Relaxed);
                if current_max > 5 {
                    self.current_batch_max
                        .store((current_max / 2).max(5), Ordering::Relaxed);
                }
                base_delay = Duration::from_secs(2);
                tokio::time::sleep(base_delay).await;
                continue;
            }

            if status.is_server_error() {
                if attempt > max_retries {
                    return Err(EmbeddingError::ApiRequest(format!(
                        "Server error: {}",
                        status
                    )));
                }
                let jitter_ms = rand::rng().random_range(0..500);
                let wait = base_delay
                    .checked_mul(2u32.pow(attempt - 1))
                    .unwrap_or(Duration::from_secs(60))
                    .min(Duration::from_secs(60))
                    + Duration::from_millis(jitter_ms);
                warn!(attempt, status = %status, "Server error, retrying");
                tokio::time::sleep(wait).await;
                continue;
            }

            let error_body = response.text().await.unwrap_or_default();
            return Err(EmbeddingError::ApiRequest(format!(
                "Unexpected status {}: {}",
                status, error_body
            )));
        }
    }

    pub async fn embed_query(&self, query: &str) -> Result<Vec<f32>, EmbeddingError> {
        let result = self
            .embed_batch(&[EmbedInput::Text(query.to_string())], "", 3)
            .await?;
        result
            .embeddings
            .into_iter()
            .next()
            .ok_or_else(|| EmbeddingError::InvalidResponse("Empty embedding response".to_string()))
    }
}
