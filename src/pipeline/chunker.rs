/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/pipeline/chunker.rs
 * Purpose: AST-aware semantic code chunking using tree-sitter for 4 languages
 */

use crate::models::chunk::CodeChunk;
use crate::util::hashing::compute_content_hash;
use crate::util::language::SupportedLanguage;
use tree_sitter::{Node, Tree};

/// Metadata context for the file being processed.
struct FileContext<'a> {
    source: &'a str,
    language: SupportedLanguage,
    path: &'a str,
    repo: &'a str,
    file_mtime: &'a str,
    file_size: u64,
    imports: &'a str,
}

/// Maximum chunk text length before attempting a sub-split (~16KB).
const MAX_CHUNK_SIZE: usize = 16_384;

/// Extract semantic code chunks from a parsed tree-sitter AST.
pub fn extract_chunks(
    tree: &Tree,
    source: &str,
    language: SupportedLanguage,
    path: &str,
    repo: &str,
    file_mtime: &str,
    file_size: u64,
) -> Vec<CodeChunk> {
    let root = tree.root_node();
    let mut chunks = Vec::new();

    // Extract file-level imports once
    let imports = extract_imports(&root, source, language);

    let ctx = FileContext {
        source,
        language,
        path,
        repo,
        file_mtime,
        file_size,
        imports: &imports,
    };

    // Walk top-level definitions
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        extract_from_node(
            &child,
            &ctx,
            None, // no parent at top level
            &mut chunks,
        );
    }

    // If a file has no extractable definitions, create a single file-level chunk
    if chunks.is_empty() && !source.trim().is_empty() {
        let content_hash = compute_content_hash(source);
        chunks.push(CodeChunk {
            repo: repo.to_string(),
            branch: None,
            commit_sha: None,
            path: path.to_string(),
            language: language.as_str().to_string(),
            symbol_name: extract_file_stem(path),
            symbol_kind: "module".to_string(),
            symbol_path: extract_file_stem(path),
            parent_symbol: None,
            line_start: 1,
            line_end: source.lines().count() as u32,
            byte_start: 0,
            byte_end: source.len() as u32,
            imports: if imports.is_empty() {
                None
            } else {
                Some(imports.clone())
            },
            signature: None,
            doc_comment: None,
            chunk_text: source.to_string(),
            content_hash,
            file_mtime: file_mtime.to_string(),
            file_size,
        });
    }

    chunks
}

/// Recursively extract chunks from an AST node if it matches a definition kind.
fn extract_from_node(
    node: &Node,
    ctx: &FileContext,
    parent_symbol: Option<&str>,
    chunks: &mut Vec<CodeChunk>,
) {
    let kind = node.kind();

    if is_definition_node(kind, ctx.language) {
        let symbol_name = extract_symbol_name(node, ctx.source, ctx.language)
            .unwrap_or_else(|| "anonymous".to_string());

        let symbol_kind = normalize_symbol_kind(kind, ctx.language);

        let symbol_path = match parent_symbol {
            Some(parent) => format!("{}::{}", parent, symbol_name),
            None => symbol_name.clone(),
        };

        let doc_comment = extract_doc_comment(node, ctx.source, ctx.language);
        let signature = extract_signature(node, ctx.source, ctx.language);

        let start_byte = node.start_byte();
        let end_byte = node.end_byte();
        let chunk_text = ctx.source[start_byte..end_byte].to_string();

        // Handle oversized chunks by splitting at secondary AST boundaries
        if chunk_text.len() > MAX_CHUNK_SIZE {
            let parent = ParentMetadata {
                name: &symbol_name,
                kind: &symbol_kind,
                path: &symbol_path,
                doc_comment: &doc_comment,
                signature: &signature,
            };
            split_oversized_node(node, ctx, &parent, chunks);
            return;
        }

        let content_hash = compute_content_hash(&chunk_text);

        chunks.push(CodeChunk {
            repo: ctx.repo.to_string(),
            branch: None,
            commit_sha: None,
            path: ctx.path.to_string(),
            language: ctx.language.as_str().to_string(),
            symbol_name,
            symbol_kind,
            symbol_path: symbol_path.clone(),
            parent_symbol: parent_symbol.map(|s| s.to_string()),
            line_start: node.start_position().row as u32 + 1,
            line_end: node.end_position().row as u32 + 1,
            byte_start: start_byte as u32,
            byte_end: end_byte as u32,
            imports: if ctx.imports.is_empty() {
                None
            } else {
                Some(ctx.imports.to_string())
            },
            signature,
            doc_comment,
            chunk_text,
            content_hash,
            file_mtime: ctx.file_mtime.to_string(),
            file_size: ctx.file_size,
        });

        // Recurse into children for nested definitions (e.g. methods in impl/class)
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            extract_from_node(&child, ctx, Some(&symbol_path), chunks);
        }
    } else {
        // Not a definition node — recurse to find definitions inside
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            extract_from_node(&child, ctx, parent_symbol, chunks);
        }
    }
}

/// Check if a node kind represents a semantic definition we should chunk.
fn is_definition_node(kind: &str, language: SupportedLanguage) -> bool {
    match language {
        SupportedLanguage::Rust => matches!(
            kind,
            "function_item"
                | "impl_item"
                | "struct_item"
                | "enum_item"
                | "type_item"
                | "const_item"
                | "static_item"
                | "mod_item"
                | "trait_item"
                | "macro_definition"
        ),
        SupportedLanguage::Python => matches!(
            kind,
            "function_definition" | "class_definition" | "decorated_definition"
        ),
        SupportedLanguage::Typescript | SupportedLanguage::Tsx => matches!(
            kind,
            "function_declaration"
                | "class_declaration"
                | "interface_declaration"
                | "type_alias_declaration"
                | "enum_declaration"
                | "export_statement"
                | "lexical_declaration"
                | "method_definition"
                | "arrow_function"
        ),
        SupportedLanguage::Go => matches!(
            kind,
            "function_declaration"
                | "method_declaration"
                | "type_declaration"
                | "const_declaration"
                | "var_declaration"
        ),
    }
}

/// Normalize AST node kinds to a canonical set of symbol kinds.
fn normalize_symbol_kind(kind: &str, _language: SupportedLanguage) -> String {
    match kind {
        "function_item" | "function_definition" | "function_declaration" | "arrow_function" => {
            "function".to_string()
        }
        "method_declaration" | "method_definition" => "method".to_string(),
        "impl_item" => "impl".to_string(),
        "struct_item" => "struct".to_string(),
        "enum_item" | "enum_declaration" => "enum".to_string(),
        "class_definition" | "class_declaration" => "class".to_string(),
        "trait_item" => "trait".to_string(),
        "interface_declaration" => "interface".to_string(),
        "type_item" | "type_alias_declaration" | "type_declaration" => "type_alias".to_string(),
        "const_item" | "const_declaration" => "const".to_string(),
        "static_item" => "static".to_string(),
        "var_declaration" | "lexical_declaration" => "variable".to_string(),
        "mod_item" => "module".to_string(),
        "macro_definition" => "macro".to_string(),
        "export_statement" => "export".to_string(),
        "decorated_definition" => "decorated".to_string(),
        _ => kind.to_string(),
    }
}

/// Extract the symbol name from a definition node.
fn extract_symbol_name(node: &Node, source: &str, language: SupportedLanguage) -> Option<String> {
    // Most definitions have a named child with kind "name" or "identifier"
    let name_node = find_child_by_field_name(node, "name")
        .or_else(|| find_child_by_kind(node, "identifier"))
        .or_else(|| find_child_by_kind(node, "type_identifier"))
        .or_else(|| find_child_by_kind(node, "name"));

    if let Some(n) = name_node {
        return Some(source[n.start_byte()..n.end_byte()].to_string());
    }

    // For decorated definitions in Python, look at the inner definition
    if node.kind() == "decorated_definition" {
        if let Some(def) = find_child_by_kind(node, "function_definition")
            .or_else(|| find_child_by_kind(node, "class_definition"))
        {
            return extract_symbol_name(&def, source, language);
        }
    }

    // For export statements in TS, look at the exported declaration
    if node.kind() == "export_statement" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if is_definition_node(child.kind(), language) {
                return extract_symbol_name(&child, source, language);
            }
        }
    }

    None
}

/// Extract doc comments preceding a definition node.
fn extract_doc_comment(node: &Node, source: &str, _language: SupportedLanguage) -> Option<String> {
    let mut comments = Vec::new();

    // Walk backwards from the node looking for adjacent comment nodes
    let mut sibling = node.prev_sibling();
    while let Some(sib) = sibling {
        let kind = sib.kind();
        if kind == "comment"
            || kind == "line_comment"
            || kind == "block_comment"
            || kind == "doc_comment"
            || kind == "string"
        // Python docstrings as first child
        {
            let text = source[sib.start_byte()..sib.end_byte()].trim().to_string();
            comments.push(text);
            sibling = sib.prev_sibling();
        } else {
            break;
        }
    }

    if comments.is_empty() {
        None
    } else {
        comments.reverse();
        Some(comments.join("\n"))
    }
}

/// Extract the function/method signature (first line or up to the body).
fn extract_signature(node: &Node, source: &str, _language: SupportedLanguage) -> Option<String> {
    let kind = node.kind();

    // Only extract signatures for function-like definitions
    if !matches!(
        kind,
        "function_item"
            | "function_definition"
            | "function_declaration"
            | "method_declaration"
            | "method_definition"
            | "arrow_function"
    ) {
        return None;
    }

    // Find the body node and take text up to it
    let body = find_child_by_kind(node, "block")
        .or_else(|| find_child_by_kind(node, "statement_block"))
        .or_else(|| find_child_by_field_name(node, "body"));

    let sig_end = body.map(|b| b.start_byte()).unwrap_or(node.end_byte());
    let sig_start = node.start_byte();

    if sig_end > sig_start {
        let sig = source[sig_start..sig_end].trim().to_string();
        Some(sig)
    } else {
        None
    }
}

/// Extract file-level imports as context.
fn extract_imports(root: &Node, source: &str, language: SupportedLanguage) -> String {
    let import_kinds = match language {
        SupportedLanguage::Rust => vec!["use_declaration", "extern_crate_declaration"],
        SupportedLanguage::Python => vec!["import_statement", "import_from_statement"],
        SupportedLanguage::Typescript | SupportedLanguage::Tsx => vec!["import_statement"],
        SupportedLanguage::Go => vec!["import_declaration"],
    };

    let mut imports = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        if import_kinds.contains(&child.kind()) {
            let text = source[child.start_byte()..child.end_byte()]
                .trim()
                .to_string();
            imports.push(text);
        }
    }

    imports.join("\n")
}

/// Metadata context for the parent node during recursive chunking.
struct ParentMetadata<'a> {
    name: &'a str,
    kind: &'a str,
    path: &'a str,
    doc_comment: &'a Option<String>,
    signature: &'a Option<String>,
}

/// Split an oversized definition node at secondary AST boundaries.
fn split_oversized_node(
    node: &Node,
    ctx: &FileContext,
    parent: &ParentMetadata,
    chunks: &mut Vec<CodeChunk>,
) {
    // Try to split at child definition boundaries
    let mut child_chunks = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if is_definition_node(child.kind(), ctx.language) {
            let child_name = extract_symbol_name(&child, ctx.source, ctx.language)
                .unwrap_or_else(|| "anonymous".to_string());
            let child_kind = normalize_symbol_kind(child.kind(), ctx.language);
            let child_path = format!("{}::{}", parent.path, child_name);

            let start = child.start_byte();
            let end = child.end_byte();
            let text = ctx.source[start..end].to_string();
            let hash = compute_content_hash(&text);

            child_chunks.push(CodeChunk {
                repo: ctx.repo.to_string(),
                branch: None,
                commit_sha: None,
                path: ctx.path.to_string(),
                language: ctx.language.as_str().to_string(),
                symbol_name: child_name,
                symbol_kind: child_kind,
                symbol_path: child_path,
                parent_symbol: Some(parent.path.to_string()),
                line_start: child.start_position().row as u32 + 1,
                line_end: child.end_position().row as u32 + 1,
                byte_start: start as u32,
                byte_end: end as u32,
                imports: if ctx.imports.is_empty() {
                    None
                } else {
                    Some(ctx.imports.to_string())
                },
                signature: extract_signature(&child, ctx.source, ctx.language),
                doc_comment: extract_doc_comment(&child, ctx.source, ctx.language),
                chunk_text: text,
                content_hash: hash,
                file_mtime: ctx.file_mtime.to_string(),
                file_size: ctx.file_size,
            });
        }
    }

    if !child_chunks.is_empty() {
        // Also add the parent definition header (up to first child) as context chunk
        let first_child_start = child_chunks.first().map(|c| c.byte_start).unwrap_or(0);
        if first_child_start > node.start_byte() as u32 {
            let header_text = ctx.source[node.start_byte()..first_child_start as usize]
                .trim()
                .to_string();
            if !header_text.is_empty() {
                let hash = compute_content_hash(&header_text);
                chunks.push(CodeChunk {
                    repo: ctx.repo.to_string(),
                    branch: None,
                    commit_sha: None,
                    path: ctx.path.to_string(),
                    language: ctx.language.as_str().to_string(),
                    symbol_name: parent.name.to_string(),
                    symbol_kind: format!("{}_header", parent.kind),
                    symbol_path: format!("{}::__header__", parent.path),
                    parent_symbol: None,
                    line_start: node.start_position().row as u32 + 1,
                    line_end: (node.start_position().row + header_text.lines().count()) as u32,
                    byte_start: node.start_byte() as u32,
                    byte_end: first_child_start,
                    imports: if ctx.imports.is_empty() {
                        None
                    } else {
                        Some(ctx.imports.to_string())
                    },
                    signature: parent.signature.clone(),
                    doc_comment: parent.doc_comment.clone(),
                    chunk_text: header_text,
                    content_hash: hash,
                    file_mtime: ctx.file_mtime.to_string(),
                    file_size: ctx.file_size,
                });
            }
        }

        chunks.extend(child_chunks);
    } else {
        // No child definitions found — fall back to byte-range splitting with context
        let full_text = ctx.source[node.start_byte()..node.end_byte()].to_string();
        let lines: Vec<&str> = full_text.lines().collect();
        let lines_per_chunk = 200;

        for (idx, line_chunk) in lines.chunks(lines_per_chunk).enumerate() {
            let text = line_chunk.join("\n");
            let hash = compute_content_hash(&text);
            let chunk_line_start =
                node.start_position().row as u32 + 1 + (idx * lines_per_chunk) as u32;

            chunks.push(CodeChunk {
                repo: ctx.repo.to_string(),
                branch: None,
                commit_sha: None,
                path: ctx.path.to_string(),
                language: ctx.language.as_str().to_string(),
                symbol_name: format!("{}__part{}", parent.name, idx),
                symbol_kind: format!("{}_part", parent.kind),
                symbol_path: format!("{}::__part{}__", parent.path, idx),
                parent_symbol: Some(parent.path.to_string()),
                line_start: chunk_line_start,
                line_end: chunk_line_start + line_chunk.len() as u32,
                byte_start: node.start_byte() as u32,
                byte_end: node.end_byte() as u32,
                imports: if ctx.imports.is_empty() {
                    None
                } else {
                    Some(ctx.imports.to_string())
                },
                signature: if idx == 0 {
                    parent.signature.clone()
                } else {
                    None
                },
                doc_comment: if idx == 0 {
                    parent.doc_comment.clone()
                } else {
                    None
                },
                chunk_text: text,
                content_hash: hash,
                file_mtime: ctx.file_mtime.to_string(),
                file_size: ctx.file_size,
            });
        }
    }
}

// -- Helper functions for AST navigation --

fn find_child_by_field_name<'a>(node: &'a Node<'a>, field: &str) -> Option<Node<'a>> {
    node.child_by_field_name(field)
}

fn find_child_by_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    let result = node
        .children(&mut cursor)
        .find(|child| child.kind() == kind);
    result
}

fn extract_file_stem(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn parse_rust(source: &str) -> Tree {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn test_extract_rust_function() {
        let source = r#"
/// A greeting function
fn greet(name: &str) -> String {
    format!("Hello, {}!", name)
}
"#;
        let tree = parse_rust(source);
        let chunks = extract_chunks(
            &tree,
            source,
            SupportedLanguage::Rust,
            "src/lib.rs",
            "test",
            "",
            0,
        );

        assert!(!chunks.is_empty());
        let func = chunks.iter().find(|c| c.symbol_name == "greet").unwrap();
        assert_eq!(func.symbol_kind, "function");
        assert!(func.chunk_text.contains("fn greet"));
    }

    #[test]
    fn test_extract_rust_struct() {
        let source = r#"
struct Config {
    name: String,
    value: i32,
}
"#;
        let tree = parse_rust(source);
        let chunks = extract_chunks(
            &tree,
            source,
            SupportedLanguage::Rust,
            "src/config.rs",
            "test",
            "",
            0,
        );

        let struct_chunk = chunks.iter().find(|c| c.symbol_name == "Config").unwrap();
        assert_eq!(struct_chunk.symbol_kind, "struct");
    }

    #[test]
    fn test_extract_rust_impl_block() {
        let source = r#"
impl Config {
    fn new() -> Self {
        Self { name: String::new(), value: 0 }
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}
"#;
        let tree = parse_rust(source);
        let chunks = extract_chunks(
            &tree,
            source,
            SupportedLanguage::Rust,
            "src/config.rs",
            "test",
            "",
            0,
        );

        // Should find the impl block and its methods
        assert!(chunks.iter().any(|c| c.symbol_kind == "impl"));
        assert!(chunks.iter().any(|c| c.symbol_name == "new"));
        assert!(chunks.iter().any(|c| c.symbol_name == "get_name"));
    }
}
