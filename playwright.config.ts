import { defineConfig, devices } from "@playwright/test";

// The UI runs in a plain browser against the mock backend in src/mock.ts, so these tests need no desktop app.
export default defineConfig({
  testDir: "e2e",
  fullyParallel: true,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["github"], ["list"]] : "list",
  use: { baseURL: "http://localhost:1420", trace: "retain-on-failure", viewport: { width: 1280, height: 800 } },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 800 } } }],
  webServer: { command: "npm run dev", url: "http://localhost:1420", reuseExistingServer: !process.env.CI, timeout: 60_000 },
});
