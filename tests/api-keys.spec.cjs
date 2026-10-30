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
    const directory = await mkdtemp(join(tmpdir(), "scrobbl-api-keys-"));
    const socket = net.createServer();
    socket.listen(0, "127.0.0.1");
    await once(socket, "listening");
    const port = socket.address().port;
    await new Promise((resolve) => socket.close(resolve));
    const baseURL = `http://127.0.0.1:${port}`;
    const child = spawn(
      resolve(
        process.env.CARGO_TARGET_DIR || "target",
        "debug",
        "scrobbl-rs" + (process.platform === "win32" ? ".exe" : ""),
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

test("API key lifecycle, one-time secret, explicit origins and mobile layout", async ({
  privateApp: { page, baseURL },
}) => {
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto("/settings/api-keys");
  const session = await (await page.request.get("/api/session")).json();
  await page.getByLabel("Username", { exact: true }).fill("owner");
  await page
    .getByLabel("Password", { exact: true })
    .fill("a local browser test password 7!");
  if (session.setup_required)
    await page
      .getByLabel("Confirm password")
      .fill("a local browser test password 7!");
  await page
    .getByRole("button", {
      name: session.setup_required ? "Create your archive" : "Sign in",
      exact: true,
    })
    .click();
  await expect(
    page.getByRole("heading", { name: "API keys", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText(
      "No origins saved yet. Enter and save an origin to allow browser requests.",
      { exact: true },
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "API keys", exact: true }),
  ).toBeVisible();
  await page
    .getByLabel("Allowed origins", { exact: true })
    .fill("https://itsmrma.github.io");
  await page.getByRole("button", { name: "Save allowed origins" }).click();
  await expect(
    page.getByRole("status").filter({ hasText: "Allowed origins saved." }),
  ).toBeVisible();
  await expect(
    page.getByText("Saved: https://itsmrma.github.io", { exact: true }),
  ).toBeVisible();
  const name = `Browser test ${Date.now()}`;
  await page.getByLabel("Key name", { exact: true }).fill(name);
  await page.getByRole("button", { name: "Create read-only key" }).click();
  const secret = await page
    .getByLabel("New key secret", { exact: true })
    .inputValue();
  expect(secret).toMatch(/^[a-f0-9]{64}$/);
  await expect(
    page.getByText("Active · No expiry", { exact: true }).first(),
  ).toBeVisible();
  const keyHeaders = { Authorization: `Bearer ${secret}` };
  const read = await page.request.get(`${baseURL}/api/v1/listens`, {
    headers: keyHeaders,
  });
  expect(read.status()).toBe(200);
  expect((await read.json()).complete).toBe(true);
  await page.getByRole("link", { name: "General", exact: true }).click();
  await page.getByRole("link", { name: "API keys", exact: true }).click();
  await expect(page.getByLabel("New key secret", { exact: true })).toHaveCount(
    0,
  );
  await expect(page.getByLabel("Allowed origins", { exact: true })).toHaveValue(
    "https://itsmrma.github.io",
  );
  await expect(page.getByText(name, { exact: true })).toBeVisible();
  await page.reload();
  await expect(page.getByLabel("New key secret", { exact: true })).toHaveCount(
    0,
  );
  await expect(page.getByLabel("API base URL", { exact: true })).toHaveValue(
    `${baseURL}/api/v1`,
  );
  // 1. Cancel via button keeps the key active
  await page
    .getByRole("button", { name: `Revoke ${name}`, exact: true })
    .click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText(`Revoke “${name}”?`);
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(dialog).not.toBeVisible();
  await expect(
    page.getByRole("button", { name: `Revoke ${name}`, exact: true }),
  ).toBeVisible();
  expect(
    (
      await page.request.get(`${baseURL}/api/v1/info`, { headers: keyHeaders })
    ).status(),
  ).toBe(200);

  // 2. Cancel via keyboard Escape keeps the key active
  await page
    .getByRole("button", { name: `Revoke ${name}`, exact: true })
    .click();
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).not.toBeVisible();
  await expect(
    page.getByRole("button", { name: `Revoke ${name}`, exact: true }),
  ).toBeVisible();

  // 3. Explicit confirmation revokes the key
  await page
    .getByRole("button", { name: `Revoke ${name}`, exact: true })
    .click();
  await expect(dialog).toBeVisible();
  await dialog
    .getByRole("button", { name: "Confirm revocation", exact: true })
    .click();
  await expect(dialog).not.toBeVisible();
  await expect(
    page.getByRole("button", { name: `Revoke ${name}`, exact: true }),
  ).toHaveCount(0);
  await expect(page.getByText(/Revoked/)).toBeVisible();
  expect(
    (
      await page.request.get(`${baseURL}/api/v1/info`, { headers: keyHeaders })
    ).status(),
  ).toBe(401);
  await expect(
    page.getByRole("button", { name: "Create read-only key" }),
  ).toBeEnabled();
  await page
    .getByRole("status")
    .filter({ hasText: "Key revoked." })
    .getByRole("button", { name: "Dismiss notification" })
    .click();
  await page.getByRole("heading", { name: "API keys", exact: true }).click();
  await page.setViewportSize({ width: 1280, height: 900 });
  mkdirSync("docs/screenshots", { recursive: true });
  await page.screenshot({
    path: "docs/screenshots/api-keys-ready-desktop.png",
    fullPage: true,
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({
    path: "docs/screenshots/api-keys-ready-mobile.png",
    fullPage: true,
  });
  // A failed create keeps the form and exposes a readable error, without a secret.
  await page.route("**/api/v1/api-keys", (route) =>
    route.request().method() === "POST"
      ? route.fulfill({
          status: 500,
          json: { error: "Test creation failed. Retry." },
        })
      : route.continue(),
  );
  await page.getByLabel("Key name", { exact: true }).fill("Failed key");
  await page.getByRole("button", { name: "Create read-only key" }).click();
  await expect(
    page.getByRole("alert").filter({ hasText: "Test creation failed." }),
  ).toBeVisible();
  await expect(page.getByLabel("Key name", { exact: true })).toHaveValue(
    "Failed key",
  );
  await page.unroute("**/api/v1/api-keys");
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  expect(errors).toEqual([]);
});
