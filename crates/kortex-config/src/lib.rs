//! Strict configuration; repository policy cannot enable outbound services.
mod policy;
use kortex_core::{Error, Result};
use policy::{read_config, validate_http, validate_repository_policy, validate_search};
#[cfg(test)]
mod readiness_tests;
use serde::{Deserialize, Serialize};
use std::{
    env,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub repository: Repository,
    pub index: Index,
    pub analysis: Analysis,
    pub search: Search,
    pub mcp: Mcp,
    pub http: Http,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Repository {
    pub root: String,
    pub follow_symlinks: bool,
    pub include_hidden: bool,
    pub max_file_size_bytes: u64,
    pub max_files: usize,
    pub max_total_bytes: u64,
    pub exclude: Vec<String>,
    pub include: Vec<String>,
    pub languages: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Index {
    pub mode: String,
    pub workers: usize,
    pub watch_debounce_ms: u64,
    pub storage: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Analysis {
    pub default_depth: usize,
    pub max_depth: usize,
    pub max_nodes: usize,
    pub query_timeout_ms: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Search {
    pub backend: String,
    pub embeddings: String,
    /// Explicit local artifact only; no repository-triggered downloads or execution.
    pub model_path: String,
    pub model_version: String,
    pub model_checksum: String,
    pub cache_path: String,
    pub keyword_weight: f64,
    pub vector_weight: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Mcp {
    pub read_only: bool,
    pub max_response_bytes: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Http {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    /// Environment variable name, never the token itself.
    pub auth_token_env: String,
    pub cors_origins: Vec<String>,
    pub allowed_repo_ids: Vec<String>,
}
impl Default for Repository {
    fn default() -> Self {
        Self {
            root: ".".into(),
            follow_symlinks: false,
            include_hidden: false,
            max_file_size_bytes: 2 * 1024 * 1024,
            max_files: 100_000,
            max_total_bytes: 256 * 1024 * 1024,
            exclude: vec![],
            include: vec![],
            languages: vec![],
        }
    }
}
impl Default for Index {
    fn default() -> Self {
        Self {
            mode: "incremental".into(),
            workers: 0,
            watch_debounce_ms: 500,
            storage: "repository".into(),
        }
    }
}
impl Default for Analysis {
    fn default() -> Self {
        Self {
            default_depth: 3,
            max_depth: 10,
            max_nodes: 500,
            query_timeout_ms: 10000,
        }
    }
}
impl Default for Search {
    fn default() -> Self {
        Self {
            backend: "sqlite_fts5".into(),
            embeddings: "disabled".into(),
            model_path: String::new(),
            model_version: String::new(),
            model_checksum: String::new(),
            cache_path: String::new(),
            keyword_weight: 1.0,
            vector_weight: 1.0,
        }
    }
}
impl Default for Mcp {
    fn default() -> Self {
        Self {
            read_only: true,
            max_response_bytes: 262144,
        }
    }
}
impl Default for Http {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "127.0.0.1".into(),
            port: 7337,
            auth_token_env: String::new(),
            cors_origins: vec![],
            allowed_repo_ids: vec![],
        }
    }
}
impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: 1,
            repository: Repository::default(),
            index: Index::default(),
            analysis: Analysis::default(),
            search: Search::default(),
            mcp: Mcp::default(),
            http: Http::default(),
        }
    }
}
fn merge(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(a), toml::Value::Table(b)) => {
            for (k, v) in b {
                if let Some(old) = a.get_mut(&k) {
                    merge(old, v);
                } else {
                    a.insert(k, v);
                }
            }
        }
        (a, b) => *a = b,
    }
}
pub fn user_config_path() -> Option<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("APPDATA").map(PathBuf::from))
        .or_else(|| env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        .map(|p| p.join("kortex/config.toml"))
}
impl Config {
    pub fn load(root: &Path, explicit: Option<&Path>) -> Result<Self> {
        let mut value = toml::Value::try_from(Self::default()).map_err(config_error)?;
        for (path, trusted) in [
            (user_config_path(), true),
            (
                Some(
                    explicit
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| root.join(".kortex.toml")),
                ),
                explicit.is_some(),
            ),
        ] {
            let Some(path) = path else {
                continue;
            };
            match read_config(&path) {
                Ok(s) => {
                    let mut overlay = toml::from_str::<toml::Value>(&s).map_err(config_error)?;
                    if !trusted {
                        validate_repository_policy(&value, &overlay)?;
                        if path.canonicalize()?.parent() != Some(root.canonicalize()?.as_path()) {
                            return Err(Error::new(
                                "SECURITY_POLICY",
                                "repository config cannot escape its root via symlink",
                                8,
                            ));
                        }
                    }
                    if let Some(field) = overlay
                        .get_mut("repository")
                        .and_then(|v| v.get_mut("root"))
                        && let Some(relative) = field.as_str()
                    {
                        if relative.is_empty() {
                            return Err(config_error("repository.root must be nonempty"));
                        }
                        let base = if trusted {
                            path.parent().unwrap_or(root)
                        } else {
                            root
                        };
                        let target = base.join(relative).canonicalize()?;
                        if !target.is_dir()
                            || (!trusted && !target.starts_with(root.canonicalize()?))
                        {
                            return Err(Error::new(
                                "SECURITY_POLICY",
                                "repository.root cannot escape the configured root",
                                8,
                            ));
                        }
                        *field = toml::Value::String(
                            target
                                .to_str()
                                .ok_or_else(|| config_error("root must be UTF-8"))?
                                .into(),
                        );
                    }
                    merge(&mut value, overlay);
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::NotFound
                        && explicit != Some(path.as_path()) => {}
                Err(e) => return Err(e.into()),
            }
        }
        for (key, raw) in env::vars().filter(|(k, _)| k.starts_with("KORTEX_") && k.contains("__"))
        {
            let parts: Vec<_> = key
                .trim_start_matches("KORTEX_")
                .split("__")
                .map(str::to_lowercase)
                .collect();
            let mut field = &mut value;
            for part in parts {
                field = field.get_mut(&part).ok_or_else(|| {
                    config_error(format!("unknown configuration environment field {key}"))
                })?;
            }
            *field = if field.is_str() {
                toml::Value::String(raw)
            } else {
                let mut wrapper: toml::Table =
                    toml::from_str(&format!("value = {raw}")).map_err(config_error)?;
                wrapper
                    .remove("value")
                    .ok_or_else(|| config_error(format!("invalid value for {key}")))?
            };
        }
        let mut config: Self = value.try_into().map_err(config_error)?;
        if let Ok(v) = env::var("KORTEX_MAX_FILE_SIZE_BYTES") {
            config.repository.max_file_size_bytes = v.parse().map_err(config_error)?;
        }
        if let Ok(v) = env::var("KORTEX_MAX_FILES") {
            config.repository.max_files = v.parse().map_err(config_error)?;
        }
        if let Ok(v) = env::var("KORTEX_INCLUDE_HIDDEN") {
            config.repository.include_hidden = v.parse().map_err(config_error)?;
        }
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        if self.repository.root.is_empty() || self.repository.root.contains('\0') {
            return Err(config_error("repository.root must be a nonempty path"));
        }
        if self.schema_version != 1 {
            return Err(config_error("unsupported schema_version; expected 1"));
        }
        if self.repository.max_total_bytes == 0
            || self.repository.max_total_bytes > 1024 * 1024 * 1024
        {
            return Err(config_error("max_total_bytes must be 1..1073741824"));
        }
        if self.repository.max_file_size_bytes == 0
            || self.repository.max_file_size_bytes > 64 * 1024 * 1024
            || self.repository.max_files == 0
            || self.repository.max_files > 1_000_000
        {
            return Err(config_error(
                "file size must be 1..67108864 and max_files 1..1000000",
            ));
        }
        if self.analysis.default_depth > self.analysis.max_depth
            || self.analysis.max_depth > 10
            || self.analysis.max_nodes == 0
            || self.analysis.max_nodes > 10000
            || self.analysis.query_timeout_ms == 0
            || self.analysis.query_timeout_ms > 60000
        {
            return Err(config_error(
                "invalid analysis bounds (depth <=10, nodes <=10000, timeout <=60000)",
            ));
        }
        validate_search(&self.search)?;
        validate_http(&self.http)?;
        if !self.mcp.read_only {
            return Err(Error::new(
                "SECURITY_POLICY",
                "MCP must remain read-only",
                8,
            ));
        }
        if !["repository", "user-cache"].contains(&self.index.storage.as_str())
            || !["full", "incremental"].contains(&self.index.mode.as_str())
            || self.index.watch_debounce_ms < 50
            || self.index.watch_debounce_ms > 60000
            || self.mcp.max_response_bytes < 1024
            || self.mcp.max_response_bytes > 1048576
        {
            return Err(config_error(
                "unsupported storage/mode or invalid debounce/response bounds",
            ));
        }
        Ok(())
    }
    /// Resolve a configured relocation without running Git or executing repository code.
    /// Config::load already confines repository-supplied roots; trusted callers may use absolute roots.
    pub fn resolved_root(&self, base: &Path) -> Result<PathBuf> {
        let target = base.join(&self.repository.root).canonicalize()?;
        if !target.is_dir() || target.to_str().is_none() {
            return Err(config_error(
                "repository.root must resolve to a UTF-8 directory",
            ));
        }
        Ok(target)
    }
    pub fn fingerprint(&self) -> Result<String> {
        Ok(kortex_core::hash(serde_json::to_vec(self)?))
    }
}
fn config_error(e: impl std::fmt::Display) -> Error {
    Error::new("INVALID_CONFIG", e.to_string(), 2)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_and_secure() {
        assert!(toml::from_str::<Config>("typo = 1").is_err());
        let mut c = Config::default();
        c.search.embeddings = "remote".into();
        assert_eq!(c.validate().unwrap_err().exit, 8);
        c = Config::default();
        c.analysis.max_depth = 11;
        assert!(c.validate().is_err());
    }
    #[test]
    fn partial_config_preserves_defaults() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join(".kortex.toml"),
            "[repository]\ninclude_hidden = true\n",
        )
        .unwrap();
        let c = Config::load(d.path(), None).unwrap();
        assert!(c.repository.include_hidden);
        assert_eq!(c.analysis.max_nodes, 500);
    }
}
