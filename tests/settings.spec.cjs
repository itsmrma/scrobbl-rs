const { test, expect } = require("@playwright/test");
const password = "a local browser test password";
async function login(page, path = "/settings/general") {
  const session = await (await page.request.get("/api/session")).json();
  await page.goto(path);
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
  await page.getByLabel("Initial dashboard period").selectOption("30d");
  await page.getByLabel("Time format").selectOption("12");
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
  await expect(page.getByLabel("Theme", { exact: true })).toHaveValue("light");
  await page.getByLabel("Theme", { exact: true }).selectOption("dark");
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
  await expect(otherPage.getByLabel("Time format")).toHaveValue("12");
  await otherPage
    .getByRole("link", { name: "Appearance", exact: true })
    .click();
  await expect(otherPage.getByLabel("Theme", { exact: true })).toHaveValue(
    "auto",
  );
  await expect(otherPage.locator("html")).toHaveClass(/theme-light/);
  await otherPage.emulateMedia({ colorScheme: "dark" });
  await expect(otherPage.locator("html")).not.toHaveClass(/theme-light/);
  await expect(page.getByLabel("Theme", { exact: true })).toHaveValue("dark");
  await other.close();
  await page.getByRole("link", { name: "Overview", exact: true }).click();
  await expect(page.getByLabel("Period", { exact: true })).toHaveValue("30d");
  await page.getByLabel("Period", { exact: true }).selectOption("all");
  await page
    .getByRole("link", { name: "Listening history", exact: true })
    .click();
  await page.getByLabel("Search", { exact: true }).fill("Earth");
  await page.getByRole("button", { name: "Search", exact: true }).click();
  await page.getByRole("link", { name: "Settings", exact: true }).click();
  await page
    .getByRole("link", { name: "Listening history", exact: true })
    .click();
  await expect(page.getByLabel("Search", { exact: true })).toHaveValue("Earth");
  await page.getByRole("link", { name: "Overview", exact: true }).click();
  await expect(page.getByLabel("Period", { exact: true })).toHaveValue("all");
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
  await expect(page.getByText("1 in progress")).toBeVisible();
  const resizer = page.getByRole("separator", { name: "Sidebar width" });
  await resizer.focus();
  await page.keyboard.press("End");
  await expect(resizer).toHaveAttribute("aria-valuenow", "360");
  await page.reload();
  await expect(resizer).toHaveAttribute("aria-valuenow", "360");
  await page.getByRole("button", { name: "Collapse sidebar" }).click();
  await page
    .getByRole("link", { name: "Activities: 1 active", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Activities." }),
  ).toBeVisible();
  await page
    .getByRole("link", { name: "Recalculate assigned calendar", exact: true })
    .click();
  await expect(page.getByRole("progressbar")).not.toHaveAttribute("value");
  await expect(
    page.getByText("12 processed · total not yet known"),
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
  await expect(page.locator(".spotify-card")).toHaveAttribute("open", "", {
    timeout: 10000,
  });
  await page.locator(".spotify-card > summary").click();
  await expect(page.locator(".spotify-card > summary")).toContainText(
    "Reconnect your account",
  );
  spotify = {
    ...spotify,
    connection: "verified",
    collection: "active",
    last_error: null,
    last_added: 0,
    verified_at_ms: Date.now(),
    last_sync_ms: Date.now(),
  };
  await expect(page.locator("#connection-badge")).toContainText(
    "Authorization verified",
    { timeout: 10000 },
  );
  await page.locator(".spotify-card > summary").click();
  await expect(page.getByText("0 — successful, no new listens")).toBeVisible();
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
    page.getByRole("link", { name: "Activities (1)", exact: true }),
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
