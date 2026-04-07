/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/pipeline/indexer.rs
 * Purpose: Stage D - Batch upsert embedded chunks into Qdrant with retry and backpressure
 */

use crate::config::OxiConfig;
use crate::models::chunk::EmbeddedChunk;
use crate::models::job::IndexJob;
use crate::qdrant::client::OxiQdrantClient;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, error, warn};

/// Run the indexer stage, consuming embedded chunks and upserting to Qdrant.
pub async fn run_indexer(
    mut rx: mpsc::Receiver<EmbeddedChunk>,
    job: &Arc<IndexJob>,
    config: &Arc<OxiConfig>,
    qdrant: &Arc<OxiQdrantClient>,
    collection_name: &str,
) {
    let batch_size = config.pipeline.index_batch_size;
    let max_retries = config.pipeline.max_retries;
    let mut buffer: Vec<EmbeddedChunk> = Vec::with_capacity(batch_size);

    loop {
        if job.is_cancelled() {
            break;
        }

        match rx.recv().await {
            Some(chunk) => {
                buffer.push(chunk);

                if buffer.len() >= batch_size {
                    let batch = std::mem::take(&mut buffer);
                    upsert_batch(batch, job, qdrant, collection_name, max_retries).await;
                }
            }
            None => {
                // Channel closed — flush remaining buffer
                if !buffer.is_empty() {
                    let batch = std::mem::take(&mut buffer);
                    upsert_batch(batch, job, qdrant, collection_name, max_retries).await;
                }
                break;
            }
        }
    }

    debug!("Indexer stage complete");
}

async fn upsert_batch(
    batch: Vec<EmbeddedChunk>,
    job: &Arc<IndexJob>,
    qdrant: &Arc<OxiQdrantClient>,
    collection_name: &str,
    max_retries: u32,
) {
    let batch_size = batch.len();

    for attempt in 1..=max_retries {
        match qdrant.upsert_chunks(collection_name, &batch).await {
            Ok(()) => {
                job.counters
                    .indexed
                    .fetch_add(batch_size as u64, Ordering::Relaxed);
                debug!(batch_size, "Qdrant upsert succeeded");
                return;
            }
            Err(e) => {
                if attempt == max_retries {
                    error!(
                        batch_size,
                        attempts = max_retries,
                        error = %e,
                        "Qdrant upsert failed after max retries"
                    );
                    job.counters
                        .failed
                        .fetch_add(batch_size as u64, Ordering::Relaxed);
                    job.add_error(format!(
                        "Qdrant upsert of {} chunks failed: {}",
                        batch_size, e
                    ));
                    return;
                }

                let wait = Duration::from_millis(500 * 2u64.pow(attempt - 1));
                warn!(
                    attempt,
                    max_retries,
                    wait_ms = wait.as_millis(),
                    error = %e,
                    "Qdrant upsert failed, retrying"
                );
                tokio::time::sleep(wait).await;
            }
        }
    }
}
