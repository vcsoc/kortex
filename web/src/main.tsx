import React, { Suspense, lazy, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { Api, ApiError, exportJSON } from "./api";
import type {
  Context,
  Edge,
  Envelope,
  Graph,
  Hit,
  IndexedFile,
  Repo,
} from "./api";
const GraphView = lazy(() =>
  import("./GraphView").then((module) => ({ default: module.GraphView })),
);
import "./style.css";

type Tab = "Explore" | "Impact" | "Changes" | "Routes" | "Rules" | "Status";
function Evidence({ value }: { value: unknown }) {
  return (
    <pre className="evidence" tabIndex={0}>
      {JSON.stringify(value, null, 2)}
    </pre>
  );
}
function Metadata({ value }: { value: Envelope<unknown> | null }) {
  return value ? (
    <div className="metadata">
      <span>Index: {value.index_status ?? "registry"}</span>
      <span>
        Generation: <code>{value.generation_id?.slice(0, 16) ?? "—"}</code>
      </span>
      <span>
        Request: <code>{value.request_id.slice(0, 12)}</code>
      </span>
      {value.partial && <strong>Partial evidence</strong>}
      {value.truncated && <strong>Truncated by bounds</strong>}
      {value.warnings?.map((w, i) => (
        <span key={i}>{w}</span>
      ))}
    </div>
  ) : null;
}
interface TreeNode {
  children: Map<string, TreeNode>;
  file?: IndexedFile;
}
function FileTree({
  files,
  onSelect,
}: {
  files: IndexedFile[];
  onSelect: (path: string) => void;
}) {
  const root: TreeNode = { children: new Map() };
  const deep: IndexedFile[] = [];
  for (const file of files) {
    const parts = file.path.split("/");
    if (parts.length > 64) {
      deep.push(file);
      continue;
    }
    let node = root;
    for (const name of parts) {
      if (!node.children.has(name))
        node.children.set(name, { children: new Map() });
      node = node.children.get(name)!;
    }
    node.file = file;
  }
  const render = (node: TreeNode): React.ReactNode => (
    <ul>
      {[...node.children]
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([name, child]) => (
          <li key={name}>
            {child.file ? (
              <button
                className="file"
                onClick={() => onSelect(child.file!.path)}
                title={child.file.path}
              >
                {name}
              </button>
            ) : (
              <details open>
                <summary>{name}/</summary>
                {render(child)}
              </details>
            )}
          </li>
        ))}
    </ul>
  );
  return (
    <nav className="file-tree" aria-label="Indexed file tree">
      {files.length ? render(root) : <p>No indexed files loaded.</p>}
      {deep.length > 0 && (
        <>
          <p>
            Paths deeper than 64 levels are listed flat to keep rendering
            bounded.
          </p>
          <ul>
            {deep.map((file) => (
              <li key={file.path}>
                <button
                  className="file"
                  title={file.path}
                  onClick={() => onSelect(file.path)}
                >
                  {file.path}
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
    </nav>
  );
}
function App() {
  const [origin, setOrigin] = useState(() =>
    window.location.protocol === "http:"
      ? window.location.origin
      : "http://127.0.0.1:7337",
  );
  const [token, setToken] = useState("");
  const [api, setApi] = useState<Api | null>(null);
  const [repos, setRepos] = useState<Repo[]>([]);
  const [repo, setRepo] = useState("");
  const [repoCursor, setRepoCursor] = useState<string | null>(null);
  const [files, setFiles] = useState<IndexedFile[]>([]);
  const [fileCursor, setFileCursor] = useState<string | null>(null);
  const [status, setStatus] = useState<Envelope<
    Record<string, unknown>
  > | null>(null);
  const [text, setText] = useState("");
  const [query, setQuery] = useState<Envelope<{ items: Hit[] }> | null>(null);
  const [context, setContext] = useState<Envelope<Context> | null>(null);
  const [graph, setGraph] = useState<Envelope<Graph> | null>(null);
  const [refresh, setRefresh] = useState(0);
  const [tab, setTab] = useState<Tab>("Explore");
  const [depth, setDepth] = useState(2);
  const [direction, setDirection] = useState("upstream");
  const [impact, setImpact] = useState<Envelope<
    Record<string, unknown>
  > | null>(null);
  const [changes, setChanges] = useState<Envelope<
    Record<string, unknown>
  > | null>(null);
  const [routes, setRoutes] = useState<Envelope<{
    items: Record<string, unknown>[];
  }> | null>(null);
  const [rules, setRules] = useState("");
  const [check, setCheck] = useState<Envelope<Record<string, unknown>> | null>(
    null,
  );
  const [base, setBase] = useState("");
  const [head, setHead] = useState("");
  const [diffMode, setDiffMode] = useState("unstaged");
  const [message, setMessage] = useState(
    "Connect to an explicitly authorized local API.",
  );
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(0);
  const [dark, setDark] = useState(
    () => window.matchMedia("(prefers-color-scheme: dark)").matches,
  );
  const searchRef = useRef<HTMLInputElement>(null);
  const epoch = useRef(0);
  const mounted = useRef(true);
  useEffect(
    () => () => {
      mounted.current = false;
    },
    [],
  );
  useEffect(() => {
    document.documentElement.dataset.theme = dark ? "dark" : "light";
  }, [dark]);
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (
        e.key === "/" &&
        !(e.target instanceof HTMLInputElement) &&
        !(e.target instanceof HTMLTextAreaElement)
      ) {
        e.preventDefault();
        searchRef.current?.focus();
      }
      if (e.key === "Escape") {
        api?.abort();
        setMessage("Requests cancelled.");
      }
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [api]);
  async function work<T>(
    label: string,
    fn: () => Promise<T>,
  ): Promise<T | undefined> {
    const version = epoch.current;
    setBusy((n) => n + 1);
    setError("");
    try {
      const result = await fn();
      if (version === epoch.current && mounted.current) setMessage(label);
      return version === epoch.current ? result : undefined;
    } catch (e) {
      if (version === epoch.current && mounted.current)
        setError(
          e instanceof ApiError
            ? `${e.code}: ${e.message}${e.requestId ? ` (request ${e.requestId})` : ""}`
            : e instanceof Error
              ? e.message
              : "Request failed.",
        );
      return undefined;
    } finally {
      if (mounted.current) setBusy((n) => Math.max(0, n - 1));
    }
  }
  function reset() {
    epoch.current++;
    api?.abort();
    setFiles([]);
    setFileCursor(null);
    setQuery(null);
    setContext(null);
    setGraph(null);
    setImpact(null);
    setChanges(null);
    setRoutes(null);
    setCheck(null);
    setStatus(null);
    setError("");
  }
  async function connect(e: React.FormEvent) {
    e.preventDefault();
    reset();
    setApi(null);
    setRepos([]);
    setRepo("");
    setRepoCursor(null);
    try {
      const client = new Api(origin, token);
      const result = await work("Authorized repositories loaded.", () =>
        client.request<{ items: Repo[] }>("/repos?limit=64"),
      );
      if (!result) return;
      setApi(client);
      setRepos(result.data.items);
      setRepoCursor(result.page?.next_cursor ?? null);
      setRepo(result.data.items[0]?.repo_id ?? "");
      setToken("");
    } catch (e) {
      setError(
        e instanceof ApiError
          ? `${e.code}: ${e.message}`
          : e instanceof Error
            ? e.message
            : "Connection failed.",
      );
    }
  }
  useEffect(() => {
    if (!api || !repo) return;
    reset();
    const version = epoch.current;
    void work("Repository status and indexed files loaded.", async () => {
      const state = await api.request<Record<string, unknown>>(
        `/repos/${repo}/status`,
      );
      if (version !== epoch.current) return;
      setStatus(state);
      const listing = await api.request<{ items: IndexedFile[] }>(
        `/repos/${repo}/files?limit=100&generation_id=${encodeURIComponent(state.generation_id ?? "")}`,
      );
      if (version === epoch.current) {
        setFiles(listing.data.items);
        setFileCursor(listing.page?.next_cursor ?? null);
      }
    });
    return () => api.abort();
  }, [api, repo, refresh]);
  const pinned = () =>
    status?.generation_id ? { generation_id: status.generation_id } : {};
  async function loadFiles() {
    if (!api || !fileCursor) return;
    const listing = await work("Next indexed-file page loaded.", () =>
      api.request<{ items: IndexedFile[] }>(
        `/repos/${repo}/files?limit=100&cursor=${encodeURIComponent(fileCursor)}&generation_id=${encodeURIComponent(status?.generation_id ?? "")}`,
      ),
    );
    if (listing) {
      setFiles((previous) =>
        [...previous, ...listing.data.items].slice(0, 1000),
      );
      setFileCursor(listing.page?.next_cursor ?? null);
    }
  }
  async function search(term = text, cursor?: string) {
    if (!api || !repo || !term.trim()) return;
    setTab("Explore");
    const result = await work("Search evidence loaded.", () =>
      api.request<{ items: Hit[] }>(`/repos/${repo}/query`, {
        text: term,
        limit: 20,
        ...pinned(),
        ...(cursor ? { cursor } : {}),
      }),
    );
    if (result) setQuery(result);
  }
  async function select(id: string) {
    if (!api || !repo) return;
    const result = await work(
      "Symbol details and bounded graph loaded.",
      async () => {
        const detail = await api.request<Context>(
          `/repos/${repo}/symbols/${encodeURIComponent(id)}${status?.generation_id ? `?generation_id=${encodeURIComponent(status.generation_id)}` : ""}`,
        );
        const neighborhood = await api.request<Graph>(`/repos/${repo}/graph`, {
          symbol: id,
          depth,
          max_nodes: 200,
          ...pinned(),
        });
        return { detail, neighborhood };
      },
    );
    if (result) {
      setContext(result.detail);
      setGraph(result.neighborhood);
      setImpact(null);
      setTab("Explore");
    }
  }
  async function runImpact() {
    if (!api || !context) return;
    const result = await work("Impact evidence loaded.", () =>
      api.request<Record<string, unknown>>(`/repos/${repo}/impact`, {
        symbol: context.data.symbol.symbol_id,
        direction,
        depth,
        max_nodes: 200,
        ...pinned(),
      }),
    );
    if (result) setImpact(result);
  }
  async function runChanges() {
    if (!api || !repo) return;
    const result = await work("Change evidence loaded.", () =>
      api.request<Record<string, unknown>>(`/repos/${repo}/changes`, {
        ...(diffMode === "commits"
          ? { ...(base ? { base } : {}), ...(head ? { head } : {}) }
          : { [diffMode]: true }),
        depth,
        max_nodes: 200,
        ...pinned(),
      }),
    );
    if (result) setChanges(result);
  }
  async function runRoutes(cursor?: string) {
    if (!api || !repo) return;
    const result = await work("Framework-evidenced routes loaded.", () =>
      api.request<{ items: Record<string, unknown>[] }>(
        `/repos/${repo}/routes`,
        {
          limit: 20,
          depth,
          max_nodes: 200,
          include_processes: true,
          ...pinned(),
          ...(cursor ? { cursor } : {}),
        },
      ),
    );
    if (result) setRoutes(result);
  }
  async function runCheck() {
    if (!api) return;
    const report = await work("Architecture report loaded.", async () => {
      if (rules.length > 16_000)
        throw new ApiError(
          "REQUEST_BUDGET",
          "Rules JSON must fit the 16 KiB HTTP body budget.",
        );
      const policy: unknown = JSON.parse(rules);
      if (!policy || typeof policy !== "object" || Array.isArray(policy))
        throw new Error("Enter a JSON rules object.");
      return api.request<Record<string, unknown>>(`/repos/${repo}/check`, {
        ...policy,
        ...pinned(),
      });
    });
    if (report) setCheck(report);
  }
  const linked = (edge: Edge, side: "source_id" | "target_id") => {
    const id = edge[side];
    return id ? (
      <button onClick={() => select(id)} title={id}>
        {graph?.data.nodes.find((n) => n.symbol_id === id)?.qualified_name ??
          id.slice(0, 16)}
      </button>
    ) : (
      <span>Unresolved</span>
    );
  };
  const edges = (
    name: string,
    items: Edge[],
    side: "source_id" | "target_id",
  ) => (
    <details open>
      <summary>
        {name} ({items.length})
      </summary>
      {items.length ? (
        <ul className="edge-list">
          {items.slice(0, 100).map((edge) => (
            <li key={edge.id}>
              {linked(edge, side)}{" "}
              <span>
                {edge.resolution} · {edge.confidence.toFixed(2)}
              </span>
              <details>
                <summary>Evidence</summary>
                <p>{edge.evidence}</p>
                <code>{edge.id}</code>
                <p>
                  Line {edge.source_span.start_line}:
                  {edge.source_span.start_col}
                </p>
                {edge.candidates.length > 0 && (
                  <p>Candidates: {edge.candidates.join(", ")}</p>
                )}
              </details>
            </li>
          ))}
        </ul>
      ) : (
        <p>None in the indexed evidence.</p>
      )}
      {items.length > 100 && (
        <p>
          Showing first 100 edges; export JSON for the complete bounded result.
        </p>
      )}
    </details>
  );
  return (
    <>
      <a href="#main" className="skip-link">
        Skip to explorer
      </a>
      <header>
        <div>
          <p className="eyebrow">Local intelligence / private workspace</p>
          <h1>Kortex</h1>
          <p>Evidence, not guesses. Indexed source is untrusted data.</p>
        </div>
        <button
          onClick={() => setDark((v) => !v)}
          aria-label={`Switch to ${dark ? "light" : "dark"} theme`}
        >
          {dark ? "Light" : "Dark"} theme
        </button>
      </header>
      <section className="connection" aria-label="API connection">
        <form onSubmit={connect}>
          <label>
            API origin
            <input
              aria-label="API origin"
              type="url"
              value={origin}
              onChange={(e) => setOrigin(e.target.value)}
              required
            />
          </label>
          <label>
            Bearer token
            <input
              aria-label="Bearer token"
              type="password"
              autoComplete="off"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              minLength={32}
              maxLength={4096}
              required
            />
          </label>
          <button type="submit">Connect</button>
          {api && (
            <button
              type="button"
              onClick={() => {
                reset();
                setApi(null);
                setToken("");
                setRepos([]);
                setRepo("");
                setRepoCursor(null);
                setMessage("Disconnected. Credentials cleared.");
              }}
            >
              Disconnect
            </button>
          )}
        </form>
        <p className="muted">
          Tokens remain in memory only. Configure this browser origin in the
          API’s exact CORS allowlist. No telemetry, remote assets or source
          uploads.
        </p>
      </section>
      <div role="status" aria-live="polite" className="notice">
        {busy ? `${busy} bounded request${busy > 1 ? "s" : ""} running… ` : ""}
        {message}{" "}
        {busy > 0 && (
          <button
            onClick={() => {
              api?.abort();
              setMessage("Requests cancelled.");
            }}
          >
            Cancel requests
          </button>
        )}
      </div>
      {error && (
        <div role="alert" className="error">
          {error}
        </div>
      )}
      <main id="main">
        <aside>
          <label>
            Repository
            <select
              aria-label="Repository"
              value={repo}
              onChange={(e) => setRepo(e.target.value)}
              disabled={!repos.length}
            >
              {!repos.length && (
                <option value="">No authorized repositories</option>
              )}
              {repos.map((r) => (
                <option key={r.repo_id} value={r.repo_id}>
                  {r.repo_id.slice(0, 16)}
                </option>
              ))}
            </select>
          </label>
          {repoCursor && (
            <button
              onClick={async () => {
                if (!api) return;
                const result = await work("More repositories loaded.", () =>
                  api.request<{ items: Repo[] }>(
                    `/repos?limit=64&cursor=${encodeURIComponent(repoCursor)}`,
                  ),
                );
                if (result) {
                  setRepos((previous) => [...previous, ...result.data.items]);
                  setRepoCursor(result.page?.next_cursor ?? null);
                }
              }}
            >
              More repositories
            </button>
          )}
          <h2>Indexed files</h2>
          <p className="muted">
            {files.length} loaded · up to 1,000 displayed. This is the index,
            not a live filesystem browser.
          </p>
          <FileTree
            files={files}
            onSelect={(path) => {
              setText(path);
              void search(path);
            }}
          />
          {fileCursor && (
            <button disabled={files.length >= 1000} onClick={loadFiles}>
              More indexed files
            </button>
          )}
        </aside>
        <div className="workspace">
          <form
            className="search"
            onSubmit={(e) => {
              e.preventDefault();
              void search();
            }}
          >
            <label>
              Search indexed code <kbd>/</kbd>
              <input
                ref={searchRef}
                aria-label="Search indexed code"
                value={text}
                maxLength={4096}
                onChange={(e) => setText(e.target.value)}
                placeholder="Symbol, path or keyword evidence"
              />
            </label>
            <button disabled={!api || !repo || !text.trim()}>Search</button>
          </form>
          <div className="tabs" role="tablist" aria-label="Explorer views">
            {(
              [
                "Explore",
                "Impact",
                "Changes",
                "Routes",
                "Rules",
                "Status",
              ] as Tab[]
            ).map((name) => (
              <button
                key={name}
                id={`tab-${name}`}
                role="tab"
                aria-selected={tab === name}
                aria-controls={`panel-${name}`}
                tabIndex={tab === name ? 0 : -1}
                onClick={() => setTab(name)}
                onKeyDown={(e) => {
                  const names: Tab[] = [
                    "Explore",
                    "Impact",
                    "Changes",
                    "Routes",
                    "Rules",
                    "Status",
                  ];
                  if (
                    ["ArrowLeft", "ArrowRight", "Home", "End"].includes(e.key)
                  ) {
                    e.preventDefault();
                    const index =
                      e.key === "Home"
                        ? 0
                        : e.key === "End"
                          ? names.length - 1
                          : (names.indexOf(name) +
                              (e.key === "ArrowRight" ? 1 : -1) +
                              names.length) %
                            names.length;
                    setTab(names[index]);
                    document.getElementById(`tab-${names[index]}`)?.focus();
                  }
                }}
              >
                {name}
              </button>
            ))}
          </div>
          <section
            id={`panel-${tab}`}
            role="tabpanel"
            aria-labelledby={`tab-${tab}`}
          >
            {tab === "Explore" && (
              <>
                <Metadata value={query ?? context ?? status} />
                {query && (
                  <section aria-label="Search results">
                    <h2>Search results</h2>
                    <ul className="results">
                      {query.data.items.map((hit) => (
                        <li key={hit.symbol.symbol_id}>
                          <button onClick={() => select(hit.symbol.symbol_id)}>
                            {hit.symbol.qualified_name}
                          </button>
                          <span>
                            {hit.symbol.file_path}:{hit.symbol.span.start_line}{" "}
                            · {hit.why_matched.join(", ")}
                          </span>
                        </li>
                      ))}
                    </ul>
                    {!query.data.items.length && <p>No matching evidence.</p>}
                    {query.page?.next_cursor && (
                      <button
                        onClick={() => search(text, query.page!.next_cursor!)}
                      >
                        Next search page
                      </button>
                    )}
                  </section>
                )}
                <div className="toolbar">
                  <label>
                    Traversal depth
                    <input
                      aria-label="Traversal depth"
                      type="number"
                      min={0}
                      max={4}
                      value={depth}
                      onChange={(e) =>
                        setDepth(
                          Math.max(0, Math.min(4, Number(e.target.value))),
                        )
                      }
                    />
                  </label>
                  <button
                    disabled={!context}
                    onClick={() =>
                      context && select(context.data.symbol.symbol_id)
                    }
                  >
                    Reload bounded graph
                  </button>
                </div>
                <Suspense
                  fallback={<p>Loading locally bundled graph renderer…</p>}
                >
                  <GraphView
                    result={graph}
                    onSelect={(id) => void select(id)}
                    dark={dark}
                  />
                </Suspense>
                {context && (
                  <section aria-label="Symbol details" className="details">
                    <h2>{context.data.symbol.qualified_name}</h2>
                    <p>
                      {context.data.symbol.kind} ·{" "}
                      {context.data.symbol.language} ·{" "}
                      <code>
                        {context.data.symbol.file_path}:
                        {context.data.symbol.span.start_line}:
                        {context.data.symbol.span.start_col}
                      </code>
                    </p>
                    <pre>{context.data.symbol.signature}</pre>
                    {context.data.symbol.doc_comment && (
                      <p>{context.data.symbol.doc_comment}</p>
                    )}
                    <Metadata value={context} />
                    <p>
                      Symbol revision:{" "}
                      <code>{context.data.symbol.symbol_revision_id}</code>
                    </p>
                    <p>
                      Indexed content:{" "}
                      <code>{context.data.symbol.content_hash}</code>
                    </p>
                    {edges("Callers", context.data.callers, "source_id")}
                    {edges("Callees", context.data.callees, "target_id")}
                    {edges(
                      "References",
                      context.data.references ?? [],
                      "source_id",
                    )}
                    {edges(
                      "Type evidence",
                      context.data.types ?? [],
                      "target_id",
                    )}
                    <button
                      onClick={() =>
                        exportJSON(context, "kortex-symbol-evidence.json")
                      }
                    >
                      Export symbol evidence JSON
                    </button>
                  </section>
                )}
              </>
            )}
            {tab === "Impact" && (
              <>
                <h2>Change impact</h2>
                <p>
                  {context
                    ? `Target: ${context.data.symbol.qualified_name}`
                    : "Select an indexed symbol in Explore first."}
                </p>
                <label>
                  Direction
                  <select
                    aria-label="Impact direction"
                    value={direction}
                    onChange={(e) => setDirection(e.target.value)}
                  >
                    <option>upstream</option>
                    <option>downstream</option>
                    <option>both</option>
                  </select>
                </label>
                <p>
                  Depth: {depth}, at most 200 nodes. Risk is heuristic, not a
                  test execution result.
                </p>
                <button disabled={!context} onClick={runImpact}>
                  Analyze impact
                </button>
                <Metadata value={impact} />
                {impact && (
                  <>
                    <Evidence value={impact.data} />
                    <button
                      onClick={() => exportJSON(impact, "kortex-impact.json")}
                    >
                      Export impact JSON
                    </button>
                  </>
                )}
              </>
            )}
            {tab === "Changes" && (
              <>
                <h2>Git changes</h2>
                <label>
                  Comparison
                  <select
                    aria-label="Change comparison"
                    value={diffMode}
                    onChange={(e) => setDiffMode(e.target.value)}
                  >
                    <option value="unstaged">Unstaged working tree</option>
                    <option value="staged">Staged changes</option>
                    <option value="commits">Commit references</option>
                  </select>
                </label>
                {diffMode === "commits" && (
                  <div className="toolbar">
                    <label>
                      Base reference
                      <input
                        value={base}
                        maxLength={1024}
                        onChange={(e) => setBase(e.target.value)}
                      />
                    </label>
                    <label>
                      Head reference
                      <input
                        value={head}
                        maxLength={1024}
                        onChange={(e) => setHead(e.target.value)}
                      />
                    </label>
                  </div>
                )}
                <button disabled={!api || !repo} onClick={runChanges}>
                  Inspect changes
                </button>
                <Metadata value={changes} />
                {changes && (
                  <>
                    <Evidence value={changes.data} />
                    <button
                      onClick={() => exportJSON(changes, "kortex-changes.json")}
                    >
                      Export changes JSON
                    </button>
                  </>
                )}
              </>
            )}
            {tab === "Routes" && (
              <>
                <h2>Framework-evidenced routes</h2>
                <p>
                  Only recognized framework bindings are reported. Missing
                  evidence is not proof that a route does not exist.
                </p>
                <button disabled={!api || !repo} onClick={() => runRoutes()}>
                  Inspect routes
                </button>
                <Metadata value={routes} />
                {routes && (
                  <>
                    <Evidence value={routes.data} />
                    {routes.page?.next_cursor && (
                      <button
                        onClick={() => runRoutes(routes.page!.next_cursor!)}
                      >
                        Next routes page
                      </button>
                    )}
                    <button
                      onClick={() => exportJSON(routes, "kortex-routes.json")}
                    >
                      Export route evidence JSON
                    </button>
                  </>
                )}
              </>
            )}
            {tab === "Rules" && (
              <>
                <h2>Architecture checks</h2>
                <p>
                  Submit explicit version-1 inline rules. No arbitrary server
                  rule files, code or commands are accepted.
                </p>
                <label>
                  Rules JSON
                  <textarea
                    aria-label="Rules JSON"
                    rows={10}
                    maxLength={16000}
                    value={rules}
                    onChange={(e) => setRules(e.target.value)}
                  />
                </label>
                <button
                  disabled={!api || !repo || !rules.trim()}
                  onClick={runCheck}
                >
                  Evaluate architecture rules
                </button>
                <Metadata value={check} />
                {check && (
                  <>
                    <Evidence value={check.data} />
                    <button
                      onClick={() =>
                        exportJSON(check, "kortex-architecture-check.json")
                      }
                    >
                      Export architecture report JSON
                    </button>
                  </>
                )}
              </>
            )}
            {tab === "Status" && (
              <>
                <h2>Index status</h2>
                <Metadata value={status} />
                {status ? (
                  <>
                    <Evidence value={status.data} />
                    <button
                      onClick={() => exportJSON(status, "kortex-status.json")}
                    >
                      Export status JSON
                    </button>
                  </>
                ) : (
                  <p>No authorized index loaded.</p>
                )}
                <button
                  disabled={!api || !repo}
                  onClick={() => setRefresh((n) => n + 1)}
                >
                  Reload indexed generation
                </button>
              </>
            )}
          </section>
        </div>
      </main>
      <footer>
        Proprietary / private · No public use or distribution authorized ·
        Read-only local evidence · <kbd>Esc</kbd> cancels requests
      </footer>
    </>
  );
}
createRoot(document.getElementById("root")!).render(<App />);
