/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/pipeline/parser.rs
 * Purpose: Tree-sitter parser pool for multi-language AST parsing
 */

use crate::models::chunk::CodeChunk;
use crate::models::job::IndexJob;
use crate::pipeline::chunker;
use crate::util::language::{detect_language, is_language_allowed, SupportedLanguage};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, warn};
use tree_sitter::Parser;

/// Create a tree-sitter parser configured for the given language.
fn create_parser(language: SupportedLanguage) -> Option<Parser> {
    let mut parser = Parser::new();

    let lang = match language {
        SupportedLanguage::Rust => tree_sitter_rust::LANGUAGE.into(),
        SupportedLanguage::Python => tree_sitter_python::LANGUAGE.into(),
        SupportedLanguage::Typescript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        SupportedLanguage::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        SupportedLanguage::Go => tree_sitter_go::LANGUAGE.into(),
    };

    if parser.set_language(&lang).is_err() {
        error!(language = %language, "Failed to set tree-sitter language");
        return None;
    }

    Some(parser)
}

/// Run multiple parser workers consuming discovered file paths.
pub async fn run_parser_workers(
    rx: mpsc::Receiver<PathBuf>,
    tx: mpsc::Sender<CodeChunk>,
    job: &Arc<IndexJob>,
    worker_count: usize,
    language_filter: Option<Vec<String>>,
    repo_name: &str,
    repo_root: &Path,
) {
    let rx = Arc::new(tokio::sync::Mutex::new(rx));

    let mut handles = Vec::new();

    for worker_id in 0..worker_count {
        let rx = Arc::clone(&rx);
        let tx = tx.clone();
        let job = Arc::clone(job);
        let lang_filter = language_filter.clone();
        let repo = repo_name.to_string();
        let root = repo_root.to_path_buf();

        handles.push(tokio::spawn(async move {
            loop {
                if job.is_cancelled() {
                    break;
                }

                // Pull the next path from the shared receiver
                let path = {
                    let mut rx = rx.lock().await;
                    rx.recv().await
                };

                let path = match path {
                    Some(p) => p,
                    None => break, // Channel closed, discovery complete
                };

                // Process this file
                match process_file(&path, &root, &repo, &lang_filter).await {
                    Ok(chunks) => {
                        job.counters.parsed.fetch_add(1, Ordering::Relaxed);
                        let chunk_count = chunks.len() as u64;
                        job.counters.chunked.fetch_add(chunk_count, Ordering::Relaxed);

                        for chunk in chunks {
                            if tx.send(chunk).await.is_err() {
                                return; // Downstream closed
                            }
                        }
                    }
                    Err(e) => {
                        // Log and continue — never crash on single file failure
                        job.counters.failed.fetch_add(1, Ordering::Relaxed);
                        job.add_error(format!("{}: {}", path.display(), e));
                        warn!(
                            worker = worker_id,
                            path = %path.display(),
                            error = %e,
                            "Failed to process file, skipping"
                        );
                    }
                }
            }
        }));
    }

    // Drop our clone of tx so downstream knows when all workers are done
    drop(tx);

    for handle in handles {
        let _ = handle.await;
    }
}

/// Process a single file: read, detect language, parse AST, extract chunks.
async fn process_file(
    path: &Path,
    repo_root: &Path,
    repo_name: &str,
    language_filter: &Option<Vec<String>>,
) -> anyhow::Result<Vec<CodeChunk>> {
    // Detect language
    let language = detect_language(path)
        .ok_or_else(|| anyhow::anyhow!("Unsupported language"))?;

    // Apply language filter
    if !is_language_allowed(&language, language_filter) {
        return Ok(Vec::new());
    }

    // Read file content
    let content = tokio::fs::read(path).await?;
    let source = String::from_utf8(content)
        .map_err(|e| anyhow::anyhow!("UTF-8 decode error: {}", e))?;

    // Skip empty files
    if source.trim().is_empty() {
        return Ok(Vec::new());
    }

    // Compute relative path
    let relative_path = path
        .strip_prefix(repo_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/"); // Normalize to forward slashes

    // Get file metadata
    let metadata = tokio::fs::metadata(path).await?;
    let file_mtime = metadata.modified().ok().map(|t| {
        chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()
    }).unwrap_or_default();
    let file_size = metadata.len();

    // Parse with tree-sitter
    let mut parser = create_parser(language)
        .ok_or_else(|| anyhow::anyhow!("Failed to create parser for {}", language))?;

    let tree = parser
        .parse(&source, None)
        .ok_or_else(|| anyhow::anyhow!("Tree-sitter parse failed"))?;

    // Extract semantic chunks via AST analysis
    let chunks = chunker::extract_chunks(
        &tree,
        &source,
        language,
        &relative_path,
        repo_name,
        &file_mtime,
        file_size,
    );

    debug!(
        path = %relative_path,
        language = %language,
        chunks = chunks.len(),
        "Parsed and chunked file"
    );

    Ok(chunks)
}
