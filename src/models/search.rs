/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/models/search.rs
 * Purpose: Search request/response types for MCP tool interactions
 */

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Parameters for the search_codebase MCP tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchRequest {
    /// Natural language or code query
    pub query: String,
    /// Filter by programming language
    pub language: Option<String>,
    /// Filter by path prefix
    pub path_prefix: Option<String>,
    /// Filter by symbol kind (function, struct, class, etc.)
    pub symbol_kind: Option<String>,
    /// Filter by repository name
    pub repo: Option<String>,
    /// Maximum number of results (default 10)
    pub limit: Option<u64>,
}

/// A single search result with provenance and context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub score: f32,
    pub repo: String,
    pub path: String,
    pub language: String,
    pub symbol_name: String,
    pub symbol_kind: String,
    pub symbol_path: String,
    pub line_start: u32,
    pub line_end: u32,
    pub signature: Option<String>,
    pub doc_comment: Option<String>,
    pub snippet: String,
}

/// Response from the search_codebase tool.
#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    pub total_candidates: u64,
    pub query_embedding_model: String,
}

/// Parameters for index_repository MCP tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct IndexRequest {
    /// Absolute path to the repository root
    pub root_path: String,
    /// Optional include glob patterns
    pub include_globs: Option<Vec<String>>,
    /// Optional exclude glob patterns
    pub exclude_globs: Option<Vec<String>>,
    /// Optional language allowlist
    pub languages: Option<Vec<String>>,
    /// "full" or "incremental" (default "full")
    pub mode: Option<String>,
}

/// Parameters for refresh_index MCP tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RefreshRequest {
    /// Absolute path to the repository root
    pub root_path: String,
    /// Optional repository name override
    pub repo: Option<String>,
}

/// Response from refresh_index tool.
#[derive(Debug, Serialize, Deserialize)]
pub struct RefreshResponse {
    pub added: u64,
    pub updated: u64,
    pub deleted: u64,
    pub unchanged: u64,
}

/// Parameters for get_index_status MCP tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct StatusRequest {
    pub job_id: String,
}

/// Parameters for cancel_index_job MCP tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CancelRequest {
    pub job_id: String,
}

/// Parameters for clear_repo_index MCP tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClearRepoRequest {
    /// Repository name to clear
    pub repo: String,
}

/// Parameters for list_indexed_repositories MCP tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListReposRequest {}
