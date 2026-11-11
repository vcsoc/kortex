//! Human summaries are presentation only; --json always retains the full Service envelope.
use serde_json::Value;
fn text(v: &Value) -> &str {
    v.as_str().unwrap_or("unknown")
}
fn count(v: &Value) -> usize {
    v.as_array().map_or(0, Vec::len)
}
fn location(symbol: &Value) -> String {
    format!(
        "{}:{}",
        text(&symbol["file_path"]),
        symbol["span"]["start_line"]
    )
}
pub(crate) fn summary(value: &Value) -> Option<String> {
    let data = &value["data"];
    let operation = data["operation"].as_str()?;
    let mut lines = vec![format!(
        "Repository: {} | Index: {} | Generation: {}",
        text(&value["repo_id"]),
        text(&value["index_status"]),
        text(&value["generation_id"])
    )];
    match operation {
        "status" => lines.push(format!(
            "{} files | {} symbols | {} edges | Integrity: {}",
            data["files"],
            data["symbols"],
            data["edges"],
            text(&data["integrity"])
        )),
        "query" => {
            lines.push(format!("{} matching symbols", count(&data["items"])));
            for hit in data["items"].as_array().into_iter().flatten() {
                lines.push(format!(
                    "{}  ({})",
                    text(&hit["symbol"]["qualified_name"]),
                    location(&hit["symbol"])
                ));
            }
        }
        "context" => {
            lines.push(format!(
                "{}  ({})",
                text(&data["symbol"]["qualified_name"]),
                location(&data["symbol"])
            ));
            lines.push(format!(
                "{} callers | {} callees | {} references | {} related tests",
                count(&data["callers"]),
                count(&data["callees"]),
                count(&data["references"]),
                count(&data["tests"])
            ));
        }
        "impact" => {
            lines.push(format!(
                "Target: {}  ({})",
                text(&data["target"]["qualified_name"]),
                location(&data["target"])
            ));
            lines.push(format!(
                "{} direct | {} indirect | {} likely tests | Risk: {} (heuristic)",
                data["direct"],
                data["indirect"],
                count(&data["likely_tests"]),
                text(&data["risk"])
            ));
        }
        "trace" => {
            lines.push(format!(
                "{} → {}",
                text(&data["from"]["qualified_name"]),
                text(&data["to"]["qualified_name"])
            ));
            lines.push(if data["found"] == true {
                format!("Evidence-backed path: {} edges", count(&data["path"]))
            } else {
                "No path found within the configured bounds".into()
            });
        }
        "changes" => {
            lines.push(format!(
                "{} changed files | {} changed symbols | {} affected symbols | {} likely tests",
                count(&data["changes"]),
                count(&data["changed_symbols"]),
                count(&data["affected_symbols"]),
                count(&data["likely_tests"])
            ));
            for change in data["changes"].as_array().into_iter().flatten().take(20) {
                lines.push(format!(
                    "{} {}",
                    text(&change["status"]),
                    text(&change["path"])
                ));
            }
        }
        "routes" => {
            lines.push(format!(
                "{} framework-evidenced routes",
                count(&data["items"])
            ));
            for route in data["items"].as_array().into_iter().flatten() {
                lines.push(format!(
                    "{} {}  ({}:{})",
                    text(&route["http_method"]),
                    text(&route["path"]),
                    text(&route["location"]["path"]),
                    route["location"]["line"]
                ));
            }
        }
        "check" => {
            lines.push(format!(
                "Architecture: {} | {} violations | {} uncertain edges | Complete: {}",
                if data["passed"] == true {
                    "passed"
                } else {
                    "not passed"
                },
                count(&data["violations"]),
                count(&data["uncertain"]),
                data["complete"]
            ));
            for violation in data["violations"].as_array().into_iter().flatten().take(20) {
                lines.push(format!(
                    "Rule {}: {} dependency edges; confidence {}",
                    text(&violation["rule_id"]),
                    count(&violation["edges"]),
                    violation["confidence"]
                ));
            }
        }
        _ => return None,
    }
    if value["partial"] == true {
        lines.push("Partial evidence: inspect diagnostics before relying on this result.".into());
    }
    if value["truncated"] == true {
        lines.push("Result truncated by configured bounds.".into());
    }
    if let Some(cursor) = value["page"]["next_cursor"].as_str() {
        lines.push(format!("Next cursor: {cursor}"));
    }
    for warning in value["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        lines.push(format!("Warning: {warning}"));
    }
    lines.push("Use --json for full structured evidence and citations.".into());
    // Indexed filenames/comments are untrusted; do not let terminal escape sequences execute.
    Some(
        lines
            .into_iter()
            .map(|line| {
                line.chars()
                    .map(|c| {
                        if c.is_control() {
                            format!("\\u{{{:x}}}", c as u32)
                        } else {
                            c.to_string()
                        }
                    })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n"),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn human_query_keeps_evidence_and_truncation() {
        let v = json!({"repo_id":"repo","generation_id":"gen","index_status":"stale","partial":true,"truncated":true,"data":{"operation":"query","items":[{"symbol":{"qualified_name":"a::b","file_path":"a.rs","span":{"start_line":42}}}]},"page":{"next_cursor":"opaque"},"warnings":["stale"]});
        let s = summary(&v).unwrap();
        for expected in [
            "a::b",
            "a.rs:42",
            "Partial evidence",
            "truncated",
            "opaque",
            "stale",
            "--json",
        ] {
            assert!(s.contains(expected));
        }
        assert!(summary(&json!({"data":{"operation":"graph"}})).is_none());
    }
}
