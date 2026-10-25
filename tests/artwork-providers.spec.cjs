const { settleDisclosure } = require("./controls.cjs");
const { test: base, expect } = require("@playwright/test");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const { mkdtemp, rm } = require("node:fs/promises");
const { tmpdir } = require("node:os");
const { join, resolve } = require("node:path");
const net = require("node:net");

// Keep this scenario's sign-in outside the shared server's real login rate limit.
const test = base.extend({
  providerApp: async ({ browser }, use) => {
    const directory = await mkdtemp(join(tmpdir(), "scrobbl-image-providers-"));
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

const password = "a local browser test password 7!";
async function login(page, path = "/settings/general") {
  const session = await (await page.request.get("/api/session")).json();
  await page.goto(path);
  await page.getByLabel("Username", { exact: true }).fill("owner");
  await page.getByLabel("Password", { exact: true }).fill(password);
  if (session.setup_required)
    await page.getByLabel("Confirm password").fill(password);
  await page
    .getByRole("button", {
      name: session.setup_required ? "Create your archive" : "Sign in",
      exact: true,
    })
    .click();
  await expect(
    page.getByRole("heading", { name: "Settings & connections." }),
  ).toBeVisible();
}

test("image providers save instance order, handle conflicts and fit mobile settings", async ({
  providerApp: { page, baseURL },
  browser,
}) => {
  await login(page, "/settings/images");
  const save = page.getByRole("button", {
    name: "Save image providers",
    exact: true,
  });
  await expect(
    page.getByRole("heading", { name: "Image providers", exact: true }),
  ).toBeVisible();
  await expect(save).toBeDisabled();
  await page
    .getByRole("button", {
      name: "Move MusicBrainz → Wikidata → Commons up",
      exact: true,
    })
    .click();
  await page
    .getByRole("checkbox", { name: "MusicBrainz → Commons image", exact: true })
    .uncheck();
  await page.route("**/api/v1/artwork-providers", (route) =>
    route.request().method() === "POST"
      ? route.fulfill({
          status: 500,
          json: { error: "Provider save failed; retry." },
        })
      : route.continue(),
  );
  await save.click();
  await expect(page.getByRole("alert").first()).toContainText(
    "Provider save failed",
  );
  await expect(
    page.getByRole("checkbox", {
      name: "MusicBrainz → Commons image",
      exact: true,
    }),
  ).not.toBeChecked();
  await page.unroute("**/api/v1/artwork-providers");
  await save.click();
  await expect(save).toBeDisabled();
  await page.reload();
  await expect(page.locator(".artwork-provider").nth(1)).toContainText(
    "MusicBrainz → Wikidata → Commons",
  );
  await expect(
    page.getByRole("checkbox", {
      name: "MusicBrainz → Commons image",
      exact: true,
    }),
  ).not.toBeChecked();
  const other = await browser.newContext({ baseURL });
  const second = await other.newPage();
  await login(second, "/settings/images");
  await expect(second.locator(".artwork-provider").nth(1)).toContainText(
    "MusicBrainz → Wikidata → Commons",
  );
  await second
    .getByRole("checkbox", {
      name: "MusicBrainz → Cover Art Archive",
      exact: true,
    })
    .uncheck();
  await second
    .getByRole("button", { name: "Save image providers", exact: true })
    .click();
  await expect(
    second.getByRole("button", { name: "Save image providers", exact: true }),
  ).toBeDisabled();
  await page
    .getByRole("checkbox", {
      name: "MusicBrainz → Cover Art Archive",
      exact: true,
    })
    .uncheck();
  await save.click();
  await expect(page.getByRole("alert").first()).toContainText(
    "changed in another session",
  );
  await page.getByRole("button", { name: "Reload saved settings" }).click();
  await expect(save).toBeDisabled();
  await expect(
    page.getByRole("checkbox", {
      name: "MusicBrainz → Cover Art Archive",
      exact: true,
    }),
  ).not.toBeChecked();
  await expect(
    page.getByRole("checkbox", { name: "Spotify", exact: true }),
  ).toHaveCount(0);
  await page.locator(".artwork-provider").first().locator("summary").click();
  await settleDisclosure(
    page.locator(".artwork-provider").first().locator("details"),
  );
  await expect(
    page.getByRole("link", { name: "Cover Art Archive API", exact: true }),
  ).toBeVisible();
  await page.screenshot({
    animations: "disabled",
    path: "docs/screenshots/image-providers-desktop.png",
    fullPage: true,
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(
    page.getByLabel("Settings section", { exact: true }),
  ).toBeVisible();
  await expect(page.locator(".artwork-provider").first()).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    animations: "disabled",
    path: "docs/screenshots/image-providers-mobile.png",
    fullPage: true,
  });
  // Restore defaults for the other browser scenarios sharing this isolated server.
  await page
    .getByRole("button", {
      name: "Move MusicBrainz → Commons image up",
      exact: true,
    })
    .click();
  await page
    .getByRole("checkbox", { name: "MusicBrainz → Commons image", exact: true })
    .check();
  await page
    .getByRole("checkbox", {
      name: "MusicBrainz → Cover Art Archive",
      exact: true,
    })
    .check();
  await save.click();
  await expect(save).toBeDisabled();
  await other.close();
});
