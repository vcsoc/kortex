use super::*;
#[test]
fn root_relocation_is_real_and_repo_escape_is_refused() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("sources")).unwrap();
    std::fs::write(
        d.path().join(".kortex.toml"),
        "[repository]\nroot='sources'\n[index]\nstorage='user-cache'",
    )
    .unwrap();
    let config = Config::load(d.path(), None).unwrap();
    assert_eq!(
        config.resolved_root(d.path()).unwrap(),
        d.path().join("sources").canonicalize().unwrap()
    );
    assert_eq!(config.index.storage, "user-cache");
    std::fs::write(d.path().join(".kortex.toml"), "[repository]\nroot='..'").unwrap();
    assert_eq!(
        Config::load(d.path(), None).unwrap_err().code,
        "SECURITY_POLICY"
    );
    // Explicit owner-supplied configuration may intentionally relocate outside the invocation root.
    let file = d.path().join("trusted.toml");
    std::fs::write(&file, "[repository]\nroot='..'").unwrap();
    assert_eq!(
        Config::load(d.path(), Some(&file))
            .unwrap()
            .resolved_root(d.path())
            .unwrap(),
        d.path().parent().unwrap().canonicalize().unwrap()
    );
}
#[test]
fn repository_policy_cannot_activate_or_redirect_http_models_or_auth() {
    let d = tempfile::tempdir().unwrap();
    for text in [
        "[http]\nenabled=true",
        "[http]\nhost='0.0.0.0'",
        "[http]\nauth_token_env='ATTACKER_TOKEN'",
        "[http]\ncors_origins=['http://evil.test']",
        "[http]\nallowed_repo_ids=['other']",
        "[search]\nembeddings='local'",
        "[search]\nmodel_path='/tmp/attacker'",
        "[http]\nauth_token='never persist me'",
    ] {
        std::fs::write(d.path().join(".kortex.toml"), text).unwrap();
        assert!(Config::load(d.path(), None).is_err(), "accepted {text}");
    }
}
#[test]
fn actual_explicit_local_artifact_and_trusted_http_config_are_accepted() {
    let d = tempfile::tempdir().unwrap();
    let model = d.path().join("local-model.json");
    std::fs::write(&model, b"explicit test artifact").unwrap();
    let model = model.canonicalize().unwrap();
    let text = format!(
        "[search]\nembeddings='local'\nmodel_path={:?}\nmodel_version='fixture-1'\nmodel_checksum='{}'\nkeyword_weight=0.7\nvector_weight=0.3\n[http]\nenabled=true\nhost='0.0.0.0'\nauth_token_env='OWNER_HTTP_TOKEN'\ncors_origins=['https://explorer.test']\nallowed_repo_ids=['repo-id']\n",
        model.to_str().unwrap(),
        kortex_core::hash(b"explicit test artifact")
    );
    let file = d.path().join("trusted.toml");
    std::fs::write(&file, text).unwrap();
    let config = Config::load(d.path(), Some(&file)).unwrap();
    assert_eq!(config.search.model_version, "fixture-1");
    assert_eq!(config.http.auth_token_env, "OWNER_HTTP_TOKEN");
    assert_eq!(config.http.allowed_repo_ids, vec!["repo-id"]);
    let raw = serde_json::to_string(&config).unwrap();
    assert!(!raw.contains("Bearer "));
    let mut bad = config.clone();
    bad.search.embeddings = "remote".into();
    assert_eq!(bad.validate().unwrap_err().code, "SECURITY_POLICY");
    bad = config.clone();
    bad.http.auth_token_env.clear();
    assert_eq!(bad.validate().unwrap_err().code, "SECURITY_POLICY");
    bad = config.clone();
    bad.http.cors_origins = vec!["https://*.test".into()];
    assert!(bad.validate().is_err());
    bad = config.clone();
    bad.search.model_path.clear();
    assert!(bad.validate().is_err());
    bad = config.clone();
    bad.search.keyword_weight = f64::NAN;
    assert!(bad.validate().is_err());
    bad = config;
    bad.http.allowed_repo_ids = vec!["../repo".into()];
    assert!(bad.validate().is_err());
}
#[test]
fn malformed_and_oversized_config_fail_without_execution() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join(".kortex.toml");
    for text in [
        "[http]\nauth_token='SECRET'",
        "schema_version=99",
        "[search]\nembeddings='remote'",
        "[[[[",
        "[http]\nhost='localhost'",
        "[search]\nkeyword_weight=-1",
        "[repository]\nroot=''",
        "[search]\ncache_path='../outside'",
    ] {
        std::fs::write(&path, text).unwrap();
        assert!(Config::load(d.path(), None).is_err(), "accepted {text}");
    }
    std::fs::write(&path, "#".repeat(1024 * 1024 + 1)).unwrap();
    assert!(Config::load(d.path(), None).is_err());
}
#[test]
fn environment_policy_child() {
    let Some(root) = std::env::var_os("KORTEX_TEST_CONFIG_ROOT") else {
        return;
    };
    let config = Config::load(Path::new(&root), None).unwrap();
    assert!(config.http.enabled);
    assert_eq!(config.http.host, "0.0.0.0");
    assert_eq!(config.http.auth_token_env, "OWNER_HTTP_TOKEN");
    let serialized = serde_json::to_string(&config).unwrap();
    assert!(!serialized.contains("secret-from-environment"));
}
#[test]
fn trusted_environment_activates_http_without_persisting_secret_values() {
    let d = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "readiness_tests::environment_policy_child",
            "--nocapture",
        ])
        .env("KORTEX_TEST_CONFIG_ROOT", d.path())
        .env("XDG_CONFIG_HOME", d.path().join("config"))
        .env("KORTEX_HTTP__ENABLED", "true")
        .env("KORTEX_HTTP__HOST", "0.0.0.0")
        .env("KORTEX_HTTP__AUTH_TOKEN_ENV", "OWNER_HTTP_TOKEN")
        .env("KORTEX_HTTP__ALLOWED_REPO_IDS", "['repo-id']")
        .env("OWNER_HTTP_TOKEN", "secret-from-environment")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
