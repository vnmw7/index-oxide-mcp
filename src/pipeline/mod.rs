/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/pipeline/mod.rs
 * Purpose: Pipeline orchestrator that wires discovery → parse → embed → index stages with bounded channels
 */

pub mod chunker;
pub mod discovery;
pub mod embedder;
pub mod indexer;
pub mod parser;
pub mod refresh;

use crate::config::OxiConfig;
use crate::gemini::client::GeminiClient;
use crate::models::chunk::{CodeChunk, EmbeddedChunk};
use crate::models::job::{IndexJob, JobStage};
use crate::qdrant::client::OxiQdrantClient;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info};

/// Run the full indexing pipeline for a repository.
pub async fn run_pipeline(
    config: Arc<OxiConfig>,
    gemini: Arc<GeminiClient>,
    qdrant: Arc<OxiQdrantClient>,
    job: Arc<IndexJob>,
    include_globs: Option<Vec<String>>,
    exclude_globs: Option<Vec<String>>,
    language_filter: Option<Vec<String>>,
    specific_files: Option<Vec<PathBuf>>,
) -> anyhow::Result<()> {
    let collection_name = qdrant.ensure_collection(&job.repo_name).await?;

    // Create bounded channels between pipeline stages
    let (discovery_tx, discovery_rx) =
        mpsc::channel::<PathBuf>(config.pipeline.discovery_channel_size);
    let (chunk_tx, chunk_rx) =
        mpsc::channel::<CodeChunk>(config.pipeline.parser_channel_size);
    let (embedded_tx, embedded_rx) =
        mpsc::channel::<EmbeddedChunk>(config.pipeline.embedder_channel_size);

    let repo_root = PathBuf::from(&job.repo_root);

    // Stage A: Discovery or Specific Files
    let disc_job = Arc::clone(&job);
    let disc_root = repo_root.clone();
    let disc_include = include_globs.clone();
    let disc_exclude = exclude_globs.clone();
    let discovery_handle = tokio::spawn(async move {
        if let Some(files) = specific_files {
            info!(count = files.len(), "Indexing specific files list");
            disc_job.set_stage(JobStage::Discovering);
            for file in files {
                if disc_job.is_cancelled() {
                    break;
                }
                disc_job.counters.discovered.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if discovery_tx.send(file).await.is_err() {
                    break;
                }
            }
        } else {
            disc_job.set_stage(JobStage::Discovering);
            if let Err(e) = discovery::discover_files(
                &disc_root,
                discovery_tx,
                &disc_job,
                disc_include,
                disc_exclude,
            )
            .await
            {
                error!(error = %e, "Discovery stage failed");
                disc_job.add_error(format!("Discovery: {}", e));
            }
        }
    });

    // Stage B: Parse + Extract (multiple workers)
    let parser_job = Arc::clone(&job);
    let parser_config = Arc::clone(&config);
    let parser_lang_filter = language_filter.clone();
    let parser_repo = job.repo_name.clone();
    let parser_root = repo_root.clone();
    let parse_handle = tokio::spawn(async move {
        parser_job.set_stage(JobStage::Parsing);
        parser::run_parser_workers(
            discovery_rx,
            chunk_tx,
            &parser_job,
            parser_config.pipeline.parser_workers,
            parser_lang_filter,
            &parser_repo,
            &parser_root,
        )
        .await;
    });

    // Stage C: Embed Batcher
    let embed_job = Arc::clone(&job);
    let embed_config = Arc::clone(&config);
    let embed_gemini = Arc::clone(&gemini);
    let embed_qdrant = Arc::clone(&qdrant);
    let embed_handle = tokio::spawn(async move {
        embed_job.set_stage(JobStage::Embedding);
        embedder::run_embedder(
            chunk_rx,
            embedded_tx,
            &embed_job,
            &embed_config,
            &embed_gemini,
            &embed_qdrant,
        )
        .await;
    });

    // Stage D: Indexer
    let index_job = Arc::clone(&job);
    let index_config = Arc::clone(&config);
    let index_qdrant = Arc::clone(&qdrant);
    let index_collection = collection_name.clone();
    let index_handle = tokio::spawn(async move {
        index_job.set_stage(JobStage::Indexing);
        indexer::run_indexer(
            embedded_rx,
            &index_job,
            &index_config,
            &index_qdrant,
            &index_collection,
        )
        .await;
    });

    // Wait for all stages to complete
    let _ = tokio::join!(discovery_handle, parse_handle, embed_handle, index_handle);

    // Final stage status
    if job.is_cancelled() {
        job.set_stage(JobStage::Cancelled);
        info!(job_id = %job.job_id, "Indexing job cancelled");
    } else {
        let counters = job.counters.snapshot();
        if counters.failed > 0 {
            info!(
                job_id = %job.job_id,
                indexed = counters.indexed,
                failed = counters.failed,
                "Indexing completed with errors"
            );
        }
        job.set_stage(JobStage::Completed);
        info!(
            job_id = %job.job_id,
            discovered = counters.discovered,
            chunked = counters.chunked,
            embedded = counters.embedded,
            indexed = counters.indexed,
            "Indexing completed"
        );
    }

    Ok(())
}
