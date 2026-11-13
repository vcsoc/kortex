//! Source-exact global selection from separately authorized operator files.
use super::*;
use crate::{AuthorizedPolicy, PolicyCategory, PolicyEvidence};
use regex_automata::{meta::Regex, util::syntax};
use std::sync::OnceLock;

/// Trusted startup inputs, never repository configuration or deserialized grants.
/// `selectors` is the caller's ordered GLOBAL/HOME/XDG/SYSTEM plan. An absent
/// assignment continues that order; a selected-but-missing file does NOT fall back.
/// Caller must capture the complete operator environment, canonical CWD and home.
/// No environment lookup, automatic grant creation, includes or Git subprocesses.
pub struct OperatorGlobalPlan {
    pub source: Arc<OperatorPolicyScope>,
    pub cwd: PathBuf,
    pub home: Option<PathBuf>,
    pub selectors: Vec<OperatorPolicyFile>,
    pub fallback: Option<OperatorPolicyFile>,
    pub targets: Vec<Arc<OperatorPolicyScope>>,
}

pub struct OperatorGlobalAcquisition {
    pub policy: Option<AuthorizedPolicy>,
    pub observations: Vec<PolicyEvidence>,
    pub digest: String,
}

fn resource() -> Error {
    Error::new(
        "RESOURCE_LIMIT",
        "operator policy acquisition exceeds bounded admission",
        9,
    )
}

fn path_ok(path: &Path) -> bool {
    path.is_absolute()
        && path.to_str().is_some()
        && path.as_os_str().as_encoded_bytes().len() <= MAX_PATH_BYTES
        && !path.as_os_str().as_encoded_bytes().contains(&0)
        && path.components().all(|c| {
            matches!(
                c,
                Component::Prefix(_) | Component::RootDir | Component::Normal(_)
            )
        })
}

// This deliberately preserves ignore 0.4.34's first-assignment byte regex,
// including non-section-aware matching and its optional-quote/token semantics.
fn selected_path(
    data: &[u8],
    home: Option<&Path>,
    control: &RequestControl,
) -> Result<Option<PathBuf>> {
    control.check()?;
    if data.len() > MAX_POLICY_BYTES as usize {
        return Err(resource());
    }
    static RE: OnceLock<std::result::Result<Regex, Box<regex_automata::meta::BuildError>>> =
        OnceLock::new();
    let re = RE
        .get_or_init(|| {
            Regex::builder()
                .configure(Regex::config().utf8_empty(false))
                .syntax(syntax::Config::new().utf8(false))
                .build(r#"(?im-u)^\s*excludesfile\s*=\s*"?\s*(\S+?)\s*"?\s*$"#)
                .map_err(Box::new)
        })
        .as_ref()
        .map_err(|_| refused("global policy selector unavailable"))?;
    let mut captures = re.create_captures();
    re.captures(data, &mut captures);
    control.check()?;
    let Some(span) = captures.get_group(1) else {
        return Ok(None);
    };
    let candidate =
        std::str::from_utf8(&data[span]).map_err(|_| refused("global policy path is not UTF-8"))?;
    if candidate.len() > MAX_PATH_BYTES {
        return Err(resource());
    }
    if candidate.contains('\0') {
        return Err(refused("global policy path contains NUL"));
    }
    let expanded = if let Some(home) = home {
        let home = home
            .to_str()
            .ok_or_else(|| refused("operator home is not UTF-8"))?;
        let count = candidate.bytes().filter(|b| *b == b'~').count();
        let length = candidate
            .len()
            .checked_sub(count)
            .and_then(|n| {
                count
                    .checked_mul(home.len())
                    .and_then(|extra| n.checked_add(extra))
            })
            .filter(|n| *n <= MAX_PATH_BYTES)
            .ok_or_else(resource)?;
        let expanded = candidate.replace('~', home);
        debug_assert_eq!(expanded.len(), length);
        expanded
    } else {
        candidate.to_owned()
    };
    control.check()?;
    Ok(Some(PathBuf::from(expanded)))
}

fn absolute_selected(path: &Path, cwd: &Path) -> Result<PathBuf> {
    let input = if path.is_absolute() {
        path.to_owned()
    } else {
        cwd.join(path)
    };
    let mut result = PathBuf::new();
    for component in input.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(refused(
                    "parent components require separately authorized exact policy paths",
                ));
            }
            _ => result.push(component.as_os_str()),
        }
    }
    if !path_ok(&result) {
        return Err(refused(
            "selected policy requires an absolute authorized path",
        ));
    }
    Ok(result)
}

fn read(
    file: &OperatorPolicyFile,
    remaining: &mut u64,
    observations: &mut Vec<PolicyEvidence>,
    cwd: &Path,
    category: Option<PolicyCategory>,
    excluded_root: Option<(i128, u128)>,
    control: &RequestControl,
) -> Result<AcquiredPolicyFile> {
    let acquired = match excluded_root {
        Some(root) => file.acquire_selector(*remaining, root, control)?,
        None => file.acquire(*remaining, control)?,
    };
    if let Some(bytes) = &acquired.bytes {
        *remaining = remaining
            .checked_sub(bytes.len() as u64)
            .ok_or_else(resource)?;
    }
    observations.push(PolicyEvidence {
        origin: acquired.origin.clone(),
        matching_base: cwd.to_owned(),
        category,
        digest: acquired.digest.clone(),
        caller_authorized: true,
    });
    Ok(acquired)
}

impl OperatorGlobalPlan {
    pub fn acquire(&self, control: &RequestControl) -> Result<OperatorGlobalAcquisition> {
        control.check()?;
        self.source.validate(control)?;
        if !path_ok(&self.cwd) || self.home.as_deref().is_some_and(|home| !path_ok(home)) {
            return Err(refused(
                "operator policy startup paths must be canonical absolute inputs",
            ));
        }
        let cwd = OperatorPolicyScope::new(&self.cwd)?;
        let source_identity = self.source.reader()?.policy_root_identity(control)?;
        if self.selectors.len() > 4 || self.targets.len() > MAX_DEPTH {
            return Err(resource());
        }
        let mut remaining = 8 * 1024 * 1024;
        let mut observations = Vec::new();
        let mut selected = None;
        for selector in &self.selectors {
            control.check()?;
            // Source-owned bytes may be rules, never an owner authority selector.
            if selector.origin().starts_with(self.source.root()) {
                return Err(refused(
                    "repository namespace cannot supply owner policy selectors",
                ));
            }
            let acquired = read(
                selector,
                &mut remaining,
                &mut observations,
                &self.cwd,
                None,
                Some(source_identity),
                control,
            )?;
            if let Some(bytes) = acquired.bytes
                && let Some(path) = selected_path(&bytes, self.home.as_deref(), control)?
            {
                let absolute = absolute_selected(&path, &self.cwd)?;
                let scope = self
                    .targets
                    .iter()
                    .filter(|scope| absolute.starts_with(scope.root()))
                    .max_by_key(|scope| scope.root().components().count())
                    .ok_or_else(|| {
                        refused("selected global policy has no independent operator scope")
                    })?;
                let relative = absolute
                    .strip_prefix(scope.root())
                    .map_err(|_| refused("policy scope mismatch"))?;
                selected = Some(scope.file(relative)?);
                break;
            }
        }
        let file = selected.as_ref().or(self.fallback.as_ref());
        let policy = if let Some(file) = file {
            let acquired = read(
                file,
                &mut remaining,
                &mut observations,
                &self.cwd,
                Some(PolicyCategory::Global),
                None,
                control,
            )?;
            acquired.bytes.map(|bytes| AuthorizedPolicy {
                origin: acquired.origin,
                category: PolicyCategory::Global,
                matching_base: self.cwd.clone(),
                bytes,
            })
        } else {
            None
        };
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"kortex.operator-global.v1\0");
        for path in [self.source.root(), self.cwd.as_path()]
            .into_iter()
            .chain(self.home.iter().map(PathBuf::as_path))
        {
            let value = path.as_os_str().as_encoded_bytes();
            hasher.update(&(value.len() as u64).to_le_bytes());
            hasher.update(value);
        }
        hasher.update(&[u8::from(self.home.is_some())]);
        for observation in &observations {
            control.check()?;
            for value in [
                observation.origin.as_os_str().as_encoded_bytes(),
                observation.digest.as_deref().unwrap_or("").as_bytes(),
            ] {
                hasher.update(&(value.len() as u64).to_le_bytes());
                hasher.update(value);
            }
            hasher.update(&[u8::from(observation.category.is_some())]);
        }
        control.check()?;
        self.source.validate(control)?;
        cwd.validate(control)?;
        Ok(OperatorGlobalAcquisition {
            policy,
            observations,
            digest: hasher.finalize().to_hex().to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, time::Duration};
    fn control() -> RequestControl {
        RequestControl::new(Duration::from_secs(30)).unwrap()
    }
    fn plan(
        owner: &Arc<OperatorPolicyScope>,
        source: &Arc<OperatorPolicyScope>,
    ) -> OperatorGlobalPlan {
        OperatorGlobalPlan {
            source: Arc::clone(source),
            cwd: source.root().to_owned(),
            home: Some(owner.root().to_owned()),
            selectors: ["env-config", "home-config", "xdg-config", "system-config"]
                .into_iter()
                .map(|s| owner.file(Path::new(s)).unwrap())
                .collect(),
            fallback: Some(owner.file(Path::new("fallback")).unwrap()),
            targets: vec![Arc::clone(owner), Arc::clone(source)],
        }
    }
    fn fixture() -> (
        tempfile::TempDir,
        Arc<OperatorPolicyScope>,
        Arc<OperatorPolicyScope>,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::create_dir(root.join("owner")).unwrap();
        fs::create_dir(root.join("source")).unwrap();
        let owner = OperatorPolicyScope::new(&root.join("owner")).unwrap();
        let source = OperatorPolicyScope::new(&root.join("source")).unwrap();
        (temp, owner, source)
    }
    #[test]
    fn byte_regex_preserves_first_assignment_quotes_case_whitespace_and_every_tilde() {
        for (input, expected) in [
            (
                b"[unrelated]\r\nEXCLUDESFILE = \"~/one~\"\r\nexcludesfile=ignored".as_slice(),
                Some("/home/one/home"),
            ),
            (
                b"# excludesfile=x\n excludesfile = a b\nexcludesfile=good".as_slice(),
                Some("good"),
            ),
            (b"include=elsewhere\n".as_slice(), None),
            (
                b"\xef\xbb\xbfexcludesfile=ignored\nexcludesfile=ok".as_slice(),
                Some("ok"),
            ),
            (b"excludesfile=\"\"".as_slice(), Some("\"")),
        ] {
            assert_eq!(
                selected_path(input, Some(Path::new("/home")), &control()).unwrap(),
                expected.map(PathBuf::from)
            );
        }
        assert!(selected_path(b"excludesfile=\xff", None, &control()).is_err());
        let huge_home = PathBuf::from(format!("/{}", "a".repeat(MAX_PATH_BYTES - 1)));
        assert_eq!(
            selected_path(b"excludesfile=~~", Some(&huge_home), &control())
                .err()
                .unwrap()
                .code,
            "RESOURCE_LIMIT"
        );
    }
    #[test]
    fn ordered_sources_relative_cwd_missing_selection_no_fallback_and_real_content_freshness() {
        let (_temp, owner, source) = fixture();
        let plan = plan(&owner, &source);
        fs::write(owner.root().join("env-config"), b"[irrelevant]\n").unwrap();
        fs::write(owner.root().join("home-config"), b"excludesfile=rules\n").unwrap();
        fs::write(
            owner.root().join("xdg-config"),
            b"excludesfile=never-open\n",
        )
        .unwrap();
        fs::write(owner.root().join("fallback"), b"fallback.py\n").unwrap();
        let missing = plan.acquire(&control()).unwrap();
        assert!(missing.policy.is_none());
        assert_eq!(missing.observations.len(), 3);
        fs::write(source.root().join("rules"), b"first.py\n").unwrap();
        let first = plan.acquire(&control()).unwrap();
        let policy = first.policy.unwrap();
        assert_eq!(policy.bytes, b"first.py\n");
        assert_eq!(policy.matching_base, source.root());
        fs::write(source.root().join("rules"), b"other.py\n").unwrap();
        let changed = plan.acquire(&control()).unwrap();
        assert_ne!(changed.digest, first.digest);
        fs::remove_file(owner.root().join("home-config")).unwrap();
        fs::remove_file(owner.root().join("xdg-config")).unwrap();
        assert_eq!(
            plan.acquire(&control()).unwrap().policy.unwrap().bytes,
            b"fallback.py\n"
        );
    }
    #[test]
    fn source_cannot_mint_owner_authority_and_ungranted_targets_are_not_opened() {
        let (_temp, owner, source) = fixture();
        let mut plan = plan(&owner, &source);
        plan.selectors = vec![source.file(Path::new("config")).unwrap()];
        fs::write(source.root().join("config"), b"excludesfile=anything\n").unwrap();
        assert_eq!(
            plan.acquire(&control()).err().unwrap().code,
            "POLICY_AUTHORIZATION_REQUIRED"
        );
        assert_eq!(source.stats().unwrap().bytes_read, 0);
        plan.selectors = vec![owner.file(Path::new("env-config")).unwrap()];
        fs::write(owner.root().join("env-config"), b"excludesfile=rules\n").unwrap();
        plan.targets = vec![Arc::clone(&owner)];
        assert_eq!(
            plan.acquire(&control()).err().unwrap().code,
            "POLICY_AUTHORIZATION_REQUIRED"
        );
        assert_eq!(source.stats().unwrap().bytes_read, 0);
    }
    #[test]
    fn source_role_uses_directory_identity_for_case_and_unicode_aliases() {
        let (temp, owner, source) = fixture();
        let parent = OperatorPolicyScope::new(&temp.path().canonicalize().unwrap()).unwrap();
        let mut plan = plan(&owner, &source);
        plan.fallback = None;
        fs::write(
            source.root().join("config"),
            format!("excludesfile={}\n", owner.root().join("rules").display()),
        )
        .unwrap();
        fs::write(owner.root().join("rules"), b"DUMMY_OWNER_RULE\n").unwrap();
        plan.selectors = vec![parent.file(Path::new("source/config")).unwrap()];
        assert_eq!(
            plan.acquire(&control()).err().unwrap().code,
            "POLICY_AUTHORIZATION_REQUIRED"
        );
        plan.selectors = vec![parent.file(Path::new("SOURCE/config")).unwrap()];
        assert!(!plan.selectors[0].origin().starts_with(source.root()));
        if parent.root().join("SOURCE/config").exists() {
            assert_eq!(
                plan.acquire(&control()).err().unwrap().code,
                "POLICY_AUTHORIZATION_REQUIRED"
            );
            eprintln!("native case-insensitive directory alias: refused before selector bytes");
        } else {
            assert!(plan.acquire(&control()).unwrap().policy.is_none());
            eprintln!(
                "case-sensitive filesystem: SOURCE is absent, not an alias; case-insensitive positive scenario unavailable"
            );
        }
        assert_eq!(parent.stats().unwrap().bytes_read, 0);
        assert_eq!(parent.stats().unwrap().file_opens, 0);
        assert_eq!(owner.stats().unwrap().bytes_read, 0);
        let unicode = parent.root().join("s\u{e9}lect");
        fs::create_dir(&unicode).unwrap();
        let unicode = OperatorPolicyScope::new(&unicode.canonicalize().unwrap()).unwrap();
        fs::write(unicode.root().join("config"), b"excludesfile=rules\n").unwrap();
        let alias = if unicode.root().file_name().unwrap() == "s\u{e9}lect" {
            "se\u{301}lect/config"
        } else {
            "s\u{e9}lect/config"
        };
        plan.source = Arc::clone(&unicode);
        plan.selectors = vec![parent.file(Path::new(alias)).unwrap()];
        assert!(!plan.selectors[0].origin().starts_with(unicode.root()));
        if plan.selectors[0].origin().exists() {
            assert_eq!(
                plan.acquire(&control()).err().unwrap().code,
                "POLICY_AUTHORIZATION_REQUIRED"
            );
            eprintln!(
                "native Unicode-normalization directory alias: refused before selector bytes"
            );
        } else {
            assert!(plan.acquire(&control()).unwrap().policy.is_none());
            eprintln!(
                "Unicode-normalization alias unavailable on this filesystem; exact identity tests remain exercised"
            );
        }
        assert_eq!(parent.stats().unwrap().bytes_read, 0);
    }

    #[test]
    fn selector_scope_below_source_root_is_denied_by_actual_ancestor_identity() {
        let (_temp, _owner, source) = fixture();
        fs::create_dir(source.root().join("inner")).unwrap();
        let inner = OperatorPolicyScope::new(&source.root().join("inner")).unwrap();
        fs::write(inner.root().join("config"), b"DUMMY_SOURCE_SELECTOR\n").unwrap();
        let identity = source
            .reader()
            .unwrap()
            .policy_root_identity(&control())
            .unwrap();
        let file = inner.file(Path::new("config")).unwrap();
        assert_eq!(
            file.acquire_selector(1024, identity, &control())
                .err()
                .unwrap()
                .code,
            "POLICY_AUTHORIZATION_REQUIRED"
        );
        assert_eq!(inner.stats().unwrap().bytes_read, 0);
        assert_eq!(inner.stats().unwrap().file_opens, 0);
    }

    #[test]
    fn parent_components_refuse_instead_of_selecting_another_effective_policy() {
        let (_temp, owner, source) = fixture();
        let plan = plan(&owner, &source);
        fs::write(source.root().join("rules"), b"DUMMY_SOURCE_RULE\n").unwrap();
        for value in ["missing/../rules", "../owner/rules"] {
            fs::write(
                owner.root().join("env-config"),
                format!("excludesfile={value}\n"),
            )
            .unwrap();
            assert_eq!(
                plan.acquire(&control()).err().unwrap().code,
                "POLICY_AUTHORIZATION_REQUIRED"
            );
            assert_eq!(source.stats().unwrap().bytes_read, 0);
        }
        #[cfg(unix)]
        {
            fs::create_dir(owner.root().join("subdir")).unwrap();
            fs::write(owner.root().join("rules"), b"DUMMY_DISTINCT_OWNER_RULE\n").unwrap();
            std::os::unix::fs::symlink(owner.root().join("subdir"), source.root().join("link"))
                .unwrap();
            assert_ne!(
                fs::read(source.root().join("link/../rules")).unwrap(),
                fs::read(source.root().join("rules")).unwrap()
            );
            fs::write(
                owner.root().join("env-config"),
                b"excludesfile=link/../rules\n",
            )
            .unwrap();
            assert_eq!(
                plan.acquire(&control()).err().unwrap().code,
                "POLICY_AUTHORIZATION_REQUIRED"
            );
            assert_eq!(source.stats().unwrap().bytes_read, 0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn selected_alias_refuses_even_when_target_has_separate_operator_scope() {
        use std::os::unix::fs::symlink;
        let (_temp, owner, source) = fixture();
        let plan = plan(&owner, &source);
        fs::write(owner.root().join("env-config"), b"excludesfile=rules\n").unwrap();
        fs::write(owner.root().join("private"), b"DUMMY_OWNER_POLICY\n").unwrap();
        symlink(owner.root().join("private"), source.root().join("rules")).unwrap();
        assert!(plan.acquire(&control()).is_err());
        assert_eq!(source.stats().unwrap().bytes_read, 0);
        assert_eq!(source.stats().unwrap().file_opens, 0);
    }
}
