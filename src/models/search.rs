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

/// Response from refresh_index operation.
#[derive(Debug, Serialize, Deserialize)]
pub struct RefreshResponse {
    pub added: u64,
    pub updated: u64,
    pub deleted: u64,
    pub unchanged: u64,
}
