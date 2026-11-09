import { test, expect, type Page } from "@playwright/test";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { copyExample, startServer, type PulseServer } from "./support/pulse";

// `pulse dev`: edits are compiled and hot-swapped into the open page.
test.describe("pulse dev hot reload", () => {
  let app: ReturnType<typeof copyExample>;
  let server: PulseServer;

  const appFile = () => join(app.dir, "src/App.tsx");
  const edit = (file: string, from: string | RegExp, to: string) => {
    const before = readFileSync(file, "utf8");
    const after = before.replace(from, to);
    expect(after, `edit of ${file} did not change anything`).not.toBe(before);
    writeFileSync(file, after);
  };
  /** Marks the current page so a full reload can be detected. */
  const markPage = (page: Page) => page.evaluate(() => ((window as any).__pulseMarker = true));
  const pageWasKept = (page: Page) => page.evaluate(() => (window as any).__pulseMarker === true);

  test.beforeEach(async () => {
    app = copyExample("counter");
    server = await startServer(["dev", "src/App.tsx", "--port", "4500"], app.dir);
  });

  test.afterEach(async ({}, testInfo) => {
    if (testInfo.status !== testInfo.expectedStatus) {
      await testInfo.attach("pulse dev log", { body: server.log(), contentType: "text/plain" });
    }
    await server?.stop();
    app?.cleanup();
  });

  test("code edits hot-swap in place and keep global state", async ({ page }) => {
    await page.goto(server.url);
    const increment = page.locator("#increment");
    for (let i = 0; i < 3; i++) await increment.click();
    await expect(increment).toHaveText("Count: 3");
    await markPage(page);

    edit(appFile(), "<h1>Pulse Counter</h1>", "<h1>Hot Counter</h1>");

    await expect(page.locator("h1")).toHaveText("Hot Counter");
    await expect(increment).toHaveText("Count: 3"); // global signal survived
    expect(await pageWasKept(page)).toBe(true); // no page reload

    // The new module is fully live, not a static copy.
    await increment.click();
    await expect(increment).toHaveText("Count: 4");
    await expect(page.locator(".stats")).toHaveText("Doubled: 8 · Parity: even");
  });

  test("compile errors show an overlay that clears when fixed", async ({ page }) => {
    await page.goto(server.url);
    await expect(page.locator("h1")).toHaveText("Pulse Counter");
    const overlay = page.locator("#__pulse_error_overlay");

    edit(appFile(), "const count = signal<number>(0);", 'const count = signal<number>("zero");');
    await expect(overlay).toBeVisible();
    await expect(overlay).toContainText("type mismatch: expected `number`, found `string`");
    await expect(overlay).toContainText("App.tsx:4:30");

    edit(appFile(), 'signal<number>("zero")', "signal<number>(10)");
    await expect(overlay).toBeHidden();
    await expect(page.locator("#increment")).toHaveText("Count: 0"); // restored state wins over new initial value
  });

  test("index.html edits trigger a full page reload", async ({ page }) => {
    await page.goto(server.url);
    await expect(page.locator("h1")).toHaveText("Pulse Counter");
    await markPage(page);

    edit(join(app.dir, "index.html"), "<title>Pulse Counter</title>", "<title>Edited</title>");

    await expect(page).toHaveTitle("Edited");
    expect(await pageWasKept(page)).toBe(false);
    await expect(page.locator("#increment")).toHaveText("Count: 0");
  });
});
