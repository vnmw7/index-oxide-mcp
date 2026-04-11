/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/mcp_server.rs
 * Purpose: MCP server exposing indexing, search, and management tools (Dual-mode SSE/Stdio)
 */

use crate::config::OxiConfig;
use crate::gemini::client::GeminiClient;
use crate::jobs::registry::JobRegistry;
use crate::models::job::IndexJob;
use crate::models::search::{
    CancelRequest, ClearRepoRequest, IndexRequest, ListReposRequest,
    RefreshRequest, SearchRequest, StatusRequest,
};
use crate::qdrant::client::OxiQdrantClient;
use crate::search::retriever;
use crate::util::hashing::sanitize_repo_name;
use rmcp::{
    tool, tool_handler, tool_router, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info};

/// Shared state for the MCP server, held across all tool invocations.
pub struct OxiServer {
    pub config: Arc<OxiConfig>,
    pub gemini: Arc<GeminiClient>,
    pub qdrant: Arc<OxiQdrantClient>,
    pub jobs: Arc<JobRegistry>,
    tool_router: ToolRouter<Self>,
}

impl OxiServer {
    pub fn new(
        config: Arc<OxiConfig>,
        gemini: Arc<GeminiClient>,
        qdrant: Arc<OxiQdrantClient>,
        jobs: Arc<JobRegistry>,
    ) -> Self {
        Self {
            config,
            gemini,
            qdrant,
            jobs,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for OxiServer {}

#[tool_router(router = tool_router)]
impl OxiServer {
    /// Index a code repository for semantic search.
    /// Spawns an async pipeline that discovers, parses, embeds, and indexes code chunks.
    #[tool(description = "Index a code repository for semantic search. Spawns an async pipeline that discovers, parses, embeds, and indexes code chunks. Returns a job ID for tracking progress.")]
    pub async fn index_repository(
        &self,
        Parameters(request): Parameters<IndexRequest>,
    ) -> String {
        info!(root_path = %request.root_path, "index_repository called");

        let mut actual_root_path = request.root_path.clone();

        // Path translation for Docker volume mounts:
        // When client sends Windows paths (e.g., D:\projects\...) and server runs in Linux container
        if let (Some(host_prefix), Some(container_prefix)) = (
            &self.config.server.host_workspace_path,
            &self.config.server.container_workspace_path,
        ) {
            let normalized_req_path = request.root_path.replace("\\", "/");
            let normalized_host_prefix = host_prefix.replace("\\", "/");

            if normalized_req_path.starts_with(&normalized_host_prefix) {
                // Determine the remainder of the path after the prefix
                let remainder = &normalized_req_path[normalized_host_prefix.len()..];
                // Ensure container prefix ends with / if remainder doesn't start with it
                let separator = if container_prefix.ends_with('/') || remainder.starts_with('/') {
                    ""
                } else {
                    "/"
                };
                actual_root_path = format!("{}{}{}", container_prefix, separator, remainder);

                info!(
                    original_path = %request.root_path,
                    translated_path = %actual_root_path,
                    "Translated host volume path to container path"
                );
            }
        }

        let root = PathBuf::from(&actual_root_path);
        if !root.exists() || !root.is_dir() {
            return serde_json::json!({
                "error": format!("Path does not exist or is not a directory: {}", actual_root_path)
            })
            .to_string();
        }

        // Use the original request path for repo name to maintain consistency with client expectations
        let repo_name = sanitize_repo_name(&request.root_path);
        let job_id = uuid::Uuid::new_v4().to_string();
        let job = IndexJob::new(job_id.clone(), actual_root_path, repo_name.clone());

        self.jobs.register_job(Arc::clone(&job));

        // Spawn the pipeline in the background
        let config = Arc::clone(&self.config);
        let gemini = Arc::clone(&self.gemini);
        let qdrant = Arc::clone(&self.qdrant);
        let include_globs = request.include_globs;
        let exclude_globs = request.exclude_globs;
        let languages = request.languages;

        let spawn_job_id = job_id.clone();
        tokio::spawn(async move {
            if let Err(e) =
                crate::pipeline::run_pipeline(config, gemini, qdrant, job, include_globs, exclude_globs, languages, None)
                    .await
            {
                error!(job_id = %spawn_job_id, error = %e, "Pipeline failed");
            }
        });

        serde_json::json!({
            "job_id": job_id,
            "repo": repo_name,
            "status": "started"
        })
        .to_string()
    }

    /// Search indexed codebases using semantic similarity with optional filters.
    #[tool(description = "Search indexed codebases using semantic similarity. Supports filtering by language, path prefix, symbol kind, and repository. Returns ranked code snippets with full provenance metadata.")]
    pub async fn search_codebase(
        &self,
        Parameters(request): Parameters<SearchRequest>,
    ) -> String {
        info!(query = %request.query, "search_codebase called");

        match retriever::search_codebase(&request, &self.gemini, &self.qdrant).await {
            Ok(response) => serde_json::to_string_pretty(&response).unwrap_or_else(|e| {
                serde_json::json!({"error": e.to_string()}).to_string()
            }),
            Err(e) => {
                error!(error = %e, "Search failed");
                serde_json::json!({"error": e.to_string()}).to_string()
            }
        }
    }

    /// Refresh a previously indexed repository, re-indexing only changed files.
    #[tool(description = "Refresh a previously indexed repository. Compares current files against indexed state, re-indexes only changed files, and removes deleted file chunks. Returns a summary of changes.")]
    pub async fn refresh_index(
        &self,
        Parameters(request): Parameters<RefreshRequest>,
    ) -> String {
        info!(root_path = %request.root_path, "refresh_index called");

        let root = PathBuf::from(&request.root_path);
        let repo_name = request
            .repo
            .unwrap_or_else(|| sanitize_repo_name(&request.root_path));

        match crate::pipeline::refresh::refresh_index(
            &root,
            &repo_name,
            &self.config,
            &self.gemini,
            &self.qdrant,
        )
        .await
        {
            Ok(response) => serde_json::to_string_pretty(&response).unwrap_or_else(|e| {
                serde_json::json!({"error": e.to_string()}).to_string()
            }),
            Err(e) => {
                error!(error = %e, "Refresh failed");
                serde_json::json!({"error": e.to_string()}).to_string()
            }
        }
    }

    /// Get the status of an indexing job.
    #[tool(description = "Get the current status of an indexing job, including progress counters and stage information.")]
    pub async fn get_index_status(
        &self,
        Parameters(request): Parameters<StatusRequest>,
    ) -> String {
        match self.jobs.get_status(&request.job_id) {
            Some(status) => serde_json::to_string_pretty(&status).unwrap_or_default(),
            None => serde_json::json!({"error": "Job not found"}).to_string(),
        }
    }

    /// Cancel a running indexing job.
    #[tool(description = "Cancel a running indexing job. The pipeline will stop processing new items and complete the current batch.")]
    pub async fn cancel_index_job(
        &self,
        Parameters(request): Parameters<CancelRequest>,
    ) -> String {
        if self.jobs.cancel_job(&request.job_id) {
            info!(job_id = %request.job_id, "Job cancelled");
            serde_json::json!({"status": "cancelled", "job_id": request.job_id}).to_string()
        } else {
            serde_json::json!({"error": "Job not found"}).to_string()
        }
    }

    /// Clear the entire index for a specific repository.
    #[tool(description = "Clear the entire index for a specific repository. Deletes the per-repo collection from Qdrant.")]
    pub async fn clear_repo_index(
        &self,
        Parameters(request): Parameters<ClearRepoRequest>,
    ) -> String {
        let repo_name = sanitize_repo_name(&request.repo);
        match self.qdrant.delete_collection(&repo_name).await {
            Ok(()) => {
                info!(repo = %repo_name, "Repository index cleared");
                serde_json::json!({"status": "cleared", "repo": repo_name}).to_string()
            }
            Err(e) => {
                error!(error = %e, "Failed to clear repository index");
                serde_json::json!({"error": e.to_string()}).to_string()
            }
        }
    }

    /// List all indexed repositories.
    #[tool(description = "List all repositories that have been indexed. Returns collection names and their repo identifiers.")]
    pub async fn list_indexed_repositories(
        &self,
        Parameters(_request): Parameters<ListReposRequest>,
    ) -> String {
        match self.qdrant.list_oxi_collections().await {
            Ok(collections) => {
                let repos: Vec<serde_json::Value> = collections
                    .iter()
                    .map(|c| {
                        let repo_name = c.strip_prefix("oxi_").unwrap_or(c);
                        serde_json::json!({
                            "collection": c,
                            "repo": repo_name
                        })
                    })
                    .collect();
                serde_json::json!({"repositories": repos}).to_string()
            }
            Err(e) => serde_json::json!({"error": e.to_string()}).to_string(),
        }
    }
}
