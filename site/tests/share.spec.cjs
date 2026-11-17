const { test, expect } = require("@playwright/test");

// Share metadata: what a link preview or crawler sees.

const meta = (page, attr, name) =>
  page.locator(`head meta[${attr}="${name}"]`).getAttribute("content");

test("page declares title, description, Open Graph, Twitter card and theme colour", async ({ page }) => {
  await page.goto("/");
  await expect(page).toHaveTitle("scrobbl.rs");
  expect((await meta(page, "name", "description")).length).toBeGreaterThan(30);
  expect(await meta(page, "property", "og:title")).toBe("scrobbl.rs");
  expect(await meta(page, "property", "og:type")).toBe("website");
  expect(await meta(page, "property", "og:description")).toBe(await meta(page, "name", "description"));
  expect(await meta(page, "name", "twitter:card")).toBe("summary_large_image");
  expect(await meta(page, "name", "twitter:title")).toBe("scrobbl.rs");
  expect(await meta(page, "name", "twitter:image")).toBe(await meta(page, "property", "og:image"));
  expect(await meta(page, "name", "theme-color")).toMatch(/^#[0-9a-f]{6}$/i);
  await expect(page.locator('head link[rel="icon"][type="image/svg+xml"]')).toHaveAttribute("href", /\.svg$/);
});

test("canonical, og:url and og:image share the one production origin", async ({ page, request }) => {
  await page.goto("/");
  const canonical = await page.locator('head link[rel="canonical"]').getAttribute("href");
  expect(canonical).toMatch(/^https:\/\/[^/]+\/$/);
  expect(await meta(page, "property", "og:url")).toBe(canonical);
  const origin = new URL(canonical).origin;
  const image = await meta(page, "property", "og:image");
  expect(image.startsWith(`${origin}/`)).toBe(true);

  // The image is served by the site itself (same path on the local server).
  const res = await request.get(new URL(image).pathname);
  expect(res.status()).toBe(200);
  expect(res.headers()["content-type"]).toMatch(/^image\//);
  expect((await res.body()).length).toBeGreaterThan(1000);

  const robots = await request.get("/robots.txt");
  expect(robots.status()).toBe(200);
  const robotsText = await robots.text();
  expect(robotsText).toMatch(/^User-agent: \*/m);
  expect(robotsText).toContain(`Sitemap: ${origin}/sitemap.xml`);

  const sitemap = await request.get("/sitemap.xml");
  expect(sitemap.status()).toBe(200);
  const xml = await sitemap.text();
  expect(xml.match(/<loc>/g)).toHaveLength(1);
  expect(xml).toContain(`<loc>${canonical}</loc>`);
});

test("sharing metadata adds no external request", async ({ page }) => {
  const external = [];
  page.on("request", (r) => {
    if (!/^(http:\/\/127\.0\.0\.1|data:|blob:)/.test(r.url())) external.push(r.url());
  });
  await page.goto("/");
  await page.waitForLoadState("networkidle");
  expect(external).toEqual([]);
});
