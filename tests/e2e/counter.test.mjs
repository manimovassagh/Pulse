// End-to-end test: build the counter example with the `pulse` CLI, serve it,
// and drive it in headless Chromium.
//
//   cargo build -p pulse-compiler && node tests/e2e/counter.test.mjs
import { execFileSync } from "node:child_process";
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { join, extname, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

// `require` also finds a globally installed Playwright (NODE_PATH).
const { chromium } = createRequire(import.meta.url)("playwright");

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const example = join(root, "examples/counter");
const dist = join(example, "dist");

execFileSync(join(root, "target/debug/pulse"), [
  "build", join(example, "src/App.tsx"), "--out", dist, "--cache-dir", join(example, ".pulse"),
], { stdio: "inherit" });

const types = { ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm" };
const server = createServer(async (req, res) => {
  const path = join(dist, req.url === "/" ? "index.html" : req.url.slice(1));
  try {
    const body = await readFile(path);
    res.writeHead(200, { "Content-Type": types[extname(path)] ?? "application/octet-stream" });
    res.end(body);
  } catch {
    res.writeHead(404).end();
  }
});
await new Promise((r) => server.listen(0, r));
const url = `http://localhost:${server.address().port}/`;

const browser = await chromium.launch();
const page = await browser.newPage();
const logs = [];
const errors = [];
page.on("console", (m) => (m.type() === "error" ? errors : logs).push(m.text()));
page.on("pageerror", (e) => errors.push(String(e)));

let passed = 0;
async function check(name, fn) {
  await fn();
  passed++;
  console.log(`  ok - ${name}`);
}
const text = (sel) => page.locator(sel).textContent();

try {
  await page.goto(url);
  await page.waitForSelector("#increment");

  await check("renders the component tree", async () => {
    assert.equal(await text("h1"), "Pulse Counter");
    assert.equal(await text("#increment"), "Count: 0");
    assert.equal(await text("#label"), "You clicked 0 clicks.");
    assert.equal(await text(".stats"), "Doubled: 0 · Parity: even");
    // Adjacent static + dynamic text is a single DOM text node.
    assert.equal(await page.locator("#increment").evaluate((b) => b.childNodes.length), 1);
  });

  await check("click updates the UI reactively", async () => {
    await page.click("#increment");
    assert.equal(await text("#increment"), "Count: 1");
    assert.equal(await text("#label"), "You clicked 1 click.");
    assert.equal(await text(".stats"), "Doubled: 2 · Parity: odd");
  });

  await check("local signal feeds a global update", async () => {
    await page.click("#step");
    await page.click("#step");
    assert.equal(await text("#step"), "Step: 3");
    await page.click("#increment");
    assert.equal(await text("#increment"), "Count: 4");
    assert.equal(await text(".stats"), "Doubled: 8 · Parity: even");
  });

  await check("reactive boolean attribute", async () => {
    assert.equal(await page.locator("#reset").isDisabled(), false);
    await page.click("#reset");
    assert.equal(await text("#increment"), "Count: 0");
    assert.equal(await text("#step"), "Step: 1");
    assert.equal(await page.locator("#reset").isDisabled(), true);
  });

  await check("component-local state is independent", async () => {
    assert.equal(await text(".toggle"), "Light is OFF");
    await page.click(".toggle");
    assert.equal(await text(".toggle"), "Light is ON");
    assert.equal(await page.locator(".toggle").getAttribute("aria-pressed"), "true");
    assert.equal(await text("#increment"), "Count: 0");
  });

  await check("effects run and batch updates", async () => {
    // Initial run, +1, +3, then reset (batched: exactly one log for 0).
    assert.deepEqual(logs, [
      "count changed to 0",
      "count changed to 1",
      "count changed to 4",
      "count changed to 0",
    ]);
  });

  await check("no errors in the console", async () => {
    assert.deepEqual(errors, []);
  });
  console.log(`\n${passed} browser checks passed`);
} finally {
  await browser.close();
  server.close();
}
