const { test, expect } = require("@playwright/test");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const { mkdtemp, rm } = require("node:fs/promises");
const { tmpdir } = require("node:os");
const { join, resolve, dirname, basename } = require("node:path");
const net = require("node:net");

test("confirmed restore survives restart and requires new access and explicit resumption", async ({
  browser,
}) => {
  const directory = await mkdtemp(join(tmpdir(), "scrobbl-restore-browser-"));
  const socket = net.createServer();
  socket.listen(0, "127.0.0.1");
  await once(socket, "listening");
  const port = socket.address().port;
  await new Promise((resolve) => socket.close(resolve));
  const context = await browser.newContext({
    baseURL: `http://127.0.0.1:${port}`,
  });
  const page = await context.newPage();
  let child;
  let logs = "";
  async function start() {
    child = spawn(
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
          SCROBBL_EXTERNAL_ORIGIN: `http://127.0.0.1:${port}`,
          SPOTIFY_CLIENT_ID: "",
          SPOTIFY_CLIENT_SECRET: "",
        },
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
    child.stdout.on("data", (data) => {
      logs += data;
    });
    child.stderr.on("data", (data) => {
      logs += data;
    });
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
  }
  async function stop() {
    if (child && child.exitCode === null) {
      const exited = once(child, "exit");
      child.kill("SIGTERM");
      await exited;
    }
  }
  try {
    await start();
    await page.goto("/");
    await page
      .getByLabel("Password", { exact: true })
      .fill("original browser password");
    await page.getByLabel("Confirm password").fill("original browser password");
    await page.getByRole("button", { name: "Create your archive" }).click();
    await expect(
      page.getByRole("heading", { name: "Your listening, at a glance." }),
    ).toBeVisible();
    await page.goto("/settings/data");
    await page
      .getByRole("button", { name: "Create backup", exact: true })
      .click();
    const backup = page
      .locator(".job")
      .filter({ hasText: "Archive backup" })
      .first();
    await expect(
      backup.getByRole("button", { name: "Restore…", exact: true }),
    ).toBeVisible();
    const oldCookies = await context.cookies();
    await backup.getByRole("button", { name: "Restore…", exact: true }).click();
    await page
      .getByLabel("New owner password", { exact: true })
      .fill("new browser recovery password");
    await page
      .getByLabel("Repeat new password", { exact: true })
      .fill("new browser recovery password");
    await page
      .getByLabel("I confirm replacement of the current archive")
      .check();
    await page
      .getByRole("button", {
        name: "Confirm replacement and prepare restart",
        exact: true,
      })
      .click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(
      page
        .getByRole("status")
        .filter({ hasText: "Restore is prepared and writes are paused" }),
    ).toBeVisible();
    await page.reload();
    await expect(
      page
        .getByRole("status")
        .filter({ hasText: "Restore is prepared and writes are paused" }),
    ).toBeVisible();
    await stop();
    await start();
    await context.addCookies(oldCookies);
    expect((await context.request.get("/api/v1/recovery")).status()).toBe(401);
    await page.goto("/settings/data");
    await expect(
      page.getByRole("button", { name: "Sign in", exact: true }),
    ).toBeVisible();
    await page
      .getByLabel("Password", { exact: true })
      .fill("new browser recovery password");
    await page.getByRole("button", { name: "Sign in", exact: true }).click();
    await expect(
      page.getByRole("button", { name: "Sign in", exact: true }),
    ).toHaveCount(0);
    await page.goto("/settings/data");
    await expect(
      page.getByRole("heading", {
        name: "Archive restored — collection and pending jobs are paused",
      }),
    ).toBeVisible();
    await page
      .getByRole("button", {
        name: "Resume collection and pending jobs",
        exact: true,
      })
      .click();
    await expect(
      page.getByRole("heading", {
        name: "Archive restored — collection and pending jobs are paused",
      }),
    ).toHaveCount(0);
  } finally {
    await stop();
    await context.close();
    // directory is the exact fresh mkdtemp result; no user archive is referenced.
    if (
      dirname(resolve(directory)) !== resolve(tmpdir()) ||
      !basename(directory).startsWith("scrobbl-restore-browser-")
    ) {
      throw new Error(
        "Refusing cleanup outside the isolated browser-test directory",
      );
    }
    await rm(directory, { recursive: true, force: true });
  }
});
