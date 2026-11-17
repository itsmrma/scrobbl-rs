const { test, expect } = require("@playwright/test");

// Observable behaviour of the site as served by the Worker itself.

function watchConsole(page) {
  const problems = [];
  page.on("console", (m) => {
    // The browser logs the 404 status of the not-found document itself; that is expected.
    if (m.location().url.endsWith("/no-such-page")) return;
    if (m.type() === "error" || /content security policy|refused to/i.test(m.text())) {
      problems.push(m.text());
    }
  });
  page.on("pageerror", (e) => problems.push(String(e)));
  return problems;
}

// Resolves any CSS colour (including color-mix) to [r, g, b, a] through a canvas.
const RESOLVE = `(value) => {
  const ctx = document.createElement("canvas").getContext("2d", { willReadFrequently: true });
  ctx.canvas.width = ctx.canvas.height = 1;
  ctx.clearRect(0, 0, 1, 1);
  ctx.fillStyle = value;
  ctx.fillRect(0, 0, 1, 1);
  return Array.from(ctx.getImageData(0, 0, 1, 1).data);
}`;
async function colors(page, selectors) {
  return page.evaluate(([resolveSrc, sels]) => {
    const resolve = eval(resolveSrc);
    const out = {};
    for (const [k, [sel, prop]] of Object.entries(sels)) {
      out[k] = resolve(getComputedStyle(document.querySelector(sel))[prop]);
    }
    return out;
  }, [RESOLVE, selectors]);
}

const PRESETS = ["Lime", "Sky", "Lavender", "Peach", "Rose", "Mint"];
const picker = (page) => page.getByRole("group", { name: "Accent colour" });

function contrast(a, b) {
  const lum = (rgb) => {
    const c = rgb.slice(0, 3).map((v) => {
      const s = v / 255;
      return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
  };
  const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

test("home page shows navigation, title with accent dot and footer", async ({ page }) => {
  const res = await page.goto("/");
  expect(res.status()).toBe(200);
  const nav = page.getByRole("navigation", { name: "Primary" });
  await expect(page.getByRole("banner").getByRole("link", { name: /scrobbl\.rs/ })).toBeVisible();
  for (const name of ["Features", "How it counts", "Install", "GitHub"]) {
    await expect(nav.getByRole("link", { name })).toBeVisible();
  }
  await expect(nav.getByRole("link", { name: "Get started" })).toBeVisible();
  const h1 = page.getByRole("heading", { level: 1 });
  await expect(h1).toBeVisible();
  await expect(h1.locator(".accent")).toHaveText(".");
  const footer = page.getByRole("contentinfo");
  await expect(footer).toContainText("GPL-3.0-or-later");
  await expect(footer.getByRole("link", { name: "Maloja" })).toBeVisible();
  await expect(footer).toContainText(/demo data/i);
});

test("dark is the default theme and light follows the system", async ({ page }) => {
  const bodyBg = async () => (await colors(page, { bg: ["body", "backgroundColor"] })).bg;
  const luminance = ([r, g, b]) => (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/");
  expect(luminance(await bodyBg())).toBeLessThan(0.2);
  await page.emulateMedia({ colorScheme: "light" });
  expect(luminance(await bodyBg())).toBeGreaterThan(0.8);
});

for (const scheme of ["dark", "light"]) {
  test(`title dot and body text are readable in ${scheme} theme`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto("/");
    const c = await colors(page, {
      bg: ["body", "backgroundColor"],
      text: ["body", "color"],
      dot: ["h1 .accent", "color"],
    });
    expect(contrast(c.text, c.bg)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(c.dot, c.bg)).toBeGreaterThanOrEqual(4.5);
  });
}

test("unknown path answers 404 with the site page and a link home", async ({ page }) => {
  const res = await page.goto("/no-such-page");
  expect(res.status()).toBe(404);
  await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
  await expect(page.getByRole("link", { name: "Back to home" })).toHaveAttribute("href", "/");
  await page.emulateMedia({ colorScheme: "dark" });
  const bg = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
  expect(bg).not.toBe("rgba(0, 0, 0, 0)");
  expect(bg).not.toBe("rgb(255, 255, 255)");
});

test("responses carry the security headers", async ({ request }) => {
  for (const path of ["/", "/no-such-page"]) {
    const res = await request.get(path);
    const h = res.headers();
    const csp = h["content-security-policy"];
    expect(csp, path).toContain("default-src 'self'");
    expect(csp).toContain("frame-ancestors 'none'");
    expect(csp).toContain("base-uri 'none'");
    expect(csp).toContain("form-action 'none'");
    expect(h["referrer-policy"]).toBe("strict-origin-when-cross-origin");
    expect(h["x-content-type-options"]).toBe("nosniff");
    expect(h["permissions-policy"]).toBeTruthy();
  }
});

test("no third-party requests, no console errors or CSP violations", async ({ page, baseURL }) => {
  const problems = watchConsole(page);
  const foreign = [];
  const origin = new URL(baseURL).origin;
  page.on("request", (r) => {
    if (/^https?:/.test(r.url()) && new URL(r.url()).origin !== origin) foreign.push(r.url());
  });
  await page.goto("/");
  await page.waitForLoadState("networkidle");
  await page.goto("/no-such-page");
  await page.waitForLoadState("networkidle");
  expect(foreign).toEqual([]);
  expect(problems).toEqual([]);
});

for (const width of [320, 390, 768, 1280]) {
  test(`no horizontal scroll at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 800 });
    for (const path of ["/", "/no-such-page"]) {
      await page.goto(path);
      const overflow = await page.evaluate(
        () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
      );
      expect(overflow, `${path} at ${width}`).toBeLessThanOrEqual(0);
    }
  });
}

test("controls are keyboard reachable and have accessible names", async ({ page, browserName }) => {
  await page.goto("/");
  const controls = page.locator("a[href], button");
  const count = await controls.count();
  expect(count).toBeGreaterThan(0);
  for (let i = 0; i < count; i++) {
    const c = controls.nth(i);
    if (!(await c.isVisible())) continue;
    const name = await c.evaluate(
      (el) => (el.getAttribute("aria-label") || el.textContent || "").trim(),
    );
    expect(name, `control #${i}`).not.toBe("");
  }
  // None is removed from the tab order.
  expect(await controls.evaluateAll((els) => els.filter((e) => e.tabIndex < 0 || e.disabled).length)).toBe(0);
  // WebKit on macOS does not Tab to links by default (Safari setting), so the
  // real Tab traversal runs on the other engines.
  test.skip(browserName === "webkit", "WebKit does not Tab to links by default");
  // Tab from the top: every visible control receives focus.
  const visible = await controls.evaluateAll((els) => els.filter((e) => e.offsetParent).length);
  const seen = new Set();
  for (let i = 0; i < visible + 2; i++) {
    await page.keyboard.press("Tab");
    seen.add(await page.evaluate(() => {
      const el = document.activeElement;
      return el && el !== document.body ? Array.from(document.querySelectorAll("a[href], button")).indexOf(el) : -1;
    }));
  }
  seen.delete(-1);
  expect(seen.size).toBe(visible);
});

test("accent picker offers six labelled, large, pressable swatches", async ({ page }) => {
  await page.goto("/");
  const swatches = picker(page).getByRole("button");
  await expect(swatches).toHaveCount(6);
  for (const name of PRESETS) {
    const b = picker(page).getByRole("button", { name, exact: true });
    const box = await b.boundingBox();
    expect(box.width, name).toBeGreaterThanOrEqual(44);
    expect(box.height, name).toBeGreaterThanOrEqual(44);
  }
  await expect(picker(page).getByRole("button", { name: "Lime", exact: true })).toHaveAttribute("aria-pressed", "true");
  await expect(picker(page).getByRole("button", { name: "Sky", exact: true })).toHaveAttribute("aria-pressed", "false");
});

test("choosing a swatch recolours button, dot and surfaces, and reload returns to lime", async ({ page }) => {
  await page.goto("/");
  const sel = {
    btn: [".btn.primary", "backgroundColor"],
    dot: ["h1 .accent", "color"],
    bg: ["body", "backgroundColor"],
    border: [".nav", "borderBottomColor"],
  };
  const before = await colors(page, sel);
  await picker(page).getByRole("button", { name: "Lavender", exact: true }).click();
  const after = await colors(page, sel);
  // Lavender #c5a3ee
  expect(after.btn.slice(0, 3)).toEqual([0xc5, 0xa3, 0xee]);
  expect(after.btn).not.toEqual(before.btn);
  expect(after.dot).not.toEqual(before.dot);
  expect(after.bg).not.toEqual(before.bg);
  expect(after.border).not.toEqual(before.border);
  await expect(picker(page).getByRole("button", { name: "Lavender", exact: true })).toHaveAttribute("aria-pressed", "true");
  await expect(picker(page).getByRole("button", { name: "Lime", exact: true })).toHaveAttribute("aria-pressed", "false");
  await page.reload();
  expect((await colors(page, sel)).btn.slice(0, 3)).toEqual([0xc9, 0xee, 0x8b]);
  await expect(picker(page).getByRole("button", { name: "Lime", exact: true })).toHaveAttribute("aria-pressed", "true");
});

test("swatches work from the keyboard and show a focus ring", async ({ page }) => {
  await page.goto("/");
  const sky = picker(page).getByRole("button", { name: "Sky", exact: true });
  await sky.focus();
  expect(await sky.evaluate((el) => getComputedStyle(el).outlineStyle)).not.toBe("none");
  await page.keyboard.press("Enter");
  await expect(sky).toHaveAttribute("aria-pressed", "true");
});

for (const scheme of ["dark", "light"]) {
  test(`primary button text has 4.5:1 contrast for every preset in ${scheme} theme`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto("/");
    for (const name of PRESETS) {
      await picker(page).getByRole("button", { name, exact: true }).click();
      const c = await colors(page, { bg: [".btn.primary", "backgroundColor"], fg: [".btn.primary", "color"] });
      expect(contrast(c.fg, c.bg), name).toBeGreaterThanOrEqual(4.5);
    }
  });
}
