/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/pipeline/discovery.rs
 * Purpose: Stage A - .gitignore-aware file discovery using the ignore crate
 */

use crate::config::OxiConfig;
use crate::models::job::IndexJob;
use crate::util::language::{detect_language, is_binary_extension};
use ignore::{WalkBuilder, WalkState};
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
    config: &Arc<OxiConfig>,
    include_globs: Option<Vec<String>>,
    exclude_globs: Option<Vec<String>>,
) -> anyhow::Result<()> {
    let root_owned = root.to_path_buf();

    // Use tokio::task::spawn_blocking since ignore::Walk is synchronous
    let (blocking_tx, mut blocking_rx) = mpsc::channel::<PathBuf>(512);

    // Pre-compile glob patterns to avoid overhead in the visitor
    let include_patterns = include_globs.map(|globs| {
        globs
            .into_iter()
            .filter_map(|g| glob::Pattern::new(&g).ok())
            .collect::<Vec<_>>()
    });
    let exclude_patterns = exclude_globs.map(|globs| {
        globs
            .into_iter()
            .filter_map(|g| glob::Pattern::new(&g).ok())
            .collect::<Vec<_>>()
    });

    let include_patterns = Arc::new(include_patterns);
    let exclude_patterns = Arc::new(exclude_patterns);
    let discovery_workers = config.pipeline.discovery_workers;

    let walk_handle = tokio::task::spawn_blocking(move || {
        let mut builder = WalkBuilder::new(&root_owned);
        builder
            .hidden(true) // skip hidden files
            .git_ignore(true) // respect .gitignore
            .git_global(true) // respect global gitignore
            .git_exclude(true) // respect .git/info/exclude
            .threads(discovery_workers);

        let walker = builder.build_parallel();

        walker.run(|| {
            let include = Arc::clone(&include_patterns);
            let exclude = Arc::clone(&exclude_patterns);
            let blocking_tx = blocking_tx.clone();

            Box::new(move |entry| {
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        warn!(error = %e, "Walk error, skipping entry");
                        return WalkState::Continue;
                    }
                };

                let path = entry.path();

                // Skip directories from the skip list
                if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if SKIP_DIRS.contains(&name) {
                            return WalkState::Skip;
                        }
                    }
                    return WalkState::Continue;
                }

                // Skip binary files
                if is_binary_extension(path) {
                    return WalkState::Continue;
                }

                // Check if file has a supported language
                if detect_language(path).is_none() {
                    return WalkState::Continue;
                }

                // Apply include glob filters
                if let Some(ref includes) = *include {
                    let path_str = path.to_string_lossy();
                    let matched = includes.iter().any(|p| p.matches(&path_str));
                    if !matched {
                        return WalkState::Continue;
                    }
                }

                // Apply exclude glob filters
                if let Some(ref excludes) = *exclude {
                    let path_str = path.to_string_lossy();
                    let excluded = excludes.iter().any(|p| p.matches(&path_str));
                    if excluded {
                        return WalkState::Continue;
                    }
                }

                // Send to blocking channel (this blocks if downstream is slow = backpressure)
                if blocking_tx.blocking_send(path.to_path_buf()).is_err() {
                    return WalkState::Quit; // Receiver dropped
                }

                WalkState::Continue
            })
        });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OxiConfig;
    use crate::models::job::IndexJob;
    use std::fs;
    use std::sync::Arc;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_discover_files() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let root = dir.path();

        // Create some dummy files
        fs::write(root.join("main.rs"), "fn main() {}")?;
        fs::write(
            root.join("lib.rs"),
            "pub fn add(a: i32, b: i32) -> i32 { a + b }",
        )?;
        fs::write(root.join("app.py"), "print('hello')")?;
        fs::write(root.join("readme.md"), "# Readme")?; // Should be ignored (unsupported language)
        fs::create_dir(root.join("target"))?;
        fs::write(root.join("target/debug.exe"), "")?; // Should be ignored (SKIP_DIRS)
        fs::create_dir(root.join("node_modules"))?;
        fs::write(root.join("node_modules/pkg.js"), "")?; // Should be ignored (SKIP_DIRS)

        let job = Arc::new(IndexJob::new(
            "test-job".to_string(),
            root.to_string_lossy().to_string(),
            "test-repo".to_string(),
        ));
        let config = Arc::new(OxiConfig::from_env().unwrap_or_else(|_| {
            // Minimal config for testing if env is missing
            unsafe {
                std::env::set_var("GEMINI_API_KEY", "dummy");
            }
            OxiConfig::from_env().unwrap()
        }));

        let (tx, mut rx) = mpsc::channel(10);

        // Run discovery
        discover_files(root, tx, &job, &config, None, None).await?;

        let mut discovered = Vec::new();
        while let Some(path) = rx.recv().await {
            discovered.push(path);
        }

        assert_eq!(discovered.len(), 3);
        let names: Vec<_> = discovered
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        assert!(names.contains(&"main.rs"));
        assert!(names.contains(&"lib.rs"));
        assert!(names.contains(&"app.py"));
        assert!(!names.contains(&"readme.md"));

        Ok(())
    }

    #[tokio::test]
    async fn test_discover_files_with_globs() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let root = dir.path();

        fs::write(root.join("main.rs"), "")?;
        fs::write(root.join("test.rs"), "")?;
        fs::write(root.join("app.py"), "")?;

        let job = Arc::new(IndexJob::new(
            "test".into(),
            root.to_string_lossy().into(),
            "test".into(),
        ));
        let config = Arc::new(OxiConfig::from_env().unwrap_or_else(|_| {
            unsafe {
                std::env::set_var("GEMINI_API_KEY", "dummy");
            }
            OxiConfig::from_env().unwrap()
        }));

        // Test include globs
        let (tx, mut rx) = mpsc::channel(10);
        discover_files(
            root,
            tx,
            &job,
            &config,
            Some(vec!["**/*.rs".to_string()]),
            None,
        )
        .await?;

        let mut discovered = Vec::new();
        while let Some(path) = rx.try_recv().ok() {
            discovered.push(path);
        }
        assert_eq!(discovered.len(), 2);

        // Test exclude globs
        let (tx, mut rx) = mpsc::channel(10);
        discover_files(
            root,
            tx,
            &job,
            &config,
            None,
            Some(vec!["**/test.rs".to_string()]),
        )
        .await?;

        let mut discovered = Vec::new();
        while let Some(path) = rx.try_recv().ok() {
            discovered.push(path);
        }
        assert_eq!(discovered.len(), 2); // main.rs and app.py

        Ok(())
    }
}
