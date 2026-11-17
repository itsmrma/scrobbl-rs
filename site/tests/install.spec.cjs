const { test, expect } = require("@playwright/test");

// Install section: steps, Docker / From source tabs, copy buttons.

const DOCKER_COMMANDS = [
  "curl -fsSLO https://raw.githubusercontent.com/itsmrma/scrobbl-rs/main/compose.yaml",
  "sudo install -d -o 10001 -g 10001 -m 0700 /srv/scrobbl/data",
  "export SCROBBL_IMAGE=ghcr.io/itsmrma/scrobbl-rs:sha-FULL_COMMIT",
  "export SCROBBL_EXTERNAL_ORIGIN=http://192.0.2.10:3210",
  "export SCROBBL_HOST_IP=192.0.2.10",
  "export SCROBBL_DATA_PATH=/srv/scrobbl/data",
  "docker compose pull",
  "docker compose up -d",
].join("\n");

const SOURCE_COMMANDS = "./scripts/run-local.sh";

async function installSection(page) {
  await page.goto("/");
  return page.locator("#install");
}

test("install section lists three numbered steps", async ({ page }) => {
  const install = await installSection(page);
  await expect(install.getByRole("heading", { level: 2 })).toBeVisible();
  const steps = install.getByRole("list").first().getByRole("listitem");
  await expect(steps).toHaveCount(3);
  await expect(steps.nth(0)).toContainText(/start the container/i);
  await expect(steps.nth(1)).toContainText(/owner account/i);
  await expect(steps.nth(2)).toContainText(/Spotify/);
  await expect(steps.nth(2)).toContainText(/import/i);
});

test("nav links to the install section", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("navigation", { name: "Primary" }).getByRole("link", { name: "Install" }).click();
  await expect(page).toHaveURL(/#install$/);
  await expect(page.locator("#install")).toBeInViewport();
});

test("tabs switch the visible panel and aria-selected, by mouse and keyboard", async ({ page }) => {
  const install = await installSection(page);
  const tablist = install.getByRole("tablist");
  const docker = tablist.getByRole("tab", { name: "Docker" });
  const source = tablist.getByRole("tab", { name: "From source" });
  await expect(docker).toHaveAttribute("aria-selected", "true");
  await expect(source).toHaveAttribute("aria-selected", "false");
  await expect(install.getByRole("tabpanel")).toHaveCount(1);
  await expect(install.getByRole("tabpanel")).toContainText("docker compose up -d");

  await source.click();
  await expect(source).toHaveAttribute("aria-selected", "true");
  await expect(docker).toHaveAttribute("aria-selected", "false");
  await expect(install.getByRole("tabpanel")).toContainText("./scripts/run-local.sh");
  await expect(install.getByRole("tabpanel")).toContainText("http://127.0.0.1:3210");
  await expect(install.getByRole("tabpanel")).not.toContainText("docker compose up -d");

  await docker.focus();
  await page.keyboard.press("ArrowRight");
  await expect(source).toHaveAttribute("aria-selected", "true");
  await expect(source).toBeFocused();
  await page.keyboard.press("ArrowLeft");
  await expect(docker).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("End");
  await expect(source).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("Home");
  await expect(docker).toHaveAttribute("aria-selected", "true");
});

test("Docker panel follows the Docker guide and links to it", async ({ page }) => {
  const install = await installSection(page);
  const panel = install.getByRole("tabpanel");
  for (const v of ["SCROBBL_IMAGE", "SCROBBL_EXTERNAL_ORIGIN", "SCROBBL_HOST_IP", "SCROBBL_DATA_PATH"]) {
    await expect(panel).toContainText(v);
  }
  await expect(panel).toContainText(/sha-FULL_COMMIT/);
  await expect(panel).toContainText(/pin/i);
  await expect(panel).not.toContainText(":main");
  await expect(install.getByRole("link", { name: /full (Docker )?guide/i })).toHaveAttribute(
    "href",
    "https://github.com/itsmrma/scrobbl-rs/blob/main/docs/docker.md",
  );
});

test("copy puts exactly the commands on the clipboard and confirms", async ({ page, context, browserName }) => {
  test.skip(browserName !== "chromium", "clipboard read permission is only grantable in Chromium");
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const install = await installSection(page);
  const copy = install.getByRole("button", { name: /^Copy/ });
  await copy.click();
  await expect(install.getByRole("button", { name: /^Copied/ })).toBeVisible();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(DOCKER_COMMANDS);
  await expect(install.getByRole("button", { name: /^Copy/ })).toBeVisible({ timeout: 4000 });

  await install.getByRole("tab", { name: "From source" }).click();
  await install.getByRole("button", { name: /^Copy/ }).click();
  await expect(install.getByRole("button", { name: /^Copied/ })).toBeVisible();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(SOURCE_COMMANDS);
});

test("command block does not cause horizontal scroll at 320px", async ({ page }) => {
  await page.setViewportSize({ width: 320, height: 800 });
  const install = await installSection(page);
  await install.scrollIntoViewIfNeeded();
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  expect(overflow).toBeLessThanOrEqual(0);
  const box = await install.getByRole("tabpanel").boundingBox();
  expect(box.x + box.width).toBeLessThanOrEqual(320);
});
