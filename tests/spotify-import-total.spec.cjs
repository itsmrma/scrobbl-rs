const { test, expect } = require("@playwright/test");

test("Import and activity detail show the whole total, counting and unavailable states", async ({
  page,
  baseURL,
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
