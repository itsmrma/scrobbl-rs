const { expect } = require("@playwright/test");

module.exports = async function pendingNavigation(page) {
  await page.addInitScript(() => {
    const fetchRequest = window.fetch.bind(window);
    window.fetch = (input, options) => {
      const url =
        typeof input === "string" ? input : input.url || String(input);
      if (url.includes("/api/v1/preferences"))
        window.pendingPreference = { observed: true, signal: options?.signal };
      return fetchRequest(input, options);
    };
  });
  let release;
  const gate = new Promise((resolve) => (release = resolve));
  await page.route("**/api/v1/preferences", async (route) => {
    await gate;
    await route.continue().catch(() => {});
  });
  try {
    await page.goto("/activities", { waitUntil: "domcontentloaded" });
    await expect
      .poll(() => page.evaluate(() => !!window.pendingPreference?.observed))
      .toBe(true);
    await page.evaluate(() => window.dispatchEvent(new Event("beforeunload")));
    await expect
      .poll(() =>
        page.evaluate(() => !!window.pendingPreference?.signal?.aborted),
      )
      .toBe(true);
    await page.goto("/history", { waitUntil: "domcontentloaded" });
    release();
    await expect(
      page.getByRole("heading", { name: "Listening history.", exact: true }),
    ).toBeVisible();
  } finally {
    release();
    await page.unrouteAll({ behavior: "ignoreErrors" });
  }
};
