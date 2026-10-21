const { test, expect } = require("@playwright/test");
const path = require("node:path");
test("ZIP receipt, saved partial report, invalid rows, exclusions and safe retry survive navigation", async ({
  page,
}) => {
  await page.goto("/import");
  await page
    .getByLabel("Password", { exact: true })
    .fill("a local browser test password");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByText("Upload limit:")).toBeVisible();
  await page
    .locator("#import-files")
    .setInputFiles(path.join(__dirname, "fixtures/spotify-history.zip"));
  const received = page.waitForResponse(
    (r) =>
      r.url().endsWith("/api/import/spotify") &&
      r.request().method() === "POST",
  );
  await page.getByRole("button", { name: "Import listening history" }).click();
  const { job_id } = await (await received).json();
  expect(job_id).toBeTruthy();
  await page.goto(`/activities/${job_id}`);
  await expect(
    page.getByRole("status").filter({ hasText: "partial" }),
  ).toBeVisible({ timeout: 15000 });
  await expect(
    page.getByText("Inventory complete.", { exact: false }),
  ).toBeVisible();
  await expect(
    page.getByText("nested/content.data", { exact: false }).first(),
  ).toBeVisible();
  await expect(
    page.getByText("bad.json", { exact: false }).first(),
  ).toBeVisible();
  await expect(page.getByText("row 5:", { exact: false })).toBeVisible();
  await expect(
    page.getByText("json_unreadable_or_row_limit:", { exact: false }),
  ).toBeVisible();
  const before = await (
    await page.request.get(`/api/v1/activities/${job_id}`)
  ).json();
  expect(before.activity.added).toBe(1);
  expect(before.activity.variants).toBe(1);
  expect(before.activity.duplicates).toBe(1);
  expect(before.activity.invalid).toBe(1);
  expect(before.activity.skipped).toBe(1);
  expect(before.members).toHaveLength(3);
  await page.getByRole("button", { name: "Retry import", exact: true }).click();
  await expect
    .poll(
      async () =>
        (await (await page.request.get(`/api/v1/activities/${job_id}`)).json())
          .activity.attempt,
    )
    .toBe(2);
  await page.reload();
  await expect(
    page.getByRole("status").filter({ hasText: "partial" }),
  ).toBeVisible();
  const after = await (
    await page.request.get(`/api/v1/activities/${job_id}`)
  ).json();
  expect(after.activity.processed).toBe(before.activity.processed);
  expect(after.activity.added).toBe(1);
  expect(after.activity.duplicates).toBe(1);
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: "test-results/spotify-import-mobile.png",
    fullPage: true,
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.screenshot({
    path: "test-results/spotify-import-desktop.png",
    fullPage: true,
  });
  await page.getByRole("link", { name: "All activities", exact: true }).click();
  await page.getByLabel("Activity type").selectOption("spotify_import");
  await expect(
    page.getByRole("main").locator(`a[href="/activities/${job_id}"]`),
  ).toBeVisible();
});
