/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/pipeline/refresh.rs
 * Purpose: Incremental refresh - detect changed/deleted files and selectively re-index
 */

use crate::config::OxiConfig;
use crate::gemini::client::GeminiClient;
use crate::models::search::RefreshResponse;
use crate::qdrant::client::OxiQdrantClient;
use crate::util::hashing::build_collection_name;
use crate::util::language::detect_language;
use ignore::{WalkBuilder, WalkState};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
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

    // Get all currently indexed paths and their metadata
    let indexed_metadata = qdrant.get_indexed_metadata(&collection_name).await?;
    info!(
        indexed_count = indexed_metadata.len(),
        "Loaded indexed file metadata for refresh"
    );

    // Walk the current filesystem in parallel
    let current_files = Arc::new(Mutex::new(HashMap::<String, (String, u64)>::new()));
    let changed_files = Arc::new(Mutex::new(Vec::<PathBuf>::new()));

    let walker = WalkBuilder::new(root_path)
        .hidden(true)
        .git_ignore(true)
        .threads(config.pipeline.discovery_workers)
        .build_parallel();

    let root_path_buf = root_path.to_path_buf();
    let indexed_metadata_arc = Arc::new(indexed_metadata);

    walker.run(|| {
        let current_files = Arc::clone(&current_files);
        let changed_files = Arc::clone(&changed_files);
        let root = root_path_buf.clone();
        let indexed = Arc::clone(&indexed_metadata_arc);

        Box::new(move |entry| {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => return WalkState::Continue,
            };

            let path = entry.path();
            if path.is_dir() || detect_language(path).is_none() {
                return WalkState::Continue;
            }

            let relative_path = path
                .strip_prefix(&root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");

            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => return WalkState::Continue,
            };

            let mtime = metadata
                .modified()
                .ok()
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
                .unwrap_or_default();
            let size = metadata.len();

            {
                let mut current = current_files.lock().unwrap();
                current.insert(relative_path.clone(), (mtime.clone(), size));
            }

            // Metadata-based comparison (Fast Path)
            match indexed.get(&relative_path) {
                Some((indexed_mtime, indexed_size, _hash)) => {
                    if indexed_mtime != &mtime || *indexed_size != size {
                        debug!(path = %relative_path, "Modified (metadata)");
                        let mut changed = changed_files.lock().unwrap();
                        changed.push(path.to_path_buf());
                    } else {
                        debug!(path = %relative_path, "Unchanged");
                    }
                }
                None => {
                    debug!(path = %relative_path, "New file");
                    let mut changed = changed_files.lock().unwrap();
                    changed.push(path.to_path_buf());
                }
            }

            WalkState::Continue
        })
    });

    let current_files = Arc::try_unwrap(current_files).unwrap().into_inner().unwrap();
    let changed_files_vec = Arc::try_unwrap(changed_files).unwrap().into_inner().unwrap();

    // Detect deleted files
    let deleted_files: Vec<String> = indexed_metadata_arc
        .keys()
        .filter(|path| !current_files.contains_key(*path))
        .cloned()
        .collect();

    let unchanged = current_files.len() as u64 - changed_files_vec.len() as u64;
    let added = changed_files_vec
        .iter()
        .filter(|p| {
            let rel = p.strip_prefix(root_path).unwrap_or(p).to_string_lossy().replace('\\', "/");
            !indexed_metadata_arc.contains_key(&rel)
        })
        .count() as u64;
    let updated = changed_files_vec.len() as u64 - added;

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
    for path_buf in &changed_files_vec {
        let relative_path = path_buf
            .strip_prefix(root_path)
            .unwrap_or(path_buf)
            .to_string_lossy()
            .replace('\\', "/");
        if let Err(e) = qdrant.delete_by_path(&collection_name, &relative_path).await {
            warn!(path = %relative_path, error = %e, "Failed to delete chunks for changed file");
        }
    }

    // Re-index changed files through the specific_files pipeline
    if !changed_files_vec.is_empty() {
        info!(count = changed_files_vec.len(), "Re-indexing changed files");
        
        let job = crate::models::job::IndexJob::new(
            format!("refresh-{}", uuid::Uuid::new_v4()),
            root_path.to_string_lossy().to_string(),
            repo_name.to_string(),
        );

        crate::pipeline::run_pipeline(
            config.clone(),
            gemini.clone(),
            qdrant.clone(),
            job,
            None,
            None,
            None,
            Some(changed_files_vec),
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
