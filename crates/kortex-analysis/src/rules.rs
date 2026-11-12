//! Explicit architecture policy evaluated only against indexed dependency evidence.
use crate::Service;
use kortex_core::{Edge, Error, RequestControl, Result, Symbol};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const DEPENDENCIES: &[&str] = &[
    "CALLS",
    "IMPORTS",
    "REFERENCES",
    "DEPENDS_ON",
    "INHERITS",
    "IMPLEMENTS",
    "USES_TYPE",
    "READS",
    "WRITES",
    "TESTS",
];
fn version() -> String {
    "1.0".into()
}
fn depth() -> usize {
    1
}
fn cap() -> usize {
    100
}
fn kinds() -> Vec<String> {
    DEPENDENCIES[..7].iter().map(|s| (*s).into()).collect()
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Selector {
    pub path: String,
    #[serde(default)]
    pub kinds: Vec<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub kind: String,
    pub from: Selector,
    pub to: Selector,
    #[serde(default = "kinds")]
    pub edge_kinds: Vec<String>,
    #[serde(default = "depth")]
    pub max_depth: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    #[serde(default = "version")]
    rules_version: String,
    rules: Vec<Rule>,
    #[serde(default = "cap")]
    max_violations: usize,
}
fn invalid(message: impl Into<String>) -> Error {
    Error::new("INVALID_RULE", message, 2)
}
fn validate_selector(selector: &Selector) -> Result<()> {
    let p = &selector.path;
    if p.is_empty()
        || p.len() > 1024
        || p.starts_with('/')
        || p.contains(['\\', ':', '[', ']', '{', '}', '\0'])
        || p.split('/').any(|s| s.is_empty() || s == "." || s == "..")
        || selector.kinds.len() > 32
        || selector.kinds.iter().any(|s| {
            !matches!(
                s.as_str(),
                "repository"
                    | "directory"
                    | "file"
                    | "module"
                    | "namespace"
                    | "package"
                    | "class"
                    | "struct"
                    | "enum"
                    | "trait"
                    | "interface"
                    | "function"
                    | "method"
                    | "constructor"
                    | "field"
                    | "property"
                    | "variable"
                    | "constant"
                    | "type_alias"
                    | "import"
                    | "export"
                    | "test"
                    | "route"
                    | "process"
                    | "external_symbol"
                    | "unresolved_reference"
            )
        })
    {
        return Err(invalid(
            "selector requires a relative path glob and known symbol kinds",
        ));
    }
    Ok(())
}
// Small bounded glob matcher, O(pattern bytes * path bytes), no regex backtracking.
pub fn path_matches(pattern: &str, path: &str) -> bool {
    let p: Vec<_> = pattern.chars().collect();
    let t: Vec<_> = path.chars().collect();
    let mut row = vec![false; t.len() + 1];
    row[0] = true;
    let mut i = 0;
    while i < p.len() {
        let mut next = vec![false; t.len() + 1];
        if p[i] == '*' {
            let double = p.get(i + 1) == Some(&'*');
            // **/ may match zero complete segments, including an empty prefix.
            let slash = double && p.get(i + 2) == Some(&'/');
            next[0] = row[0];
            if slash {
                let mut active = row[0];
                for j in 1..=t.len() {
                    active |= row[j];
                    next[j] = row[j] || (t[j - 1] == '/' && active);
                }
            } else {
                for j in 1..=t.len() {
                    next[j] = row[j] || (next[j - 1] && (double || t[j - 1] != '/'));
                }
            }
            i += if slash {
                3
            } else if double {
                2
            } else {
                1
            };
        } else {
            for j in 1..=t.len() {
                next[j] = row[j - 1] && (p[i] == t[j - 1] || (p[i] == '?' && t[j - 1] != '/'));
            }
            i += 1;
        }
        row = next;
    }
    row[t.len()]
}
fn matches(selector: &Selector, symbol: &Symbol) -> bool {
    path_matches(&selector.path, &symbol.file_path)
        && (selector.kinds.is_empty() || selector.kinds.contains(&symbol.kind))
}
pub fn evaluate(service: &Service, args: &Value, control: &RequestControl) -> Result<Value> {
    control.check()?;
    let mut policy = args.clone();
    if let Some(object) = policy.as_object_mut() {
        for common in [
            "require_fresh",
            "max_response_bytes",
            "timeout_ms",
            "generation_id",
        ] {
            object.remove(common);
        }
    }
    let request: Request = serde_json::from_value(policy).map_err(|e| invalid(e.to_string()))?;
    if request.rules_version != "1.0"
        || request.rules.is_empty()
        || request.rules.len() > 100
        || !(1..=500).contains(&request.max_violations)
    {
        return Err(invalid(
            "rules_version 1.0, 1..100 rules, max_violations 1..500 required",
        ));
    }
    let mut ids = BTreeSet::new();
    for rule in &request.rules {
        if rule.id.is_empty()
            || rule.id.len() > 128
            || !ids.insert(&rule.id)
            || rule.kind != "forbid_dependency"
            || !(1..=10).contains(&rule.max_depth)
            || rule.edge_kinds.is_empty()
            || rule.edge_kinds.len() > DEPENDENCIES.len()
            || rule
                .edge_kinds
                .iter()
                .any(|s| !DEPENDENCIES.contains(&s.as_str()))
        {
            return Err(invalid(
                "unique rule IDs, forbid_dependency, depth 1..10 and dependency edge kinds required",
            ));
        }
        validate_selector(&rule.from)?;
        validate_selector(&rule.to)?;
    }
    let mut symbols = BTreeMap::new();
    for symbol in service.snapshot.symbols() {
        control.check()?;
        symbols.insert(symbol.symbol_id.as_str(), symbol);
    }
    let mut adjacency: BTreeMap<&str, Vec<&Edge>> = BTreeMap::new();
    for edge in &service.snapshot.edges {
        control.check()?;
        adjacency.entry(&edge.source_id).or_default().push(edge);
    }
    for edges in adjacency.values_mut() {
        kortex_search::sort_with_control(edges, |a, b| a.id.cmp(&b.id), control)?;
    }
    let mut violations = vec![];
    let mut uncertain = BTreeMap::new();
    let mut truncated = false;
    let mut steps = 0;
    let mut evaluated = vec![];
    for rule in &request.rules {
        let start_steps = steps;
        let start_uncertain = uncertain.len();
        let mut sources_examined = 0;
        for source in symbols.values() {
            control.check()?;
            if !matches(&rule.from, source) {
                continue;
            }
            sources_examined += 1;
            let mut queue =
                VecDeque::from([(source.symbol_id.as_str(), Vec::<&Edge>::new(), 1.0_f64)]);
            let mut seen = BTreeSet::from([source.symbol_id.as_str()]);
            while let Some((node, path, confidence)) = queue.pop_front() {
                control.check()?;
                if path.len() >= rule.max_depth {
                    continue;
                }
                for edge in adjacency
                    .get(node)
                    .into_iter()
                    .flatten()
                    .filter(|e| rule.edge_kinds.contains(&e.kind))
                {
                    control.check()?;
                    steps += 1;
                    if steps > 1_000_000 {
                        truncated = true;
                        break;
                    }
                    let target = edge
                        .target_id
                        .as_deref()
                        .and_then(|id| symbols.get(id))
                        .copied();
                    if !matches!(edge.resolution.as_str(), "exact" | "inferred")
                        || target.is_none()
                        || !edge.confidence.is_finite()
                        || edge.confidence <= 0.0
                        || edge.confidence > 1.0
                    {
                        uncertain.insert((rule.id.clone(),edge.id.clone()),json!({"rule_id":rule.id,"edge_id":edge.id,"resolution":edge.resolution,"candidates":edge.candidates,"evidence":edge.evidence}));
                        continue;
                    }
                    let target = target.unwrap();
                    let mut next = path.clone();
                    next.push(edge);
                    let confidence = confidence.min(edge.confidence);
                    if matches(&rule.to, target) {
                        if violations.len() >= request.max_violations {
                            truncated = true;
                            break;
                        }
                        violations.push(json!({"rule_id":rule.id,"source_id":source.symbol_id,"target_id":target.symbol_id,"edge_path":next.iter().map(|e| &e.id).collect::<Vec<_>>(),"edges":next,"confidence":confidence}));
                    }
                    if seen.insert(&target.symbol_id) {
                        queue.push_back((&target.symbol_id, next, confidence));
                    }
                }
                if truncated {
                    break;
                }
            }
            if truncated {
                break;
            }
        }
        evaluated.push(json!({"rule_id":rule.id,"sources_examined":sources_examined,"edges_examined":steps-start_steps,"complete":!truncated && uncertain.len()==start_uncertain}));
        if truncated {
            break;
        }
    }
    let complete = !truncated
        && uncertain.is_empty()
        && service.snapshot.diagnostics.is_empty()
        && service
            .snapshot
            .files
            .iter()
            .all(|f| f.diagnostics.is_empty());
    let exit = if !violations.is_empty() {
        10
    } else if !complete {
        7
    } else {
        0
    };
    Ok(
        json!({"schema_version":"1.0","operation":"check","rules_version":"1.0","passed":exit==0,"complete":complete,"exit_code":exit,"violations":violations,"uncertain":uncertain.into_values().collect::<Vec<_>>(),"rules_evaluated":evaluated.len(),"evaluated":evaluated,"partial":!complete,"truncated":truncated}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn glob_segments_and_zero_segment_recursive_match() {
        for (pattern, path, yes) in [
            ("src/**", "src/a/b.rs", true),
            ("src/*", "src/a/b.rs", false),
            ("**/*.rs", "a.rs", true),
            ("**/*.rs", "x/a.rs", true),
            ("a/?", "a/x", true),
            ("a/?", "a/xy", false),
            ("a/?", "a/é", true),
            ("a/?", "a/文", true),
            ("src/**/a.rs", "src/a.rs", true),
            ("src/**/a.rs", "src/z/a.rs", true),
            ("src/**/a.rs", "srcz/a.rs", false),
        ] {
            assert_eq!(path_matches(pattern, path), yes, "{pattern} {path}");
        }
        assert!(
            validate_selector(&Selector {
                path: "../escape".into(),
                kinds: vec![]
            })
            .is_err()
        );
    }
}
