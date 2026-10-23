const { test, expect } = require("@playwright/test");
const { mkdirSync } = require("node:fs");

test("Spotify opens a separate tab; the original dialog follows decline, closure, retry and success", async ({
  page,
  context,
  baseURL,
  browserName,
}) => {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/settings/connections");
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
    page.getByRole("heading", { name: "Settings & connections." }),
  ).toBeVisible();
  const owner = await (await page.request.get("/api/session")).json();
  const saved = await page.request.post("/api/spotify/credentials", {
    headers: { Origin: baseURL, "X-CSRF-Token": owner.csrf },
    data: {
      client_id: "browsertestclient",
      client_secret: "browsertestsecret",
    },
  });
  expect(saved.ok()).toBeTruthy();
  const base = await (await page.request.get("/api/spotify/status")).json();
  // A previous connection must never turn a rejected reconnect into success.
  await page.route("**/api/spotify/status", (route) =>
    route.fulfill({ json: { ...base, configured: true, connected: true } }),
  );
  await context.route("https://accounts.spotify.com/authorize?**", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: "<h1>Spotify test authorization</h1>",
    }),
  );
  await page.reload();
  await page.locator(".spotify-card > summary").click();
  const start = async () => {
    const opened = context.waitForEvent("page");
    await page
      .getByRole("button", { name: "Reconnect Spotify", exact: true })
      .click();
    const tab = await opened;
    await tab.waitForURL("https://accounts.spotify.com/authorize?**");
    return tab;
  };
  let tab = await start();
  expect(await tab.evaluate(() => window.opener)).toBeNull();
  await expect(page).toHaveURL(`${baseURL}/settings/connections`);
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("Waiting for Spotify");
  await expect(dialog.locator(".connect-spinner")).toBeVisible();
  mkdirSync("docs/screenshots", { recursive: true });
  if (browserName === "chromium")
    await page.screenshot({
      path: "docs/screenshots/spotify-connect-waiting.png",
    });
  const secret = new URL(tab.url()).searchParams.get("state");
  await tab
    .goto(
      `${baseURL}/auth/spotify/callback?state=${secret}&error=access_denied`,
    )
    .catch(() => {});
  await expect(dialog).toContainText("authorization was declined");
  await expect(dialog).not.toContainText("Connected. Your Spotify");
  await page.setViewportSize({ width: 390, height: 844 });
  if (browserName === "chromium")
    await page.screenshot({
      path: "docs/screenshots/spotify-connect-declined-mobile.png",
    });
  expect(
    await dialog.evaluate((el) => el.getBoundingClientRect().width),
  ).toBeLessThanOrEqual(390);

  const next = context.waitForEvent("page");
  await dialog.getByRole("button", { name: "Try again" }).click();
  tab = await next;
  await tab.waitForURL("https://accounts.spotify.com/authorize?**");
  await tab.close();
  await expect(dialog).toContainText("tab was closed");
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(dialog).not.toBeVisible();

  tab = await start();
  const statusRequest = page.waitForRequest((request) =>
    /\/api\/spotify\/connect\/[a-f0-9]{64}$/.test(request.url()),
  );
  const request = await statusRequest;
  let completed = false;
  await context.route(request.url(), (route) =>
    route.request().method() === "GET"
      ? route.fulfill({
          json: {
            outcome: completed ? "connected" : "processing",
            message: null,
          },
        })
      : route.continue(),
  );
  const attempt = new URL(request.url()).pathname.split("/").pop();
  await tab.goto(`${baseURL}/spotify/connect-result/${attempt}`);
  completed = true;
  await expect(dialog).toContainText(
    "Connected. Your Spotify authorization is saved.",
  );
  await expect(dialog.locator(".material-icons-outlined")).toHaveText(
    "check_circle",
  );
  // Browser isolation may prevent remote closure even after the callback.
  // The return page remains usable; the user can close that tab themselves.
  if (!tab.isClosed()) await tab.close();
  if (browserName === "chromium") {
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.screenshot({
      path: "docs/screenshots/spotify-connect-success.png",
    });
  }
  await dialog.getByRole("button", { name: "Done" }).click();
  await expect(dialog).not.toBeVisible();
  await context.unroute(request.url());

  await page.evaluate(() => {
    window.open = () => null;
  });
  await page
    .getByRole("button", { name: "Reconnect Spotify", exact: true })
    .click();
  await expect(dialog).toContainText("browser blocked the Spotify tab");
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(errors).toEqual([]);
});
