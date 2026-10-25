const { test: base, expect } = require("@playwright/test");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const { mkdtemp, rm } = require("node:fs/promises");
const { tmpdir } = require("node:os");
const { join, resolve } = require("node:path");
const net = require("node:net");

// Keep this scenario's sign-in outside the shared server's real login rate limit.
const test = base.extend({
  importApp: async ({ browser }, use) => {
    const directory = await mkdtemp(join(tmpdir(), "scrobbl-import-total-"));
    const socket = net.createServer();
    socket.listen(0, "127.0.0.1");
    await once(socket, "listening");
    const port = socket.address().port;
    await new Promise((resolve) => socket.close(resolve));
    const baseURL = `http://127.0.0.1:${port}`;
    const child = spawn(
      resolve(
        "target/debug/scrobbl-rs" +
          (process.platform === "win32" ? ".exe" : ""),
      ),
      [],
      {
        windowsHide: true,
        env: {
          ...process.env,
          SCROBBL_PORT: String(port),
          SCROBBL_BIND: "127.0.0.1",
          SCROBBL_DATA_DIR: directory,
          SCROBBL_EXTERNAL_ORIGIN: baseURL,
          SPOTIFY_CLIENT_ID: "",
          SPOTIFY_CLIENT_SECRET: "",
        },
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
    let logs = "";
    child.stdout.on("data", (data) => (logs += data));
    child.stderr.on("data", (data) => (logs += data));
    const context = await browser.newContext({ baseURL });
    try {
      await expect
        .poll(async () => {
          if (child.exitCode !== null) throw new Error(logs);
          try {
            return (await context.request.get("/healthz")).status();
          } catch {
            return 0;
          }
        })
        .toBe(200);
      await use({ page: await context.newPage(), baseURL });
    } finally {
      await context.close().catch(() => {});
      if (child.exitCode === null) {
        const stopped = once(child, "exit");
        child.kill("SIGINT");
        await stopped;
      }
      await rm(directory, { recursive: true, force: true });
    }
  },
});

test("Import and activity detail show the whole total, counting and unavailable states", async ({
  importApp: { page, baseURL },
}) => {
  const session = await (await page.request.get("/api/session")).json();
  const signedIn = await page.request.post(
    session.setup_required ? "/api/setup" : "/api/login",
    {
      data: { username: "owner", password: "a local browser test password 7!" },
      headers: { Origin: baseURL },
    },
  );
  expect(signedIn.ok()).toBeTruthy();
  const activity = {
    id: "import-total-fixture",
    created_at_ms: Date.now(),
    kind: "spotify_import",
    state: "running",
    filename: "Synthetic history",
    attempt: 1,
    phase: "counting",
    inventory_complete: true,
    expected_rows: null,
    processed: 0,
    added: 0,
    originals: 0,
    variants: 0,
    reconciled: 0,
    duplicates: 0,
    invalid: 0,
    skipped: 0,
    ambiguous: 0,
    below_threshold: 0,
    retry_at_ms: null,
    error_code: null,
    outcome: null,
    target_timezone: null,
    target_week_start: null,
  };
  await page.route("**/api/import/spotify", (route) =>
    route.fulfill({ json: { job_id: activity.id } }),
  );
  await page.route("**/api/v1/activities/import-total-fixture", (route) =>
    route.fulfill({
      json: { activity, members: [], issues: [], source_parameters: null },
    }),
  );
  await page.goto("/import");
  await page.locator("#import-files").setInputFiles({
    name: "synthetic.json",
    mimeType: "application/json",
    buffer: Buffer.from("[]"),
  });
  await page.getByRole("button", { name: "Import listening history" }).click();
  const result = page.locator("#import-result");
  await expect(result).toContainText("Counting source rows across all files");
  await expect(result).toContainText(
    "Total available after all files are counted",
  );
  await expect(result.getByRole("progressbar")).not.toHaveAttribute("value");
  await expect(result).not.toContainText("of 0");

  Object.assign(activity, {
    phase: "processing",
    expected_rows: 6000,
    processed: 2000,
    added: 100,
  });
  await expect(result).toContainText("2,000 of 6,000 processed");
  await expect(result.getByRole("progressbar")).toHaveAttribute("max", "6000");
  await expect(result.getByRole("progressbar")).toHaveAttribute(
    "value",
    "2000",
  );
  await expect(result).toContainText("100 added");
  await page
    .getByRole("link", { name: "Open full saved report and retry controls" })
    .click();
  const detail = page.locator(".activity-detail");
  await expect(detail).toContainText("2,000 of 6,000 processed");

  Object.assign(activity, { phase: "counting", expected_rows: null });
  await expect(detail).toContainText("Counting source rows across all files");
  await expect(detail.getByRole("progressbar")).not.toHaveAttribute("value");
  Object.assign(activity, { phase: "processing" });
  await expect(detail).toContainText(
    "2,000 source rows processed · total unavailable: some files could not be counted",
  );
  await expect(detail.getByRole("progressbar")).not.toHaveAttribute("value");
  await page.screenshot({
    path: "test-results/import-total-unavailable.png",
    fullPage: true,
  });

  Object.assign(activity, {
    phase: "finished",
    state: "completed",
    outcome: "no_music",
    expected_rows: 0,
    processed: 0,
    added: 0,
  });
  await expect(detail).toContainText("0 of 0 processed");
  await expect(detail.getByRole("progressbar")).toHaveCount(0);
});
