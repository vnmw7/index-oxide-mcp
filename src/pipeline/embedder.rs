/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/pipeline/embedder.rs
 * Purpose: Stage C - Adaptive batch embedding via Gemini API with concurrency control
 */

use crate::config::OxiConfig;
use crate::gemini::client::{EmbedInput, GeminiClient};
use crate::models::chunk::{CodeChunk, EmbeddedChunk};
use crate::models::job::IndexJob;
use chrono::Utc;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};
use tracing::{debug, error, warn};

/// Run the embedding stage, consuming chunks and producing embedded chunks.
pub async fn run_embedder(
    mut rx: mpsc::Receiver<CodeChunk>,
    tx: mpsc::Sender<EmbeddedChunk>,
    job: &Arc<IndexJob>,
    config: &Arc<OxiConfig>,
    gemini: &Arc<GeminiClient>,
) {
    let semaphore = Arc::new(Semaphore::new(config.pipeline.embed_concurrency));
    let max_tokens = config.pipeline.embed_batch_max_tokens;
    let max_retries = config.pipeline.max_retries;

    // Accumulate chunks into batches
    let mut batch_chunks: Vec<CodeChunk> = Vec::new();
    let mut batch_tokens: usize = 0;
    let batch_max_items = config.pipeline.embed_batch_max_items;

    loop {
        if job.is_cancelled() {
            break;
        }

        // Try to fill a batch
        let chunk = rx.recv().await;

        match chunk {
            Some(c) => {
                let estimated_tokens = c.chunk_text.len() / 4;
                batch_tokens += estimated_tokens;
                batch_chunks.push(c);

                // Flush batch if full
                let current_limit = gemini.get_current_batch_max() as usize;
                if batch_chunks.len() >= current_limit.min(batch_max_items)
                    || batch_tokens >= max_tokens
                {
                    let batch = std::mem::take(&mut batch_chunks);
                    batch_tokens = 0;

                    process_batch(
                        batch,
                        &tx,
                        job,
                        gemini,
                        &semaphore,
                        max_retries,
                        &config.gemini.model,
                    )
                    .await;
                }
            }
            None => {
                // Channel closed — flush remaining batch
                if !batch_chunks.is_empty() {
                    let batch = std::mem::take(&mut batch_chunks);
                    process_batch(
                        batch,
                        &tx,
                        job,
                        gemini,
                        &semaphore,
                        max_retries,
                        &config.gemini.model,
                    )
                    .await;
                }
                break;
            }
        }
    }

    debug!("Embedder stage complete");
}

async fn process_batch(
    batch: Vec<CodeChunk>,
    tx: &mpsc::Sender<EmbeddedChunk>,
    job: &Arc<IndexJob>,
    gemini: &Arc<GeminiClient>,
    semaphore: &Arc<Semaphore>,
    max_retries: u32,
    model: &str,
) {
    // Acquire semaphore permit for concurrency control
    let _permit = match semaphore.acquire().await {
        Ok(p) => p,
        Err(_) => return, // Semaphore closed
    };

    let inputs: Vec<EmbedInput> = batch
        .iter()
        .map(|c| {
            // Build embedding input with context prefix for better retrieval
            let text = format_chunk_for_embedding(c);
            EmbedInput::Text(text)
        })
        .collect();

    let batch_size = inputs.len();
    debug!(batch_size, "Embedding batch");

    match gemini.embed_batch(&inputs, "RETRIEVAL_DOCUMENT", max_retries).await {
        Ok(result) => {
            if result.embeddings.len() != batch_size {
                warn!(
                    expected = batch_size,
                    actual = result.embeddings.len(),
                    "Embedding count mismatch"
                );
            }

            let now = Utc::now().to_rfc3339();

            for (chunk, embedding) in batch.into_iter().zip(result.embeddings.into_iter()) {
                job.counters.embedded.fetch_add(1, Ordering::Relaxed);

                let embedded = EmbeddedChunk {
                    chunk,
                    embedding,
                    embedding_model: model.to_string(),
                    embedding_version: "1".to_string(),
                    indexed_at: now.clone(),
                };

                if tx.send(embedded).await.is_err() {
                    return; // Downstream closed
                }
            }
        }
        Err(e) => {
            // Log failure but do not crash the pipeline
            error!(batch_size, error = %e, "Embedding batch failed");
            job.counters.failed.fetch_add(batch_size as u64, Ordering::Relaxed);
            job.add_error(format!("Embedding batch of {} failed: {}", batch_size, e));
        }
    }
}

/// Format a code chunk with metadata prefix for better embedding quality.
fn format_chunk_for_embedding(chunk: &CodeChunk) -> String {
    let mut parts = Vec::new();

    // Language and path context
    parts.push(format!("Language: {}", chunk.language));
    parts.push(format!("File: {}", chunk.path));
    parts.push(format!("{} {}", chunk.symbol_kind, chunk.symbol_name));

    // Signature if available
    if let Some(ref sig) = chunk.signature {
        parts.push(format!("Signature: {}", sig));
    }

    // Doc comment if available
    if let Some(ref doc) = chunk.doc_comment {
        parts.push(format!("Documentation: {}", doc));
    }

    parts.push(String::new()); // blank line separator
    parts.push(chunk.chunk_text.clone());

    parts.join("\n")
}
