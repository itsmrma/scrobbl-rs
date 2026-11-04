const { chooseOption, settleDisclosure } = require("./controls.cjs");
const { test, expect } = require("@playwright/test");
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

test("general preferences, explicit calendar scope, failed saves, browser isolation and appearance", async ({
  page,
  browser,
}) => {
  const errors = [];
  await page.addInitScript(() =>
    localStorage.setItem("scrobbl-theme", "light"),
  );
  page.on("pageerror", (error) => errors.push(error.message));
  await login(page);
  await page.getByLabel("Instance name").fill("Music room");
  await chooseOption(page.getByLabel("Initial dashboard period"), "30d");
  await chooseOption(page.getByLabel("Time format"), "12");
  await page.route("**/api/v1/preferences", (route) =>
    route.request().method() === "POST"
      ? route.fulfill({
          status: 500,
          json: { error: "Test save failed — please retry." },
        })
      : route.continue(),
  );
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByLabel("Instance name")).toHaveValue("Music room");
  await expect(page.getByRole("alert").first()).toContainText(
    "Test save failed",
  );
  await page.unroute("**/api/v1/preferences");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Save", exact: true }),
  ).toBeDisabled();
  await expect(page).toHaveTitle(/Music room/);
  await page.getByLabel("Time zone").fill("UTC");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Apply calendar", exact: true }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(
    (await (await page.request.get("/api/v1/preferences")).json()).timezone,
  ).toBe("Europe/Rome");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await page
    .getByRole("radio", {
      name: "Only listens collected or imported from now on",
    })
    .check();
  await page
    .getByRole("button", { name: "Apply calendar", exact: true })
    .click();
  await expect(page.getByRole("dialog")).not.toBeVisible();
  await page.reload();
  await expect(page.getByLabel("Time zone")).toHaveValue("UTC");
  await expect(page.getByLabel("Instance name")).toHaveValue("Music room");
  await page.getByLabel("Time zone").fill("Asia/Tokyo");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await page
    .getByRole("radio", { name: "The whole listening history" })
    .check();
  await page
    .getByRole("button", { name: "Apply calendar", exact: true })
    .click();
  await expect(page.getByRole("dialog")).not.toBeVisible();
  await expect(page.getByLabel("Time zone")).toHaveValue("Asia/Tokyo", {
    timeout: 10000,
  });
  await page.getByRole("link", { name: "Appearance", exact: true }).click();
  await expect(page.getByLabel("Theme", { exact: true })).toHaveAttribute(
    "data-value",
    "light",
  );
  await chooseOption(page.getByLabel("Theme", { exact: true }), "dark");
  await page.getByLabel("HEX colour").fill("#ffffff");
  await expect(page.getByLabel("HEX colour")).toHaveValue("#ffffff");
  await expect(page.locator(".preview.dark span")).toHaveCSS(
    "color",
    "rgb(0, 0, 0)",
  );
  await page.getByLabel("HEX colour").fill("#000000");
  await expect(page.locator(".preview.light span")).toHaveCSS(
    "color",
    "rgb(255, 255, 255)",
  );
  await page.getByLabel("HEX colour").fill("broken");
  await expect(page.getByText("Enter a six-digit HEX colour")).toBeVisible();
  await page.getByRole("button", { name: "Reset accent" }).click();
  await expect(page.getByLabel("HEX colour")).toHaveValue("#c9ee8b");
  const other = await browser.newContext();
  const otherPage = await other.newPage();
  await otherPage.emulateMedia({ colorScheme: "light" });
  await login(otherPage);
  await expect(otherPage.getByLabel("Instance name")).toHaveValue("Music room");
  await expect(otherPage.getByLabel("Time format")).toHaveAttribute(
    "data-value",
    "12",
  );
  await otherPage
    .getByRole("link", { name: "Appearance", exact: true })
    .click();
  await expect(otherPage.getByLabel("Theme", { exact: true })).toHaveAttribute(
    "data-value",
    "auto",
  );
  await expect(otherPage.locator("html")).toHaveClass(/theme-light/);
  await otherPage.emulateMedia({ colorScheme: "dark" });
  await expect(otherPage.locator("html")).not.toHaveClass(/theme-light/);
  await expect(page.getByLabel("Theme", { exact: true })).toHaveAttribute(
    "data-value",
    "dark",
  );
  await other.close();
  await page.getByRole("link", { name: "Overview", exact: true }).click();
  await expect(page.getByLabel("Period", { exact: true })).toHaveAttribute(
    "data-value",
    "30d",
  );
  await chooseOption(page.getByLabel("Period", { exact: true }), "all");
  await page
    .getByRole("link", { name: "Listening history", exact: true })
    .click();
  await page.getByRole("button", { name: "Filters", exact: true }).click();
  await settleDisclosure(page.locator(".history-filter-disclosure"));
  await page
    .getByRole("textbox", { name: "Search", exact: true })
    .fill("Earth");
  await page.getByRole("button", { name: "Search", exact: true }).click();
  await expect(page).toHaveURL(/(?:\?|&)q=Earth(?:&|$)/);
  await page.getByRole("link", { name: "Settings", exact: true }).click();
  await page
    .getByRole("link", { name: "Listening history", exact: true })
    .click();
  await expect(
    page.getByRole("textbox", { name: "Search", exact: true }),
  ).toHaveValue("Earth");
  await page.getByRole("link", { name: "Overview", exact: true }).click();
  await expect(page.getByLabel("Period", { exact: true })).toHaveAttribute(
    "data-value",
    "all",
  );
  await require("./recovery-flow.cjs")(page);
  await require("./navigation-flow.cjs")(page);
  // Test Refresh missing artwork is NOT in Settings
  await page.goto("/settings/images");
  await expect(
    page.getByRole("button", { name: "Refresh missing artwork" }),
  ).toHaveCount(0);

  // Test Refresh missing artwork is present and working in Activities worker card
  await page.goto("/activities");
  const activitiesWorkerBtn = page
    .locator(".artwork-work")
    .getByRole("button", { name: "Refresh missing artwork" });
  await expect(activitiesWorkerBtn).toBeVisible();
  await page.route("**/api/v1/artwork/backfill", (route) =>
    route.fulfill({
      json: { ok: true, queued: 5, queued_albums: 3, queued_artists: 2 },
    }),
  );
  await activitiesWorkerBtn.click();
  await expect(page.locator(".toast").first()).toContainText("Queued 5 items");

  // Test Refresh missing artwork is present and functioning in Library tools menu
  await page.locator("#library-tools-trigger").click();
  const libraryToolsBtn = page
    .locator("#library-tools-menu")
    .getByRole("button", { name: "Refresh missing artwork" });
  await expect(libraryToolsBtn).toBeVisible();
  await libraryToolsBtn.click();
  await expect(page.locator(".toast").first()).toBeVisible();
  await page.unroute("**/api/v1/artwork/backfill");
  expect(errors).toEqual([]);
});

test("global activities, connection states, timed toasts, sidebar and direct routes", async ({
  page,
}) => {
  const activity = {
    id: "demo-calendar",
    kind: "calendar",
    filename: null,
    state: "running",
    attempt: 1,
    processed: 12,
    added: 0,
    reconciled: 0,
    duplicates: 0,
    invalid: 0,
    skipped: 0,
    ambiguous: 0,
    expected_rows: null,
    retry_at_ms: null,
    error_code: null,
  };
  await page.route("**/api/v1/activities", (route) =>
    route.fulfill({ json: [activity] }),
  );
  await page.route("**/api/v1/activities/demo-calendar", (route) =>
    route.fulfill({
      json: {
        activity,
        members: [
          {
            member: "History.json",
            kind: "music",
            state: "processing",
            expected_rows: null,
            processed: 12,
            error_code: null,
          },
        ],
      },
    }),
  );
  await login(page);
  await page
    .getByRole("button", { name: "Activities: 1 active", exact: true })
    .click();
  await expect(page.getByText("1 in progress")).toBeVisible();
  await page.keyboard.press("Escape");
  const resizer = page.getByRole("separator", { name: "Sidebar width" });
  await resizer.focus();
  await page.keyboard.press("End");
  await expect(resizer).toHaveAttribute("aria-valuenow", "360");
  await page.reload();
  await expect(resizer).toHaveAttribute("aria-valuenow", "360");
  await page.getByRole("button", { name: "Collapse sidebar" }).click();
  await page
    .getByRole("button", { name: "Activities: 1 active", exact: true })
    .click();
  await page.getByRole("link", { name: "View all", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Activities." }),
  ).toBeVisible();
  await page
    .getByRole("link", { name: "Recalculate assigned calendar", exact: true })
    .click();
  await expect(page.getByRole("progressbar")).not.toHaveAttribute("value");
  await expect(
    page
      .locator(".activity-detail")
      .getByText("12 processed · total not yet known"),
  ).toBeVisible();
  await page.reload();
  await expect(
    page.getByRole("heading", {
      name: "Recalculate assigned calendar",
      exact: true,
    }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Expand sidebar" }).click();
  await page.goto("/settings/connections");
  const base = await (await page.request.get("/api/spotify/status")).json();
  let spotify = {
    ...base,
    configured: true,
    connected: true,
    connection: "unverified",
    collection: "active",
    last_attempt_ms: null,
    last_sync_ms: null,
    last_added: null,
    verified_at_ms: null,
    last_error: null,
  };
  await page.route("**/api/spotify/status", (route) =>
    route.fulfill({ json: spotify }),
  );
  await page.reload();
  await expect(page.locator("#connection-badge")).toContainText(
    "not yet verified",
  );
  spotify = {
    ...spotify,
    connection: "action_required",
    collection: "paused",
    last_error: "Reconnect your account to resume collection.",
  };
  await expect(page.locator(".spotify-card")).toBeVisible({
    timeout: 10000,
  });
  await expect(page.locator("#connection-badge")).toContainText(
    "Action required",
  );
  await expect(
    page.getByRole("alert").filter({ hasText: "Spotify needs your attention" }),
  ).toContainText("Reconnect your account");
  spotify = {
    ...spotify,
    connection: "verified",
    collection: "active",
    last_error: null,
    last_added: 0,
    verified_at_ms: Date.now(),
    last_sync_ms: Date.now(),
  };
  await expect(page.locator("#connection-badge")).toContainText("Connected", {
    timeout: 10000,
  });
  await expect(page.getByText("0 — successful, no new listens")).toBeVisible();

  // Ticket #164: operational flat card assertions
  await expect(
    page.locator(".spotify-operational-card, .spotify-card"),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Verify & sync now" }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Disconnect" })).toBeVisible();
  await expect(page.getByLabel("Collection interval (seconds)")).toBeVisible();
  await expect(page.getByLabel("Now playing interval (seconds)")).toBeVisible();

  // Test HTTPS guide modal
  await page.getByRole("button", { name: /HTTPS/i }).click();
  const guideModal = page.getByRole("dialog");
  await expect(guideModal).toBeVisible();
  await expect(
    guideModal.getByText("Use a free DuckDNS name", { exact: true }),
  ).toBeVisible();
  await guideModal.getByRole("button", { name: "Close" }).click();
  await expect(guideModal).not.toBeVisible();

  await page.getByRole("link", { name: "General", exact: true }).click();
  await page.clock.install();
  await page.getByLabel("Instance name").fill("Toast one");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.locator(".toast")).toHaveCount(1);
  await page.clock.runFor(1000);
  await page.getByLabel("Instance name").fill("Toast two");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.locator(".toast")).toHaveCount(2);
  await expect(page.locator(".toast-time").first()).toHaveCSS(
    "animation-duration",
    "5s",
  );
  await page.getByRole("link", { name: "Appearance", exact: true }).click();
  await page.clock.runFor(4100);
  await expect(page.locator(".toast")).toHaveCount(1);
  await page.clock.runFor(1000);
  await expect(page.locator(".toast")).toHaveCount(0);
  await page.clock.resume();
  await page.getByRole("link", { name: "General", exact: true }).click();
  await page.getByLabel("Instance name").fill("Music room");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Save", exact: true }),
  ).toBeDisabled();
  await page.getByRole("link", { name: "Appearance", exact: true }).click();
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(
    page.getByRole("button", { name: "Activities: 1 active", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("link", { name: "Overview", exact: true }),
  ).not.toBeVisible();
  await page.getByRole("button", { name: "Open navigation" }).click();
  await expect(
    page.getByRole("link", { name: "Overview", exact: true }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("button", { name: "Open navigation" }),
  ).toBeFocused();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: "test-results/settings-mobile.png",
    fullPage: true,
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.screenshot({
    path: "test-results/settings-desktop.png",
    fullPage: true,
  });
});

test("restart application triggers full-screen transition, polls boot-status and reconnects", async ({
  page,
}) => {
  await login(page, "/settings/application");
  let bootStatusCalls = 0;
  let isReady = false;

  await page.route("**/api/v1/application", (route) => {
    if (route.request().method() === "POST") {
      return route.fulfill({
        status: 202,
        json: { accepted: true },
      });
    }
    return route.continue();
  });

  await page.route("**/api/boot-status", (route) => {
    bootStatusCalls++;
    return route.fulfill({
      status: 200,
      json: {
        ready: isReady,
        stage: isReady ? "ready" : "initializing",
        error: null,
      },
    });
  });

  await page
    .getByRole("button", { name: "Restart application", exact: true })
    .first()
    .click();
  await page.getByRole("button", { name: "Restart now", exact: true }).click();

  const lifecycle = page.locator(".lifecycle-screen");
  await expect(lifecycle).toBeVisible();
  await expect(lifecycle).toContainText("Restarting scrobbl.rs");
  await expect(lifecycle.locator(".lifecycle-brand")).toBeVisible();
  await expect(lifecycle.locator(".lifecycle-loader")).toBeVisible();

  await expect.poll(() => bootStatusCalls).toBeGreaterThan(0);

  isReady = true;
  await expect(lifecycle).toContainText(
    /restarted|pronto|Riavvio completato/i,
    { timeout: 10000 },
  );
});

test("stop application triggers full-screen transition with safe shutdown message and reconnect button", async ({
  page,
}) => {
  await login(page, "/settings/application");
  let bootStatusChecked = false;

  await page.route("**/api/v1/application", (route) => {
    if (route.request().method() === "POST") {
      return route.fulfill({
        status: 202,
        json: { accepted: true },
      });
    }
    return route.continue();
  });

  await page.route("**/api/boot-status", (route) => {
    bootStatusChecked = true;
    return route.fulfill({
      status: 200,
      json: {
        ready: false,
        stage: "stopped",
        error: null,
      },
    });
  });

  await page
    .getByRole("button", { name: "Stop application", exact: true })
    .first()
    .click();
  await page.getByRole("button", { name: "Stop now", exact: true }).click();

  const lifecycle = page.locator(".lifecycle-screen");
  await expect(lifecycle).toBeVisible();
  await expect(lifecycle).toContainText(
    "Application stopped. The archive has been closed safely. Restart the container from Docker when you are ready.",
  );
  await expect(lifecycle.locator(".lifecycle-brand")).toBeVisible();

  const reconnectBtn = lifecycle.getByRole("button", {
    name: /Check availability|Reconnect|Verifica disponibilità|Riconnetti/i,
  });
  await expect(reconnectBtn).toBeVisible();
  await reconnectBtn.click();
  expect(bootStatusChecked).toBe(true);
});
