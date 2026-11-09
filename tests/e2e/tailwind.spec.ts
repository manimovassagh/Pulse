import { test, expect } from "@playwright/test";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { copyExample, pulse, startServer, type PulseServer } from "./support/pulse";

// Tailwind's default palette, as computed by the browser.
const SLATE_900 = "oklch(0.208 0.042 265.755)";
const PINK_600 = "oklch(0.592 0.249 0.584)";
const GREEN_600 = "oklch(0.627 0.194 149.214)";

test.describe("tailwind example", () => {
  let app: ReturnType<typeof copyExample>;
  let server: PulseServer | undefined;

  test.beforeEach(() => {
    app = copyExample("tailwind");
  });

  test.afterEach(async () => {
    await server?.stop();
    server = undefined;
    app?.cleanup();
  });

  test("production build: Tailwind classes, reactive classes and public assets", async ({ page }) => {
    pulse(["build", "src/App.tsx", "--out", "dist"], app.dir);
    server = await startServer(["serve", join(app.dir, "dist"), "--port", "4600"], app.dir);
    await page.goto(server.url);

    const like = page.locator("#like");
    await expect(like).toHaveText("♡ Like");
    await expect(like).toHaveCSS("background-color", SLATE_900);
    await expect(page.locator("h1")).toHaveCSS("font-weight", "700");

    await like.click();
    await page.mouse.move(0, 0); // leave the button so `hover:` styles do not apply
    await expect(like).toHaveText("♥ 1");
    await expect(like).toHaveCSS("background-color", PINK_600); // class swapped by a signal

    const card = page.locator("#card");
    await expect(card).toHaveCSS("background-color", "rgb(255, 255, 255)");
    await page.locator("#theme").click();
    await expect(card).toHaveCSS("background-color", SLATE_900); // `dark:` variant

    // public/logo.svg is copied next to the app and loads.
    const logo = page.locator("img[alt='Pulse logo']");
    await expect.poll(() => logo.evaluate((img: HTMLImageElement) => img.naturalWidth)).toBeGreaterThan(0);
  });

  test("dev: classes added in TSX are generated on save", async ({ page }) => {
    server = await startServer(["dev", "src/App.tsx", "--port", "4650"], app.dir);
    await page.goto(server.url);
    const title = page.locator("h1");
    await expect(title).toHaveText("Pulse + Tailwind");
    await page.locator("#like").click();

    const file = join(app.dir, "src/App.tsx");
    const source = readFileSync(file, "utf8");
    writeFileSync(file, source.replace('<h1 class="text-2xl', '<h1 class="text-green-600 text-2xl'));

    await expect(title).toHaveCSS("color", GREEN_600);
    await expect(page.locator("#like")).toHaveText("♥ 1"); // global state kept
  });
});
