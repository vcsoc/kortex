//! Opt-in HTTP startup policy. No HTTP dependency is compiled into the default CLI.
use kortex_core::{Error, Result};

pub(crate) fn serve_http(
    host: Option<String>,
    port: Option<u16>,
    auth_token_env: Option<String>,
    allow_repo: Vec<String>,
    cors_origin: Vec<String>,
    config_path: Option<&std::path::Path>,
    web_assets: Option<&std::path::Path>,
) -> Result<()> {
    let policy = kortex_config::Config::load(&std::env::current_dir()?, config_path)?.http;
    let host = host.unwrap_or(policy.host);
    let port = port.unwrap_or(policy.port);
    let allow_repo = if allow_repo.is_empty() {
        policy.allowed_repo_ids
    } else {
        allow_repo
    };
    let cors_origin = if cors_origin.is_empty() {
        policy.cors_origins
    } else {
        cors_origin
    };
    let auth_token_env = auth_token_env
        .or_else(|| (!policy.auth_token_env.is_empty()).then_some(policy.auth_token_env));
    let ip: std::net::IpAddr = host.parse().map_err(|_| {
        Error::new(
            "INVALID_CONFIG",
            "HTTP host must be a numeric IP address",
            2,
        )
    })?;
    if web_assets.is_some() && !ip.is_loopback() {
        return Err(Error::new(
            "ASSET_POLICY",
            "private web assets may only be served on loopback",
            8,
        ));
    }
    // Require credentials even on loopback: hostile local web pages are not trusted clients.
    let env = auth_token_env.ok_or_else(|| {
        Error::new(
            "AUTH_REQUIRED",
            "HTTP requires --auth-token-env and explicit --allow-repo IDs",
            8,
        )
    })?;
    let token = std::env::var(&env).map_err(|_| {
        Error::new(
            "AUTH_REQUIRED",
            "named HTTP bearer-token environment variable is missing",
            8,
        )
    })?;
    if token.len() < 32 || token.len() > 4096 || !token.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(Error::new(
            "AUTH_REQUIRED",
            "HTTP bearer token must be 32..4096 printable ASCII bytes",
            8,
        ));
    }
    #[cfg(feature = "http")]
    {
        let mut config = kortex_http::Config::loopback(token, allow_repo);
        config.address = std::net::SocketAddr::new(ip, port);
        config.cors_origins = cors_origin.into_iter().collect();
        config.web_assets = web_assets.map(std::path::Path::to_path_buf);
        config.validate()?;
        kortex_http::serve(config)
    }
    #[cfg(not(feature = "http"))]
    {
        let _ = (ip, port, token, allow_repo, cors_origin, web_assets);
        Err(Error::new(
            "FEATURE_DISABLED",
            "HTTP is optional; build kortex-cli with --features http or use serve --mcp stdio",
            2,
        ))
    }
}
