const { test: base, expect } = require("@playwright/test");
const { mkdirSync } = require("node:fs");
const { mkdtemp, rm } = require("node:fs/promises");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const { tmpdir } = require("node:os");
const { join, resolve } = require("node:path");
const net = require("node:net");

// Own storage and process: this scenario must not create the owner used by local.spec.
const test = base.extend({
  privateApp: async ({ browser }, use) => {
    const directory = await mkdtemp(join(tmpdir(), "scrobbl-listenbrainz-"));
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

test("ListenBrainz upload runs in background, preserves compact provenance and partial reports, and reimports safely", async ({
  privateApp: { page, baseURL },
  browser,
}) => {
  await page.goto("/import");
  const session = await (await page.request.get("/api/session")).json();
  await page.getByLabel("Username", { exact: true }).fill("owner");
  await page
    .getByLabel("Password", { exact: true })
    .fill("a local browser test password");
  if (session.setup_required)
    await page
      .getByLabel("Confirm password")
      .fill("a local browser test password");
  await page
    .getByRole("button", {
      name: session.setup_required ? "Create your archive" : "Sign in",
      exact: true,
    })
    .click();
  await expect(
    page.getByRole("heading", { name: "ListenBrainz", exact: true }),
  ).toBeVisible();
  const baseline = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  const rows = [
    {
      listened_at: 1700000000,
      track_metadata: {
        artist_name: "LB artist",
        track_name: "LB actual",
        additional_info: { duration_played: 30 },
      },
    },
    {
      listened_at: 1700000300,
      track_metadata: {
        artist_name: "LB artist",
        track_name: "<script>window.lbBad=1</script>",
      },
    },
    {
      listened_at: 1700000600,
      track_metadata: {
        artist_name: "LB artist",
        track_name: "LB short",
        additional_info: { duration_played: 0, duration_ms: 180000 },
      },
    },
    {
      listened_at: 1700000900,
      track_metadata: {
        artist_name: "LB artist",
        track_name: "LB estimated",
        additional_info: { duration_ms: 20000, duration_played: "malformed" },
      },
    },
  ];
  const buffer = Buffer.from(
    rows.map((r) => JSON.stringify(r)).join("\n") + "\n{broken row\n",
  );
  await page
    .getByLabel("ListenBrainz username for files (optional)")
    .fill("browser-sample");
  await page.getByLabel("ListenBrainz history files").setInputFiles({
    name: "browser-history.jsonl",
    mimeType: "application/json",
    buffer,
  });
  await page
    .getByRole("button", { name: "Upload ListenBrainz files", exact: true })
    .click();
  const activity = page
    .getByRole("link", { name: "View ListenBrainz activity", exact: true })
    .first();
  await expect(activity).toBeVisible();
  const path = await activity.getAttribute("href");
  // Navigate away from the import page while the server processes the received file.
  await page.goto("/settings/general");
  await page.goto(path);
  await expect(
    page.getByText("Import finished with rejected rows", { exact: false }),
  ).toBeVisible();
  await expect(
    page.getByText(/browser-history.jsonl.*row 5: Invalid JSONL row/),
  ).toBeVisible();
  await expect(page.getByText(/row 4: Invalid duration_played/)).toBeVisible();
  await page.getByRole("button", { name: "Read saved source rows" }).click();
  await expect(
    page.getByText("5 saved source rows", { exact: false }),
  ).toBeVisible();
  await page
    .getByText(/browser-history.jsonl.*row 2.*accepted/)
    .last()
    .click();
  await expect(
    page.locator("details[open] > p").filter({
      hasText: "Raw row payload unavailable under compact storage.",
    }),
  ).toBeVisible();
  expect(await page.evaluate(() => window.lbBad)).toBeUndefined();
  const dashboard = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  expect(dashboard.totals.listens - baseline.totals.listens).toBe(3);
  expect(dashboard.totals.actual_ms - baseline.totals.actual_ms).toBe(30000);
  expect(dashboard.totals.estimated_ms - baseline.totals.estimated_ms).toBe(
    20000,
  );
  expect(
    dashboard.totals.unknown_durations - baseline.totals.unknown_durations,
  ).toBe(1);
  await page.screenshot({
    path: "test-results/listenbrainz-report-desktop.png",
    fullPage: true,
  });
  await page.goto("/import");
  await page
    .getByLabel("ListenBrainz username for files (optional)")
    .fill("browser-sample");
  await page.getByLabel("ListenBrainz history files").setInputFiles({
    name: "renamed-history.jsonl",
    mimeType: "application/json",
    buffer,
  });
  await page
    .getByRole("button", { name: "Upload ListenBrainz files", exact: true })
    .click();
  await page
    .getByRole("link", { name: "View ListenBrainz activity", exact: true })
    .first()
    .click();
  await expect(
    page.getByText("Import finished with rejected rows", { exact: false }),
  ).toBeVisible();
  const jobs = await (await page.request.get("/api/v1/activities")).json();
  const reimport = jobs.find((j) => j.filename === "renamed-history.jsonl");
  expect(reimport.added).toBe(0);
  expect(reimport.duplicates).toBe(4);
  // Large public requests are rejected locally with the ZIP action, before any API call.
  await page.goto("/import");
  await page
    .getByRole("button", { name: "Fetch public listens", exact: true })
    .click();
  await page.getByLabel("Public ListenBrainz username").fill("browser-sample");
  await page.getByLabel("ListenBrainz start date").fill("2020-01-01");
  await page
    .getByRole("button", { name: "Fetch public ListenBrainz listens" })
    .click();
  await expect(
    page.getByText("For more than 31 days", { exact: false }).first(),
  ).toBeVisible();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({
    path: "test-results/listenbrainz-import-mobile.png",
    fullPage: true,
  });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth + 1,
    ),
  ).toBe(true);
  // Authentication is required to read original source data.
  const anonymous = await browser.newContext({
    baseURL,
  });
  const response = await anonymous.request.get(
    `/api/v1/listenbrainz/jobs/${reimport.id}/originals`,
  );
  expect(response.status()).toBe(401);
  await anonymous.close();
});
