/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/pipeline/embedder.rs
 * Purpose: Stage C - Adaptive batch embedding via Gemini API with concurrency control and caching
 */

use crate::config::OxiConfig;
use crate::gemini::client::{EmbedInput, GeminiClient};
use crate::models::chunk::{CodeChunk, EmbeddedChunk};
use crate::models::job::IndexJob;
use crate::qdrant::client::OxiQdrantClient;
use crate::util::hashing::build_collection_name;
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
    qdrant: &Arc<OxiQdrantClient>,
) {
    let semaphore = Arc::new(Semaphore::new(config.pipeline.embed_concurrency));
    let max_tokens = config.pipeline.embed_batch_max_tokens;
    let max_retries = config.pipeline.max_retries;
    let collection_name = build_collection_name(&job.repo_name);

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
                        qdrant,
                        &collection_name,
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
                        qdrant,
                        &collection_name,
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
    qdrant: &Arc<OxiQdrantClient>,
    collection_name: &str,
    semaphore: &Arc<Semaphore>,
    max_retries: u32,
    model: &str,
) {
    // Acquire semaphore permit for concurrency control
    let _permit = match semaphore.acquire().await {
        Ok(p) => p,
        Err(_) => return, // Semaphore closed
    };

    let now = Utc::now().to_rfc3339();

    // 1. Check Cache: Try to find existing embeddings by content_hash
    let hashes: Vec<String> = batch.iter().map(|c| c.content_hash.clone()).collect();
    let cached_embeddings = match qdrant
        .get_embeddings_by_hashes(collection_name, &hashes)
        .await
    {
        Ok(map) => map,
        Err(e) => {
            warn!(error = %e, "Failed to fetch cached embeddings, proceeding with full batch");
            std::collections::HashMap::new()
        }
    };

    // 2. Split batch into cached and non-cached
    let mut to_embed: Vec<CodeChunk> = Vec::new();
    let mut results: Vec<EmbeddedChunk> = Vec::new();

    for chunk in batch {
        if let Some(embedding) = cached_embeddings.get(&chunk.content_hash) {
            // Cache Hit
            results.push(EmbeddedChunk {
                chunk,
                embedding: embedding.clone(),
                embedding_model: model.to_string(),
                embedding_version: "1".to_string(),
                indexed_at: now.clone(),
            });
        } else {
            // Cache Miss
            to_embed.push(chunk);
        }
    }

    let cache_hits = results.len();
    let cache_misses = to_embed.len();
    debug!(cache_hits, cache_misses, "Embedding batch cache analysis");

    // 3. Call Gemini for cache misses
    if !to_embed.is_empty() {
        let inputs: Vec<EmbedInput> = to_embed
            .iter()
            .map(|c| {
                let text = format_chunk_for_embedding(c);
                EmbedInput::Text(text)
            })
            .collect();

        match gemini
            .embed_batch(&inputs, "RETRIEVAL_DOCUMENT", max_retries)
            .await
        {
            Ok(embed_result) => {
                for (chunk, embedding) in to_embed
                    .into_iter()
                    .zip(embed_result.embeddings.into_iter())
                {
                    results.push(EmbeddedChunk {
                        chunk,
                        embedding,
                        embedding_model: model.to_string(),
                        embedding_version: "1".to_string(),
                        indexed_at: now.clone(),
                    });
                }
            }
            Err(e) => {
                error!(count = cache_misses, error = %e, "Embedding batch failed");
                job.counters
                    .failed
                    .fetch_add(cache_misses as u64, Ordering::Relaxed);
                job.add_error(format!(
                    "Embedding failed for {} chunks: {}",
                    cache_misses, e
                ));
                // We still send the cached ones though
            }
        }
    }

    // 4. Send all successful results downstream
    for embedded in results {
        job.counters.embedded.fetch_add(1, Ordering::Relaxed);
        if tx.send(embedded).await.is_err() {
            return; // Downstream closed
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
