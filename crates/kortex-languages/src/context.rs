//! Explicit, repository-confined resolution inputs. No filesystem or build execution here.
use kortex_core::{Result, Snapshot};
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct ResolutionContext {
    pub go_module: Option<String>,
    /// Repository-relative target patterns, preserving declared fallback/ambiguity sets.
    pub ts_paths: Vec<(String, Vec<String>)>,
    pub ts_base_url: Option<String>,
}
impl ResolutionContext {
    pub fn with_tsconfig(mut self, source: &str) -> Result<Self> {
        let json: serde_json::Value = serde_json::from_str(&jsonc(source))?;
        if !json["extends"].is_null() {
            return Err(kortex_core::Error::new(
                "UNSUPPORTED_RESOLUTION_CONFIG",
                "tsconfig inheritance is not resolved; refusing to guess inherited baseUrl/path semantics",
                7,
            ));
        }
        let options = &json["compilerOptions"];
        let base = options["baseUrl"].as_str().unwrap_or(".");
        let Some(base) = super::normalized(Path::new(base)) else {
            return Ok(self);
        };
        self.ts_base_url = options["baseUrl"].as_str().map(|_| base.clone());
        if let Some(paths) = options["paths"].as_object() {
            for (pattern, targets) in paths.iter().take(1024) {
                if pattern.matches('*').count() > 1 {
                    continue;
                }
                let targets = targets
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(64)
                    .filter_map(|v| {
                        let value = v.as_str()?;
                        if value.matches('*').count() > 1 {
                            return None;
                        }
                        super::normalized(&Path::new(&base).join(value))
                    })
                    .collect();
                self.ts_paths.push((pattern.clone(), targets));
            }
        }
        Ok(self)
    }
}
/// Trusted compatibility oracle; no aggregate resource policy is enforced.
pub fn resolve_with_context(
    snapshot: &Snapshot,
    context: &ResolutionContext,
) -> Vec<kortex_core::Edge> {
    resolve_context_impl(snapshot, context, None)
        .expect("unbounded resolution has no admission failures")
}
pub fn resolve_bounded_with_context(
    snapshot: &Snapshot,
    context: &ResolutionContext,
    limits: &super::ResolutionLimits,
) -> Result<Vec<kortex_core::Edge>> {
    resolve_context_impl(snapshot, context, Some(limits))
}
fn resolve_context_impl(
    snapshot: &Snapshot,
    context: &ResolutionContext,
    limits: Option<&super::ResolutionLimits>,
) -> Result<Vec<kortex_core::Edge>> {
    let mut budget = super::bounds::Budget::new(limits);
    // No transformed semantic input means the original immutable facts are sufficient.
    // This avoids cloning the entire graph for Python/Rust/Go/P1 and plain relative JS imports.
    if (context.ts_paths.is_empty() && context.ts_base_url.is_none())
        || !snapshot
            .files
            .iter()
            .any(|f| matches!(f.language.as_str(), "javascript" | "typescript"))
    {
        return super::resolve_impl(snapshot, context.go_module.as_deref(), &mut budget);
    }
    let mut fact_bytes = 0usize;
    if let Some(limits) = limits {
        for file in &snapshot.files {
            let mut size = super::bounds::Counter::default();
            serde_json::to_writer(&mut size, file)?;
            fact_bytes = super::bounds::reserve(
                "alias workspace JSON bytes",
                fact_bytes,
                size.0,
                limits.max_fact_bytes,
            )?;
        }
    }
    // Resolution reads only repo_id and immutable parse facts, never previous resolved edges.
    let mut facts = Snapshot {
        repo_id: snapshot.repo_id.clone(),
        files: snapshot.files.clone(),
        ..Default::default()
    };
    let mut import_count = snapshot
        .files
        .iter()
        .filter(|f| !matches!(f.language.as_str(), "javascript" | "typescript"))
        .map(|f| f.imports.len())
        .sum::<usize>();
    for file in &mut facts.files {
        if !matches!(file.language.as_str(), "javascript" | "typescript") {
            continue;
        }
        let parent = Path::new(&file.path).parent().unwrap_or(Path::new(""));
        let up = "../".repeat(parent.components().count());
        let mut imports = vec![];
        let mut scopes = vec![];
        let mut spans = vec![];
        for (index, import) in file.imports.iter().enumerate() {
            let mut targets = vec![];
            if !import.module.starts_with('.') {
                // Exact mappings take precedence, then longest prefix, as TypeScript paths do.
                let best = context
                    .ts_paths
                    .iter()
                    .filter_map(|(pattern, targets)| {
                        if let Some((prefix, suffix)) = pattern.split_once('*') {
                            let value = import.module.strip_prefix(prefix)?.strip_suffix(suffix)?;
                            Some((
                                prefix.len(),
                                targets
                                    .iter()
                                    .map(|t| t.replace('*', value))
                                    .collect::<Vec<_>>(),
                            ))
                        } else if pattern == &import.module {
                            Some((usize::MAX, targets.clone()))
                        } else {
                            None
                        }
                    })
                    .max_by_key(|(rank, _)| *rank);
                if let Some((_, mapped)) = best {
                    targets = mapped;
                } else if let Some(base) = &context.ts_base_url {
                    targets.push(format!("{base}/{}", import.module));
                }
            }
            if targets.is_empty() {
                targets.push(import.module.clone());
            }
            for target in targets {
                if let Some(limits) = limits {
                    import_count = super::bounds::reserve(
                        "transformed imports",
                        import_count,
                        1,
                        limits.max_imports,
                    )?;
                    let mut size = super::bounds::Counter::default();
                    // Substitute borrowed module JSON in the original import's exact size.
                    serde_json::to_writer(&mut size, import)?;
                    let mut old_module = super::bounds::Counter::default();
                    serde_json::to_writer(&mut old_module, &import.module)?;
                    size.0 -= old_module.0;
                    serde_json::to_writer(&mut size, &target)?;
                    serde_json::to_writer(
                        &mut size,
                        file.import_scopes
                            .get(index)
                            .map(String::as_str)
                            .unwrap_or(""),
                    )?;
                    serde_json::to_writer(
                        &mut size,
                        file.import_spans.get(index).unwrap_or(&Default::default()),
                    )?;
                    let prefix = if target != import.module {
                        2 + up.len()
                    } else {
                        0
                    };
                    let payload = super::bounds::reserve(
                        "alias workspace JSON bytes",
                        size.0,
                        prefix + 3, // conservatively reserve a comma in each output array
                        limits.max_fact_bytes,
                    )?;
                    fact_bytes = super::bounds::reserve(
                        "alias workspace JSON bytes",
                        fact_bytes,
                        payload,
                        limits.max_fact_bytes,
                    )?;
                }
                let mut mapped = import.clone();
                if target != import.module {
                    mapped.module = format!("./{up}{target}");
                }
                imports.push(mapped);
                scopes.push(file.import_scopes.get(index).cloned().unwrap_or_default());
                spans.push(file.import_spans.get(index).cloned().unwrap_or_default());
            }
        }
        file.imports = imports;
        file.import_scopes = scopes;
        file.import_spans = spans;
    }
    super::resolve_impl(&facts, context.go_module.as_deref(), &mut budget)
}
// JSONC comments/trailing commas are handled with a string-aware scanner, not regular expressions.
fn jsonc(source: &str) -> String {
    let mut bytes = source.as_bytes().to_vec();
    let mut i = 0;
    let mut quoted = false;
    while i < bytes.len() {
        if quoted {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == b'"' {
                quoted = false;
            }
        } else if bytes[i] == b'"' {
            quoted = true;
        } else if bytes.get(i..i + 2) == Some(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                bytes[i] = b' ';
                i += 1;
            }
            continue;
        } else if bytes.get(i..i + 2) == Some(b"/*") {
            bytes[i] = b' ';
            bytes[i + 1] = b' ';
            i += 2;
            while i < bytes.len() {
                if bytes.get(i..i + 2) == Some(b"*/") {
                    bytes[i] = b' ';
                    bytes[i + 1] = b' ';
                    i += 2;
                    break;
                }
                if bytes[i] != b'\n' {
                    bytes[i] = b' ';
                }
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    quoted = false;
    i = 0;
    while i < bytes.len() {
        if quoted {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == b'"' {
                quoted = false;
            }
        } else if bytes[i] == b'"' {
            quoted = true;
        } else if bytes[i] == b',' {
            let next = bytes[i + 1..]
                .iter()
                .copied()
                .find(|b| !b.is_ascii_whitespace());
            if matches!(next, Some(b'}' | b']')) {
                bytes[i] = b' ';
            }
        }
        i += 1;
    }
    String::from_utf8(bytes).unwrap_or_default()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jsonc_aliases_preserve_strings_and_confine_targets() {
        let context=ResolutionContext::default().with_tsconfig(r#"{//λ
            "compilerOptions":{"baseUrl":"src","paths":{"@ok/*":["lib/*"],"@escape/*":["../../../outside/*"]},},
            "url":"https://example.invalid//not-a-comment",
        }"#).unwrap();
        assert!(
            context
                .ts_paths
                .iter()
                .any(|(p, t)| p == "@ok/*" && t == &["src/lib/*"])
        );
        assert!(
            context
                .ts_paths
                .iter()
                .find(|(p, _)| p == "@escape/*")
                .unwrap()
                .1
                .is_empty()
        );
        assert!(
            ResolutionContext::default()
                .with_tsconfig("{broken")
                .is_err()
        );
        assert!(
            ResolutionContext::default()
                .with_tsconfig(
                    r#"{"extends":"./base.json","compilerOptions":{"paths":{"x":["wrong"]}}}"#
                )
                .is_err()
        );
    }
}
