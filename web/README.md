# Kortex private explorer

Proprietary/private. The root `LICENSE` applies. No public use, distribution,
publishing, deployment or source upload is authorized. `package.json` is private
and has no publish/deploy scripts. All production assets are locally bundled;
there are no CDNs, remote fonts, telemetry or embedding calls.

## Local operation

1. At build time only: `cargo build -p kortex-cli --features http` at the repo
   root, then `cd web && npm ci --ignore-scripts && npm run build`. This produces
   the explicit owner-supplied `web/dist` directory. No Node installation is
   needed on the runtime/private-install machine.
2. Index your repository and obtain its registered ID with `kortex repos list --json`.
3. Supply a strong bearer token through an explicitly named environment variable.
   Do not put secrets in source, configuration files, command-line token arguments,
   URLs or shared logs.
4. Run `kortex serve --auth-token-env KORTEX_HTTP_TOKEN --allow-repo REPO_ID --web-assets /absolute/path/to/prebuilt/dist`.
   Add `--allow-repo` for each authorized registered repository. The API binds to
   loopback by default and requires bearer credentials even there. The explicit
   asset option is never enabled by repository configuration and never downloads
   or builds anything. Static-asset mode refuses all non-loopback bindings.
5. Open `http://127.0.0.1:7337`. The UI defaults to that same API origin, with no
   CORS configuration needed for same-origin requests. Enter the bearer token
   and connect. Tokens remain in memory only and are cleared on disconnect.

Rust snapshots the prebuilt directory at startup through the shared confined
regular-file reader. Bounds: 512 entries, eight directory levels, 2 MiB/file,
16 MiB total. Current accepted assets are UTF-8 HTML/JS/MJS/CSS/JSON/SVG/TXT;
binary assets await a shared confined-byte reader. Symlinks, special files,
unsafe names and oversized inventories are refused. Source maps, directory
listing, traversal and unknown-route SPA fallback are not served. Responses
set correct MIME, no-store, nosniff, same-origin CORP, restrictive CSP and
no-referrer. Initial static HTML is available only after explicit local owner
opt-in; every API data endpoint still requires bearer and repository policy.
Vite is a build/development tool, not the production delivery server.

The tree lists **indexed files**, not arbitrary filesystem paths. API results are
pinned to the selected index generation. Reload the indexed generation after
reindexing; a stale cursor/generation is rejected, not silently resumed.
Graphs request at most 200 nodes, display at most 1,000 edges, and preserve the
full returned bounded evidence in JSON exports. The indexed file view displays
at most 1,000 loaded files and supports explicit pagination. Partial/truncated
results and request IDs remain visible. SVG/PNG represent the loaded bounded
neighborhood, never an entire repository graph. No repository code is executed.

Keyboard: `/` focuses search, `Esc` cancels outstanding fetches, arrow/Home/End
keys navigate view tabs. Native file-directory disclosures and a separate graph
node list provide keyboard navigation equivalent to the canvas. Light/dark and
mobile layouts are supported. Source comments and names are escaped text, never
HTML instructions.

## Executable acceptance

Avoid races with peers rebuilding the default CLI without HTTP:

```sh
# From repository root: isolate the real API binary used by browser tests.
cargo build -p kortex-cli --features http --target-dir target/transport-http
cd web
npm run build
PLAYWRIGHT_CHROMIUM_EXECUTABLE='/Users/chris/Library/Caches/ms-playwright/chromium-1248/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing' npm test
npm run format:check
npm audit
```

`KORTEX_TEST_BINARY` can explicitly select another private HTTP-enabled build.
`PLAYWRIGHT_CHROMIUM_EXECUTABLE` selects an already approved installed browser;
the scripts do **not** download browsers. On other machines explicitly approve
and provide an appropriate browser first. Tests create temporary real source
repositories, run actual index/CLI/API processes on ephemeral loopback ports and
consume the production-built UI **served by Rust**, without a Vite server. They
do not mock API responses or ship fixture
data into advertised UI paths. Accessibility checks use axe against real loaded
evidence in light, dark and mobile layouts; these automated checks are not a
substitute for owner/platform assistive-technology acceptance.

## Dependencies and provenance

Pinned direct runtime dependencies: React/react-dom 19.3.1 and Cytoscape.js 3.35.0
(MIT, third-party licenses only). React provides accessible native controls and
escaped rendering; Cytoscape provides bounded local graph rendering/export.
Development-only: TypeScript 5.9.3 (Apache-2.0), Vite 8.3.8 (MIT), Playwright 1.65.0
(Apache-2.0), Prettier 3.10.0 (MIT), type packages (MIT), and
`@axe-core/playwright`/axe-core 4.14.0 (MPL-2.0, unchanged test-only accessibility
scanner, not included in production bundles). `package-lock.json` pins integrity
metadata. `npm install --ignore-scripts` and `npm audit` are acceptance commands;
no dependency's license grants rights to first-party Kortex code. Parent owns
final license/SBOM review and all distribution decisions.
