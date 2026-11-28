const { test, expect } = require("@playwright/test");
const { chooseOption } = require("./controls.cjs");

const preferences = {
  name: "Skeleton test archive",
  initial_period: "all",
  hour_format: "24",
  timezone: "UTC",
  week_start: 1,
  version: 1,
};
const spotify = {
  configured: false,
  connected: false,
  connection: "disconnected",
  collection: "paused",
  imports: [],
  sync_seconds: 60,
  now_playing_seconds: 30,
  review_count: 0,
};

test.beforeEach(async ({ page }) => {
  // UI-only fixtures: no owner account, personal archive or external provider.
  await page.route("**/api/**", (route) => {
    const path = new URL(route.request().url()).pathname;
    const responses = {
      "/api/session": { authenticated: true, csrf: "test" },
      "/api/v1/preferences": preferences,
      "/api/spotify/status": spotify,
      "/api/v1/account": { username: "owner", name: "Owner", avatar_url: null },
      "/api/v1/activities": [],
      "/api/v1/recovery": {
        assets: [],
        paused: false,
        restart_required: false,
      },
      "/api/v1/review": {
        total: 0,
        groups: [],
        recent: [],
        decisions_total: 0,
      },
    };
    return route.fulfill({
      status: path in responses ? 200 : 503,
      json: responses[path] ?? { error: "Fixture unavailable" },
    });
  });
});

async function hold(page, pattern) {
  let release;
  const gate = new Promise((resolve) => (release = resolve));
  await page.route(pattern, async (route) => {
    const response = await gate;
    await route.fulfill(response);
  });
  return release;
}

const cases = [
  [
    "music editor",
    "/music/tracks/test/edit",
    "**/api/v1/library/tracks/test",
    "Loading details",
  ],
  [
    "listening patterns",
    "/music/artists/test",
    "**/api/v1/consultation/patterns?*",
    "Loading listening patterns",
  ],
  ["overview", "/dashboard", "**/api/dashboard?*", "Loading your archive"],
  ["history", "/history", "**/api/history?*", "Loading listening history"],
  [
    "rankings",
    "/rankings/artists",
    "**/api/v1/consultation/rankings/artists?*",
    "Loading rankings",
  ],
  [
    "music",
    "/music/artists/test",
    "**/api/v1/consultation/music/artists/test?*",
    "Loading music page",
  ],
  ["account", "/settings/account", "**/api/v1/account", "Loading account"],
  ["API keys", "/settings/api-keys", "**/api/v1/api-keys", "Loading API keys"],
  [
    "image providers",
    "/settings/images",
    "**/api/v1/artwork-providers",
    "Loading image providers",
  ],
  [
    "connection",
    "/settings/connections",
    "**/api/spotify/status",
    "Loading connection status",
  ],
  ["backups", "/settings/data", "**/api/v1/recovery", "Loading backups…"],
  ["activities", "/activities", "**/api/v1/activities", "Loading activities"],
  [
    "activity report",
    "/activities/test",
    "**/api/v1/activities/test",
    "Loading activity report",
  ],
  [
    "Spotify import",
    "/import",
    "**/api/v1/import/limits",
    "Checking upload availability",
  ],
  [
    "ListenBrainz import",
    "/import",
    "**/api/v1/listenbrainz/limits",
    "Loading ListenBrainz upload limits",
  ],
  [
    "automatic review",
    "/review",
    "**/api/v1/review/automatic",
    "Checking automatic matches",
  ],
  [
    "Spotify callback",
    `/spotify/connect-result/${"a".repeat(64)}`,
    `**/api/spotify/connect/${"a".repeat(64)}`,
    "Checking the connection",
  ],
];

for (const [name, path, endpoint, label] of cases) {
  test(`${name}: skeleton while reading, error replaces it`, async ({
    page,
  }) => {
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    const release = await hold(page, endpoint);
    try {
      await page.goto(path);
      if (name === "automatic review")
        await page
          .getByRole("heading", {
            name: "Resolve duplicates automatically",
          })
          .click();
      const skeleton = page
        .locator('.skeleton-container[aria-busy="true"]')
        .filter({ hasText: label });
      await expect(skeleton).toBeVisible();
      await expect(skeleton.locator(".skeleton-block").first()).toBeVisible();
      await expect(skeleton.locator(".visually-hidden").first()).toHaveCSS(
        "position",
        "absolute",
      );
      if (name === "activities")
        await expect(
          page.getByRole("heading", { name: "No activities to show" }),
        ).toHaveCount(0);
      release({ status: 503, json: { error: "Delayed read failed" } });
      await expect(skeleton).toHaveCount(0);
      await expect(
        page
          .getByRole(name === "Spotify callback" ? "status" : "alert")
          .filter({ hasText: "Delayed read failed" })
          .first(),
      ).toBeVisible();
      expect(errors).toEqual([]);
    } finally {
      release({ status: 503, json: { error: "Delayed read failed" } });
    }
  });
}

test("entity search replaces finding text with a skeleton and then results", async ({
  page,
}) => {
  const release = await hold(page, "**/api/v1/consultation/choices/artists?*");
  try {
    await page.goto("/history");
    await page.locator(".history-filter-disclosure > summary").click();
    await page.getByRole("searchbox", { name: "Find artist" }).fill("sample");
    const skeleton = page.locator(".entity-options .skeleton-container");
    await expect(skeleton).toBeVisible();
    release({ json: [] });
    await expect(page.locator(".entity-options")).toContainText("No matches");
    await expect(skeleton).toHaveCount(0);
  } finally {
    release({ json: [] });
  }
});

for (const [width, scheme] of [
  [390, "light"],
  [1280, "dark"],
]) {
  test(`activity skeleton fits ${width}px in ${scheme} with reduced motion`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 844 });
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
    const release = await hold(page, "**/api/v1/activities");
    try {
      await page.goto("/activities");
      const skeleton = page
        .locator("#main .skeleton-container")
        .filter({ hasText: "Loading activities" });
      await expect(skeleton).toBeVisible();
      await expect(skeleton.locator(".skeleton-block").first()).toHaveCSS(
        "animation-name",
        "none",
        { pseudo: "after" },
      );
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
      await page.screenshot({
        path: testInfo.outputPath(`skeleton-${width}-${scheme}.png`),
        fullPage: true,
      });
    } finally {
      release({ json: [] });
    }
  });
}

test("Now Playing distinguishes a delayed read, a successful null and background polling", async ({
  page,
}) => {
  await page.route("**/api/spotify/status", (route) =>
    route.fulfill({
      json: {
        ...spotify,
        connected: true,
        configured: true,
        now_playing_seconds: 1,
      },
    }),
  );
  const release = await hold(page, "**/api/spotify/now-playing");
  await page.goto("/music/now-playing");
  const skeleton = page
    .locator("#main .skeleton-container")
    .filter({ hasText: "Loading now playing" });
  await expect(skeleton).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Nothing playing" }),
  ).toHaveCount(0);
  release({ status: 200, json: null });
  await expect(
    page.getByRole("heading", { name: "Nothing playing" }),
  ).toBeVisible();
  const finishPoll = await hold(page, "**/api/spotify/now-playing");
  try {
    await page.waitForRequest("**/api/spotify/now-playing");
    await expect(skeleton).toHaveCount(0);
    await expect(
      page.getByRole("heading", { name: "Nothing playing" }),
    ).toBeVisible();
  } finally {
    finishPoll({ status: 200, json: null });
  }
});

test("filtered activities do not flash an empty state while reading", async ({
  page,
}) => {
  await page.goto("/activities");
  await expect(
    page.getByRole("heading", { name: "No activities to show" }),
  ).toBeVisible();
  const release = await hold(page, "**/api/v1/activities?*");
  try {
    await chooseOption(
      page.getByRole("combobox", { name: "Activity type" }),
      "spotify_import",
    );
    await expect(
      page
        .locator(".skeleton-container")
        .filter({ hasText: "Loading activities" }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "No activities to show" }),
    ).toHaveCount(0);
    release({ json: [] });
    await expect(
      page.getByRole("heading", { name: "No activities to show" }),
    ).toBeVisible();
  } finally {
    release({ json: [] });
  }
});

test("overview updates period and custom dates in-place without skeletons or empty-custom fetch", async ({
  page,
}) => {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));

  const makeDashboard = (listens) => ({
    totals: {
      records: listens,
      listens,
      tracks: 5,
      actual_ms: listens * 60000,
      estimated_ms: 0,
      unknown_durations: 0,
    },
    rankings: {
      artists: [],
      albums: [],
      tracks: [],
    },
    recent: [],
    timezone: "UTC",
  });

  await page.route("**/api/v1/consultation/activity?*", (route) =>
    route.fulfill({ json: { grain: "day", buckets: [], mixed_week_starts: false } }),
  );
  await page.route("**/api/v1/consultation/patterns?*", (route) =>
    route.fulfill({ json: { total: 0, series: {}, coverage: {} } }),
  );
  await page.route("**/api/v1/consultation/discovery-ratio?*", (route) =>
    route.fulfill({
      json: { total_listens: 0, new_tracks: 0, repeat_listens: 0, discovery_pct: 0 },
    }),
  );

  let dashboardRequests = 0;
  await page.route("**/api/dashboard?*", (route) => {
    dashboardRequests++;
    return route.fulfill({ json: makeDashboard(42) });
  });

  await page.goto("/dashboard");
  await expect(page.locator("#total-listens")).toHaveText("42");

  // 1. Changing Period updates in-place without showing skeletons
  const releasePeriod = await hold(page, "**/api/dashboard?*");
  try {
    await chooseOption(page.getByLabel("Period", { exact: true }), "month");
    await expect(page.locator("#total-listens")).toHaveText("42");
    await expect(page.locator(".skeleton-container:visible")).toHaveCount(0);
    releasePeriod({ json: makeDashboard(12) });
    await expect(page.locator("#total-listens")).toHaveText("12");
  } finally {
    releasePeriod({ json: makeDashboard(12) });
  }

  // 2. Selecting Custom dates with empty From & Through shows inputs without firing a fetch or skeleton
  const requestsBeforeCustom = dashboardRequests;
  await page.unroute("**/api/dashboard?*");
  await page.route("**/api/dashboard?*", (route) => {
    dashboardRequests++;
    return route.fulfill({ json: makeDashboard(12) });
  });

  await chooseOption(page.getByLabel("Period", { exact: true }), "custom");
  const fromInput = page.getByLabel("From", { exact: true });
  const throughInput = page.getByLabel("Through", { exact: true });
  await expect(fromInput).toBeVisible();
  await expect(throughInput).toBeVisible();
  await expect(page.locator(".skeleton-container:visible")).toHaveCount(0);
  await expect(page.locator("#total-listens")).toHaveText("12");
  expect(dashboardRequests).toBe(requestsBeforeCustom);

  // 3. Filling From and Through updates in-place and preserves focus
  const releaseFrom = await hold(page, "**/api/dashboard?*");
  try {
    await fromInput.focus();
    await fromInput.fill("2025-06-01");
    await expect(fromInput).toBeFocused();
    await expect(page.locator(".skeleton-container:visible")).toHaveCount(0);
    await expect(page.locator("#total-listens")).toHaveText("12");
    releaseFrom({ json: makeDashboard(7) });
    await expect(page.locator("#total-listens")).toHaveText("7");
    await expect(fromInput).toBeFocused();
  } finally {
    releaseFrom({ json: makeDashboard(7) });
  }

  const releaseThrough = await hold(page, "**/api/dashboard?*");
  try {
    await throughInput.focus();
    await throughInput.fill("2025-06-02");
    await expect(throughInput).toBeFocused();
    await expect(page.locator(".skeleton-container:visible")).toHaveCount(0);
    releaseThrough({ json: makeDashboard(3) });
    await expect(page.locator("#total-listens")).toHaveText("3");
    await expect(throughInput).toBeFocused();
  } finally {
    releaseThrough({ json: makeDashboard(3) });
  }

  expect(errors).toEqual([]);
});

test("music page keeps header and rating mounted on period change and shows skeleton on entity change", async ({
  page,
}) => {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));

  const makeArtist = (id, name, listens) => ({
    id,
    name,
    kind: "artists",
    cover_url: null,
    album_id: null,
    album: null,
    artists: [],
    totals: {
      records: listens,
      listens,
      tracks: 3,
      actual_ms: listens * 60000,
      estimated_ms: 0,
      unknown_durations: 0,
    },
    rank: 1,
  });

  await page.route("**/api/v1/library/artists/*", (route) =>
    route.fulfill({
      json: {
        id: "test",
        name: "Test artist",
        original_name: "Test artist",
        rating: 4,
        custom_image: false,
      },
    }),
  );
  await page.route("**/api/v1/consultation/activity?*", (route) =>
    route.fulfill({ json: { grain: "day", buckets: [], mixed_week_starts: false } }),
  );
  await page.route("**/api/v1/consultation/patterns?*", (route) =>
    route.fulfill({ json: { total: 0, series: {}, coverage: {} } }),
  );
  await page.route("**/api/v1/consultation/rankings/**", (route) =>
    route.fulfill({ json: { items: [], total: 0, page: 1, page_size: 50 } }),
  );
  await page.route("**/api/v1/consultation/calendar?*", (route) =>
    route.fulfill({ json: { days: [], years: [], total_listens: 0 } }),
  );
  await page.route("**/api/v1/consultation/milestones?*", (route) =>
    route.fulfill({
      json: {
        first_listen: null,
        last_listen: null,
        current_streak: 0,
        longest_streak: 0,
        milestones: [],
      },
    }),
  );
  await page.route("**/api/history?*", (route) =>
    route.fulfill({ json: { items: [], total: 0, page: 1, page_size: 50 } }),
  );

  let musicRequests = 0;
  await page.route("**/api/v1/consultation/music/artists/test?*", (route) => {
    musicRequests++;
    return route.fulfill({ json: makeArtist("test", "Test artist", 42) });
  });

  await page.goto("/music/artists/test");
  await expect(
    page.getByRole("heading", { name: "Test artist." }),
  ).toBeVisible();
  await expect(page.locator("#total-listens")).toHaveText("42");
  await expect(page.locator(".music-personal")).toBeVisible();

  // 1. Changing Period keeps header, personal rating, and PeriodFilters mounted without skeletons
  const releasePeriod = await hold(
    page,
    "**/api/v1/consultation/music/artists/test?*",
  );
  try {
    await chooseOption(page.getByLabel("Period", { exact: true }), "month");
    await expect(
      page.getByRole("heading", { name: "Test artist." }),
    ).toBeVisible();
    await expect(page.locator(".music-personal")).toBeVisible();
    await expect(page.locator(".skeleton-container:visible")).toHaveCount(0);
    await expect(page.locator("#total-listens")).toHaveText("42");
    releasePeriod({ json: makeArtist("test", "Test artist", 10) });
    await expect(page.locator("#total-listens")).toHaveText("10");
  } finally {
    releasePeriod({ json: makeArtist("test", "Test artist", 10) });
  }

  // 2. Selecting Custom dates with empty From & Through does not fire a fetch or show skeleton
  const requestsBeforeCustom = musicRequests;
  await page.unroute("**/api/v1/consultation/music/artists/test?*");
  await page.route("**/api/v1/consultation/music/artists/test?*", (route) => {
    musicRequests++;
    return route.fulfill({ json: makeArtist("test", "Test artist", 10) });
  });
  await chooseOption(page.getByLabel("Period", { exact: true }), "custom");
  const fromInput = page.getByLabel("From", { exact: true });
  await expect(fromInput).toBeVisible();
  await expect(page.locator(".skeleton-container:visible")).toHaveCount(0);
  expect(musicRequests).toBe(requestsBeforeCustom);

  // 3. Filling From updates in-place and preserves focus
  const releaseFrom = await hold(
    page,
    "**/api/v1/consultation/music/artists/test?*",
  );
  try {
    await fromInput.focus();
    await fromInput.fill("2025-06-01");
    await expect(fromInput).toBeFocused();
    await expect(page.locator(".skeleton-container:visible")).toHaveCount(0);
    releaseFrom({ json: makeArtist("test", "Test artist", 5) });
    await expect(page.locator("#total-listens")).toHaveText("5");
    await expect(fromInput).toBeFocused();
  } finally {
    releaseFrom({ json: makeArtist("test", "Test artist", 5) });
  }

  // 4. Navigating to a different entity still shows the initial loading skeleton
  const releaseOther = await hold(
    page,
    "**/api/v1/consultation/music/artists/other?*",
  );
  try {
    await page.goto("/music/artists/other");
    const skeleton = page
      .locator('.skeleton-container[aria-busy="true"]')
      .filter({ hasText: "Loading music page" });
    await expect(skeleton).toBeVisible();
    releaseOther({ json: makeArtist("other", "Other artist", 8) });
    await expect(
      page.getByRole("heading", { name: "Other artist." }),
    ).toBeVisible();
  } finally {
    releaseOther({ json: makeArtist("other", "Other artist", 8) });
  }

  expect(errors).toEqual([]);
});
