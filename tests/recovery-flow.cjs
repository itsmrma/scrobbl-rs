const { chooseOption } = require("./controls.cjs");
const { expect } = require("@playwright/test");

module.exports = async function recoveryFlow(page) {
  await page.goto("/settings/data");
  await expect(
    page.getByRole("heading", { name: "Manual backup" }),
  ).toBeVisible();
  const denied = await page.request.get("/api/v1/recovery", {
    headers: { Authorization: "Bearer read-only-test" },
  });
  expect(denied.status()).toBe(403);
  await page.getByLabel("From date").fill("2026-01-01");
  await page.getByLabel("Through date").fill("2026-12-31");
  await chooseOption(page.getByLabel("Export format"), "json");
  const accepted = page.waitForResponse(
    (response) =>
      response.url().endsWith("/api/v1/recovery") &&
      response.request().method() === "POST",
  );
  await page
    .getByRole("button", { name: "Create export", exact: true })
    .click();
  expect((await accepted).status()).toBe(200);
  // Once receipt is confirmed, generation must continue independently of the
  // page. A hard navigation before receipt can cancel the POST in WebKit.
  await page.goto("/activities");
  await page.goto("/settings/data");
  const exportRow = page
    .locator(".job")
    .filter({ hasText: "JSON export" })
    .first();
  await expect(
    exportRow.getByRole("link", { name: "Download", exact: true }),
  ).toBeVisible();
  const exportResponse = await page.request.get(
    await exportRow
      .getByRole("link", { name: "Download", exact: true })
      .getAttribute("href"),
  );
  const exported = await exportResponse.json();
  expect(exported.format).toBe("scrobbl-export");
  expect(exported.format_version).toBe(2);
  expect(exported.original_retention).toBe("compact");
  expect(exported.selection.from).toBe("2026-01-01");
  expect(Array.isArray(exported.listens)).toBe(true);
  await page
    .getByRole("button", { name: "Create backup", exact: true })
    .click();
  const backupRow = page
    .locator(".job")
    .filter({ hasText: "Archive backup" })
    .first();
  await expect(
    backupRow.getByRole("button", { name: "Verify backup", exact: true }),
  ).toBeVisible();
  await expect(backupRow).toContainText("Restore not yet verified");
  await backupRow
    .getByRole("button", { name: "Verify backup", exact: true })
    .click();
  await expect(backupRow).toContainText("Restore verified:");
  await backupRow
    .getByRole("button", { name: "Restore…", exact: true })
    .click();
  await expect(page.getByRole("dialog")).toContainText(
    "All old sessions and API keys become invalid",
  );
  const confirm = page.getByRole("button", {
    name: "Confirm replacement and prepare restart",
    exact: true,
  });
  await expect(confirm).toBeDisabled();
  await page
    .getByLabel("New owner password", { exact: true })
    .fill("a different recovery password 7!");
  await page
    .getByLabel("Repeat new password", { exact: true })
    .fill("a different recovery password 7!");
  await expect(confirm).toBeDisabled();
  await page.getByLabel("I confirm replacement of the current archive").check();
  await expect(confirm).toBeEnabled();
  await page.keyboard.press("Escape");
  await expect(page.locator(".toast")).toHaveCount(0, { timeout: 7000 });
  await page
    .getByRole("heading", { name: "Data & recovery", exact: true })
    .click();
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(
    page.getByRole("heading", { name: "Manual backup" }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: "test-results/recovery-mobile.png",
    fullPage: true,
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.screenshot({
    path: "test-results/recovery-desktop.png",
    fullPage: true,
  });
};
