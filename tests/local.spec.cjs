const { chooseOption } = require("./controls.cjs");
const { test, expect } = require("@playwright/test");
const rows = [
  {
    ts: "2025-06-01T12:03:00Z",
    ms_played: 180000,
    master_metadata_track_name: "First song",
    master_metadata_album_artist_name: "Favourite artist",
    master_metadata_album_album_name: "First album",
    spotify_track_uri: "spotify:track:0000000000000000000001",
  },
  {
    ts: "2025-06-01T12:06:00Z",
    ms_played: 180000,
    master_metadata_track_name: "Second song",
    master_metadata_album_artist_name: "Another artist",
    master_metadata_album_album_name: "Second album",
    spotify_track_uri: "spotify:track:0000000000000000000002",
  },
  {
    ts: "2025-06-01T12:07:00Z",
    ms_played: 10000,
    master_metadata_track_name: "Short song",
    master_metadata_album_artist_name: "Favourite artist",
    master_metadata_album_album_name: "First album",
    spotify_track_uri: "spotify:track:0000000000000000000003",
  },
  {
    ts: "2025-06-01T12:08:00Z",
    ms_played: 30000,
    master_metadata_track_name: "<script>window.bad=1</script>",
    master_metadata_album_artist_name: "Earth, Wind & Fire",
    master_metadata_album_album_name: "Special album",
    spotify_track_uri: "spotify:track:0000000000000000000004",
  },
];
test("owner setup, private history, imports, search, preferences and Spotify configuration", async ({
  page,
  request,
}) => {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  const anon = await request.get("/api/history");
  expect(anon.status()).toBe(401);
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Make this space yours" }),
  ).toBeVisible();
  await page.getByLabel("Username", { exact: true }).fill("owner");
  await page.getByLabel("Password", { exact: true }).fill("aaaaaaaaaaaaaaa");
  await page.getByLabel("Confirm password").fill("aaaaaaaaaaaaaaa");
  await expect(
    page.getByRole("button", { name: "Create your archive" }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Show password" }).click();
  await expect(page.getByLabel("Password", { exact: true })).toHaveAttribute(
    "type",
    "text",
  );
  await page.getByRole("button", { name: "Hide password" }).click();

  await page
    .getByLabel("Password", { exact: true })
    .fill("a local browser test password 7!");
  await page
    .getByLabel("Confirm password")
    .fill("a local browser test password 7!");
  await page.getByRole("button", { name: "Create your archive" }).click();
  await expect(
    page.getByRole("heading", { name: "Your listening, at a glance." }),
  ).toBeVisible();
  await expect(page.locator("#total-listens")).toHaveText("0");
  await page.getByRole("link", { name: "Import history", exact: true }).click();
  await page.locator("#import-files").setInputFiles({
    name: "Streaming_History_Audio_test.json",
    mimeType: "application/json",
    buffer: Buffer.from(JSON.stringify(rows)),
  });
  await page.getByRole("button", { name: "Import listening history" }).click();
  await expect(page.locator("#import-result")).toContainText("4 added");
  await expect(page.locator("#import-result")).toContainText("0 invalid");
  await page.getByRole("button", { name: "Import listening history" }).click();
  await expect(page.locator("#import-result")).toContainText(
    "4 already present",
  );
  await page.getByRole("link", { name: "Overview", exact: true }).click();
  await expect(page.locator("#total-listens")).toHaveText("3");
  await expect(page.locator("#total-tracks")).toHaveText("3");
  await expect(page.locator("#total-time")).toHaveText("6m");
  expect(await page.evaluate(() => window.bad)).toBeUndefined();
  await page.screenshot({
    path: "test-results/dashboard-desktop.png",
    fullPage: true,
  });
  await page
    .getByRole("link", { name: "Listening history", exact: true })
    .click();
  await expect(page.locator("#history-total")).toHaveText("3 records");
  // Returning to the same selection must retain its data while refreshing,
  // even when the network response is delayed.
  let releaseOverview;
  const overviewResponse = new Promise((resolve) => {
    releaseOverview = resolve;
  });
  await page.route("**/api/dashboard?*", async (route) => {
    await overviewResponse;
    await route.continue();
  });
  try {
    await page.getByRole("link", { name: "Overview", exact: true }).click();
    await expect(page.locator("#total-listens")).toHaveText("3", {
      timeout: 1000,
    });
    await expect(
      page.getByRole("status").filter({ hasText: "Loading your archive" }),
    ).toHaveCount(0);
  } finally {
    releaseOverview();
    await page.unrouteAll({ behavior: "wait" });
  }
  await page
    .getByRole("link", { name: "Listening history", exact: true })
    .click();
  await page.getByRole("button", { name: "Filters", exact: true }).click();
  await chooseOption(page.locator("#history-status"), "short");
  await page.getByRole("button", { name: "Search", exact: true }).click();
  await expect(page.locator("#history-total")).toHaveText("1 record");
  await expect(page.locator("#history-list")).toContainText("Short song");
  await page.getByRole("button", { name: "Reset", exact: true }).click();
  await page.locator("#history-q").fill("Earth, Wind & Fire");
  await page.getByRole("button", { name: "Search", exact: true }).click();
  await expect(page.locator("#history-list")).toContainText(
    "<script>window.bad=1</script>",
  );
  expect(await page.evaluate(() => window.bad)).toBeUndefined();
  await page.getByRole("link", { name: "Settings", exact: true }).click();
  await page.getByRole("link", { name: "Connections", exact: true }).click();
  await expect(page.locator("#redirect-uri")).toHaveText(
    `${new URL(page.url()).origin}/auth/spotify/callback`,
  );
  await page
    .getByLabel("Client ID", { exact: true })
    .fill("testclient1234567890");
  await page
    .getByLabel("Client Secret", { exact: true })
    .fill("testsecret1234567890");
  await page.getByRole("button", { name: "Save Spotify app" }).click();
  await expect(page.locator("#connection-badge")).toContainText(
    "Ready to connect",
  );
  await expect(page.locator(".credentials-saved")).toContainText(
    "App credentials saved",
  );
  await expect(page.getByLabel("Client Secret", { exact: true })).toHaveValue(
    "",
  );
  const status = await page.evaluate(() =>
    fetch("/api/spotify/status").then((r) => r.json()),
  );
  expect(JSON.stringify(status)).not.toContain("testsecret");
  await page.getByRole("link", { name: "Appearance", exact: true }).click();
  await chooseOption(page.locator("#theme"), "light");
  await page.reload();
  await expect(page.locator("html")).toHaveClass(/theme-light/);
  await page.getByRole("link", { name: "Overview", exact: true }).click();
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.locator("#total-listens")).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: "test-results/dashboard-mobile.png",
    fullPage: true,
  });
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.getByRole("button", { name: "Sign out" }).click();
  await expect(
    page.getByRole("heading", { name: "Welcome back" }),
  ).toBeVisible();
  await page.getByLabel("Username", { exact: true }).fill("owner");
  await page
    .getByLabel("Password", { exact: true })
    .fill("a local browser test password 7!");
  await page.getByRole("button", { name: /^Sign in/ }).click();
  await expect(page.locator("#total-listens")).toHaveText("3");
  expect(errors).toEqual([]);
});
