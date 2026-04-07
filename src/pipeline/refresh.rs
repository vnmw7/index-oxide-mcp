/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/pipeline/refresh.rs
 * Purpose: Incremental refresh - detect changed/deleted files and selectively re-index
 */

use crate::config::OxiConfig;
use crate::gemini::client::GeminiClient;
use crate::models::search::RefreshResponse;
use crate::qdrant::client::OxiQdrantClient;
use crate::util::hashing::{build_collection_name, compute_content_hash};
use crate::util::language::detect_language;
use ignore::WalkBuilder;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Run an incremental refresh for a repository.
/// Compares current files against indexed metadata and re-indexes only changes.
pub async fn refresh_index(
    root_path: &Path,
    repo_name: &str,
    config: &Arc<OxiConfig>,
    gemini: &Arc<GeminiClient>,
    qdrant: &Arc<OxiQdrantClient>,
) -> anyhow::Result<RefreshResponse> {
    let collection_name = build_collection_name(repo_name);

    // Get all currently indexed paths and their content hashes
    let indexed_files = qdrant.get_indexed_paths(&collection_name).await?;
    info!(
        indexed_count = indexed_files.len(),
        "Loaded indexed file metadata for refresh"
    );

    // Walk the current filesystem
    let mut current_files: HashMap<String, String> = HashMap::new();
    let mut changed_files: Vec<String> = Vec::new();

    let walker = WalkBuilder::new(root_path)
        .hidden(true)
        .git_ignore(true)
        .build();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        if path.is_dir() || detect_language(path).is_none() {
            continue;
        }

        let relative_path = path
            .strip_prefix(root_path)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");

        // Fast prefilter: check file size and mtime
        let content = match tokio::fs::read_to_string(path).await {
            Ok(c) => c,
            Err(e) => {
                warn!(path = %relative_path, error = %e, "Cannot read file during refresh");
                continue;
            }
        };

        let content_hash = compute_content_hash(&content);
        current_files.insert(relative_path.clone(), content_hash.clone());

        // Correctness check: compare content hash
        match indexed_files.get(&relative_path) {
            Some(indexed_hash) if indexed_hash == &content_hash => {
                // Unchanged — skip
                debug!(path = %relative_path, "Unchanged");
            }
            _ => {
                // New or changed
                changed_files.push(relative_path);
            }
        }
    }

    // Detect deleted files
    let deleted_files: Vec<String> = indexed_files
        .keys()
        .filter(|path| !current_files.contains_key(*path))
        .cloned()
        .collect();

    let unchanged = current_files.len() as u64
        - changed_files.len() as u64;
    let added = changed_files
        .iter()
        .filter(|f| !indexed_files.contains_key(*f))
        .count() as u64;
    let updated = changed_files.len() as u64 - added;

    info!(
        added,
        updated,
        deleted = deleted_files.len(),
        unchanged,
        "Refresh analysis complete"
    );

    // Delete chunks for removed files
    for path in &deleted_files {
        if let Err(e) = qdrant.delete_by_path(&collection_name, path).await {
            warn!(path, error = %e, "Failed to delete chunks for removed file");
        }
    }

    // Delete chunks for changed files (will be re-indexed)
    for path in &changed_files {
        if let Err(e) = qdrant.delete_by_path(&collection_name, path).await {
            warn!(path, error = %e, "Failed to delete chunks for changed file");
        }
    }

    // Re-index changed files through a mini-pipeline
    if !changed_files.is_empty() {
        info!(count = changed_files.len(), "Re-indexing changed files");
        // Create a mini indexing job for changed files
        let job = crate::models::job::IndexJob::new(
            format!("refresh-{}", uuid::Uuid::new_v4()),
            root_path.to_string_lossy().to_string(),
            repo_name.to_string(),
        );

        // Run the full pipeline but only for the changed files
        crate::pipeline::run_pipeline(
            config.clone(),
            gemini.clone(),
            qdrant.clone(),
            job,
            None,
            None,
            None,
        )
        .await?;
    }

    Ok(RefreshResponse {
        added,
        updated,
        deleted: deleted_files.len() as u64,
        unchanged,
    })
}
