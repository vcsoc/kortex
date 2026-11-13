//! Admission policy only; never a semantic truncation or cache fingerprint input.
use kortex_core::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResourceLimits {
    pub max_symbols: usize,
    pub max_references: usize,
    pub max_imports: usize,
    pub max_parsed_bytes: usize,
    pub max_edges: usize,
    pub max_candidate_memberships: usize,
    pub max_edge_bytes: usize,
    pub max_file_ast_nodes: usize,
    pub max_file_ast_depth: usize,
}
impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_symbols: 500_000,
            max_references: 1_000_000,
            max_imports: 250_000,
            max_parsed_bytes: 512 * 1024 * 1024,
            max_edges: 2_000_000,
            max_candidate_memberships: 2_000_000,
            max_edge_bytes: 512 * 1024 * 1024,
            max_file_ast_nodes: 250_000,
            max_file_ast_depth: 192,
        }
    }
}
impl ResourceLimits {
    pub(crate) fn is_default(&self) -> bool {
        self == &Self::default()
    }
    pub(crate) fn validate(&self) -> Result<()> {
        let value = toml::Value::try_from(self).map_err(super::config_error)?;
        let cap = toml::Value::try_from(Self::default()).map_err(super::config_error)?;
        for (name, value) in value.as_table().expect("resource table") {
            let n = value.as_integer().unwrap_or(0);
            if n <= 0 || n > cap[name].as_integer().unwrap_or(0) {
                return Err(super::config_error(format!(
                    "index.resources.{name} must be positive and <= {}",
                    cap[name]
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, policy::validate_repository_policy};

    #[test]
    fn resource_policy_is_strict_downward_only_and_not_semantic() {
        let original = Config::default();
        let raw = serde_json::to_vec(&original).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("resources"));
        assert_eq!(original.fingerprint().unwrap(), kortex_core::hash(&raw));
        let mut lower = original.clone();
        lower.index.resources.max_symbols = 10;
        lower.validate().unwrap();
        assert_eq!(
            lower.fingerprint().unwrap(),
            original.fingerprint().unwrap()
        );
        let mut base = toml::Value::try_from(&lower).unwrap();
        base["index"]["resources"]["max_symbols"] = toml::Value::Integer(10);
        for (n, accepted) in [(9, true), (10, true), (11, false), (0, false), (-1, false)] {
            let overlay: toml::Value =
                toml::from_str(&format!("[index.resources]\nmax_symbols={n}")).unwrap();
            assert_eq!(
                validate_repository_policy(&base, &overlay).is_ok(),
                accepted
            );
        }
        lower.index.resources.max_symbols = ResourceLimits::default().max_symbols + 1;
        assert_eq!(lower.validate().unwrap_err().code, "INVALID_CONFIG");
        assert!(toml::from_str::<Config>("[index.resources]\ntypo=1").is_err());
        assert!(toml::from_str::<Config>("[index.resources]\nmax_symbols=-1").is_err());
    }

    #[test]
    fn resource_environment_child() {
        let Some(root) = std::env::var_os("KORTEX_RESOURCE_CONFIG_TEST_ROOT") else {
            return;
        };
        let config = Config::load(std::path::Path::new(&root), None).unwrap();
        assert_eq!(config.index.resources.max_symbols, 75);
        assert_eq!(config.index.resources.max_edges, 100);
    }

    #[test]
    fn trusted_environment_can_set_defaulted_resource_fields_without_global_mutation() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join(".kortex.toml"),
            "[index.resources]\nmax_symbols=50",
        )
        .unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "resources::tests::resource_environment_child",
                "--nocapture",
            ])
            .env("KORTEX_RESOURCE_CONFIG_TEST_ROOT", d.path())
            .env("XDG_CONFIG_HOME", d.path().join("user-config"))
            .env("KORTEX_INDEX__RESOURCES__MAX_SYMBOLS", "75")
            .env("KORTEX_INDEX__RESOURCES__MAX_EDGES", "100")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn partial_resource_table_loads_and_inherits_other_defaults() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join(".kortex.toml"),
            "[index.resources]\nmax_candidate_memberships=42",
        )
        .unwrap();
        let c = Config::load(d.path(), None).unwrap();
        assert_eq!(c.index.resources.max_candidate_memberships, 42);
        assert_eq!(
            c.index.resources.max_symbols,
            ResourceLimits::default().max_symbols
        );
    }
}
