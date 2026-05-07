/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/gemini/client.rs
 * Purpose: Gemini embedding API client with adaptive batching, rate limiting, and multimodal support
 */

use crate::config::GeminiConfig;
use crate::errors::EmbeddingError;
use rand::RngExt;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// Gemini embedding client with connection pooling, adaptive batching, and rate limiting.
pub struct GeminiClient {
    http: reqwest::Client,
    config: GeminiConfig,
    dimensions: u32,
    /// Current maximum batch size (shrinks on 4xx/429)
    current_batch_max: AtomicU32,
    /// Consecutive 429 counter for circuit breaker
    consecutive_rate_limits: AtomicU32,
}

// -- Request / Response types matching official Gemini API --

#[derive(Debug, Serialize)]
struct BatchEmbedRequest {
    requests: Vec<EmbedContentRequest>,
}

#[derive(Debug, Serialize)]
struct EmbedContentRequest {
    model: String,
    content: Content,
    #[serde(rename = "taskType")]
    task_type: String,
    #[serde(rename = "outputDimensionality")]
    output_dimensionality: u32,
}

#[derive(Debug, Serialize)]
struct Content {
    parts: Vec<Part>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum Part {
    Text {
        text: String,
    },
    InlineData {
        #[serde(rename = "inlineData")]
        inline_data: InlineData,
    },
}

#[derive(Debug, Serialize)]
struct InlineData {
    #[serde(rename = "mimeType")]
    mime_type: String,
    data: String,
}

#[derive(Debug, Deserialize)]
struct BatchEmbedResponse {
    embeddings: Vec<EmbeddingValue>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingValue {
    values: Vec<f32>,
}

/// Input for embedding: either text or multimodal (text + binary data).
#[derive(Debug, Clone)]
pub enum EmbedInput {
    /// Pure text embedding
    Text(String),
    /// Multimodal: text with inline binary data (mime_type, base64-encoded bytes)
    Multimodal {
        text: Option<String>,
        mime_type: String,
        data_base64: String,
    },
}

impl EmbedInput {
    fn to_parts(&self) -> Vec<Part> {
        match self {
            EmbedInput::Text(text) => vec![Part::Text { text: text.clone() }],
            EmbedInput::Multimodal {
                text,
                mime_type,
                data_base64,
            } => {
                let mut parts = Vec::new();
                if let Some(t) = text {
                    parts.push(Part::Text { text: t.clone() });
                }
                parts.push(Part::InlineData {
                    inline_data: InlineData {
                        mime_type: mime_type.clone(),
                        data: data_base64.clone(),
                    },
                });
                parts
            }
        }
    }

    /// Rough token estimate for batch sizing (4 chars ≈ 1 token).
    fn estimate_tokens(&self) -> usize {
        match self {
            EmbedInput::Text(text) => text.len() / 4,
            EmbedInput::Multimodal {
                text, data_base64, ..
            } => {
                let text_tokens = text.as_ref().map(|t| t.len() / 4).unwrap_or(0);
                // Image/binary data counts as ~258 tokens (Gemini estimate for images)
                text_tokens + 258 + (data_base64.len() / 1000)
            }
        }
    }
}

/// Result of a batch embedding request.
pub struct BatchEmbedResult {
    pub embeddings: Vec<Vec<f32>>,
}

impl GeminiClient {
    pub fn new(config: GeminiConfig, dimensions: u32) -> Self {
        let http = reqwest::Client::builder()
            .pool_max_idle_per_host(10)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .tcp_keepalive(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            http,
            config,
            dimensions,
            current_batch_max: AtomicU32::new(50),
            consecutive_rate_limits: AtomicU32::new(0),
        }
    }

    /// Get the current adaptive batch max.
    pub fn get_current_batch_max(&self) -> u32 {
        self.current_batch_max.load(Ordering::Relaxed)
    }

    /// Build batches from inputs respecting estimated token limits and adaptive batch max.
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
            let tokens = input.estimate_tokens();

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

    /// Embed a batch of inputs with retry, backoff, and adaptive batch sizing.
    pub async fn embed_batch(
        &self,
        inputs: &[EmbedInput],
        task_type: &str,
        max_retries: u32,
    ) -> Result<BatchEmbedResult, EmbeddingError> {
        // Circuit breaker: if 3+ consecutive 429s, wait 30s
        let consecutive = self.consecutive_rate_limits.load(Ordering::Relaxed);
        if consecutive >= 3 {
            warn!(
                "Circuit breaker: {} consecutive rate limits, pausing 30s",
                consecutive
            );
            tokio::time::sleep(Duration::from_secs(30)).await;
            self.consecutive_rate_limits.store(0, Ordering::Relaxed);
        }

        let model_path = format!("models/{}", self.config.model);
        let url = format!(
            "{}/models/{}:batchEmbedContents?key={}",
            self.config.base_url, self.config.model, self.config.api_key
        );

        let requests: Vec<EmbedContentRequest> = inputs
            .iter()
            .map(|input| EmbedContentRequest {
                model: model_path.clone(),
                content: Content {
                    parts: input.to_parts(),
                },
                task_type: task_type.to_string(),
                output_dimensionality: self.dimensions,
            })
            .collect();

        let body = BatchEmbedRequest { requests };

        let mut attempt = 0u32;
        let mut base_delay = Duration::from_secs(1);

        loop {
            attempt += 1;
            debug!(
                attempt,
                batch_size = inputs.len(),
                "Sending embedding batch"
            );

            let response = self
                .http
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| EmbeddingError::ApiRequest(e.to_string()))?;

            let status = response.status();

            if status.is_success() {
                // Reset consecutive 429 counter on success
                self.consecutive_rate_limits.store(0, Ordering::Relaxed);

                let embed_response: BatchEmbedResponse = response
                    .json()
                    .await
                    .map_err(|e| EmbeddingError::InvalidResponse(e.to_string()))?;

                info!(
                    batch_size = inputs.len(),
                    embedding_count = embed_response.embeddings.len(),
                    "Embedding batch succeeded"
                );

                return Ok(BatchEmbedResult {
                    embeddings: embed_response
                        .embeddings
                        .into_iter()
                        .map(|e| e.values)
                        .collect(),
                });
            }

            if status == StatusCode::TOO_MANY_REQUESTS {
                self.consecutive_rate_limits.fetch_add(1, Ordering::Relaxed);

                if attempt > max_retries {
                    error!(
                        "Max retries exceeded for embedding batch after {} attempts",
                        attempt
                    );
                    return Err(EmbeddingError::MaxRetriesExceeded);
                }

                // Check Retry-After header first
                let wait_time = if let Some(header) = response.headers().get("retry-after") {
                    header
                        .to_str()
                        .ok()
                        .and_then(|v| v.parse::<u64>().ok())
                        .map(Duration::from_secs)
                        .unwrap_or(base_delay)
                } else {
                    // Exponential backoff with jitter (0-500ms)
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
                    "Rate limited (429), backing off"
                );

                // Shrink batch max adaptively
                let current_max = self.current_batch_max.load(Ordering::Relaxed);
                if current_max > 5 {
                    let new_max = (current_max / 2).max(5);
                    self.current_batch_max.store(new_max, Ordering::Relaxed);
                    warn!(old_max = current_max, new_max, "Shrinking batch size");
                }

                tokio::time::sleep(wait_time).await;
                continue;
            }

            if status.is_client_error() {
                let error_body = response.text().await.unwrap_or_default();
                error!(status = %status, body = %error_body, "Embedding API client error");

                if attempt > max_retries {
                    return Err(EmbeddingError::ApiRequest(format!(
                        "Client error {}: {}",
                        status, error_body
                    )));
                }

                // Shrink batch on 4xx
                let current_max = self.current_batch_max.load(Ordering::Relaxed);
                if current_max > 5 {
                    self.current_batch_max
                        .store((current_max / 2).max(5), Ordering::Relaxed);
                }

                base_delay = Duration::from_secs(2);
                tokio::time::sleep(base_delay).await;
                continue;
            }

            // 5xx server errors: retry with backoff
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

                warn!(attempt, status = %status, wait_ms = wait.as_millis(), "Server error, retrying");
                tokio::time::sleep(wait).await;
                continue;
            }

            // Unexpected status
            let error_body = response.text().await.unwrap_or_default();
            return Err(EmbeddingError::ApiRequest(format!(
                "Unexpected status {}: {}",
                status, error_body
            )));
        }
    }

    /// Convenience: embed a single text query (for search).
    pub async fn embed_query(&self, query: &str) -> Result<Vec<f32>, EmbeddingError> {
        let result = self
            .embed_batch(&[EmbedInput::Text(query.to_string())], "RETRIEVAL_QUERY", 3)
            .await?;

        result
            .embeddings
            .into_iter()
            .next()
            .ok_or_else(|| EmbeddingError::InvalidResponse("Empty embedding response".to_string()))
    }
}
