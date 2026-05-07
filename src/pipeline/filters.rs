/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/pipeline/filters.rs
 * Purpose: Shared file discovery filtering and Walker configuration
 */

use crate::util::language::{detect_language, is_binary_extension};
use ignore::{DirEntry, WalkBuilder};
use std::path::Path;

/// Standard directories to always skip during discovery.
pub const SKIP_DIRS: &[&str] = &[
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

/// Result of a filter check.
pub enum FilterResult {
    /// Skip this directory and do not descend into it.
    SkipDir,
    /// This file should be processed (indexed/refreshed).
    ProcessFile,
    /// This entry should be ignored, but the walk should continue.
    Ignore,
}

/// Compiled glob filters for file discovery.
pub struct FileFilter {
    include_patterns: Option<Vec<glob::Pattern>>,
    exclude_patterns: Option<Vec<glob::Pattern>>,
}

impl FileFilter {
    /// Create a new filter from optional include and exclude glob patterns.
    pub fn new(include_globs: Option<Vec<String>>, exclude_globs: Option<Vec<String>>) -> Self {
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

        Self {
            include_patterns,
            exclude_patterns,
        }
    }

    /// Checks if a path string matches the configured globs.
    /// Standardizes on forward slashes for cross-platform glob matching.
    pub fn matches_globs(&self, path_str: &str) -> bool {
        let normalized = path_str.replace('\\', "/");

        // Apply include glob filters
        if let Some(ref includes) = self.include_patterns {
            let matched = includes.iter().any(|p| p.matches(&normalized));
            if !matched {
                return false;
            }
        }

        // Apply exclude glob filters
        if let Some(ref excludes) = self.exclude_patterns {
            let excluded = excludes.iter().any(|p| p.matches(&normalized));
            if excluded {
                return false;
            }
        }

        true
    }

    /// Determines if an entry should be processed, ignored, or if a directory should be skipped entirely.
    pub fn check(&self, entry: &DirEntry, relative_path: &str) -> FilterResult {
        let path = entry.path();

        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if SKIP_DIRS.contains(&name) {
                    return FilterResult::SkipDir;
                }
            }
            return FilterResult::Ignore;
        }

        // It's a file - check binary status
        if is_binary_extension(path) {
            return FilterResult::Ignore;
        }

        // Check if file has a supported language
        if detect_language(path).is_none() {
            return FilterResult::Ignore;
        }

        // Use the centralized glob matcher with the relative path
        if !self.matches_globs(relative_path) {
            return FilterResult::Ignore;
        }

        FilterResult::ProcessFile
    }
}

/// Create a pre-configured WalkBuilder for consistent discovery across stages.
pub fn build_walker(root: &Path, threads: usize) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(true) // skip hidden files
        .git_ignore(true) // respect .gitignore
        .git_global(true) // respect global gitignore
        .git_exclude(true) // respect .git/info/exclude
        .threads(threads);
    builder
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matches_globs_normalization() {
        let filter = FileFilter::new(Some(vec!["src/**/*.rs".to_string()]), None);

        // Linux path
        assert!(filter.matches_globs("src/main.rs"));
        // Windows path
        assert!(filter.matches_globs("src\\main.rs"));
        // Mismatch
        assert!(!filter.matches_globs("app/main.py"));
    }

    #[test]
    fn test_matches_globs_exclude() {
        let filter = FileFilter::new(
            Some(vec!["src/**/*.rs".to_string()]),
            Some(vec!["src/generated/*.rs".to_string()]),
        );

        assert!(filter.matches_globs("src/main.rs"));
        assert!(!filter.matches_globs("src/generated/types.rs"));
    }

    #[test]
    fn test_matches_globs_empty() {
        let filter = FileFilter::new(None, None);
        assert!(filter.matches_globs("any/path.rs"));
    }
}
