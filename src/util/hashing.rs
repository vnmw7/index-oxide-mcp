/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/util/hashing.rs
 * Purpose: Content hashing (BLAKE3) and deterministic UUID v5 generation for chunk IDs
 */

use uuid::Uuid;

/// Namespace UUID for oxidized-index-mcp chunk IDs.
const OXI_NAMESPACE: Uuid = Uuid::from_bytes([
    0x6f, 0x78, 0x69, 0x64, // "oxid"
    0x69, 0x7a, 0x65, 0x64, // "ized"
    0x2d, 0x69, 0x6e, 0x64, // "-ind"
    0x65, 0x78, 0x6d, 0x63, // "exmc"
]);

/// Compute BLAKE3 hash of content, returned as a hex string.
pub fn compute_content_hash(content: &str) -> String {
    blake3::hash(content.as_bytes()).to_hex().to_string()
}

/// Generate a deterministic UUID v5 for a code chunk.
/// Derived from: repo + path + symbol_path + byte_range + content_hash.
/// This ensures idempotent upserts across retries and refreshes.
pub fn generate_chunk_id(
    repo: &str,
    path: &str,
    symbol_path: &str,
    byte_start: u32,
    byte_end: u32,
    content_hash: &str,
) -> String {
    let input = format!(
        "{}:{}:{}:{}:{}:{}",
        repo, path, symbol_path, byte_start, byte_end, content_hash
    );
    Uuid::new_v5(&OXI_NAMESPACE, input.as_bytes()).to_string()
}

/// Sanitize a repository path into a valid collection name component.
/// Produces lowercase alphanumeric with underscores only.
pub fn sanitize_repo_name(repo_path: &str) -> String {
    let name = std::path::Path::new(repo_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

/// Build the per-repo Qdrant collection name.
pub fn build_collection_name(repo_name: &str) -> String {
    format!("oxi_{}", repo_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_hash_deterministic() {
        let hash1 = compute_content_hash("fn main() {}");
        let hash2 = compute_content_hash("fn main() {}");
        assert_eq!(hash1, hash2);
        assert!(!hash1.is_empty());
    }

    #[test]
    fn test_chunk_id_deterministic() {
        let id1 = generate_chunk_id("myrepo", "src/main.rs", "main", 0, 50, "abc123");
        let id2 = generate_chunk_id("myrepo", "src/main.rs", "main", 0, 50, "abc123");
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_chunk_id_changes_with_content() {
        let id1 = generate_chunk_id("myrepo", "src/main.rs", "main", 0, 50, "abc123");
        let id2 = generate_chunk_id("myrepo", "src/main.rs", "main", 0, 50, "def456");
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_sanitize_repo_name() {
        assert_eq!(sanitize_repo_name("/home/user/my-project"), "my_project");
        assert_eq!(sanitize_repo_name("C:\\Users\\dev\\MyApp"), "myapp");
        assert_eq!(sanitize_repo_name("some.repo.name"), "some_repo_name");
    }

    #[test]
    fn test_collection_name() {
        assert_eq!(build_collection_name("my_project"), "oxi_my_project");
    }
}
