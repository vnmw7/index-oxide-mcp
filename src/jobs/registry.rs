/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/jobs/registry.rs
 * Purpose: Thread-safe job registry for tracking active and completed indexing jobs
 */

use crate::models::job::{IndexJob, JobStatus};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

/// Thread-safe registry of indexing jobs.
pub struct JobRegistry {
    jobs: RwLock<HashMap<String, Arc<IndexJob>>>,
}

impl JobRegistry {
    pub fn new() -> Self {
        Self {
            jobs: RwLock::new(HashMap::new()),
        }
    }

    /// Register a new job.
    pub fn register_job(&self, job: Arc<IndexJob>) {
        self.jobs.write().insert(job.job_id.clone(), job);
    }

    /// Get a job's current status.
    pub fn get_status(&self, job_id: &str) -> Option<JobStatus> {
        self.jobs.read().get(job_id).map(|j| j.to_status())
    }

    /// Cancel a running job.
    pub fn cancel_job(&self, job_id: &str) -> bool {
        if let Some(job) = self.jobs.read().get(job_id) {
            job.cancel();
            true
        } else {
            false
        }
    }

    /// List all tracked jobs.
    pub fn list_jobs(&self) -> Vec<JobStatus> {
        self.jobs.read().values().map(|j| j.to_status()).collect()
    }

    /// Remove completed/cancelled jobs older than the retention limit.
    pub fn cleanup(&self, max_completed_jobs: usize) {
        let mut jobs = self.jobs.write();

        // Keep only the most recent completed jobs
        let mut completed: Vec<(String, String)> = jobs
            .iter()
            .filter(|(_, j)| {
                matches!(
                    *j.stage.read(),
                    crate::models::job::JobStage::Completed
                        | crate::models::job::JobStage::Failed
                        | crate::models::job::JobStage::Cancelled
                )
            })
            .map(|(id, j)| (id.clone(), j.started_at.to_rfc3339()))
            .collect();

        if completed.len() > max_completed_jobs {
            completed.sort_by(|a, b| a.1.cmp(&b.1));
            let to_remove = completed.len() - max_completed_jobs;
            for (id, _) in completed.into_iter().take(to_remove) {
                jobs.remove(&id);
            }
        }
    }
}

impl Default for JobRegistry {
    fn default() -> Self {
        Self::new()
    }
}
