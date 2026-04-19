/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/pipeline/discovery.rs
 * Purpose: Stage A - .gitignore-aware file discovery using the ignore crate
 */

use crate::config::OxiConfig;
use crate::models::job::IndexJob;
use crate::pipeline::filters::{self, FilterResult};
use ignore::WalkState;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, warn};

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

    let filter = Arc::new(filters::FileFilter::new(include_globs, exclude_globs));
    let discovery_workers = config.pipeline.discovery_workers;

    let walk_handle = tokio::task::spawn_blocking(move || {
        let builder = filters::build_walker(&root_owned, discovery_workers);
        let walker = builder.build_parallel();

        walker.run(|| {
            let filter = Arc::clone(&filter);
            let blocking_tx = blocking_tx.clone();

            Box::new(move |entry| {
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        warn!(error = %e, "Walk error, skipping entry");
                        return WalkState::Continue;
                    }
                };

                match filter.check(&entry) {
                    FilterResult::SkipDir => return WalkState::Skip,
                    FilterResult::Ignore => return WalkState::Continue,
                    FilterResult::ProcessFile => {
                        // Send to blocking channel (this blocks if downstream is slow = backpressure)
                        if blocking_tx
                            .blocking_send(entry.path().to_path_buf())
                            .is_err()
                        {
                            return WalkState::Quit; // Receiver dropped
                        }
                    }
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
        while let Ok(path) = rx.try_recv() {
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
        while let Ok(path) = rx.try_recv() {
            discovered.push(path);
        }
        assert_eq!(discovered.len(), 2); // main.rs and app.py

        Ok(())
    }
}
