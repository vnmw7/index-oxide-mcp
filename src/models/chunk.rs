/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/models/chunk.rs
 * Purpose: Code chunk data model with full context envelope for semantic indexing
 */

use serde::{Deserialize, Serialize};

/// A semantic code chunk extracted from a source file via AST analysis.
/// Contains the full context envelope needed by agentic AIs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeChunk {
    /// Repository identifier (sanitized name)
    pub repo: String,
    /// Branch name if available
    pub branch: Option<String>,
    /// Commit SHA if available
    pub commit_sha: Option<String>,
    /// Repo-relative file path
    pub path: String,
    /// Programming language
    pub language: String,
    /// Symbol name (e.g. function name, struct name)
    pub symbol_name: String,
    /// Symbol kind (function, struct, enum, impl, class, etc.)
    pub symbol_kind: String,
    /// Full symbol path (e.g. "module::StructName::method_name")
    pub symbol_path: String,
    /// Enclosing parent symbol name if any
    pub parent_symbol: Option<String>,
    /// Start line in source file (1-indexed)
    pub line_start: u32,
    /// End line in source file (1-indexed)
    pub line_end: u32,
    /// Start byte offset in source file
    pub byte_start: u32,
    /// End byte offset in source file
    pub byte_end: u32,
    /// Imports / use / package context for this file
    pub imports: Option<String>,
    /// Normalized signature if available
    pub signature: Option<String>,
    /// Doc comments / docstrings attached to the symbol
    pub doc_comment: Option<String>,
    /// Full chunk source text
    pub chunk_text: String,
    /// BLAKE3 hash of chunk_text
    pub content_hash: String,
    /// File modification time (ISO 8601)
    pub file_mtime: String,
}

/// A code chunk with its computed embedding vector, ready for indexing.
#[derive(Debug, Clone)]
pub struct EmbeddedChunk {
    pub chunk: CodeChunk,
    pub embedding: Vec<f32>,
    pub embedding_model: String,
    pub embedding_version: String,
    pub indexed_at: String,
}
