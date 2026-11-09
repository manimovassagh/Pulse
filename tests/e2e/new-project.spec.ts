import { test, expect } from "@playwright/test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pulse, startServer, type PulseServer } from "./support/pulse";

// The first-run experience: `pulse new`, then commands without arguments.
test("pulse new creates an app that runs with plain `pulse dev`", async ({ page }) => {
  const workspace = mkdtempSync(join(tmpdir(), "pulse-new-"));
  const app = join(workspace, "my-app");
  let server: PulseServer | undefined;
  try {
    pulse(["new", "my-app"], workspace);
    server = await startServer(["dev", "--port", "4900"], app);

    await page.goto(server.url);
    await expect(page).toHaveTitle("my-app");
    await expect(page.locator("h1")).toHaveText("Hello, Pulse");
    const button = page.locator("button");
    // The template's stylesheet is applied (checked before hovering).
    await expect(button).toHaveCSS("background-color", "rgb(14, 165, 233)");
    await button.click();
    await expect(button).toHaveText("Count: 1");
    await expect(page.locator("img.logo")).toHaveJSProperty("complete", true);
  } finally {
    await server?.stop();
    rmSync(workspace, { recursive: true, force: true });
  }
});
