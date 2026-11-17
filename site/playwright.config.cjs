const { defineConfig, devices } = require("@playwright/test");

const port = process.env.SITE_PORT || 8788;

module.exports = defineConfig({
  testDir: "./tests",
  testMatch: "*.spec.cjs",
  workers: process.env.CI ? 4 : undefined,
  timeout: 30000,
  expect: { timeout: 10000 },
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    headless: true,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    { name: "firefox", use: { ...devices["Desktop Firefox"] } },
    { name: "webkit", use: { ...devices["Desktop Safari"] } },
  ],
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  webServer: {
    command: `npx wrangler dev --port ${port} --ip 127.0.0.1`,
    url: `http://127.0.0.1:${port}/`,
    reuseExistingServer: false,
    timeout: 120000,
    gracefulShutdown: { signal: "SIGTERM", timeout: 10000 },
  },
});
