use kortex_config::{Config, SelectionOrigin};
#[test]
fn origin_is_retained_but_is_not_config_input_output_or_fingerprint() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let config = Config::load(&root, None).unwrap();
    assert_eq!(config.origin.as_ref().unwrap().invocation_root, root);
    assert!(config.origin.as_ref().unwrap().explicit_config.is_none());
    let before = config.fingerprint().unwrap();
    let serialized = serde_json::to_value(&config).unwrap();
    assert!(serialized.get("origin").is_none());
    let mut other = config.clone();
    other.origin = Some(SelectionOrigin {
        invocation_root: root.join("not-semantic"),
        explicit_config: Some(root.join("not-a-secret-value.toml")),
    });
    assert_eq!(other.fingerprint().unwrap(), before);
    let mut injected = serialized;
    injected["origin"] = serde_json::json!({"invocation_root":"/attacker"});
    assert!(serde_json::from_value::<Config>(injected).is_err());
    assert!(toml::from_str::<Config>("[origin]\ninvocation_root='/attacker'\n").is_err());
    assert!(
        toml::from_str::<Config>("[root_selection]\nconfigured='.'\nresolved='/attacker'\n")
            .is_err()
    );
}

#[test]
fn displayed_root_preserves_configured_spelling_without_changing_resolution_or_old_fingerprint() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let invocation = base.join("invocation");
    let settings = base.join("settings");
    std::fs::create_dir_all(invocation.join("different")).unwrap();
    std::fs::create_dir_all(settings.join("src")).unwrap();
    let explicit = settings.join("chosen.toml");
    std::fs::write(&explicit, "[repository]\nroot='src'\n").unwrap();
    let mut loaded = Config::load(&invocation, Some(&explicit)).unwrap();
    assert_eq!(loaded.repository.root, "src");
    let output = serde_json::to_value(&loaded).unwrap();
    assert_eq!(output["repository"]["root"], "src");
    assert!(output.get("root_selection").is_none());
    assert_eq!(
        loaded.resolved_root(&invocation).unwrap(),
        settings.join("src")
    );
    let mut old = loaded.clone();
    old.repository.root = settings.join("src").to_str().unwrap().into();
    old.root_selection = None;
    assert_eq!(
        loaded.fingerprint().unwrap(),
        kortex_core::hash(serde_json::to_vec(&old).unwrap())
    );
    loaded.repository.root = "different".into();
    assert_eq!(
        loaded.resolved_root(&invocation).unwrap(),
        invocation.join("different")
    );
    assert_ne!(loaded.fingerprint().unwrap(), old.fingerprint().unwrap());
}
