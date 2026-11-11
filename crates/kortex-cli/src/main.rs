//! Native Kortex command line. Machine stdout contains JSON/protocol traffic only.
mod http;
mod indexing;
mod output;
use clap::{Args, CommandFactory, Parser, Subcommand};
use http::serve_http;
use kortex_analysis::{Service, error_json};
use kortex_config::Config;
use kortex_core::{Error, Result};
use kortex_storage::{Store, registry::Registry};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Parser)]
#[command(
    name = "kortex",
    version,
    about = "Local-first code intelligence",
    arg_required_else_help = true
)]
struct Cli {
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true)]
    repo: Option<String>,
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[arg(long, global = true)]
    require_fresh: bool,
    #[arg(long, global = true)]
    generation_id: Option<String>,
    #[arg(long, global = true, value_parser = clap::value_parser!(u64).range(1..=10000))]
    timeout_ms: Option<u64>,
    #[arg(long, global = true)]
    max_nodes: Option<usize>,
    #[arg(long, global = true)]
    max_response_bytes: Option<usize>,
    #[command(subcommand)]
    command: Command,
}
#[derive(Args)]
struct IndexArgs {
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Explicit version-1 workspace manifest; repository paths resolve relative to it.
    #[arg(long, conflicts_with = "path")]
    workspace: Option<PathBuf>,
    #[arg(long, conflicts_with = "incremental")]
    full: bool,
    #[arg(long)]
    incremental: bool,
    #[arg(long)]
    index_only: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    verify: bool,
    #[arg(long)]
    include_hidden: bool,
    #[arg(long)]
    follow_symlinks: bool,
    #[arg(long)]
    max_file_size: Option<u64>,
    #[arg(long)]
    max_files: Option<usize>,
    #[arg(long)]
    include: Vec<String>,
    #[arg(long)]
    exclude: Vec<String>,
    #[arg(long, value_delimiter = ',')]
    languages: Vec<String>,
}
#[derive(Subcommand)]
enum Command {
    Init {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    #[command(alias = "analyze")]
    Index(IndexArgs),
    Watch {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        debounce_ms: Option<u64>,
    },
    Status {
        path: Option<PathBuf>,
    },
    Repos {
        #[command(subcommand)]
        command: Repos,
    },
    Query {
        text: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        language: Option<String>,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        path_prefix: Option<String>,
    },
    Context {
        symbol: String,
    },
    Impact {
        symbol: String,
        #[arg(long,default_value="upstream",value_parser=["upstream","downstream","both"])]
        direction: String,
        #[arg(long, default_value_t = 3)]
        depth: usize,
    },
    Trace {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long, default_value_t = 10)]
        depth: usize,
    },
    Changes {
        #[arg(long)]
        base: Option<String>,
        #[arg(long)]
        head: Option<String>,
        #[arg(long, conflicts_with = "unstaged")]
        staged: bool,
        #[arg(long)]
        unstaged: bool,
    },
    Graph {
        symbol: String,
        #[arg(long, default_value_t = 2)]
        depth: usize,
        #[arg(long,default_value="json",value_parser=["json","dot","graphml"])]
        format: String,
    },
    /// Discover framework-evidenced routes and bounded process/test associations.
    Routes {
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long, value_parser = ["fastapi", "flask", "express"])]
        framework: Option<String>,
        #[arg(long)]
        path_prefix: Option<String>,
        #[arg(long)]
        include_processes: bool,
    },
    /// Evaluate inline validated architecture rules; proven violations exit 10.
    Check {
        /// A JSON object containing rules_version, rules and optional max_violations.
        #[arg(long)]
        rules_json: String,
    },
    Mcp {
        #[arg(long,default_value="stdio",value_parser=["stdio"])]
        transport: String,
    },
    Serve {
        /// Alias for `mcp --transport stdio`; never starts an HTTP listener.
        #[arg(long, value_parser = ["stdio"], conflicts_with_all = ["host", "port", "auth_token_env", "allow_repo", "cors_origin", "web_assets"])]
        mcp: Option<String>,
        /// Numeric IP; trusted config or 127.0.0.1 by default.
        #[arg(long)]
        host: Option<String>,
        /// Trusted config or 7337 by default.
        #[arg(long)]
        port: Option<u16>,
        #[arg(long)]
        auth_token_env: Option<String>,
        /// Explicit registered repository IDs authorized for this bearer principal.
        #[arg(long)]
        allow_repo: Vec<String>,
        /// Exact browser origins permitted to call the API (default: none).
        #[arg(long)]
        cors_origin: Vec<String>,
        /// Explicit owner-supplied prebuilt web directory, loopback-only; never downloaded.
        #[arg(long)]
        web_assets: Option<PathBuf>,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Doctor {
        #[arg(long)]
        repair: bool,
    },
    Completion {
        shell: clap_complete::Shell,
    },
    Version,
}
#[derive(Subcommand)]
enum Repos {
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    Remove {
        repo_id: String,
        #[arg(long)]
        delete_index: bool,
        #[arg(long)]
        yes: bool,
    },
}
#[derive(Subcommand)]
enum ConfigCommand {
    Show,
    Validate,
    Path,
}
fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            if e.use_stderr() && !protocol_requested() && std::env::args_os().any(|a| a == "--json")
            {
                println!(
                    "{}",
                    error_json(&Error::new("INVALID_USAGE", e.to_string(), 2))
                );
            } else {
                let _ = e.print();
            }
            std::process::exit(e.exit_code());
        }
    };
    let json = cli.json;
    let protocol = matches!(
        &cli.command,
        Command::Mcp { .. } | Command::Serve { mcp: Some(_), .. }
    );
    if let Err(e) = run(cli) {
        if json && !protocol {
            println!("{}", error_json(&e));
        } else {
            eprintln!("{e}");
        }
        std::process::exit(e.exit);
    }
}
// Even parse failures in stdio mode must not contaminate protocol stdout.
fn protocol_requested() -> bool {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let value_flags = [
            "--repo",
            "--config",
            "--generation-id",
            "--timeout-ms",
            "--max-nodes",
            "--max-response-bytes",
        ];
        if value_flags.iter().any(|flag| args[i] == *flag) {
            i += 2;
        } else if args[i] == "--json"
            || args[i] == "--require-fresh"
            || value_flags
                .iter()
                .any(|flag| args[i].to_string_lossy().starts_with(&format!("{flag}=")))
        {
            i += 1;
        } else if args[i].to_string_lossy().starts_with('-') {
            // Even an unknown global flag before the stdio command must not turn
            // its parse error into JSON on protocol stdout.
            i += 1;
        } else {
            return args[i] == "mcp"
                || (args[i] == "serve"
                    && args[i + 1..]
                        .iter()
                        .any(|a| a == "--mcp" || a.to_string_lossy().starts_with("--mcp=")));
        }
    }
    false
}
fn emit(value: Value, json_output: bool) {
    if json_output {
        println!("{value}");
    } else if let Some(summary) = output::summary(&value) {
        println!("{summary}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| "serialization failed".into())
        );
    }
}
fn run(cli: Cli) -> Result<()> {
    let output = cli.json;
    match cli.command {
        Command::Version => emit(
            json!({"schema_version":"1.0","version":env!("CARGO_PKG_VERSION"),"index_format_version":1,"release_status":"proprietary/private; no public use or distribution authorized"}),
            output,
        ),
        Command::Completion { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "kortex", &mut std::io::stdout())
        }
        Command::Init { path } => {
            let root = kortex_repo::root(&path)?;
            let config_path = cli.config.unwrap_or_else(|| root.join(".kortex.toml"));
            let config = toml::to_string_pretty(&Config::default())
                .map_err(|e| Error::new("INVALID_CONFIG", e.to_string(), 2))?;
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&config_path)?;
            file.write_all(config.as_bytes())?;
            emit(
                json!({"config_path":config_path,"note":"Source ignore files were not modified. Run kortex index."}),
                output,
            );
        }
        Command::Index(args) => {
            let (report, exit) = indexing::run(args, cli.config.as_deref())?;
            emit(report, output);
            if exit != 0 {
                std::process::exit(exit);
            }
        }
        Command::Watch { path, debounce_ms } => {
            let base = kortex_repo::root(&path)?;
            let config = Config::load(&base, cli.config.as_deref())?;
            let root = config.resolved_root(&base)?;
            let debounce_ms = debounce_ms.unwrap_or(config.index.watch_debounce_ms);
            if !(50..=60000).contains(&debounce_ms) {
                return Err(Error::new(
                    "INVALID_CONFIG",
                    "debounce must be at least 50ms",
                    2,
                ));
            }
            let running = Arc::new(AtomicBool::new(true));
            let signal = running.clone();
            ctrlc::set_handler(move || signal.store(false, Ordering::SeqCst))
                .map_err(|e| Error::new("SIGNAL_ERROR", e.to_string(), 1))?;
            let events = kortex_repo::watch::Events::new(&root)?;
            let first =
                kortex_indexer::index_at_root(&root, &config, &kortex_indexer::Options::default())?;
            Registry::open()?.register(&first.snapshot)?;
            emit(
                json!({"watching":root,"generation_id":first.snapshot.generation_id,"strategy":"native events with reconciliation","interval_ms":debounce_ms}),
                output,
            );
            while running.load(Ordering::SeqCst) {
                events.wait(std::time::Duration::from_millis(debounce_ms))?;
                if !running.load(Ordering::SeqCst) {
                    break;
                }
                let c = Config::load(&base, cli.config.as_deref())?;
                if c.resolved_root(&base)? != root {
                    return Err(Error::new(
                        "WATCH_ROOT_CHANGED",
                        "configured root changed; restart the watcher explicitly",
                        2,
                    ));
                }
                let service = Service::open_at_root(&root, c.clone())?;
                if service.freshness()? == "stale" {
                    match kortex_indexer::index_at_root(
                        &root,
                        &c,
                        &kortex_indexer::Options {
                            verify: true,
                            ..Default::default()
                        },
                    ) {
                        Ok(r) => {
                            Registry::open()?.register(&r.snapshot)?;
                            emit(
                                json!({"generation_id":r.snapshot.generation_id,"parsed":r.parsed,"reused":r.reused}),
                                output,
                            );
                        }
                        Err(e) => eprintln!("watch: {e}; retrying on next scan"),
                    }
                }
            }
        }
        Command::Repos { command } => match command {
            Repos::List { limit, cursor } => {
                if cursor
                    .as_ref()
                    .is_some_and(|s| !s.starts_with("kr1.") || s.len() > 512)
                {
                    return Err(Error::new(
                        "INVALID_CURSOR",
                        "expected shared pinned registry cursor",
                        2,
                    ));
                }
                emit(
                    kortex_analysis::list_repos_page(&json!({"limit":limit,"cursor":cursor}))?,
                    output,
                );
            }
            Repos::Remove {
                repo_id,
                delete_index,
                yes,
            } => {
                let registry = Registry::open()?;
                let row = registry
                    .list()?
                    .into_iter()
                    .find(|r| r.0 == repo_id)
                    .ok_or_else(|| Error::new("REPO_NOT_FOUND", &repo_id, 3))?;
                if delete_index {
                    if !yes {
                        return Err(Error::new(
                            "CONFIRMATION_REQUIRED",
                            "--delete-index requires --yes",
                            8,
                        ));
                    }
                    let root = Path::new(&row.1);
                    let config = Config::load(root, cli.config.as_deref())?;
                    let store = Store::open_with_storage(root, &config.index.storage, false)?;
                    let snapshot = store.load()?;
                    if snapshot.repo_id != repo_id {
                        return Err(Error::new(
                            "INDEX_OWNER",
                            "index does not belong to registry entry",
                            8,
                        ));
                    }
                    let _lock = store.writer_lock()?;
                    let directory = store.directory.clone();
                    drop(store);
                    for name in ["index.db", "index.db-wal", "index.db-shm"] {
                        let p = directory.join(name);
                        if p.exists() {
                            if p.symlink_metadata()?.file_type().is_symlink() {
                                return Err(Error::new(
                                    "PATH_REFUSED",
                                    "index member is a symlink",
                                    8,
                                ));
                            }
                            std::fs::remove_file(p)?;
                        }
                    }
                }
                registry.remove(&repo_id)?;
                emit(
                    json!({"removed":repo_id,"index_deleted":delete_index}),
                    output,
                );
            }
        },
        Command::Config { command } => {
            let root = if let Some(r) = cli.repo.as_deref() {
                kortex_analysis::select_repo(Some(r))?
            } else {
                std::env::current_dir()?
            };
            let config = Config::load(&root, cli.config.as_deref())?;
            match command {
                ConfigCommand::Show => emit(serde_json::to_value(config)?, output),
                ConfigCommand::Validate => emit(json!({"valid":true}), output),
                ConfigCommand::Path => emit(
                    json!({"repository":cli.config.unwrap_or_else(||root.join(".kortex.toml")),"user":kortex_config::user_config_path(),"registry":Registry::path()?}),
                    output,
                ),
            }
        }
        Command::Mcp { .. } => kortex_mcp::serve(cli.repo.as_deref(), cli.config.as_deref())?,
        Command::Serve { mcp: Some(_), .. } => {
            kortex_mcp::serve(cli.repo.as_deref(), cli.config.as_deref())?
        }
        Command::Serve {
            host,
            port,
            auth_token_env,
            allow_repo,
            cors_origin,
            web_assets,
            ..
        } => {
            serve_http(
                host,
                port,
                auth_token_env,
                allow_repo,
                cors_origin,
                cli.config.as_deref(),
                web_assets.as_deref(),
            )?;
        }
        Command::Doctor { repair } => {
            let base = kortex_analysis::select_repo(cli.repo.as_deref())?;
            let c = Config::load(&base, cli.config.as_deref())?;
            let root = c.resolved_root(&base)?;
            let mut preservation = None;
            if repair {
                let report = Store::repair(&root, &c.index.storage)?;
                preservation = Some(
                    json!({"backup_directory":report.backup_directory,"preserved_files":report.preserved_files}),
                );
                let r = kortex_indexer::index_at_root(
                    &root,
                    &c,
                    &kortex_indexer::Options {
                        full: true,
                        verify: true,
                        dry_run: false,
                    },
                )?;
                Registry::open()?.register(&r.snapshot)?;
            }
            let service = Service::open_at_root(&root, c)?;
            let mut status = service.execute("status", &json!({}))?;
            if let Some(preservation) = preservation {
                status["repair"] = preservation;
            }
            emit(status, output);
        }
        command => {
            let (op, mut args, format, path) = match command {
                Command::Status { path } => ("status", json!({}), None, path),
                Command::Query {
                    text,
                    limit,
                    cursor,
                    language,
                    kind,
                    path_prefix,
                } => {
                    if cursor
                        .as_ref()
                        .is_some_and(|s| !s.starts_with("kx1.") || s.len() > 512)
                    {
                        return Err(Error::new(
                            "INVALID_CURSOR",
                            "expected shared generation-pinned cursor",
                            2,
                        ));
                    }
                    let mut args = json!({"text":text,"limit":limit});
                    for (key, value) in [
                        ("cursor", cursor),
                        ("language", language),
                        ("kind", kind),
                        ("path_prefix", path_prefix),
                    ] {
                        if let Some(value) = value {
                            args[key] = json!(value);
                        }
                    }
                    ("query", args, None, None)
                }
                Command::Context { symbol } => ("context", json!({"symbol":symbol}), None, None),
                Command::Impact {
                    symbol,
                    direction,
                    depth,
                } => (
                    "impact",
                    json!({"symbol":symbol,"direction":direction,"depth":depth}),
                    None,
                    None,
                ),
                Command::Trace { from, to, depth } => (
                    "trace",
                    json!({"from":from,"to":to,"depth":depth}),
                    None,
                    None,
                ),
                Command::Changes {
                    base,
                    head,
                    staged,
                    unstaged,
                } => {
                    let mut args = json!({"staged":staged,"unstaged":unstaged});
                    if let Some(base) = base {
                        args["base"] = json!(base);
                    }
                    if let Some(head) = head {
                        args["head"] = json!(head);
                    }
                    ("changes", args, None, None)
                }
                Command::Graph {
                    symbol,
                    depth,
                    format,
                } => (
                    "graph",
                    json!({"symbol":symbol,"depth":depth}),
                    Some(format),
                    None,
                ),
                Command::Routes {
                    limit,
                    cursor,
                    framework,
                    path_prefix,
                    include_processes,
                } => {
                    if cursor
                        .as_ref()
                        .is_some_and(|s| !s.starts_with("kx1.") || s.len() > 512)
                    {
                        return Err(Error::new(
                            "INVALID_CURSOR",
                            "expected shared generation-pinned cursor",
                            2,
                        ));
                    }
                    let mut args = json!({"limit":limit,"include_processes":include_processes});
                    for (key, value) in [
                        ("cursor", cursor),
                        ("framework", framework),
                        ("path_prefix", path_prefix),
                    ] {
                        if let Some(value) = value {
                            args[key] = json!(value);
                        }
                    }
                    ("routes", args, None, None)
                }
                Command::Check { rules_json } => {
                    if rules_json.len() > 65536 {
                        return Err(Error::new("INVALID_INPUT", "rules JSON exceeds 64 KiB", 2));
                    }
                    ("check", serde_json::from_str(&rules_json)?, None, None)
                }
                _ => return Err(Error::new("INVALID_COMMAND", "unexpected command", 2)),
            };
            if cli.require_fresh {
                args["require_fresh"] = json!(true);
            }
            if let Some(generation) = cli.generation_id {
                args["generation_id"] = json!(generation);
            }
            if let Some(timeout) = cli.timeout_ms {
                args["timeout_ms"] = json!(timeout);
            }
            if let Some(budget) = cli.max_response_bytes {
                args["max_response_bytes"] = json!(budget);
            }
            if let Some(nodes) = cli.max_nodes {
                args["max_nodes"] = json!(nodes);
            }
            let root = if let Some(path) = path {
                kortex_repo::root(&path)?
            } else {
                kortex_analysis::select_repo(cli.repo.as_deref())?
            };
            let service = Service::open_with_config(&root, cli.config.as_deref())?;
            let result = service.execute(op, &args)?;
            let check_exit = if op == "check" {
                result["data"]["exit_code"].as_i64().unwrap_or(7) as i32
            } else {
                0
            };
            match format.as_deref() {
                Some("dot") => print!("{}", kortex_analysis::dot(&result["data"])),
                Some("graphml") => print!("{}", graphml(&result["data"])),
                _ => emit(result, output),
            }
            if check_exit != 0 {
                drop(service);
                std::process::exit(check_exit);
            }
        }
    }
    Ok(())
}
fn graphml(graph: &Value) -> String {
    fn escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<graphml xmlns=\"http://graphml.graphdrawing.org/xmlns\"><key id=\"name\" for=\"node\" attr.name=\"name\" attr.type=\"string\"/><graph edgedefault=\"directed\">\n",
    );
    if let Some(nodes) = graph["nodes"].as_array() {
        for n in nodes {
            out.push_str(&format!(
                "<node id=\"{}\"><data key=\"name\">{}</data></node>\n",
                escape(n["symbol_id"].as_str().unwrap_or("")),
                escape(n["qualified_name"].as_str().unwrap_or(""))
            ));
        }
    }
    if let Some(edges) = graph["edges"].as_array() {
        for e in edges {
            if let Some(t) = e["target_id"].as_str() {
                out.push_str(&format!(
                    "<edge source=\"{}\" target=\"{}\"/>\n",
                    escape(e["source_id"].as_str().unwrap_or("")),
                    escape(t)
                ));
            }
        }
    }
    out.push_str("</graph></graphml>\n");
    out
}
