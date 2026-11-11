//! Live dispatch and OpenAPI share this route table and application-owned schemas.
use kortex_core::{Error, Result};
use serde_json::{Value, json};

pub(crate) const ROUTES: &[(&str, &str)] = &[
    ("/healthz", "healthz"),
    ("/openapi.json", "openapi"),
    ("/repos", "repos"),
    ("/repos/{id}/status", "status"),
    ("/repos/{id}/files", "files"),
    ("/repos/{id}/query", "query"),
    ("/repos/{id}/symbols/{symbol}", "context"),
    ("/repos/{id}/impact", "impact"),
    ("/repos/{id}/trace", "trace"),
    ("/repos/{id}/changes", "changes"),
    ("/repos/{id}/graph", "graph"),
    ("/repos/{id}/routes", "routes"),
    ("/repos/{id}/check", "check"),
];

pub fn input_schema(operation: &str) -> Value {
    if !matches!(operation, "healthz" | "openapi" | "repos") {
        let mut schema = kortex_analysis::input_schema(operation);
        // HTTP tightens application bounds; domain semantics remain application-owned.
        if schema["properties"].get("max_nodes").is_some() {
            schema["properties"]["max_nodes"]["maximum"] = json!(500);
        }
        if let Some(cursor) = schema["properties"].get_mut("cursor") {
            cursor["description"] = json!(
                "Only shared kx1 generation/request-pinned cursors accepted; legacy numeric offsets refused."
            );
        }
        // Context's symbol is supplied by the path, never a competing body/query field.
        if operation == "context" {
            schema["properties"]
                .as_object_mut()
                .unwrap()
                .remove("symbol");
            schema["required"]
                .as_array_mut()
                .unwrap()
                .retain(|v| v != "symbol");
        }
        return schema;
    }
    let properties = if operation == "repos" {
        json!({"limit":{"type":"integer","minimum":1,"maximum":500},"cursor":{"type":"string","minLength":1,"maxLength":96}})
    } else {
        json!({})
    };
    json!({"type":"object","properties":properties,"required":[],"additionalProperties":false})
}

pub(crate) fn validate(operation: &str, value: &Value) -> Result<()> {
    // Service::execute validates all application requests against the exact shared schema,
    // including recursive inline architecture rules. This validates transport-only inputs.
    let schema = input_schema(operation);
    let object = value
        .as_object()
        .ok_or_else(|| Error::new("INVALID_INPUT", "expected an object", 2))?;
    for (k, v) in object {
        let rule = schema["properties"]
            .get(k)
            .ok_or_else(|| Error::new("INVALID_INPUT", format!("unknown argument {k}"), 2))?;
        if matches!(operation, "healthz" | "openapi" | "repos") {
            let valid = match rule["type"].as_str() {
                Some("integer") => v.as_u64().is_some_and(|n| (1..=500).contains(&n)),
                Some("string") => v.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 96),
                _ => false,
            };
            if !valid {
                return Err(Error::new(
                    "INVALID_INPUT",
                    "invalid transport arguments",
                    2,
                ));
            }
        }
    }
    Ok(())
}
fn relocate_refs(value: &mut Value, prefix: &str) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                if k == "$ref" {
                    if let Some(name) = v.as_str().and_then(|s| s.strip_prefix("#/$defs/")) {
                        *v = json!(format!("{prefix}/$defs/{name}"));
                    }
                } else {
                    relocate_refs(v, prefix);
                }
            }
        }
        Value::Array(items) => {
            for v in items {
                relocate_refs(v, prefix);
            }
        }
        _ => {}
    }
}
pub fn openapi() -> Value {
    let mut paths = json!({});
    let mut schemas = json!({});
    for (path, operation) in ROUTES {
        let mut parameters = Vec::new();
        for name in ["id", "symbol"] {
            if path.contains(&format!("{{{name}}}")) {
                parameters.push(json!({"name":name,"in":"path","required":true,"schema":{"type":"string","pattern":"^[0-9a-f]{64}$"}}));
            }
        }
        let input = input_schema(operation);
        for (name, schema) in input["properties"].as_object().into_iter().flatten() {
            // Inline rules are JSON-body only; browsers must POST check requests.
            parameters.push(json!({"name":name,"in":"query","required":input["required"].as_array().is_some_and(|r|r.contains(&json!(name))),"schema":schema}));
        }
        let response = if matches!(*operation, "healthz" | "repos") {
            json!({"type":"object","required":["schema_version","request_id","data","partial","truncated"],"properties":{"schema_version":{"const":"1.0"},"request_id":{"type":"string"},"data":{"type":"object"},"partial":{"type":"boolean"},"truncated":{"type":"boolean"}}})
        } else if *operation == "openapi" {
            json!({"type":"object","required":["openapi","info","paths","components"]})
        } else {
            let name = format!("{operation}Response");
            let pointer = format!("#/components/schemas/{name}");
            let mut schema = kortex_analysis::response_schema(operation);
            relocate_refs(&mut schema, &pointer);
            schemas[&name] = schema;
            json!({"$ref":pointer})
        };
        let mut handler = json!({"operationId":operation,"parameters":parameters,"security":[{"bearerAuth":[]}],"responses":{
            "200":{"description":"Versioned application result; generation, partial and truncated describe evidence limitations.","headers":{"x-request-id":{"schema":{"type":"string"}}},"content":{"application/json":{"schema":response}}},
            "400":{"$ref":"#/components/responses/Error"},"401":{"$ref":"#/components/responses/Error"},"403":{"$ref":"#/components/responses/Error"},"404":{"$ref":"#/components/responses/Error"},"405":{"$ref":"#/components/responses/Error"},"413":{"$ref":"#/components/responses/Error"},"422":{"$ref":"#/components/responses/Error"},"429":{"$ref":"#/components/responses/Error"},"500":{"$ref":"#/components/responses/Error"},"504":{"$ref":"#/components/responses/Error"}
        }});
        let full = format!("/api/v1{path}");
        if *operation != "check" {
            paths[&full]["get"] = handler.clone();
        }
        if !matches!(
            *operation,
            "healthz" | "openapi" | "repos" | "status" | "context"
        ) {
            handler["operationId"] = json!(format!("{operation}Post"));
            handler["parameters"] = json!(
                parameters
                    .into_iter()
                    .filter(|p| p["in"] == "path")
                    .collect::<Vec<_>>()
            );
            handler["requestBody"] =
                json!({"required":true,"content":{"application/json":{"schema":input}}});
            paths[&full]["post"] = handler;
        }
    }
    let error_schema = json!({"type":"object","required":["schema_version","request_id","error"],"properties":{"schema_version":{"const":"1.0"},"request_id":{"type":"string"},"error":{"type":"object","required":["code","message","exit_code"],"properties":{"code":{"type":"string"},"message":{"type":"string"},"exit_code":{"type":"integer"}}}}});
    json!({"openapi":"3.1.0","info":{"title":"Kortex read-only API","version":"1.0"},"paths":paths,"components":{"schemas":schemas,"securitySchemes":{"bearerAuth":{"type":"http","scheme":"bearer"}},"responses":{"Error":{"description":"Structured error; no unauthorized repository metadata.","content":{"application/json":{"schema":error_schema}}}}}})
}
