import { test, expect, type ConsoleMessage } from "@playwright/test";
import { join } from "node:path";
import { copyExample, pulse, startServer, type PulseServer } from "./support/pulse";

// The milestone-1 demo: a production build served by `pulse serve`.
test.describe("counter example (production build)", () => {
  let app: ReturnType<typeof copyExample>;
  let server: PulseServer;

  test.beforeAll(async () => {
    app = copyExample("counter");
    pulse(["build", "src/App.tsx", "--out", "dist"], app.dir);
    server = await startServer(["serve", join(app.dir, "dist"), "--port", "4400"], app.dir);
  });

  test.afterAll(async () => {
    await server?.stop();
    app?.cleanup();
  });

  test("renders, reacts to clicks and batches effects", async ({ page }) => {
    const logs: string[] = [];
    const errors: string[] = [];
    page.on("console", (m: ConsoleMessage) => (m.type() === "error" ? errors : logs).push(m.text()));
    page.on("pageerror", (e) => errors.push(String(e)));

    await page.goto(server.url);
    const increment = page.locator("#increment");
    const step = page.locator("#step");
    const reset = page.locator("#reset");

    await test.step("initial render", async () => {
      await expect(page.locator("h1")).toHaveText("Pulse Counter");
      await expect(increment).toHaveText("Count: 0");
      await expect(page.locator("#label")).toHaveText("You clicked 0 clicks.");
      await expect(page.locator(".stats")).toHaveText("Doubled: 0 · Parity: even");
      await expect(reset).toBeDisabled();
      // Static and dynamic text are merged into a single DOM text node.
      expect(await increment.evaluate((b) => b.childNodes.length)).toBe(1);
    });

    await test.step("click updates dependent text, computed values and attributes", async () => {
      await increment.click();
      await expect(increment).toHaveText("Count: 1");
      await expect(page.locator("#label")).toHaveText("You clicked 1 click.");
      await expect(page.locator(".stats")).toHaveText("Doubled: 2 · Parity: odd");
      await expect(reset).toBeEnabled();
    });

    await test.step("local signal drives a global update", async () => {
      await step.click();
      await step.click();
      await expect(step).toHaveText("Step: 3");
      await increment.click();
      await expect(increment).toHaveText("Count: 4");
    });

    await test.step("batched reset", async () => {
      await reset.click();
      await expect(increment).toHaveText("Count: 0");
      await expect(step).toHaveText("Step: 1");
      await expect(reset).toBeDisabled();
    });

    await test.step("component-local state", async () => {
      const toggle = page.locator(".toggle");
      await expect(toggle).toHaveText("Light is OFF");
      await toggle.click();
      await expect(toggle).toHaveText("Light is ON");
      await expect(toggle).toHaveAttribute("aria-pressed", "true");
    });

    // Initial run, +1, +3, then the batched reset logs exactly once.
    expect(logs).toEqual(["count changed to 0", "count changed to 1", "count changed to 4", "count changed to 0"]);
    expect(errors).toEqual([]);
  });
});
