const { test, expect } = require("@playwright/test");

// Hero section: copy, calls to action and the demo Overview preview.

test("hero shows title, description, both calls to action and the fact line", async ({ page }) => {
  await page.goto("/");
  const hero = page.locator("#hero");
  await expect(hero.getByRole("heading", { level: 1 })).toHaveText("Your listening history, kept on your server.");
  await expect(hero.locator(".lede")).not.toBeEmpty();
  await expect(hero.getByRole("link", { name: "Install with Docker" })).toHaveAttribute("href", "#install");
  await expect(hero.getByRole("link", { name: "View source" })).toHaveAttribute(
    "href",
    "https://github.com/itsmrma/scrobbl-rs",
  );
  const facts = hero.getByRole("list", { name: "Highlights" });
  await expect(facts.getByRole("listitem")).toHaveText(["Rust + SQLite", "Self-hosted", "No telemetry", "GPL-3.0-or-later"]);
});

test("Overview preview has an accessible name saying it is demo data, with no visible tag", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("#hero").getByText(/demo archive/i)).toHaveCount(0);
  await expect(page.locator("footer")).toContainText("demo data");
  const preview = page.locator("#hero [role=img]");
  await expect(preview).toHaveAttribute("aria-label", /demo data/i);
  await expect(preview).toBeVisible();
  await expect(preview).toContainText("Overview");
  await expect(preview).toContainText("Now playing");
  await expect(preview.locator(".stat")).toHaveCount(3);
  await expect(preview.locator(".bars span")).toHaveCount(12);
  await expect(preview.locator(".rank .row")).toHaveCount(4);
  await expect(preview.locator(".mock-title .accent")).toHaveText(".");
});

test("the page sets bar heights without style attributes or inline scripts", async ({ page }) => {
  await page.goto("/");
  expect(await page.locator("[style]").count()).toBe(0);
  expect(await page.locator("script:not([src])").count()).toBe(0);
  const heights = await page
    .locator("#hero .bars span")
    .evaluateAll((els) => els.map((e) => e.offsetHeight));
  expect(new Set(heights).size).toBeGreaterThan(6);
  for (const h of heights) expect(h).toBeGreaterThan(10);
});

test("preview never uses the app screenshots", async ({ page }) => {
  await page.goto("/");
  expect(await page.locator("#hero img").count()).toBe(0);
  expect(await page.locator("#hero").innerHTML()).not.toContain("screenshots");
});

test("hero is one column under 900px and two columns on desktop", async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 900 });
  await page.goto("/");
  const copy = await page.locator("#hero .hero-copy").boundingBox();
  const preview = await page.locator("#hero .hero-preview").boundingBox();
  expect(preview.y).toBeGreaterThanOrEqual(copy.y + copy.height - 1);
  await page.setViewportSize({ width: 1280, height: 900 });
  const c2 = await page.locator("#hero .hero-copy").boundingBox();
  const p2 = await page.locator("#hero .hero-preview").boundingBox();
  expect(p2.x).toBeGreaterThan(c2.x + c2.width - 1);
});

for (const width of [320, 390]) {
  test(`hero has no horizontal scroll at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 800 });
    await page.goto("/");
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);
  });
}

test("at 390px the preview sidebar is hidden, at 1280px it is shown", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 800 });
  await page.goto("/");
  await expect(page.locator("#hero .mock-side")).toBeHidden();
  await expect(page.locator("#hero .stat").first()).toBeVisible();
  await page.setViewportSize({ width: 1280, height: 800 });
  await expect(page.locator("#hero .mock-side")).toBeVisible();
});

test("with reduced motion no animation runs", async ({ browser, baseURL }) => {
  const ctx = await browser.newContext({ reducedMotion: "reduce", baseURL });
  const page = await ctx.newPage();
  await page.goto("/");
  expect(await page.evaluate(() => document.getAnimations().length)).toBe(0);
  const names = await page
    .locator("#hero .bars span")
    .evaluateAll((els) => els.map((e) => getComputedStyle(e).animationName));
  expect(names).toHaveLength(12);
  expect(new Set(names)).toEqual(new Set(["none"]));
  await ctx.close();
});

test("without reduced motion the bars grow", async ({ browser, baseURL }) => {
  const ctx = await browser.newContext({ reducedMotion: "no-preference", baseURL });
  const page = await ctx.newPage();
  await page.goto("/");
  const name = await page.locator("#hero .bars span").first().evaluate((e) => getComputedStyle(e).animationName);
  expect(name).not.toBe("none");
  await ctx.close();
});

test("buttons follow DESIGN.md metrics", async ({ page }) => {
  await page.goto("/");
  const btn = await page.locator(".btn").first().evaluate((el) => getComputedStyle(el).padding);
  expect(btn).toBe("10px 15px");
  const small = await page.evaluate(() => [...document.querySelectorAll("body *")].filter((el) => el.childNodes.length && parseFloat(getComputedStyle(el).fontSize) === 9).length);
  expect(small).toBe(0);
});
