/*
 * System: Index Oxide MCP
 * File URL: inxe-index-mcp/src/mcp_server.rs
 * Purpose: MCP server for searching in indexed codebases (Dual-mode Streamable HTTP/Stdio)
 */

use crate::config::InxeConfig;
use crate::gemini::client::GeminiClient;
use crate::jobs::registry::JobRegistry;
use crate::models::search::SearchRequest;
use crate::qdrant::client::InxeQdrantClient;
use crate::search::retriever;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, ServerHandler,
};
use std::sync::Arc;
use tracing::{error, info};

/// Shared state for the MCP server, held across all tool invocations.
pub struct InxeServer {
    pub config: Arc<InxeConfig>,
    pub gemini: Arc<GeminiClient>,
    pub qdrant: Arc<InxeQdrantClient>,
    pub jobs: Arc<JobRegistry>,
    tool_router: ToolRouter<Self>,
}

impl InxeServer {
    pub fn new(
        config: Arc<InxeConfig>,
        gemini: Arc<GeminiClient>,
        qdrant: Arc<InxeQdrantClient>,
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
impl ServerHandler for InxeServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Search indexed codebases using semantic similarity with optional filters.",
        )
    }
}

#[tool_router(router = tool_router)]
impl InxeServer {
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
