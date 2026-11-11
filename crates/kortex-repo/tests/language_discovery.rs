use kortex_config::Config;
use kortex_repo::{discover, language};

#[test]
fn extended_provider_ids_and_all_extensions_match_real_discovery() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    let mappings = [
        ("java", "java"),
        ("cs", "csharp"),
        ("c", "c"),
        ("h", "c"),
        ("cc", "cpp"),
        ("cpp", "cpp"),
        ("cxx", "cpp"),
        ("hpp", "cpp"),
        ("hh", "cpp"),
        ("hxx", "cpp"),
        ("rb", "ruby"),
        ("rake", "ruby"),
        ("gemspec", "ruby"),
        ("php", "php"),
        ("phtml", "php"),
    ];
    for (extension, provider) in mappings {
        let name = format!("source.{extension}");
        assert_eq!(language(&name), Some(provider));
        std::fs::write(root.join(&name), "fixture source data; never executed").unwrap();
    }
    for name in [
        "README",
        "source.txt",
        "source.class",
        "source.dll",
        "source.exe",
    ] {
        assert_eq!(language(name), None, "{name}");
        std::fs::write(root.join(name), "not source").unwrap();
    }
    let inventory = discover(&root, &Config::default()).unwrap();
    assert_eq!(inventory.files.len(), mappings.len());
    for (extension, provider) in mappings {
        let mut config = Config::default();
        config.repository.languages = vec![provider.into()];
        let filtered = discover(&root, &config).unwrap();
        assert!(
            filtered
                .files
                .iter()
                .any(|file| file.path == format!("source.{extension}"))
        );
        assert!(
            filtered
                .files
                .iter()
                .all(|file| language(&file.path) == Some(provider))
        );
    }
}
