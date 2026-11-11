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
    const deep = join(root, ...Array<string>(65).fill("d"));
    await mkdir(deep, { recursive: true });
    await writeFile(join(deep, "deep.py"), "def deep_evidence():\n    pass\n");
    fixtureGit(root, ["add", "--", "app.py"]);
    fixtureGit(root, ["commit", "--quiet", "-m", "local browser fixture"]);
    ids.push(cli(root, ["index", "."]).repo_id);
  }
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
