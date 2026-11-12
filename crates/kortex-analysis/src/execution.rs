//! Shared eager/lazy request, freshness and response contracts.
use crate::{
    contracts::{self, CursorContext},
    number,
};
use kortex_config::Config;
use kortex_core::{Error, RequestControl, Result, hash};
use serde_json::{Value, json};
use std::{
    path::Path,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) fn prepare(
    context: &CursorContext<'_>,
    operation: &str,
    args: &Value,
    control: &RequestControl,
) -> Result<(Instant, RequestControl)> {
    control.check()?;
    let started = Instant::now();
    contracts::validate_input(operation, args)?;
    let timeout = number(
        args,
        "timeout_ms",
        context.config.analysis.query_timeout_ms as usize,
    )? as u64;
    if timeout > context.config.analysis.query_timeout_ms {
        return Err(Error::new(
            "QUERY_LIMIT",
            "timeout exceeds repository policy",
            2,
        ));
    }
    let control = control.restrict_deadline(started + std::time::Duration::from_millis(timeout));
    if args
        .get("generation_id")
        .and_then(Value::as_str)
        .is_some_and(|id| id != context.generation_id)
    {
        return Err(Error::new(
            "GENERATION_MISMATCH",
            "requested generation is not the pinned reader generation",
            2,
        ));
    }
    Ok((started, control))
}

pub(crate) struct FreshnessHeader<'a> {
    pub resolution_hash: &'a str,
    pub config_hash: &'a str,
    pub grammar_hash: &'a str,
    pub git_head: &'a Option<String>,
}
pub(crate) fn freshness_header(
    root: &Path,
    config: &Config,
    header: FreshnessHeader<'_>,
    control: &RequestControl,
) -> Result<bool> {
    control.check()?;
    let head = kortex_repo::git_with_control(root, &["rev-parse", "--verify", "HEAD"], control)
        .ok()
        .map(|s| s.trim().to_owned());
    control.check()?;
    Ok(
        kortex_repo::inputs::resolution_fingerprint_with_control(root, control)?
            == header.resolution_hash
            && config.fingerprint()? == header.config_hash
            && header.grammar_hash == kortex_core::PARSER_VERSION
            && &head == header.git_head,
    )
}
pub(crate) fn freshness_files<'a>(
    root: &Path,
    config: &Config,
    before: impl Iterator<Item = (&'a str, &'a str)>,
    control: &RequestControl,
) -> Result<String> {
    control.check()?;
    // Discovery still reads/hashes every source; no metadata-only freshness shortcut.
    let inventory = kortex_repo::discover_with_control(root, config, control)?;
    control.check()?;
    let mut before = before;
    for file in &inventory.files {
        control.check()?;
        if before.next() != Some((file.path.as_str(), file.content_hash.as_str())) {
            return Ok("stale".into());
        }
    }
    Ok(if before.next().is_none() {
        "fresh"
    } else {
        "stale"
    }
    .into())
}

pub(crate) struct ResponseParts {
    pub started: Instant,
    pub freshness: String,
    pub indexed_partial: bool,
    pub truncated: bool,
    pub next_cursor: Option<String>,
    pub data: Value,
}
pub(crate) fn finish(
    context: &CursorContext<'_>,
    operation: &str,
    args: &Value,
    parts: ResponseParts,
    control: &RequestControl,
) -> Result<Value> {
    let ResponseParts {
        started,
        freshness,
        indexed_partial,
        mut truncated,
        next_cursor,
        mut data,
    } = parts;
    control.check()?;
    data["schema_version"] = json!("1.0");
    data["operation"] = json!(operation);
    if operation == "check" && freshness != "fresh" {
        data["complete"] = json!(false);
        data["passed"] = json!(false);
        data["partial"] = json!(true);
        if data["exit_code"] == 0 {
            data["exit_code"] = json!(7);
        }
    }
    truncated |= data["truncated"].as_bool().unwrap_or(false);
    let partial = data["partial"].as_bool().unwrap_or(false) || indexed_partial;
    let mut warnings = if freshness == "fresh" {
        vec![]
    } else {
        vec!["Index does not match repository state; citations refer to indexed generation."]
    };
    if operation == "changes"
        && let Some(items) = data["warnings"].as_array()
    {
        warnings.extend(items.iter().filter_map(Value::as_str));
    }
    let request_id = hash(format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let mut response = json!({"schema_version":"1.0","request_id":request_id,"repo_id":context.repo_id,"generation_id":context.generation_id,"index_status":freshness,"partial":partial,"truncated":truncated,"warnings":warnings,"data":data,"page":{"next_cursor":next_cursor},"timing_ms":started.elapsed().as_millis()});
    let budget = number(
        args,
        "max_response_bytes",
        context.config.mcp.max_response_bytes,
    )?;
    if budget > context.config.mcp.max_response_bytes {
        return Err(Error::new(
            "QUERY_LIMIT",
            "response budget exceeds repository policy",
            2,
        ));
    }
    while serde_json::to_vec(&response)?.len() > budget {
        if !matches!(operation, "query" | "routes" | "files")
            || response["data"]["items"]
                .as_array()
                .is_none_or(|items| items.len() <= 1)
        {
            return Err(Error::new(
                "RESPONSE_BUDGET",
                "response exceeds byte budget; narrow the query or reduce limit/depth",
                7,
            ));
        }
        let items = response["data"]["items"].as_array_mut().unwrap();
        items.pop();
        let count = items.len();
        response["page"]["next_cursor"] = json!(context.cursor(
            operation,
            args,
            context.cursor_offset(operation, args)? + count
        ));
        response["truncated"] = json!(true);
        control.check()?;
    }
    control.check()?;
    Ok(response)
}
