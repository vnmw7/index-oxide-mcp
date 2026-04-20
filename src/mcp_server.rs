/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/mcp_server.rs
 * Purpose: MCP server for searching in indexed codebases (Dual-mode SSE/Stdio)
 */

use crate::config::OxiConfig;
use crate::gemini::client::GeminiClient;
use crate::jobs::registry::JobRegistry;
use crate::models::search::SearchRequest;
use crate::qdrant::client::OxiQdrantClient;
use crate::search::retriever;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    tool, tool_handler, tool_router, ServerHandler,
};
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
    /// Search indexed codebases using semantic similarity with optional filters.
    #[tool(
        description = "Search indexed codebases using semantic similarity. Supports filtering by language, path prefix, symbol kind, and repository. Returns ranked code snippets with full provenance metadata."
    )]
    pub async fn search_codebase(&self, Parameters(request): Parameters<SearchRequest>) -> String {
        info!(query = %request.query, "search_codebase called");

        match retriever::search_codebase(&request, &self.gemini, &self.qdrant).await {
            Ok(response) => serde_json::to_string_pretty(&response)
                .unwrap_or_else(|e| serde_json::json!({"error": e.to_string()}).to_string()),
            Err(e) => {
                error!(error = %e, "Search failed");
                serde_json::json!({"error": e.to_string()}).to_string()
            }
        }
    }
}
