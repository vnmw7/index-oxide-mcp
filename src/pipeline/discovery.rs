/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/pipeline/discovery.rs
 * Purpose: Stage A - .gitignore-aware file discovery using the ignore crate
 */

use crate::models::job::IndexJob;
use crate::util::language::{detect_language, is_binary_extension};
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Standard directories to always skip during discovery.
const SKIP_DIRS: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
    ".tox",
    ".venv",
    "venv",
    "dist",
    "build",
    ".next",
    ".nuxt",
    "vendor",
    "coverage",
    ".idea",
    ".vscode",
    ".vs",
];

/// Walk the repository discovering source files, respecting .gitignore.
/// Sends discovered paths into a bounded channel.
pub async fn discover_files(
    root: &Path,
    tx: mpsc::Sender<PathBuf>,
    job: &Arc<IndexJob>,
    include_globs: Option<Vec<String>>,
    exclude_globs: Option<Vec<String>>,
) -> anyhow::Result<()> {
    let root_owned = root.to_path_buf();

    // Use tokio::task::spawn_blocking since ignore::Walk is synchronous
    let (blocking_tx, mut blocking_rx) = mpsc::channel::<PathBuf>(512);

    let include = include_globs.clone();
    let exclude = exclude_globs.clone();

    let walk_handle = tokio::task::spawn_blocking(move || {
        let mut builder = WalkBuilder::new(&root_owned);
        builder
            .hidden(true) // skip hidden files
            .git_ignore(true) // respect .gitignore
            .git_global(true) // respect global gitignore
            .git_exclude(true); // respect .git/info/exclude

        let walker = builder.build();

        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    warn!(error = %e, "Walk error, skipping entry");
                    continue;
                }
            };

            let path = entry.path();

            // Skip directories from the skip list
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if SKIP_DIRS.contains(&name) {
                        continue;
                    }
                }
                continue;
            }

            // Skip binary files
            if is_binary_extension(path) {
                continue;
            }

            // Check if file has a supported language
            if detect_language(path).is_none() {
                continue;
            }

            // Apply include glob filters
            if let Some(ref includes) = include {
                let path_str = path.to_string_lossy();
                let matched = includes.iter().any(|pattern| {
                    glob::Pattern::new(pattern)
                        .map(|p| p.matches(&path_str))
                        .unwrap_or(false)
                });
                if !matched {
                    continue;
                }
            }

            // Apply exclude glob filters
            if let Some(ref excludes) = exclude {
                let path_str = path.to_string_lossy();
                let excluded = excludes.iter().any(|pattern| {
                    glob::Pattern::new(pattern)
                        .map(|p| p.matches(&path_str))
                        .unwrap_or(false)
                });
                if excluded {
                    continue;
                }
            }

            // Send to blocking channel (this blocks if downstream is slow = backpressure)
            if blocking_tx.blocking_send(path.to_path_buf()).is_err() {
                break; // Receiver dropped
            }
        }
    });

    // Bridge from blocking channel to async channel
    while let Some(path) = blocking_rx.recv().await {
        if job.is_cancelled() {
            break;
        }

        job.counters.discovered.fetch_add(1, Ordering::Relaxed);
        debug!(path = %path.display(), "Discovered");

        if tx.send(path).await.is_err() {
            break; // Downstream closed
        }
    }

    let _ = walk_handle.await;
    Ok(())
}
