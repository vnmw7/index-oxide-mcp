/*
 * System: Index Oxide MCP
 * File URL: oxidized-index-mcp/src/util/language.rs
 * Purpose: File extension to programming language detection and supported language registry
 */

use serde::{Deserialize, Serialize};

/// Supported programming languages for AST-aware parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SupportedLanguage {
    Rust,
    Python,
    Typescript,
    Tsx,
    Go,
}

impl SupportedLanguage {
    /// Return the string identifier for this language.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::Typescript => "typescript",
            Self::Tsx => "tsx",
            Self::Go => "go",
        }
    }
}

impl std::fmt::Display for SupportedLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Detect language from file extension. Returns None for unsupported files.
pub fn detect_language(path: &std::path::Path) -> Option<SupportedLanguage> {
    let ext = path.extension()?.to_str()?;
    match ext {
        "rs" => Some(SupportedLanguage::Rust),
        "py" | "pyi" => Some(SupportedLanguage::Python),
        "ts" | "mts" | "cts" => Some(SupportedLanguage::Typescript),
        "tsx" => Some(SupportedLanguage::Tsx),
        "go" => Some(SupportedLanguage::Go),
        _ => None,
    }
}

/// Check if a language string matches a supported language.
pub fn is_language_allowed(language: &SupportedLanguage, allowlist: &Option<Vec<String>>) -> bool {
    match allowlist {
        None => true,
        Some(allowed) => allowed
            .iter()
            .any(|a| a.eq_ignore_ascii_case(language.as_str())),
    }
}

/// Check if a file path matches binary/non-text patterns that should be skipped.
pub fn is_binary_extension(path: &std::path::Path) -> bool {
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => e.to_lowercase(),
        None => return false,
    };

    matches!(
        ext.as_str(),
        "exe"
            | "dll"
            | "so"
            | "dylib"
            | "o"
            | "a"
            | "zip"
            | "tar"
            | "gz"
            | "bz2"
            | "xz"
            | "7z"
            | "rar"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "bmp"
            | "ico"
            | "svg"
            | "webp"
            | "mp3"
            | "mp4"
            | "avi"
            | "mov"
            | "mkv"
            | "wav"
            | "flac"
            | "pdf"
            | "doc"
            | "docx"
            | "xls"
            | "xlsx"
            | "ppt"
            | "pptx"
            | "wasm"
            | "pyc"
            | "class"
            | "jar"
            | "lock"
            | "sum"
            | "ttf"
            | "otf"
            | "woff"
            | "woff2"
            | "eot"
            | "sqlite"
            | "db"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_detect_rust() {
        assert_eq!(
            detect_language(Path::new("main.rs")),
            Some(SupportedLanguage::Rust)
        );
    }

    #[test]
    fn test_detect_python() {
        assert_eq!(
            detect_language(Path::new("app.py")),
            Some(SupportedLanguage::Python)
        );
        assert_eq!(
            detect_language(Path::new("types.pyi")),
            Some(SupportedLanguage::Python)
        );
    }

    #[test]
    fn test_detect_typescript() {
        assert_eq!(
            detect_language(Path::new("index.ts")),
            Some(SupportedLanguage::Typescript)
        );
        assert_eq!(
            detect_language(Path::new("App.tsx")),
            Some(SupportedLanguage::Tsx)
        );
    }

    #[test]
    fn test_detect_go() {
        assert_eq!(
            detect_language(Path::new("main.go")),
            Some(SupportedLanguage::Go)
        );
    }

    #[test]
    fn test_detect_unsupported() {
        assert_eq!(detect_language(Path::new("readme.md")), None);
        assert_eq!(detect_language(Path::new("Makefile")), None);
    }

    #[test]
    fn test_binary_detection() {
        assert!(is_binary_extension(Path::new("image.png")));
        assert!(is_binary_extension(Path::new("archive.zip")));
        assert!(!is_binary_extension(Path::new("code.rs")));
    }
}
