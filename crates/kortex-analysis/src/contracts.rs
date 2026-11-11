//! Versioned transport-neutral contracts and generation-pinned continuations.
use crate::Service;
use kortex_core::{Error, Result, hash};
use serde_json::{Value, json};

fn fingerprint(service: &Service, operation: &str, args: &Value) -> String {
    let mut request = args.clone();
    if let Some(map) = request.as_object_mut() {
        for key in [
            "cursor",
            "limit",
            "max_response_bytes",
            "timeout_ms",
            "require_fresh",
        ] {
            map.remove(key);
        }
    }
    hash(
        serde_json::to_vec(&json!([
            service.snapshot.repo_id,
            service.snapshot.generation_id,
            operation,
            request
        ]))
        .unwrap_or_default(),
    )
}
pub fn cursor(service: &Service, operation: &str, args: &Value, offset: usize) -> String {
    format!("kx1.{}.{offset}", fingerprint(service, operation, args))
}
pub fn cursor_offset(service: &Service, operation: &str, args: &Value) -> Result<usize> {
    let invalid = || {
        Error::new(
            "INVALID_CURSOR",
            "cursor must match repository, generation and request; restart pagination",
            2,
        )
    };
    let Some(value) = args.get("cursor").filter(|v| !v.is_null()) else {
        return Ok(0);
    };
    let token = value.as_str().ok_or_else(invalid)?;
    if token.len() > 512 {
        return Err(invalid());
    }
    // Compatibility adapter for existing transports. New responses never emit numeric cursors.
    let offset = if token.bytes().all(|b| b.is_ascii_digit()) && !token.is_empty() {
        token.parse::<usize>().map_err(|_| invalid())?
    } else {
        let parts = token.split('.').collect::<Vec<_>>();
        if parts.len() != 3
            || parts[0] != "kx1"
            || parts[1] != fingerprint(service, operation, args)
        {
            return Err(invalid());
        }
        parts[2].parse::<usize>().map_err(|_| invalid())?
    };
    if offset > 100_000 {
        return Err(invalid());
    }
    Ok(offset)
}
fn string() -> Value {
    json!({"type":"string","minLength":1,"maxLength":4096})
}
fn array(item: Value) -> Value {
    json!({"type":"array","items":item})
}
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required})
}
fn reference(name: &str) -> Value {
    json!({"$ref":format!("#/$defs/{name}")})
}
fn nullable(schema: Value) -> Value {
    json!({"anyOf":[schema,{"type":"null"}]})
}
pub fn input_schema(operation: &str) -> Value {
    let mut properties = json!({"require_fresh":{"type":"boolean"},"generation_id":string(),"max_response_bytes":{"type":"integer","minimum":1024,"maximum":16777216},"timeout_ms":{"type":"integer","minimum":1,"maximum":10000}});
    let bounds = json!({"depth":{"type":"integer","minimum":0,"maximum":10},"max_nodes":{"type":"integer","minimum":1,"maximum":10000}});
    let mut required = vec![];
    let mut add = |value: Value| {
        properties
            .as_object_mut()
            .unwrap()
            .extend(value.as_object().unwrap().clone());
    };
    match operation {
        "list_repos" => {
            add(
                json!({"limit":{"type":"integer","minimum":1,"maximum":500},"cursor":nullable(string())}),
            );
            properties.as_object_mut().unwrap().remove("generation_id");
            properties.as_object_mut().unwrap().remove("require_fresh");
        }
        "status" => {}
        "files" => {
            add(
                json!({"path_prefix":{"type":"string","maxLength":4096},"limit":{"type":"integer","minimum":1,"maximum":500},"cursor":nullable(string())}),
            );
        }
        "query" => {
            add(
                json!({"text":string(),"limit":{"type":"integer","minimum":1,"maximum":500},"cursor":nullable(string()),"language":string(),"kind":string(),"path_prefix":{"type":"string","maxLength":4096}}),
            );
            required.push("text");
        }
        "context" => {
            add(json!({"symbol":string()}));
            required.push("symbol");
        }
        "impact" => {
            add(
                json!({"symbol":string(),"direction":{"type":"string","enum":["upstream","downstream","both"]}}),
            );
            add(bounds);
            required.push("symbol");
        }
        "trace" => {
            add(json!({"from":string(),"to":string()}));
            add(bounds);
            required.extend(["from", "to"]);
        }
        "graph" => {
            add(json!({"symbol":string()}));
            add(bounds);
            required.push("symbol");
        }
        "changes" => {
            add(
                json!({"base":{"type":"string","minLength":1,"maxLength":1024},"head":{"type":"string","minLength":1,"maxLength":1024},"staged":{"type":"boolean"},"unstaged":{"type":"boolean"}}),
            );
            add(bounds);
            add(json!({"max_nodes":{"type":"integer","minimum":0,"maximum":10000}}));
        }
        "routes" => {
            add(
                json!({"limit":{"type":"integer","minimum":1,"maximum":500},"cursor":nullable(string()),"framework":{"type":"string","enum":["fastapi","flask","express"]},"path_prefix":{"type":"string","maxLength":4096},"include_processes":{"type":"boolean"}}),
            );
            add(bounds);
        }
        "check" => {
            let selector = json!({"type":"object","additionalProperties":false,"required":["path"],"properties":{"path":{"type":"string","minLength":1,"maxLength":1024},"kinds":{"type":"array","maxItems":32,"items":string()}}});
            let rule = json!({"type":"object","additionalProperties":false,"required":["id","kind","from","to"],"properties":{"id":{"type":"string","minLength":1,"maxLength":128},"kind":{"const":"forbid_dependency","type":"string"},"from":selector,"to":selector,"edge_kinds":{"type":"array","minItems":1,"maxItems":10,"items":{"type":"string","enum":["CALLS","IMPORTS","REFERENCES","DEPENDS_ON","INHERITS","IMPLEMENTS","USES_TYPE","READS","WRITES","TESTS"]}},"max_depth":{"type":"integer","minimum":1,"maximum":10}}});
            add(
                json!({"rules_version":{"type":"string","const":"1.0"},"rules":{"type":"array","minItems":1,"maxItems":100,"items":rule},"max_violations":{"type":"integer","minimum":1,"maximum":500}}),
            );
            required.push("rules");
        }
        _ => return json!({"$schema":"https://json-schema.org/draft/2020-12/schema","not":{}}),
    }
    json!({"$schema":"https://json-schema.org/draft/2020-12/schema","$id":format!("https://kortex.local/schemas/1.0/{operation}-input.json"),"type":"object","additionalProperties":false,"properties":properties,"required":required})
}
fn validate(schema: &Value, value: &Value, path: &str) -> Result<()> {
    let fail = || {
        Error::new(
            "INVALID_INPUT",
            format!("{path}: value does not satisfy operation input schema"),
            2,
        )
    };
    if let Some(options) = schema["anyOf"].as_array() {
        return if options.iter().any(|s| validate(s, value, path).is_ok()) {
            Ok(())
        } else {
            Err(fail())
        };
    }
    if schema.get("const").is_some_and(|v| v != value)
        || schema["enum"]
            .as_array()
            .is_some_and(|a| !a.contains(value))
    {
        return Err(fail());
    }
    match schema["type"].as_str() {
        Some("object") => {
            let map = value.as_object().ok_or_else(fail)?;
            for key in schema["required"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                if !map.contains_key(key) {
                    return Err(fail());
                }
            }
            for (key, value) in map {
                if let Some(property) = schema["properties"].get(key) {
                    validate(property, value, &format!("{path}.{key}"))?;
                } else if schema["additionalProperties"] == false {
                    return Err(fail());
                }
            }
        }
        Some("string") => {
            let s = value.as_str().ok_or_else(fail)?;
            let len = s.chars().count() as u64;
            if schema["minLength"].as_u64().is_some_and(|n| len < n)
                || schema["maxLength"].as_u64().is_some_and(|n| len > n)
            {
                return Err(fail());
            }
        }
        Some("integer") => {
            let n = value.as_u64().ok_or_else(fail)?;
            if schema["minimum"].as_u64().is_some_and(|m| n < m)
                || schema["maximum"].as_u64().is_some_and(|m| n > m)
            {
                return Err(fail());
            }
        }
        Some("boolean") if !value.is_boolean() => return Err(fail()),
        Some("null") if !value.is_null() => return Err(fail()),
        Some("array") => {
            let items = value.as_array().ok_or_else(fail)?;
            if schema["minItems"]
                .as_u64()
                .is_some_and(|m| items.len() < (m as usize))
                || schema["maxItems"]
                    .as_u64()
                    .is_some_and(|m| items.len() > (m as usize))
            {
                return Err(fail());
            }
            for item in items {
                validate(&schema["items"], item, path)?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub fn validate_input(operation: &str, args: &Value) -> Result<()> {
    let schema = input_schema(operation);
    if schema.get("not").is_some() {
        return Err(Error::new("UNKNOWN_OPERATION", operation, 2));
    }
    validate(&schema, args, operation)
}
pub fn response_schema(operation: &str) -> Value {
    let symbols = array(reference("symbol"));
    let edges = array(reference("edge"));
    let data = match operation {
        "files" => object(
            json!({"items":array(object(json!({"path":string(),"language":string(),"size_bytes":{"type":"integer","minimum":0},"content_hash":string()}),&["path","language","size_bytes","content_hash"]))}),
            &["items"],
        ),
        "list_repos" => object(
            json!({"items":array(object(json!({"repo_id":string(),"root":string(),"generation_id":string(),"index_status":{"enum":["fresh","stale","unavailable"]}}),&["repo_id","root","generation_id","index_status"]))}),
            &["items"],
        ),
        "query" => object(
            json!({"items":array(object(json!({"symbol":reference("symbol"),"score":{"type":"number"},"why_matched":array(string())}),&["symbol","score","why_matched"]))}),
            &["items"],
        ),
        "status" => object(
            json!({"root":string(),"files":{"type":"integer"},"symbols":{"type":"integer"},"edges":{"type":"integer"},"git_head":nullable(string()),"diagnostics":array(string()),"parser_diagnostics":array(string()),"integrity":string()}),
            &[
                "root",
                "files",
                "symbols",
                "edges",
                "git_head",
                "diagnostics",
                "parser_diagnostics",
                "integrity",
            ],
        ),
        "context" => object(
            json!({"symbol":reference("symbol"),"owner":nullable(reference("symbol")),"callers":edges,"callees":edges,"tests":symbols,"references":edges,"types":edges,"test_associations":array(reference("reach"))}),
            &[
                "symbol",
                "owner",
                "callers",
                "callees",
                "tests",
                "references",
                "types",
                "test_associations",
            ],
        ),
        "impact" => object(
            json!({"target":reference("symbol"),"risk":{"enum":["low","medium","high"]},"risk_is_heuristic":{"const":true},"direct":{"type":"integer"},"indirect":{"type":"integer"},"traversal":reference("traversal"),"symbols":symbols,"likely_tests":symbols,"likely_test_associations":array(object(json!({"symbol_id":string(),"depth":{"type":"integer"},"confidence":{"type":"number"},"edge_path":array(string()),"test_evidence":{"type":"object"}}),&["symbol_id","depth","confidence","edge_path","test_evidence"]))}),
            &[
                "target",
                "risk",
                "risk_is_heuristic",
                "direct",
                "indirect",
                "traversal",
                "symbols",
                "likely_tests",
            ],
        ),
        "trace" => object(
            json!({"from":reference("symbol"),"to":reference("symbol"),"found":{"type":"boolean"},"path":nullable(edges),"search_bounded":{"const":true}}),
            &["from", "to", "found", "path", "search_bounded"],
        ),
        "graph" => object(json!({"nodes":symbols,"edges":edges}), &["nodes", "edges"]),
        "changes" => object(
            json!({"index_head":nullable(string()),"diff_base":nullable(string()),"diff_head":nullable(string()),"resolved_diff_base":nullable(json!({"type":"string"})),"resolved_diff_head":nullable(json!({"type":"string"})),"diff_base_kind":string(),"diff_head_kind":string(),"staged":{"type":"boolean"},"unstaged":{"type":"boolean"},"changes":array(reference("change")),"changed_symbols":symbols,"affected_symbols":symbols,"likely_tests":symbols,"partial":{"type":"boolean"},"truncated":{"type":"boolean"},"warnings":array(string())}),
            &[
                "changes",
                "changed_symbols",
                "affected_symbols",
                "likely_tests",
                "partial",
                "truncated",
                "warnings",
            ],
        ),
        "routes" => object(
            json!({"items":array(reference("route")),"diagnostics":array(object(json!({"path":string(),"message":string(),"location":reference("location")}),&["path","message"])),"partial":{"type":"boolean"},"truncated":{"type":"boolean"}}),
            &["items", "diagnostics", "partial", "truncated"],
        ),
        "check" => object(
            json!({"rules_version":{"const":"1.0"},"passed":{"type":"boolean"},"complete":{"type":"boolean"},"exit_code":{"enum":[0,7,10]},"violations":array(object(json!({"rule_id":string(),"source_id":string(),"target_id":string(),"edge_path":array(string()),"edges":edges,"confidence":{"type":"number"}}),&["rule_id","source_id","target_id","edge_path","edges","confidence"])),"uncertain":array(object(json!({"rule_id":string(),"edge_id":string(),"resolution":string(),"candidates":array(string()),"evidence":string()}),&["rule_id","edge_id","resolution","candidates","evidence"])),"rules_evaluated":{"type":"integer"},"evaluated":array(object(json!({"rule_id":string(),"sources_examined":{"type":"integer"},"edges_examined":{"type":"integer"},"complete":{"type":"boolean"}}),&["rule_id","sources_examined","edges_examined","complete"])),"partial":{"type":"boolean"},"truncated":{"type":"boolean"}}),
            &[
                "rules_version",
                "passed",
                "complete",
                "exit_code",
                "violations",
                "uncertain",
                "rules_evaluated",
                "evaluated",
                "partial",
                "truncated",
            ],
        ),
        _ => return json!({"not":{}}),
    };
    let span = object(
        json!({"start_byte":{"type":"integer"},"end_byte":{"type":"integer"},"start_line":{"type":"integer"},"start_col":{"type":"integer"},"end_line":{"type":"integer"},"end_col":{"type":"integer"}}),
        &[
            "start_byte",
            "end_byte",
            "start_line",
            "start_col",
            "end_line",
            "end_col",
        ],
    );
    let symbol = object(
        json!({"symbol_id":string(),"symbol_revision_id":string(),"repo_id":string(),"kind":string(),"language":string(),"name":string(),"qualified_name":string(),"file_path":string(),"span":reference("span"),"signature":{"type":"string"},"visibility":string(),"attributes":{"type":"object"},"content_hash":string(),"doc_comment":nullable(json!({"type":"string"})),"owner_id":nullable(string())}),
        &[
            "symbol_id",
            "symbol_revision_id",
            "repo_id",
            "kind",
            "language",
            "name",
            "qualified_name",
            "file_path",
            "span",
            "signature",
            "visibility",
            "attributes",
            "content_hash",
            "doc_comment",
            "owner_id",
        ],
    );
    let edge = object(
        json!({"id":string(),"repo_id":string(),"source_id":string(),"target_id":nullable(string()),"kind":string(),"source_span":reference("span"),"resolution":{"enum":["exact","inferred","ambiguous","unresolved"]},"confidence":{"type":"number","minimum":0,"maximum":1},"resolver":string(),"evidence":{"type":"string"},"candidates":array(string())}),
        &[
            "id",
            "repo_id",
            "source_id",
            "target_id",
            "kind",
            "source_span",
            "resolution",
            "confidence",
            "resolver",
            "evidence",
            "candidates",
        ],
    );
    let reach = object(
        json!({"symbol_id":string(),"depth":{"type":"integer"},"confidence":{"type":"number"},"edge_path":array(string())}),
        &["symbol_id", "depth", "confidence", "edge_path"],
    );
    let location = object(
        json!({"path":string(),"line":{"type":"integer","minimum":1},"column":{"type":"integer","minimum":1},"start_byte":{"type":"integer"},"end_byte":{"type":"integer"}}),
        &["path", "line", "column", "start_byte", "end_byte"],
    );
    let provenance = object(
        json!({"adapter_version":string(),"source_hash":string(),"generation_id":string(),"location":reference("location"),"basis":string()}),
        &[
            "adapter_version",
            "source_hash",
            "generation_id",
            "location",
            "basis",
        ],
    );
    let process = object(
        json!({"process_id":string(),"algorithm":{"const":"bounded-call-v1"},"heuristic":{"const":true},"entry_id":string(),"items":array(reference("reach")),"edges":array(reference("edge")),"ambiguous_relationships":{"type":"integer"},"unresolved_relationships":{"type":"integer"},"partial":{"type":"boolean"},"truncated":{"type":"boolean"},"provenance":reference("provenance")}),
        &[
            "process_id",
            "algorithm",
            "heuristic",
            "entry_id",
            "items",
            "edges",
            "ambiguous_relationships",
            "unresolved_relationships",
            "partial",
            "truncated",
            "provenance",
        ],
    );
    let route = object(
        json!({"route_id":string(),"framework":{"enum":["fastapi","flask","express"]},"http_method":{"enum":["GET","POST","PUT","PATCH","DELETE","OPTIONS","HEAD"]},"path":string(),"handler_id":string(),"location":reference("location"),"confidence":{"type":"number"},"resolution":{"const":"exact"},"provenance":reference("provenance"),"consumers":array(object(json!({"symbol_id":string(),"edge_id":string(),"kind":{"const":"direct_handler_call"},"resolution":string(),"confidence":{"type":"number"},"evidence":{"type":"string"},"source_span":reference("span")}),&["symbol_id","edge_id","kind","resolution","confidence","evidence","source_span"])),"tests":array(object(json!({"symbol_id":string(),"edge_path":array(string()),"confidence":{"type":"number"}}),&["symbol_id","edge_path","confidence"])),"process":nullable(process),"partial":{"type":"boolean"},"truncated":{"type":"boolean"}}),
        &[
            "route_id",
            "framework",
            "http_method",
            "path",
            "handler_id",
            "location",
            "confidence",
            "resolution",
            "provenance",
            "consumers",
            "tests",
            "process",
            "truncated",
        ],
    );
    let change = object(
        json!({"status":string(),"old_path":nullable(string()),"path":string(),"hunks":array(object(json!({"old_start":{"type":"integer"},"old_count":{"type":"integer"},"new_start":{"type":"integer"},"new_count":{"type":"integer"}}),&["old_start","old_count","new_start","new_count"])),"old_line_ranges":array(array(json!({"type":"integer"}))),"new_line_ranges":array(array(json!({"type":"integer"}))),"line_ranges":array(array(json!({"type":"integer"}))),"indexed_sides":array(string()),"mapping_status":{"enum":["content_verified","unavailable"]},"partial":{"type":"boolean"},"indexed_file_ids":array(string()),"symbol_ids":array(string())}),
        &[
            "status",
            "old_path",
            "path",
            "hunks",
            "old_line_ranges",
            "new_line_ranges",
            "line_ranges",
            "indexed_sides",
            "mapping_status",
            "partial",
            "indexed_file_ids",
            "symbol_ids",
        ],
    );
    let mut data = data;
    data["properties"]["schema_version"] = json!({"const":"1.0"});
    data["properties"]["operation"] = json!({"const":operation});
    data["required"]
        .as_array_mut()
        .unwrap()
        .extend([json!("schema_version"), json!("operation")]);
    json!({"$schema":"https://json-schema.org/draft/2020-12/schema","$id":format!("https://kortex.local/schemas/1.0/{operation}-response.json"),"type":"object","required":["schema_version","request_id","repo_id","generation_id","index_status","partial","truncated","warnings","data","page","timing_ms"],"properties":{"schema_version":{"const":"1.0"},"request_id":string(),"repo_id":if operation=="list_repos"{nullable(string())}else{string()},"generation_id":if operation=="list_repos"{nullable(string())}else{string()},"index_status":{"enum":["fresh","stale","unavailable"]},"partial":{"type":"boolean"},"truncated":{"type":"boolean"},"warnings":array(json!({"type":"string"})),"data":data,"page":object(json!({"next_cursor":nullable(string())}),&["next_cursor"]),"timing_ms":{"type":"integer","minimum":0}},"$defs":{"symbol":symbol,"edge":edge,"span":span,"reach":reach,"location":location,"provenance":provenance,"route":route,"change":change,"traversal":object(json!({"items":array(reference("reach")),"edges":array(reference("edge")),"truncated":{"type":"boolean"},"ambiguous_relationships":{"type":"integer"},"unresolved_relationships":{"type":"integer"}}),&["items","edges","truncated","ambiguous_relationships","unresolved_relationships"])}})
}
