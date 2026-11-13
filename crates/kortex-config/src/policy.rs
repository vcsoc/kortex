use crate::{Http, Search, config_error};
use kortex_core::{Error, Result};
use std::{io::Read, path::Path};
fn policy(message: &str) -> Error {
    Error::new("SECURITY_POLICY", message, 8)
}
pub(crate) fn read_config(path: &Path) -> std::io::Result<String> {
    // Preserve the initial nonregular refusal where no nonblocking file-open
    // primitive is wired. This is not a claim of race-free non-Unix config IO.
    #[cfg(not(unix))]
    if !std::fs::metadata(path)?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "configuration must be a regular file",
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Open before checking the handle type. A metadata-before-open check
        // lets a same-user replacement FIFO block the config reader forever.
        options.custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32);
    }
    read_opened_config(options.open(path)?)
}
pub(crate) fn read_opened_config(file: std::fs::File) -> std::io::Result<String> {
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "configuration must be a regular file",
        ));
    }
    let mut bytes = vec![];
    file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "configuration exceeds 1 MiB",
        ));
    }
    String::from_utf8(bytes).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "configuration is not UTF-8",
        )
    })
}
pub(crate) fn validate_repository_policy(base: &toml::Value, overlay: &toml::Value) -> Result<()> {
    if let Some(resources) = overlay.get("index").and_then(|v| v.get("resources")) {
        let fields = resources
            .as_table()
            .ok_or_else(|| policy("index.resources must be a table"))?;
        for (name, value) in fields {
            let cap = base
                .get("index")
                .and_then(|v| v.get("resources"))
                .and_then(|v| v.get(name))
                .and_then(toml::Value::as_integer);
            if !matches!((value.as_integer(), cap), (Some(n), Some(cap)) if n > 0 && n <= cap) {
                return Err(policy(
                    "repository configuration can only lower inherited resource limits",
                ));
            }
        }
    }
    // Repository policy may retain or disable a capability, never activate/redirect it.
    for (section, fields) in [
        (
            "http",
            &[
                "host",
                "port",
                "auth_token_env",
                "cors_origins",
                "allowed_repo_ids",
            ][..],
        ),
        (
            "search",
            &[
                "model_path",
                "model_version",
                "model_checksum",
                "cache_path",
            ][..],
        ),
    ] {
        for field in fields {
            if let Some(value) = overlay.get(section).and_then(|v| v.get(*field))
                && Some(value) != base.get(section).and_then(|v| v.get(*field))
            {
                return Err(policy(
                    "repository configuration cannot change trusted HTTP/model policy; use user config, explicit --config or environment",
                ));
            }
        }
    }
    if overlay
        .get("http")
        .and_then(|v| v.get("enabled"))
        .is_some_and(|v| {
            v.as_bool() != Some(false) && Some(v) != base.get("http").and_then(|v| v.get("enabled"))
        })
    {
        return Err(policy("repository configuration cannot enable HTTP"));
    }
    if overlay
        .get("search")
        .and_then(|v| v.get("embeddings"))
        .is_some_and(|v| {
            v.as_str() != Some("disabled")
                && Some(v) != base.get("search").and_then(|v| v.get("embeddings"))
        })
    {
        return Err(policy(
            "repository configuration cannot enable embedding models",
        ));
    }
    Ok(())
}
pub(crate) fn validate_search(search: &Search) -> Result<()> {
    if search.backend != "sqlite_fts5"
        || !["disabled", "local"].contains(&search.embeddings.as_str())
    {
        return Err(policy(
            "only offline FTS5 and explicit local embeddings are permitted; remote providers await owner approval",
        ));
    }
    if !search.keyword_weight.is_finite()
        || !search.vector_weight.is_finite()
        || search.keyword_weight <= 0.0
        || search.vector_weight < 0.0
        || search.keyword_weight > 100.0
        || search.vector_weight > 100.0
    {
        return Err(config_error(
            "ranking weights must be finite: keyword (0,100], vector [0,100]",
        ));
    }
    if !search.cache_path.is_empty()
        && (!Path::new(&search.cache_path).is_absolute()
            || Path::new(&search.cache_path)
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir)))
    {
        return Err(config_error(
            "search.cache_path must be an absolute local path without traversal",
        ));
    }
    if search.embeddings == "local" {
        let path = Path::new(&search.model_path);
        if !path.is_absolute()
            || !path.is_file()
            || path.symlink_metadata()?.file_type().is_symlink()
            || search.model_version.is_empty()
            || search.model_version.len() > 128
            || search.model_checksum.len() != 64
            || !search.model_checksum.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(config_error(
                "local embeddings require an explicit absolute regular model file, model_version and 64-hex BLAKE3 model_checksum; models are never downloaded",
            ));
        }
    }
    Ok(())
}
pub(crate) fn validate_http(http: &Http) -> Result<()> {
    let address: std::net::IpAddr = http
        .host
        .parse()
        .map_err(|_| config_error("http.host must be a literal IPv4/IPv6 address"))?;
    if http.port == 0 {
        return Err(config_error("http.port must be nonzero"));
    }
    if !http.auth_token_env.is_empty()
        && (!http
            .auth_token_env
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
            || http.auth_token_env.len() > 128)
    {
        return Err(config_error(
            "http.auth_token_env must be an environment variable NAME, never a token",
        ));
    }
    if http.enabled
        && !address.is_loopback()
        && (http.auth_token_env.is_empty() || http.allowed_repo_ids.is_empty())
    {
        return Err(policy(
            "non-loopback HTTP requires explicit auth_token_env and allowed_repo_ids in trusted configuration",
        ));
    }
    if http.cors_origins.len() > 64
        || http.allowed_repo_ids.len() > 1024
        || http.allowed_repo_ids.iter().any(|id| {
            id.is_empty()
                || id.len() > 256
                || id.chars().any(char::is_control)
                || id.contains(['/', '\\', '*'])
        })
    {
        return Err(config_error(
            "invalid or excessive allowed repository IDs/CORS origins",
        ));
    }
    for origin in &http.cors_origins {
        let Some(authority) = origin
            .strip_prefix("https://")
            .or_else(|| origin.strip_prefix("http://"))
        else {
            return Err(config_error(
                "CORS origins must be explicit http(s) origins",
            ));
        };
        if authority.is_empty()
            || authority.contains(['/', '?', '#', '@', '*'])
            || authority
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(config_error(
                "CORS origins cannot contain wildcards, paths, credentials or controls",
            ));
        }
    }
    Ok(())
}
