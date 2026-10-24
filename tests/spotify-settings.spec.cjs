const { settleDisclosure } = require("./controls.cjs");
const { test, expect } = require("@playwright/test");
const { mkdirSync } = require("node:fs");

async function openSpotify(page) {
  const card = page.locator(".spotify-card");
  await expect(card).toBeVisible();
  if (!(await card.evaluate((element) => element.open)))
    await card.locator("summary").first().click();
  const intervals = card.locator(".connection-monitor").filter({
    has: page
      .locator("summary > span")
      .filter({ hasText: /^Refresh intervals$/ }),
  });
  if (!(await intervals.evaluate((element) => element.open)))
    await intervals.locator("summary").click();
  await settleDisclosure(card);
}

async function saveIntervals(page) {
  const saved = page.waitForResponse(
    (response) =>
      new URL(response.url()).pathname === "/api/spotify/intervals" &&
      response.request().method() === "POST",
  );
  await page.getByRole("button", { name: "Save intervals" }).click();
  expect((await saved).status()).toBe(200);
  await expect(
    page.getByRole("button", { name: "Save intervals" }),
  ).toBeDisabled();
}

test("Spotify intervals save on the server, survive reload and are shared; API keys belongs to Settings", async ({
  page,
  browser,
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
    page.getByRole("heading", { name: "Settings & connections." }),
  ).toBeVisible();
  await openSpotify(page);
  const collection = page.getByLabel("Collection interval (seconds)");
  const now = page.getByLabel("Now playing interval (seconds)");
  const baseline = await (await page.request.get("/api/spotify/status")).json();
  await expect(collection).toHaveValue(String(baseline.sync_seconds));
  await collection.fill("90");
  await now.fill("20");
  await page.route("**/api/spotify/intervals", (route) =>
    route.fulfill({
      status: 503,
      json: { error: "Save unavailable. Try again." },
    }),
  );
  await page.getByRole("button", { name: "Save intervals" }).click();
  await expect(
    page.getByRole("alert").filter({ hasText: "Save unavailable" }),
  ).toContainText("Save unavailable");
  await expect(collection).toHaveValue("90");
  await page.unroute("**/api/spotify/intervals");
  await saveIntervals(page);
  let effective = await (await page.request.get("/api/spotify/status")).json();
  expect([effective.sync_seconds, effective.now_playing_seconds]).toEqual([
    90, 20,
  ]);
  await page.reload();
  await openSpotify(page);
  await expect(collection).toHaveValue("90");
  await expect(now).toHaveValue("20");
  const owner = await (await page.request.get("/api/session")).json();
  const invalid = await page.request.post("/api/spotify/intervals", {
    headers: { Origin: baseURL, "X-CSRF-Token": owner.csrf },
    data: { sync_seconds: 59, now_playing_seconds: 14 },
  });
  expect(invalid.status()).toBe(400);
  effective = await (await page.request.get("/api/spotify/status")).json();
  expect([effective.sync_seconds, effective.now_playing_seconds]).toEqual([
    90, 20,
  ]);

  const other = await browser.newContext({
    storageState: await page.context().storageState(),
  });
  const otherPage = await other.newPage();
  await otherPage.goto(`${baseURL}/settings/connections`);
  await expect(
    otherPage.getByRole("heading", { name: "Settings & connections." }),
  ).toBeVisible();
  await openSpotify(otherPage);
  await expect(
    otherPage.getByLabel("Collection interval (seconds)"),
  ).toHaveValue("90");
  await otherPage.getByLabel("Now playing interval (seconds)").fill("25");
  await saveIntervals(otherPage);
  await expect(now).toHaveValue("25", { timeout: 10000 });
  await other.close();

  await collection.fill("60");
  await now.fill("15");
  await saveIntervals(page);
  if (browserName === "chromium") {
    mkdirSync("docs/screenshots", { recursive: true });
    await collection.scrollIntoViewIfNeeded();
    await page.screenshot({
      path: "docs/screenshots/spotify-interval-settings.png",
    });
  }
  const navigation = page.getByRole("navigation", { name: "Main navigation" });
  await expect(navigation.getByRole("link", { name: "API keys" })).toHaveCount(
    0,
  );
  await page
    .getByRole("navigation", { name: "Settings sections" })
    .getByRole("link", { name: "API keys" })
    .click();
  await expect(page).toHaveURL(`${baseURL}/settings/api-keys`);
  await expect(
    page.getByRole("heading", { name: "API keys", exact: true }),
  ).toBeVisible();
  await expect(
    page
      .getByRole("navigation", { name: "Settings sections" })
      .getByRole("link", { name: "API keys" }),
  ).toHaveAttribute("aria-current", "page");
  await page.goto("/integrations/api-keys");
  await expect(page).toHaveURL(`${baseURL}/settings/api-keys`);
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(
    page.getByRole("heading", { name: "API keys", exact: true }),
  ).toBeVisible();
  if (browserName === "chromium")
    await page.screenshot({
      path: "docs/screenshots/api-keys-settings-mobile.png",
    });
  expect(errors).toEqual([]);
});
