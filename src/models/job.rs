/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/models/job.rs
 * Purpose: Index job state tracking for progress reporting, cancellation, and resumability
 */

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Current stage of the indexing pipeline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum JobStage {
    Queued,
    Discovering,
    Parsing,
    Embedding,
    Indexing,
    Completed,
    Failed,
    Cancelled,
}

/// Atomic counters for tracking job progress across async tasks.
#[derive(Debug)]
pub struct JobCounters {
    pub discovered: AtomicU64,
    pub parsed: AtomicU64,
    pub chunked: AtomicU64,
    pub embedded: AtomicU64,
    pub indexed: AtomicU64,
    pub failed: AtomicU64,
    pub skipped: AtomicU64,
}

impl JobCounters {
    pub fn new() -> Self {
        Self {
            discovered: AtomicU64::new(0),
            parsed: AtomicU64::new(0),
            chunked: AtomicU64::new(0),
            embedded: AtomicU64::new(0),
            indexed: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
        }
    }

    pub fn snapshot(&self) -> JobCountersSnapshot {
        JobCountersSnapshot {
            discovered: self.discovered.load(Ordering::Relaxed),
            parsed: self.parsed.load(Ordering::Relaxed),
            chunked: self.chunked.load(Ordering::Relaxed),
            embedded: self.embedded.load(Ordering::Relaxed),
            indexed: self.indexed.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            skipped: self.skipped.load(Ordering::Relaxed),
        }
    }
}

impl Default for JobCounters {
    fn default() -> Self {
        Self::new()
    }
}

/// Serializable snapshot of job counters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobCountersSnapshot {
    pub discovered: u64,
    pub parsed: u64,
    pub chunked: u64,
    pub embedded: u64,
    pub indexed: u64,
    pub failed: u64,
    pub skipped: u64,
}

/// An indexing job with shared state for concurrent pipeline stages.
#[derive(Debug)]
pub struct IndexJob {
    pub job_id: String,
    pub repo_root: String,
    pub repo_name: String,
    pub started_at: DateTime<Utc>,
    pub stage: parking_lot::RwLock<JobStage>,
    pub counters: JobCounters,
    pub cancel_flag: AtomicBool,
    pub errors: parking_lot::RwLock<Vec<String>>,
}

impl IndexJob {
    pub fn new(job_id: String, repo_root: String, repo_name: String) -> Arc<Self> {
        Arc::new(Self {
            job_id,
            repo_root,
            repo_name,
            started_at: Utc::now(),
            stage: parking_lot::RwLock::new(JobStage::Queued),
            counters: JobCounters::new(),
            cancel_flag: AtomicBool::new(false),
            errors: parking_lot::RwLock::new(Vec::new()),
        })
    }

    pub fn set_stage(&self, stage: JobStage) {
        *self.stage.write() = stage;
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel_flag.load(Ordering::Relaxed)
    }

    pub fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }

    pub fn add_error(&self, error: String) {
        self.errors.write().push(error);
    }

    pub fn to_status(&self) -> JobStatus {
        JobStatus {
            job_id: self.job_id.clone(),
            repo_root: self.repo_root.clone(),
            repo_name: self.repo_name.clone(),
            started_at: self.started_at.to_rfc3339(),
            stage: self.stage.read().clone(),
            counters: self.counters.snapshot(),
            recent_errors: self.errors.read().iter().rev().take(10).cloned().collect(),
        }
    }
}

/// Serializable job status for MCP tool responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobStatus {
    pub job_id: String,
    pub repo_root: String,
    pub repo_name: String,
    pub started_at: String,
    pub stage: JobStage,
    pub counters: JobCountersSnapshot,
    pub recent_errors: Vec<String>,
}
