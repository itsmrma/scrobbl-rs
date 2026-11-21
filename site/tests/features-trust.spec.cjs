const { test, expect } = require("@playwright/test");

// Features panels and "Numbers you can trust" notices (ticket #221).

const PANELS = ["Collect", "Import", "Explore", "Curate", "Ask", "Keep"];
const NOTICES = [
  { kind: "success", title: /imported plays count from 30 seconds/i },
  { kind: "warning", title: /unknown stays unknown/i },
  { kind: "info", title: /duplicates/i },
];

test("features section shows the six panels with title, description and example", async ({ page }) => {
  await page.goto("/");
  const section = page.locator("section#features");
  await expect(section).toBeVisible();
  const panels = section.locator("article.panel");
  await expect(panels).toHaveCount(6);
  for (const [i, name] of PANELS.entries()) {
    const panel = panels.nth(i);
    await expect(panel.getByRole("heading", { level: 3, name })).toBeVisible();
    await expect(panel.locator("p").first()).not.toBeEmpty();
    await expect(panel.locator(".demo")).toBeVisible();
  }
});

test("Ask panel states the privacy facts and its labels", async ({ page }) => {
  await page.goto("/");
  const ask = page.locator("section#features article.panel", { hasText: "Ask" }).first();
  await expect(ask.getByText("Local model", { exact: true })).toBeVisible();
  await expect(ask.getByText("Off by default", { exact: true })).toBeVisible();
  const text = (await ask.innerText()).toLowerCase();
  expect(text).toContain("optional");
  expect(text).toContain("never sees");
  expect(text).toContain("sandbox");
  expect(text).toContain("local gguf");
  expect(text).toContain("qwen3.5 2b");
  expect(text).toContain("gpu offload");
  expect(text).toContain("metal");
  expect(text).toContain("vulkan");
  expect(text).toContain("cpu fallback");
});

test("Curate shows a half-star rating with an accessible label", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("#features [aria-label='4.5 stars']")).toBeVisible();
});

test("trust section shows three notices with icon, title and text", async ({ page }) => {
  await page.goto("/");
  const section = page.locator("section#trust");
  await expect(section.getByRole("heading", { level: 2, name: /Numbers you can trust/ })).toBeVisible();
  const notices = section.locator(".notice");
  await expect(notices).toHaveCount(3);
  for (const [i, n] of NOTICES.entries()) {
    const el = notices.nth(i);
    await expect(el).toHaveClass(new RegExp(`notice-${n.kind}`));
    await expect(el.locator("svg")).toBeVisible();
    await expect(el.locator("b")).toHaveText(n.title);
    await expect(el.locator("p")).not.toBeEmpty();
  }
});

for (const scheme of ["dark", "light"]) {
  test(`notice text and icons are readable in ${scheme} theme`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto("/");
    const ratios = await page.evaluate(() => {
      const ctx = document.createElement("canvas").getContext("2d", { willReadFrequently: true });
      ctx.canvas.width = ctx.canvas.height = 1;
      const rgba = (v) => {
        ctx.clearRect(0, 0, 1, 1);
        ctx.fillStyle = v;
        ctx.fillRect(0, 0, 1, 1);
        return Array.from(ctx.getImageData(0, 0, 1, 1).data);
      };
      const lum = (c) => {
        const [r, g, b] = c.slice(0, 3).map((v) => {
          const s = v / 255;
          return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
        });
        return 0.2126 * r + 0.7152 * g + 0.0722 * b;
      };
      const ratio = (a, b) => {
        const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
        return (hi + 0.05) / (lo + 0.05);
      };
      const out = [];
      for (const n of document.querySelectorAll("#trust .notice")) {
        const bg = rgba(getComputedStyle(n).backgroundColor);
        for (const sel of ["b", "p", "svg"]) {
          const color = getComputedStyle(n.querySelector(sel)).color;
          out.push({ sel, kind: n.className, r: ratio(rgba(color), bg) });
        }
      }
      return out;
    });
    expect(ratios.length).toBe(9);
    for (const x of ratios) expect(x.r, `${x.kind} ${x.sel}`).toBeGreaterThanOrEqual(x.sel === "svg" ? 3 : 4.5);
  });
}

test("navigation anchors point to existing sections", async ({ page }) => {
  await page.goto("/");
  for (const [name, id] of [["Features", "features"], ["How it counts", "trust"]]) {
    const link = page.getByRole("navigation", { name: "Primary" }).getByRole("link", { name });
    await expect(link).toHaveAttribute("href", `#${id}`);
    await expect(page.locator(`section#${id}`)).toHaveCount(1);
  }
});

for (const width of [320, 390]) {
  test(`features and trust do not scroll horizontally at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 800 });
    await page.goto("/");
    for (const id of ["features", "trust"]) {
      const over = await page.locator(`#${id}`).evaluate((el) => el.scrollWidth - el.clientWidth);
      expect(over, id).toBeLessThanOrEqual(0);
    }
    const doc = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(doc).toBeLessThanOrEqual(0);
  });
}

test("panel grid uses 3, 2 and 1 columns", async ({ page }) => {
  const cols = async (w) => {
    await page.setViewportSize({ width: w, height: 800 });
    return page.locator("#features .grid").evaluate((el) => getComputedStyle(el).gridTemplateColumns.split(" ").length);
  };
  await page.goto("/");
  expect(await cols(1280)).toBe(3);
  expect(await cols(800)).toBe(2);
  expect(await cols(390)).toBe(1);
});

test("sections use no inline style attributes", async ({ page }) => {
  await page.goto("/");
  expect(await page.locator("#features [style], #trust [style]").count()).toBe(0);
});
