const { defineConfig } = require("@playwright/test");
module.exports = defineConfig({
  testDir: "./tests",
  testMatch: "*.spec.cjs",
  workers: 1,
  timeout: 60000,
  use: {
    baseURL: `http://127.0.0.1:${process.env.SCROBBL_E2E_PORT || 3211}`,
    headless: true,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  webServer: {
    command: "node scripts/e2e-server.cjs",
    url: `http://127.0.0.1:${process.env.SCROBBL_E2E_PORT || 3211}`,
    reuseExistingServer: false,
    timeout: 120000,
    gracefulShutdown: { signal: "SIGTERM", timeout: 10000 },
  },
});
