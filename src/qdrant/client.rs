/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/qdrant/client.rs
 * Purpose: Qdrant vector DB client wrapper for collection management, upsert, query, and delete
 */

use crate::config::QdrantConfig;
use crate::errors::StorageError;
use crate::models::chunk::EmbeddedChunk;
use crate::util::hashing::{build_collection_name, generate_chunk_id};
use qdrant_client::qdrant::{
    CreateCollectionBuilder, Distance, FieldType, Filter, HnswConfigDiffBuilder,
    PointStruct, QueryPointsBuilder, UpsertPointsBuilder, VectorParamsBuilder,
    Condition, CreateFieldIndexCollectionBuilder,
    PointsSelector, ScrollPointsBuilder, PayloadIncludeSelector, Vector,
};
use qdrant_client::Qdrant;
use serde_json::json;
use tracing::{debug, info, warn};

/// Wrapper around the Qdrant gRPC client for oxidized-index-mcp operations.
pub struct OxiQdrantClient {
    client: Qdrant,
    dimensions: u32,
}

impl OxiQdrantClient {
    /// Connect to Qdrant via gRPC.
    pub fn new(config: &QdrantConfig, dimensions: u32) -> Result<Self, StorageError> {
        let client = Qdrant::from_url(&config.url)
            .build()
            .map_err(|e| StorageError::QdrantOperation(e.to_string()))?;

        Ok(Self { client, dimensions })
    }

    /// Ensure a per-repo collection exists with correct configuration.
    /// Creates the collection if missing; skips if already present.
    pub async fn ensure_collection(&self, repo_name: &str) -> Result<String, StorageError> {
        let collection_name = build_collection_name(repo_name);

        // Check if collection exists
        let exists = self
            .client
            .collection_exists(&collection_name)
            .await
            .map_err(|e| StorageError::CollectionCreation(e.to_string()))?;

        if exists {
            debug!(collection = %collection_name, "Collection already exists");
            return Ok(collection_name);
        }

        info!(collection = %collection_name, dimensions = self.dimensions, "Creating collection");

        // Create collection with on-disk storage for large-scale indexing
        self.client
            .create_collection(
                CreateCollectionBuilder::new(&collection_name)
                    .vectors_config(
                        VectorParamsBuilder::new(self.dimensions as u64, Distance::Cosine)
                            .on_disk(true),
                    )
                    .hnsw_config(HnswConfigDiffBuilder::default().on_disk(true))
                    .on_disk_payload(true),
            )
            .await
            .map_err(|e| StorageError::CollectionCreation(e.to_string()))?;

        // Create payload indexes for fields used in filtering
        let indexed_fields = vec![
            ("repo", FieldType::Keyword),
            ("language", FieldType::Keyword),
            ("path", FieldType::Keyword),
            ("symbol_name", FieldType::Keyword),
            ("symbol_kind", FieldType::Keyword),
            ("content_hash", FieldType::Keyword),
        ];

        for (field, field_type) in indexed_fields {
            if let Err(e) = self
                .client
                .create_field_index(
                    CreateFieldIndexCollectionBuilder::new(
                        &collection_name,
                        field,
                        field_type,
                    ),
                )
                .await
            {
                warn!(field, collection = %collection_name, error = %e, "Failed to create field index (non-fatal)");
            }
        }

        info!(collection = %collection_name, "Collection created with payload indexes");
        Ok(collection_name)
    }

    /// Batch upsert embedded chunks into the collection.
    pub async fn upsert_chunks(
        &self,
        collection_name: &str,
        chunks: &[EmbeddedChunk],
    ) -> Result<(), StorageError> {
        if chunks.is_empty() {
            return Ok(());
        }

        let points: Vec<PointStruct> = chunks
            .iter()
            .map(|ec| {
                let chunk_id = generate_chunk_id(
                    &ec.chunk.repo,
                    &ec.chunk.path,
                    &ec.chunk.symbol_path,
                    ec.chunk.byte_start,
                    ec.chunk.byte_end,
                    &ec.chunk.content_hash,
                );

                let payload = json!({
                    "repo": ec.chunk.repo,
                    "branch": ec.chunk.branch,
                    "commit_sha": ec.chunk.commit_sha,
                    "path": ec.chunk.path,
                    "language": ec.chunk.language,
                    "symbol_name": ec.chunk.symbol_name,
                    "symbol_kind": ec.chunk.symbol_kind,
                    "symbol_path": ec.chunk.symbol_path,
                    "parent_symbol": ec.chunk.parent_symbol,
                    "line_start": ec.chunk.line_start,
                    "line_end": ec.chunk.line_end,
                    "byte_start": ec.chunk.byte_start,
                    "byte_end": ec.chunk.byte_end,
                    "imports": ec.chunk.imports,
                    "signature": ec.chunk.signature,
                    "doc_comment": ec.chunk.doc_comment,
                    "chunk_text": ec.chunk.chunk_text,
                    "content_hash": ec.chunk.content_hash,
                    "file_mtime": ec.chunk.file_mtime,
                    "file_size": ec.chunk.file_size,
                    "embedding_model": ec.embedding_model,
                    "embedding_version": ec.embedding_version,
                    "indexed_at": ec.indexed_at,
                });

                PointStruct::new(
                    chunk_id,
                    ec.embedding.clone(),
                    payload.as_object().unwrap().clone(),
                )
            })
            .collect();

        debug!(
            collection = %collection_name,
            count = points.len(),
            "Upserting points"
        );

        self.client
            .upsert_points(UpsertPointsBuilder::new(collection_name, points).wait(true))
            .await
            .map_err(|e| StorageError::UpsertFailed(e.to_string()))?;

        Ok(())
    }

    /// Query vectors with optional metadata filters.
    pub async fn query_chunks(
        &self,
        collection_name: &str,
        query_vector: Vec<f32>,
        limit: u64,
        filter: Option<Filter>,
    ) -> Result<Vec<qdrant_client::qdrant::ScoredPoint>, StorageError> {
        let mut query = QueryPointsBuilder::new(collection_name)
            .query(query_vector)
            .limit(limit)
            .with_payload(true);

        if let Some(f) = filter {
            query = query.filter(f);
        }

        let response = self
            .client
            .query(query)
            .await
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;

        Ok(response.result)
    }

    /// Build a Qdrant filter from optional search parameters.
    pub fn build_filter(
        language: &Option<String>,
        path_prefix: &Option<String>,
        symbol_kind: &Option<String>,
        repo: &Option<String>,
    ) -> Option<Filter> {
        let mut conditions: Vec<Condition> = Vec::new();

        if let Some(lang) = language {
            conditions.push(Condition::matches("language", lang.to_lowercase()));
        }

        if let Some(prefix) = path_prefix {
            conditions.push(Condition::matches("path", prefix.clone()));
        }

        if let Some(kind) = symbol_kind {
            conditions.push(Condition::matches("symbol_kind", kind.to_lowercase()));
        }

        if let Some(r) = repo {
            conditions.push(Condition::matches("repo", r.clone()));
        }

        if conditions.is_empty() {
            None
        } else {
            Some(Filter::must(conditions))
        }
    }

    /// Delete all points matching a file path filter (used during refresh).
    pub async fn delete_by_path(
        &self,
        collection_name: &str,
        path: &str,
    ) -> Result<(), StorageError> {
        let filter = Filter::must(vec![Condition::matches("path", path.to_string())]);

        self.client
            .delete_points(
                qdrant_client::qdrant::DeletePoints {
                    collection_name: collection_name.to_string(),
                    points: Some(PointsSelector::from(filter)),
                    wait: Some(true),
                    ..Default::default()
                }
            )
            .await
            .map_err(|e| StorageError::DeleteFailed(e.to_string()))?;

        Ok(())
    }

    /// Delete an entire per-repo collection (for clear_repo_index tool).
    pub async fn delete_collection(&self, repo_name: &str) -> Result<(), StorageError> {
        let collection_name = build_collection_name(repo_name);

        self.client
            .delete_collection(&collection_name)
            .await
            .map_err(|e| StorageError::DeleteFailed(e.to_string()))?;

        info!(collection = %collection_name, "Collection deleted");
        Ok(())
    }

    /// List all collections matching the oxi_ prefix.
    pub async fn list_oxi_collections(&self) -> Result<Vec<String>, StorageError> {
        let collections = self
            .client
            .list_collections()
            .await
            .map_err(|e| StorageError::QdrantOperation(e.to_string()))?;

        Ok(collections
            .collections
            .into_iter()
            .filter(|c| c.name.starts_with("oxi_"))
            .map(|c| c.name)
            .collect())
    }

    /// Scroll all indexed metadata for a repo (used during refresh comparison).
    /// Returns a map of path -> (file_mtime, file_size, content_hash)
    pub async fn get_indexed_metadata(
        &self,
        collection_name: &str,
    ) -> Result<std::collections::HashMap<String, (String, u64, String)>, StorageError> {
        let mut result = std::collections::HashMap::new();
        let mut offset: Option<qdrant_client::qdrant::PointId> = None;

        loop {
            let mut scroll = ScrollPointsBuilder::new(collection_name).limit(1000);
            scroll = scroll.with_payload(PayloadIncludeSelector {
                fields: vec![
                    "path".to_string(),
                    "file_mtime".to_string(),
                    "file_size".to_string(),
                    "content_hash".to_string(),
                ],
            });

            if let Some(off) = offset.clone() {
                scroll = scroll.offset(off);
            }

            let response = self
                .client
                .scroll(scroll)
                .await
                .map_err(|e| StorageError::QueryFailed(e.to_string()))?;

            for point in &response.result {
                let payload = &point.payload;
                if let (Some(path_val), Some(mtime_val), Some(size_val), Some(hash_val)) = (
                    payload.get("path"),
                    payload.get("file_mtime"),
                    payload.get("file_size"),
                    payload.get("content_hash"),
                ) {
                    if let (Some(path), Some(mtime), Some(hash)) = (
                        path_val.as_str(),
                        mtime_val.as_str(),
                        hash_val.as_str(),
                    ) {
                        let size = size_val.as_integer().unwrap_or(0) as u64;
                        // Use the last seen one (should be consistent per file)
                        result.insert(path.to_string(), (mtime.to_string(), size, hash.to_string()));
                    }
                }
            }

            offset = response.next_page_offset;
            if offset.is_none() {
                break;
            }
        }

        Ok(result)
    }

    /// Fetch embeddings for a list of content hashes (Embedding Cache).
    pub async fn get_embeddings_by_hashes(
        &self,
        collection_name: &str,
        hashes: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<f32>>, StorageError> {
        if hashes.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        let filter = Filter::must(vec![Condition::matches(
            "content_hash".to_string(),
            hashes.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        )]);

        let scroll = ScrollPointsBuilder::new(collection_name)
            .filter(filter)
            .limit(hashes.len() as u32)
            .with_payload(PayloadIncludeSelector {
                fields: vec!["content_hash".to_string()],
            })
            .with_vectors(true);

        let response = self
            .client
            .scroll(scroll)
            .await
            .map_err(|e| StorageError::QueryFailed(e.to_string()))?;

        let mut result = std::collections::HashMap::new();
        for point in response.result {
            if let Some(hash_val) = point.payload.get("content_hash") {
                if let Some(hash) = hash_val.as_str() {
                    if let Some(vectors) = point.vectors {
                        if let Some(qdrant_client::qdrant::vectors_output::VectorsOptions::Vector(v)) = vectors.vectors_options {
                            if let Some(Vector::Dense(dense)) = v.into_vector() {
                                result.insert(hash.to_string(), dense.data);
                            }
                        }
                    }
                }
            }
        }

        Ok(result)
    }
}
