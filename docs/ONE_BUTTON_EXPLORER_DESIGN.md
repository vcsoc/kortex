# One-button local explorer: design and remaining work

Status: proposed, not implemented. The existing explorer still requires the setup in [web/README.md](../web/README.md).

## User experience

1. From an indexed project, run `kortex dashboard` (or provide a project path). Kortex starts the local service, launches the browser and connects the selected project automatically, without a separate picker, token entry or repository ID.
2. Alternatively, open the local dashboard and select a project connection file or use **Open a Kortex graph** to choose a project. Kortex validates the selection and connects without manual configuration. A connection file must not expose reusable secrets in the project tree or version control.
3. Open the explorer with its status, indexed files, search and graph views.
4. If the graph is missing, stale, inaccessible, corrupt or ambiguous, explain the specific condition. Do not silently create or rebuild an index, select a different project, or claim the graph is fresh. Offer any indexing/recovery action separately with explicit consent.

This workflow is about an existing managed knowledge graph, not automatic indexing of every selected folder. Rendering remains a bounded, explorable neighborhood for large graphs, with partial results clearly identified.

## Why the existing browser cannot simply open it

A browser folder picker supplies browser-scoped file handles, not a trusted server-side filesystem path. The current explorer talks to a separate Rust HTTP service that operates on registered repository identities and configuration provenance; it cannot safely infer those from a browser-provided folder name. SQLite may also have WAL sidecars and the explorer may need source and registry context, so copying an arbitrary database file into the browser is not equivalent to opening the live Kortex graph.

The current server requires an explicit bearer token and fixed repository allowlist, including on loopback. Removing these checks or accepting an arbitrary browser-supplied path would expose source-derived data and weaken an intentional security boundary. The desired simplicity must hide setup from the user, not remove authorization.

## Proposed local companion

Use a trusted, explicitly started local Kortex companion that serves the dashboard on loopback and can invoke an OS-native folder picker. A browser button requests a picker from an authenticated, local session; only the picker result grants access, not an arbitrary path supplied in a request. Validate the selected path and graph identity using existing confinement and registered-selection logic. Grant the session access only to the selected repository and keep graph operations read-only. Provide a clear way to close the session and choose a different graph.

Bootstrap a short-lived, in-memory session without placing credentials in URLs, persistent browser storage, logs, repository files or command arguments. An unauthenticated first-browser-wins endpoint is not safe: another local client or hostile site could race it. One candidate for fully automatic CLI launch is an owned browser session with a private automation pipe that delivers a fresh token directly to the selected tab; another secure local IPC mechanism may be suitable. A project connection file can hold non-secret selection metadata but must not become a reusable plaintext bearer credential. The exact browser/companion handshake must be designed and tested against hostile websites, DNS rebinding, cross-origin requests and unauthorized local clients before implementation. Binding to loopback alone is insufficient. Restrict the management interface to local mode; do not extend the existing remote API with arbitrary filesystem access.

A local companion must be running to serve a normal browser dashboard; a static web page cannot start a native process by itself. Packaging should provide an ordinary local app/launcher that starts it and opens the browser automatically, while the dashboard itself retains the requested single folder-selection action. Platform-native picker and launcher implementations require separate Windows, macOS and Linux runtime validation. If the companion or picker is unavailable, show an actionable error rather than suggesting the graph was opened.

## Acceptance criteria

- Real browser tests: `kortex dashboard` from an indexed project starts and connects automatically; alternatively, one dashboard action opens a picker and selecting a pre-indexed project or connection file loads the correct generation. Neither flow requires manual token/ID/path entry.
- Cover cancellation, missing/unregistered/ambiguous graphs, stale generation, sidecars and relocated/shared storage, permission failures and corrupted indexes. Never overwrite, reindex or delete user data merely on selection.
- Verify only the selected registered repository is authorized; unrelated origins and unauthenticated clients cannot select a folder or read graph data. Test DNS rebinding, CSRF/origin checks, session expiration and companion shutdown.
- Exercise actual native pickers and browser session bootstrap on supported operating systems, as well as keyboard, mobile-sized viewport and bounded large-graph behavior. Unit tests or a mocked picker alone are not platform acceptance.
- Resolve or explicitly contain the open ignore-policy symlink issue before introducing any automatic indexing flow. Do not infer production readiness from this design.
