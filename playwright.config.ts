import { defineConfig, devices } from "@playwright/test";

/**
 * Browser end-to-end tests. Each spec starts the `pulse` processes it needs
 * (see tests/e2e/support/pulse.ts); the CLI is compiled once in global setup.
 */
export default defineConfig({
  testDir: "tests/e2e",
  globalSetup: "./tests/e2e/support/global-setup.ts",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? "github" : "list",
  timeout: 60_000,
  use: {
    trace: "off",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
});
