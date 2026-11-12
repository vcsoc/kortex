//! Shared public domain model. Coordinates are 1-based UTF-8 byte columns.
mod control;
pub use control::RequestControl;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, thiserror::Error)]
#[error("{code}: {message}")]
pub struct Error {
    pub code: &'static str,
    pub message: String,
    pub exit: i32,
}
impl Error {
    pub fn new(code: &'static str, message: impl Into<String>, exit: i32) -> Self {
        Self {
            code,
            message: message.into(),
            exit,
        }
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::new("IO_ERROR", e.to_string(), 1)
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::new("INVALID_JSON", e.to_string(), 2)
    }
}
pub fn hash(value: impl AsRef<[u8]>) -> String {
    blake3::hash(value.as_ref()).to_hex().to_string()
}
pub fn id(parts: &[&str]) -> String {
    let mut h = blake3::Hasher::new();
    for p in parts {
        h.update(&(p.len() as u64).to_le_bytes());
        h.update(p.as_bytes());
    }
    h.finalize().to_hex().to_string()
}
pub fn public_path(path: &Path) -> String {
    #[cfg(not(windows))]
    {
        path.to_string_lossy().into_owned()
    }
    #[cfg(windows)]
    {
        let normalized = path.to_string_lossy().replace('\\', "/");
        if let Some(unc) = normalized.strip_prefix("//?/UNC/") {
            format!("//{unc}")
        } else {
            normalized
                .strip_prefix("//?/")
                .unwrap_or(&normalized)
                .to_owned()
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Span {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Symbol {
    pub symbol_id: String,
    pub symbol_revision_id: String,
    pub repo_id: String,
    pub kind: String,
    pub language: String,
    pub name: String,
    pub qualified_name: String,
    pub file_path: String,
    pub span: Span,
    pub signature: String,
    pub visibility: String,
    pub attributes: serde_json::Value,
    pub content_hash: String,
    pub doc_comment: Option<String>,
    pub owner_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Reference {
    pub source_id: String,
    pub name: String,
    pub qualifier: Option<String>,
    pub span: Span,
    pub kind: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Import {
    pub local: String,
    pub module: String,
    pub imported: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParsedFile {
    pub path: String,
    pub language: String,
    pub content_hash: String,
    pub size_bytes: usize,
    pub symbols: Vec<Symbol>,
    pub references: Vec<Reference>,
    pub imports: Vec<Import>,
    #[serde(default)]
    pub import_scopes: Vec<String>,
    #[serde(default)]
    pub import_spans: Vec<Span>,
    pub diagnostics: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Edge {
    pub id: String,
    pub repo_id: String,
    pub source_id: String,
    pub target_id: Option<String>,
    pub kind: String,
    pub source_span: Span,
    pub resolution: String,
    pub confidence: f64,
    pub resolver: String,
    pub evidence: String,
    pub candidates: Vec<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub repo_id: String,
    pub root: String,
    pub generation_id: String,
    pub git_head: Option<String>,
    pub config_hash: String,
    pub grammar_hash: String,
    #[serde(default)]
    pub resolution_hash: String,
    pub files: Vec<ParsedFile>,
    pub edges: Vec<Edge>,
    pub diagnostics: Vec<String>,
}
impl Snapshot {
    pub fn symbols(&self) -> impl Iterator<Item = &Symbol> {
        self.files.iter().flat_map(|f| &f.symbols)
    }
    pub fn symbol(&self, id: &str) -> Option<&Symbol> {
        self.symbols().find(|s| s.symbol_id == id)
    }
}
pub const PARSER_VERSION: &str = "tree-sitter-0.25-kortex-7";

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_path_encoding() {
        #[cfg(not(windows))]
        assert_eq!(
            public_path(Path::new("literal\\name.py")),
            "literal\\name.py"
        );
        #[cfg(windows)]
        {
            assert_eq!(
                public_path(Path::new(r"\\?\C:\source\main.rs")),
                "C:/source/main.rs"
            );
            assert_eq!(
                public_path(Path::new(r"\\?\UNC\host\share\main.rs")),
                "//host/share/main.rs"
            );
        }
    }
    #[test]
    fn identity_is_delimited() {
        assert_ne!(id(&["ab", "c"]), id(&["a", "bc"]));
        assert_eq!(id(&["x"]), id(&["x"]));
    }
}
