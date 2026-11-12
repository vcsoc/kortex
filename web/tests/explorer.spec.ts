import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { spawn, spawnSync } from "node:child_process";
import type { ChildProcess } from "node:child_process";
import { mkdtemp, mkdir, writeFile, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve, join } from "node:path";
import net from "node:net";
let directory: string;
let server: ChildProcess;
let apiOrigin: string;
let ids: string[] = [];
const token = "browser-test-only-bearer-token-32-bytes";
const hostileLabel =
  '"</text><script>window.__injected=1</script>&svg_hostile\u0001🔒"';
let selectedConfig: string;
let selectedIndex: number;
let largeIndex: number;
const binary =
  process.env.KORTEX_TEST_BINARY ??
  resolve("../target/transport-http/debug/kortex");
function cli(root: string, args: string[]) {
  const result = spawnSync(binary, [...args, "--json"], {
    cwd: root,
    env: { ...process.env, KORTEX_REGISTRY: join(directory, "registry.db") },
    encoding: "utf8",
  });
  expect(result.status, result.stdout + result.stderr).toBe(0);
  return JSON.parse(result.stdout);
}
function fixtureGit(root: string, args: string[]) {
  const empty = join(directory, "empty-config");
  const result = spawnSync(
    "git",
    [
      "-c",
      `core.hooksPath=${join(directory, "empty-hooks")}`,
      "-c",
      `core.attributesFile=${empty}`,
      "-c",
      "user.name=Kortex Browser Test",
      "-c",
      "user.email=browser@example.invalid",
      "-c",
      "commit.gpgsign=false",
      ...args,
    ],
    {
      cwd: root,
      env: {
        ...process.env,
        GIT_CONFIG_NOSYSTEM: "1",
        GIT_CONFIG_GLOBAL: empty,
        GIT_CONFIG_SYSTEM: empty,
        GIT_CONFIG_COUNT: "0",
        GIT_ATTR_NOSYSTEM: "1",
      },
      encoding: "utf8",
    },
  );
  expect(result.status, result.stdout + result.stderr).toBe(0);
}
test.beforeAll(async () => {
  directory = await mkdtemp(join(tmpdir(), "kortex-private-browser-"));
  await writeFile(join(directory, "empty-config"), "");
  await mkdir(join(directory, "empty-hooks"));
  ids = [];
  for (const name of ["first", "second"]) {
    const root = join(directory, name);
    await mkdir(root);
    fixtureGit(root, ["init", "--quiet", "--template="]);
    await writeFile(
      join(root, "app.py"),
      `from fastapi import FastAPI\napp = FastAPI()\n\ndef leaf_${name}():\n    return True\n\n@app.get('/${name}')\ndef entry_${name}():\n    \"\"\"<img src=x onerror=window.__injected=1>\"\"\"\n    return leaf_${name}()\n`,
    );
    if (name === "first")
      await writeFile(
        join(root, "labels<&.ts"),
        `class Hostile { ${hostileLabel}() { return 1; } }\n`,
      );
    const deep = join(root, ...Array<string>(65).fill("d"));
    await mkdir(deep, { recursive: true });
    await writeFile(join(deep, "deep.py"), "def deep_evidence():\n    pass\n");
    fixtureGit(root, ["add", "--", "app.py"]);
    fixtureGit(root, ["commit", "--quiet", "-m", "local browser fixture"]);
    ids.push(cli(root, ["index", "."]).repo_id);
  }
  // Explicit relocated selection must survive ID-only access from the server's unrelated cwd.
  const selected = join(directory, "provenance");
  await mkdir(join(selected, "src"), { recursive: true });
  await writeFile(
    join(selected, "src", "selected.py"),
    "def provenance_entry():\n    pass\n",
  );
  selectedConfig = join(selected, "chosen.toml");
  await writeFile(selectedConfig, "[repository]\nroot='src'\n");
  selectedIndex = ids.length;
  ids.push(cli(selected, ["--config", "chosen.toml", "index", "."]).repo_id);
  const large = join(directory, "large");
  await mkdir(large);
  const branches = Array.from({ length: 20 }, (_, i) => `b${i}`);
  const source =
    "def large_hub():\n" +
    branches.map((name) => `    ${name}()\n`).join("") +
    branches
      .map(
        (name, i) =>
          `\ndef ${name}():\n` +
          Array.from({ length: 15 }, (_, j) => `    f${i}_${j}()\n`).join("") +
          Array.from(
            { length: 15 },
            (_, j) => `\ndef f${i}_${j}():\n    pass\n`,
          ).join(""),
      )
      .join("");
  await writeFile(join(large, "large.py"), source);
  largeIndex = ids.length;
  ids.push(cli(large, ["index", "."]).repo_id);
  const reservation = net.createServer();
  await new Promise<void>((resolve) =>
    reservation.listen(0, "127.0.0.1", resolve),
  );
  const port = (reservation.address() as net.AddressInfo).port;
  await new Promise<void>((resolve) => reservation.close(() => resolve()));
  apiOrigin = `http://127.0.0.1:${port}`;
  server = spawn(
    binary,
    [
      "serve",
      "--host",
      "127.0.0.1",
      "--port",
      String(port),
      "--auth-token-env",
      "KORTEX_BROWSER_TOKEN",
      ...ids.flatMap((id) => ["--allow-repo", id]),
      "--web-assets",
      resolve("dist"),
    ],
    {
      cwd: directory,
      env: {
        ...process.env,
        KORTEX_REGISTRY: join(directory, "registry.db"),
        KORTEX_BROWSER_TOKEN: token,
      },
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  await new Promise<void>((resolve, reject) => {
    let diagnostics = "";
    const timeout = setTimeout(
      () => reject(new Error("Loopback API startup timed out")),
      10_000,
    );
    server.once("exit", (code) => {
      clearTimeout(timeout);
      reject(
        new Error(
          `API exited ${code}; build the isolated CLI with --features http. ${diagnostics.replaceAll(token, "[redacted]")}`,
        ),
      );
    });
    server.stderr!.on("data", (chunk) => {
      diagnostics = (diagnostics + String(chunk)).slice(-16384);
      if (String(chunk).includes("listening")) {
        clearTimeout(timeout);
        resolve();
      }
    });
  });
});
test.afterAll(async () => {
  server?.kill();
  if (server?.exitCode === null)
    await new Promise((resolve) => server.once("exit", resolve));
  if (directory) await rm(directory, { recursive: true, force: true });
});
async function connect(page: Page) {
  await page.goto(`${apiOrigin}/`);
  await expect(page.getByLabel("API origin", { exact: true })).toHaveValue(
    apiOrigin,
  );
  await page.getByLabel("Bearer token", { exact: true }).fill(token);
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  await expect(page.getByLabel("Repository", { exact: true })).toHaveValue(
    ids[0],
  );
  await expect(page.getByText("Index: fresh", { exact: true })).toBeVisible();
}
async function selectEntry(page: Page, name = "first") {
  await page
    .getByLabel("Search indexed code", { exact: true })
    .fill(`entry_${name}`);
  await page.getByRole("button", { name: "Search", exact: true }).click();
  await page
    .getByRole("button", { name: `entry_${name}`, exact: true })
    .click();
  await expect(
    page.getByRole("region", { name: "Symbol details" }),
  ).toContainText(`entry_${name}`);
  await expect(
    page.getByRole("img", { name: /Dependency graph with/ }),
  ).toBeVisible();
}
test("WCAG checks on real evidence in light, dark and mobile layouts", async ({
  page,
}) => {
  await connect(page);
  await selectEntry(page);
  for (const dark of [true, false]) {
    const current = await page.locator("html").getAttribute("data-theme");
    if ((current === "dark") !== dark)
      await page.getByRole("button", { name: /Switch to .* theme/ }).click();
    const scan = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa"])
      .analyze();
    expect(scan.violations).toEqual([]);
  }
  await page.setViewportSize({ width: 390, height: 844 });
  const mobile = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa"])
    .analyze();
  expect(mobile.violations).toEqual([]);
});

test("indexed file tree uses actual pinned file-list API", async ({ page }) => {
  await connect(page);
  await expect(
    page.getByRole("button", { name: "app.py", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("navigation", { name: "Indexed file tree" }),
  ).toContainText("Paths deeper than 64 levels are listed flat");
  await page
    .getByRole("navigation", { name: "Indexed file tree" })
    .getByRole("button", { name: /deep.py$/ })
    .click();
  await expect(
    page.getByRole("region", { name: "Search results" }),
  ).toContainText("deep_evidence");
  await page.getByRole("button", { name: "app.py", exact: true }).click();
  await expect(
    page.getByLabel("Search indexed code", { exact: true }),
  ).toHaveValue("app.py");
  await expect(
    page.getByRole("region", { name: "Search results" }),
  ).toContainText("entry_first");
});

test("real API journey, evidence, bounded graph, exports, routes, impact and changes", async ({
  page,
}) => {
  const origins = new Set<string>();
  page.on("request", (request) => {
    if (request.url().startsWith("http"))
      origins.add(new URL(request.url()).origin);
  });
  await connect(page);
  await selectEntry(page);
  await expect(
    page
      .getByRole("region", { name: "Symbol details" })
      .getByText("Callees (1)", { exact: true }),
  ).toBeVisible();
  await page.getByLabel("Traversal depth", { exact: true }).fill("0");
  await page.getByRole("button", { name: "Reload bounded graph" }).click();
  await expect(
    page.getByRole("img", { name: "Dependency graph with 1 indexed nodes" }),
  ).toBeVisible();
  await page.getByLabel("Traversal depth", { exact: true }).fill("2");
  await page.getByRole("button", { name: "Reload bounded graph" }).click();
  await expect(
    page.getByRole("img", {
      name: /Dependency graph with [2-9] indexed nodes/,
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("region", { name: "Symbol details" }),
  ).toContainText("app.py");
  expect(await page.locator("img").count()).toBe(0);
  expect(
    await page.evaluate(() =>
      Object.prototype.hasOwnProperty.call(window, "__injected"),
    ),
  ).toBe(false);
  for (const format of ["JSON", "SVG", "PNG"]) {
    const downloadPromise = page.waitForEvent("download");
    await page
      .getByRole("button", { name: `Export ${format}`, exact: true })
      .click();
    const download = await downloadPromise;
    const path = await download.path();
    expect(path).not.toBeNull();
    const bytes = await readFile(path!);
    if (format === "JSON") {
      const graph = JSON.parse(bytes.toString());
      expect(graph.repo_id).toBe(ids[0]);
      expect(graph.data.nodes.length).toBeLessThanOrEqual(200);
      expect(
        graph.data.nodes.some(
          (node: { name: string }) => node.name === "entry_first",
        ),
      ).toBe(true);
    } else if (format === "SVG") {
      expect(bytes.toString()).toContain("<svg");
      expect(bytes.toString()).not.toContain("<script");
      expect(bytes.toString()).toContain("marker-end");
      expect(bytes.toString()).toContain("app.py");
    } else {
      expect(bytes.subarray(1, 4).toString()).toBe("PNG");
      expect(bytes.readUInt32BE(16)).toBeLessThanOrEqual(4096);
      expect(bytes.readUInt32BE(20)).toBeLessThanOrEqual(4096);
    }
  }
  await page.getByRole("tab", { name: "Impact", exact: true }).click();
  await page
    .getByLabel("Impact direction", { exact: true })
    .selectOption("downstream");
  await page.getByRole("button", { name: "Analyze impact" }).click();
  await expect(page.getByRole("tabpanel")).toContainText("risk_is_heuristic");
  await expect(page.getByRole("tabpanel")).toContainText("leaf_first");
  const source = join(directory, "first", "app.py");
  const original = await readFile(source, "utf8");
  try {
    await writeFile(
      source,
      original.replace(
        "return leaf_first()",
        "return leaf_first() # real worktree edit",
      ),
    );
    await page.getByRole("tab", { name: "Changes", exact: true }).click();
    await page.getByRole("button", { name: "Inspect changes" }).click();
    await expect(page.getByRole("tabpanel")).toContainText('"path": "app.py"');
    await expect(page.getByRole("tabpanel")).toContainText("entry_first");
  } finally {
    await writeFile(source, original);
  }
  await page.getByRole("tab", { name: "Routes", exact: true }).click();
  await page.getByRole("button", { name: "Inspect routes" }).click();
  await expect(page.getByRole("tabpanel")).toContainText("/first");
  await page.getByRole("tab", { name: "Status", exact: true }).click();
  await expect(page.getByRole("tabpanel")).toContainText("integrity");
  expect([...origins]).toEqual([apiOrigin]);
  expect(
    await page.evaluate(() =>
      [...Object.values(localStorage), ...Object.values(sessionStorage)].some(
        (value) => String(value).includes("bearer-token"),
      ),
    ),
  ).toBe(false);
});
test("repository isolation, keyboard navigation, responsive themes and denial", async ({
  page,
}) => {
  await connect(page);
  await page.getByLabel("Repository", { exact: true }).selectOption(ids[1]);
  await page.getByRole("tab", { name: "Status", exact: true }).click();
  await expect(page.getByRole("tabpanel")).toContainText("/second");
  await page.getByRole("tab", { name: "Explore", exact: true }).click();
  await selectEntry(page, "second");
  await expect(
    page.getByRole("region", { name: "Symbol details" }),
  ).not.toContainText("entry_first");
  await page.getByRole("button", { name: /Switch to .* theme/ }).click();
  await expect(page.locator("html")).toHaveAttribute(
    "data-theme",
    /light|dark/,
  );
  await page.locator("body").click({ position: { x: 2, y: 2 } });
  await page.keyboard.press("/");
  await expect(
    page.getByLabel("Search indexed code", { exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("status")).toContainText("cancelled");
  await page.getByRole("tab", { name: "Explore", exact: true }).focus();
  await page.keyboard.press("ArrowRight");
  await expect(
    page.getByRole("tab", { name: "Impact", exact: true }),
  ).toBeFocused();
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  await page.getByRole("button", { name: "Disconnect", exact: true }).click();
  await expect(page.getByLabel("Repository", { exact: true })).toHaveValue("");
  await page.getByLabel("Bearer token", { exact: true }).fill("密".repeat(32));
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("AUTH_REQUIRED");
  await expect(page.getByRole("alert")).not.toContainText("密");
  await page
    .getByLabel("Bearer token", { exact: true })
    .fill("incorrect-token-that-still-has-32-bytes");
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("AUTH_REQUIRED");
});

test("registered selection provenance and config tamper fail closed through real API", async ({
  page,
}) => {
  await connect(page);
  await page
    .getByLabel("Repository", { exact: true })
    .selectOption(ids[selectedIndex]);
  await expect(page.getByRole("status")).toContainText("loaded");
  await page
    .getByLabel("Search indexed code", { exact: true })
    .fill("provenance_entry");
  await page.getByRole("button", { name: "Search", exact: true }).click();
  await expect(
    page.getByRole("region", { name: "Search results" }),
  ).toContainText("provenance_entry");
  const original = await readFile(selectedConfig, "utf8");
  try {
    // The destination is a different real indexed repository, not a nonexistent path.
    await writeFile(selectedConfig, "[repository]\nroot='../second'\n");
    const response = await page.request.get(
      `${apiOrigin}/api/v1/repos/${ids[selectedIndex]}/status`,
      {
        headers: { Authorization: `Bearer ${token}` },
      },
    );
    expect(response.status()).toBe(403);
    const error = await response.json();
    expect(error.error.code).toBe("REPO_FORBIDDEN");
    expect(JSON.stringify(error)).not.toContain("entry_second");
    expect(response.headers()["x-request-id"]).toBe(error.request_id);
    await page.getByRole("button", { name: "Search", exact: true }).click();
    await expect(page.getByRole("alert")).toContainText("REPO_FORBIDDEN");
  } finally {
    await writeFile(selectedConfig, original);
  }
  await page.getByRole("button", { name: "Search", exact: true }).click();
  await expect(
    page.getByRole("region", { name: "Search results" }),
  ).toContainText("provenance_entry");
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("actual hostile symbol names remain inert XML text in exported SVG", async ({
  page,
}) => {
  await connect(page);
  await page
    .getByLabel("Search indexed code", { exact: true })
    .fill("svg_hostile");
  await page.getByRole("button", { name: "Search", exact: true }).click();
  await page
    .getByRole("region", { name: "Search results" })
    .getByRole("button")
    .filter({ hasText: "svg_hostile" })
    .click();
  await expect(
    page.getByRole("region", { name: "Symbol details" }),
  ).toContainText("labels<&.ts");
  const downloadPromise = page.waitForEvent("download");
  await page.getByRole("button", { name: "Export SVG", exact: true }).click();
  const path = await (await downloadPromise).path();
  const svg = await readFile(path!, "utf8");
  const parsed = await page.evaluate((text) => {
    const doc = new DOMParser().parseFromString(text, "image/svg+xml");
    return {
      errors: doc.getElementsByTagName("parsererror").length,
      scripts: doc.getElementsByTagName("script").length,
      foreign: doc.getElementsByTagName("foreignObject").length,
      events: Array.from(doc.querySelectorAll("*")).flatMap((n) =>
        n.getAttributeNames().filter((a) => a.startsWith("on")),
      ),
      labels: Array.from(
        doc.getElementsByTagName("text"),
        (n) => n.textContent,
      ),
      titles: Array.from(
        doc.getElementsByTagName("title"),
        (n) => n.textContent,
      ).join("\n"),
    };
  }, svg);
  expect(parsed.errors).toBe(0);
  expect(parsed.scripts).toBe(0);
  expect(parsed.foreign).toBe(0);
  expect(parsed.events).toEqual([]);
  expect(parsed.labels).toContain(
    hostileLabel.slice(0, 64).replace("\u0001", "\uFFFD"),
  );
  expect(parsed.titles).toContain("labels<&.ts");
  expect(parsed.titles).toContain("\uFFFD🔒");
  expect(
    await page.evaluate(() =>
      Object.prototype.hasOwnProperty.call(window, "__injected"),
    ),
  ).toBe(false);
  expect(
    (
      await new AxeBuilder({ page })
        .withTags(["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa"])
        .analyze()
    ).violations,
  ).toEqual([]);
});

test("large indexed graph is bounded, explicitly truncated, exportable and keyboard navigable", async ({
  page,
}) => {
  await connect(page);
  await page
    .getByLabel("Repository", { exact: true })
    .selectOption(ids[largeIndex]);
  await page.getByLabel("Maximum traversal nodes", { exact: true }).fill("60");
  await page
    .getByLabel("Search indexed code", { exact: true })
    .fill("large_hub");
  await page.getByRole("button", { name: "Search", exact: true }).click();
  await page.getByRole("button", { name: "large_hub", exact: true }).click();
  await expect(
    page.getByRole("img", { name: /Dependency graph with/ }),
  ).toBeVisible();
  await expect(page.getByText(/Traversal truncated/)).toBeVisible();
  const downloadPromise = page.waitForEvent("download");
  await page.getByRole("button", { name: "Export JSON", exact: true }).click();
  const path = await (await downloadPromise).path();
  const graph = JSON.parse(await readFile(path!, "utf8"));
  expect(graph.repo_id).toBe(ids[largeIndex]);
  expect(graph.truncated).toBe(true);
  expect(graph.data.nodes.length).toBeLessThanOrEqual(60);
  expect(graph.data.nodes.length).toBeGreaterThan(20);
  const included = new Set(
    graph.data.nodes.map((n: { symbol_id: string }) => n.symbol_id),
  );
  for (const edge of graph.data.edges) {
    expect(included.has(edge.source_id)).toBe(true);
    if (edge.target_id) expect(included.has(edge.target_id)).toBe(true);
  }
  await page.getByText("Keyboard graph navigation", { exact: true }).click();
  await expect(page.locator(".node-list button")).toHaveCount(
    graph.data.nodes.length,
  );
  await page.locator(".node-list button").first().focus();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("status")).toContainText(
    "Symbol details and bounded graph loaded",
  );
  expect(
    (
      await new AxeBuilder({ page })
        .withTags(["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa"])
        .analyze()
    ).violations,
  ).toEqual([]);
});

test("real API rejects missing auth and browser abort recovers without retained credentials", async ({
  page,
}) => {
  await connect(page);
  const unauthorized = await page.request.get(
    `${apiOrigin}/api/v1/repos/${ids[0]}/status`,
  );
  expect(unauthorized.status()).toBe(401);
  expect((await unauthorized.json()).error.code).toBe("AUTH_REQUIRED");
  // Actual browser fetch/AbortSignal, no Playwright routing or response mocks. This
  // proves client cancellation, not forcible interruption of synchronous Rust work.
  const cancelled = await page.evaluate(
    async ({ origin, id, credential }) => {
      const controller = new AbortController();
      const request = fetch(`${origin}/api/v1/repos/${id}/query?text=entry`, {
        headers: { Authorization: `Bearer ${credential}` },
        signal: controller.signal,
      });
      controller.abort();
      try {
        await request;
        return "unexpected success";
      } catch (error) {
        return error instanceof DOMException ? error.name : "other";
      }
    },
    { origin: apiOrigin, id: ids[0], credential: token },
  );
  expect(cancelled).toBe("AbortError");
  await selectEntry(page);
  await page.getByRole("button", { name: "Disconnect", exact: true }).click();
  expect(
    await page.getByRole("region", { name: "Symbol details" }).count(),
  ).toBe(0);
  expect(
    await page.getByLabel("Bearer token", { exact: true }).inputValue(),
  ).toBe("");
  await connect(page);
  await selectEntry(page);
});

test("real architecture violation evidence exports and invalid rule paths are refused", async ({
  page,
}) => {
  await connect(page);
  await page.getByRole("tab", { name: "Rules", exact: true }).click();
  await page.getByLabel("Rules JSON", { exact: true }).fill(
    JSON.stringify({
      rules_version: "1.0",
      rules: [
        {
          id: "no-app-call",
          kind: "forbid_dependency",
          from: { path: "app.py", kinds: ["function"] },
          to: { path: "app.py", kinds: ["function"] },
          edge_kinds: ["CALLS"],
          max_depth: 1,
        },
      ],
    }),
  );
  await page
    .getByRole("button", { name: "Evaluate architecture rules", exact: true })
    .click();
  await expect(page.getByRole("tabpanel")).toContainText('"exit_code": 10');
  const downloadPromise = page.waitForEvent("download");
  await page
    .getByRole("button", {
      name: "Export architecture report JSON",
      exact: true,
    })
    .click();
  const path = await (await downloadPromise).path();
  const report = JSON.parse(await readFile(path!, "utf8"));
  expect(report.repo_id).toBe(ids[0]);
  expect(report.data.exit_code).toBe(10);
  expect(report.data.violations.length).toBeGreaterThan(0);
  expect(report.data.violations[0].edges[0].kind).toBe("CALLS");
  expect(
    report.data.violations[0].edges[0].source_span.start_line,
  ).toBeGreaterThan(0);
  await page
    .getByLabel("Rules JSON", { exact: true })
    .fill('{"rules_file":"/etc/passwd"}');
  await page
    .getByRole("button", { name: "Evaluate architecture rules", exact: true })
    .click();
  await expect(page.getByRole("alert")).toContainText("INVALID_INPUT");
  await expect(page.getByRole("alert")).not.toContainText("root:x:");
});

test("generation pin rejects changed index until explicit browser reload", async ({
  page,
}) => {
  await connect(page);
  await page.getByLabel("Repository", { exact: true }).selectOption(ids[1]);
  await selectEntry(page, "second");
  const root = join(directory, "second");
  const source = join(root, "app.py");
  const original = await readFile(source, "utf8");
  try {
    await writeFile(source, original + "\ndef generation_added():\n    pass\n");
    cli(root, ["index", "."]);
    await page
      .getByLabel("Search indexed code", { exact: true })
      .fill("generation_added");
    await page.getByRole("button", { name: "Search", exact: true }).click();
    await expect(page.getByRole("alert")).toContainText("GENERATION_MISMATCH");
    await expect(
      page.getByRole("region", { name: "Search results" }),
    ).not.toContainText("generation_added");
    await page.getByRole("tab", { name: "Status", exact: true }).click();
    await page
      .getByRole("button", { name: "Reload indexed generation", exact: true })
      .click();
    await expect(page.getByRole("alert")).toHaveCount(0);
    await page.getByRole("tab", { name: "Explore", exact: true }).click();
    await page.getByRole("button", { name: "Search", exact: true }).click();
    await expect(
      page.getByRole("region", { name: "Search results" }),
    ).toContainText("generation_added");
  } finally {
    await writeFile(source, original);
    cli(root, ["index", "."]);
  }
});
