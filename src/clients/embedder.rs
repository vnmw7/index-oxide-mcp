/*
 * System: Index Oxide MCP
 * Module: Clients
 * File URL: index-oxide-mcp/src/clients/embedder.rs
 * Purpose: Unified embedder client abstraction over Gemini and Ollama
 */

use crate::clients::OllamaClient;
use crate::clients::{BatchEmbedResult, EmbedInput, GeminiClient};
use crate::errors::EmbeddingError;

/// Unified client that delegates embedding requests to either Gemini or Ollama.
pub enum EmbedderClient {
    Gemini(GeminiClient),
    Ollama(OllamaClient),
}

impl EmbedderClient {
    /// Get the current adaptive batch max.
    pub fn get_current_batch_max(&self) -> u32 {
        match self {
            Self::Gemini(c) => c.get_current_batch_max(),
            Self::Ollama(c) => c.get_current_batch_max(),
        }
    }

    /// Build batches from inputs respecting estimated token limits and adaptive batch max.
    pub fn build_batches(
        &self,
        inputs: Vec<EmbedInput>,
        max_tokens_per_batch: usize,
    ) -> Vec<Vec<EmbedInput>> {
        match self {
            Self::Gemini(c) => c.build_batches(inputs, max_tokens_per_batch),
            Self::Ollama(c) => c.build_batches(inputs, max_tokens_per_batch),
        }
    }

    /// Embed a batch of inputs.
    pub async fn embed_batch(
        &self,
        inputs: &[EmbedInput],
        task_type: &str,
        max_retries: u32,
    ) -> Result<BatchEmbedResult, EmbeddingError> {
        match self {
            Self::Gemini(c) => c.embed_batch(inputs, task_type, max_retries).await,
            Self::Ollama(c) => c.embed_batch(inputs, task_type, max_retries).await,
        }
    }

    /// Convenience: embed a single text query (for search).
    pub async fn embed_query(&self, query: &str) -> Result<Vec<f32>, EmbeddingError> {
        match self {
            Self::Gemini(c) => c.embed_query(query).await,
            Self::Ollama(c) => c.embed_query(query).await,
        }
    }
}
