/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/search/retriever.rs
 * Purpose: Hybrid code retrieval with vector search, filtering, and deterministic reranking
 */

use crate::gemini::client::GeminiClient;
use crate::models::search::{SearchRequest, SearchResponse, SearchResult};
use crate::qdrant::client::OxiQdrantClient;
use crate::util::hashing::build_collection_name;
use std::sync::Arc;
use tracing::info;

/// Execute a hybrid search across indexed code chunks.
pub async fn search_codebase(
    request: &SearchRequest,
    gemini: &Arc<GeminiClient>,
    qdrant: &Arc<OxiQdrantClient>,
) -> anyhow::Result<SearchResponse> {
    let repo = request.repo.as_deref().unwrap_or("unknown");
    let collection_name = build_collection_name(repo);
    let limit = request.limit.unwrap_or(10);

    // Step 1: Embed the query
    let query_embedding = gemini.embed_query(&request.query).await?;

    // Step 2: Build filters
    let filter = OxiQdrantClient::build_filter(
        &request.language,
        &request.path_prefix,
        &request.symbol_kind,
        &request.repo,
    );

    // Step 3: Vector search in Qdrant
    // Fetch more candidates than needed for reranking
    let fetch_limit = (limit * 3).min(100);
    let scored_points = qdrant
        .query_chunks(&collection_name, query_embedding, fetch_limit, filter)
        .await?;

    let total_candidates = scored_points.len() as u64;

    // Step 4: Convert to search results with deterministic reranking
    let mut results: Vec<SearchResult> = scored_points
        .into_iter()
        .filter_map(|point| {
            let payload = &point.payload;

            let score = point.score;
            let repo_val = get_payload_str(payload, "repo");
            let path = get_payload_str(payload, "path");
            let language = get_payload_str(payload, "language");
            let symbol_name = get_payload_str(payload, "symbol_name");
            let symbol_kind = get_payload_str(payload, "symbol_kind");
            let symbol_path = get_payload_str(payload, "symbol_path");
            let line_start = get_payload_u32(payload, "line_start");
            let line_end = get_payload_u32(payload, "line_end");
            let signature = get_payload_opt_str(payload, "signature");
            let doc_comment = get_payload_opt_str(payload, "doc_comment");
            let snippet = get_payload_str(payload, "chunk_text");

            // Deterministic reranking signals
            let mut adjusted_score = score;

            // Boost exact symbol name matches
            let query_lower = request.query.to_lowercase();
            if symbol_name.to_lowercase() == query_lower {
                adjusted_score += 0.15;
            } else if symbol_name.to_lowercase().contains(&query_lower) {
                adjusted_score += 0.08;
            }

            // Boost if query terms appear in symbol path
            let query_words: Vec<&str> = request.query.split_whitespace().collect();
            let symbol_path_lower = symbol_path.to_lowercase();
            let word_match_ratio = query_words
                .iter()
                .filter(|w| symbol_path_lower.contains(&w.to_lowercase()))
                .count() as f32
                / query_words.len().max(1) as f32;
            adjusted_score += word_match_ratio * 0.05;

            // Boost if doc comment contains query terms
            if let Some(ref doc) = doc_comment {
                let doc_lower = doc.to_lowercase();
                let doc_match_ratio = query_words
                    .iter()
                    .filter(|w| doc_lower.contains(&w.to_lowercase()))
                    .count() as f32
                    / query_words.len().max(1) as f32;
                adjusted_score += doc_match_ratio * 0.03;
            }

            // Slight boost for same-language match if language filter is specified
            if let Some(ref lang_filter) = request.language {
                if language.eq_ignore_ascii_case(lang_filter) {
                    adjusted_score += 0.02;
                }
            }

            Some(SearchResult {
                score: adjusted_score,
                repo: repo_val,
                path,
                language,
                symbol_name,
                symbol_kind,
                symbol_path,
                line_start,
                line_end,
                signature,
                doc_comment,
                snippet,
            })
        })
        .collect();

    // Sort by adjusted score descending
    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    // Truncate to requested limit
    results.truncate(limit as usize);

    info!(
        query = %request.query,
        results = results.len(),
        total_candidates,
        "Search completed"
    );

    Ok(SearchResponse {
        results,
        total_candidates,
        query_embedding_model: "gemini-embedding-2-preview".to_string(),
    })
}

// -- Payload extraction helpers --

fn get_payload_str(
    payload: &std::collections::HashMap<String, qdrant_client::qdrant::Value>,
    key: &str,
) -> String {
    payload
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

fn get_payload_opt_str(
    payload: &std::collections::HashMap<String, qdrant_client::qdrant::Value>,
    key: &str,
) -> Option<String> {
    payload
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn get_payload_u32(
    payload: &std::collections::HashMap<String, qdrant_client::qdrant::Value>,
    key: &str,
) -> u32 {
    payload
        .get(key)
        .and_then(|v| v.as_integer())
        .map(|i| i as u32)
        .unwrap_or(0)
}
