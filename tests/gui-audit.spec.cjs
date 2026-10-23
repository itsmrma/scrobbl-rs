const { test: base, expect } = require("@playwright/test");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const { mkdtemp, rm } = require("node:fs/promises");
const { tmpdir } = require("node:os");
const { join, resolve } = require("node:path");
const net = require("node:net");
const test = base.extend({
  auditApp: [
    async ({ playwright }, use) => {
      const directory = await mkdtemp(join(tmpdir(), "scrobbl-gui-"));
      const socket = net.createServer();
      socket.listen(0, "127.0.0.1");
      await once(socket, "listening");
      const port = socket.address().port;
      await new Promise((resolve) => socket.close(resolve));
      const baseURL = `http://127.0.0.1:${port}`;
      const child = spawn(
        resolve(
          "target/debug/scrobbl-rs" +
            (process.platform === "win32" ? ".exe" : ""),
        ),
        [],
        {
          env: {
            ...process.env,
            SCROBBL_PORT: String(port),
            SCROBBL_BIND: "127.0.0.1",
            SCROBBL_DATA_DIR: directory,
            SCROBBL_EXTERNAL_ORIGIN: baseURL,
            SPOTIFY_CLIENT_ID: "",
            SPOTIFY_CLIENT_SECRET: "",
          },
          stdio: ["ignore", "pipe", "pipe"],
        },
      );
      let logs = "";
      child.stdout.on("data", (chunk) => (logs += chunk));
      child.stderr.on("data", (chunk) => (logs += chunk));
      const request = await playwright.request.newContext({ baseURL });
      try {
        await expect
          .poll(async () => {
            if (child.exitCode !== null) throw new Error(logs);
            try {
              return (await request.get("/healthz")).status();
            } catch {
              return 0;
            }
          })
          .toBe(200);
        const session = await (
          await request.post("/api/setup", {
            data: {
              username: "owner",
              password: "a local browser test password",
            },
            headers: { Origin: baseURL },
          })
        ).json();
        const received = await (
          await request.post("/api/import/spotify", {
            headers: { Origin: baseURL, "X-CSRF-Token": session.csrf },
            multipart: {
              files: {
                name: "seed.json",
                mimeType: "application/json",
                buffer: Buffer.from(
                  JSON.stringify([
                    {
                      ts: "2025-05-01T12:03:00Z",
                      ms_played: 180000,
                      master_metadata_track_name: "Seed song",
                      master_metadata_album_artist_name: "Seed artist",
                      master_metadata_album_album_name: "Seed album",
                      spotify_track_uri: "spotify:track:0000000000000000000088",
                    },
                  ]),
                ),
              },
            },
          })
        ).json();
        await expect
          .poll(
            async () =>
              (
                await (
                  await request.get(`/api/v1/activities/${received.job_id}`)
                ).json()
              ).activity.state,
          )
          .toBe("completed");
        await use(baseURL);
      } finally {
        await request.dispose();
        if (child.exitCode === null) {
          const stopped = once(child, "exit");
          child.kill("SIGINT");
          await stopped;
        }
        await rm(directory, { recursive: true, force: true });
      }
    },
    { scope: "test" },
  ],
  baseURL: async ({ auditApp }, use) => use(auditApp),
});

async function login(page, path = "/dashboard") {
  const session = await (await page.request.get("/api/session")).json();
  await page.goto(path);
  await expect(page.getByLabel("Password", { exact: true })).toBeVisible();
  await page.screenshot({
    path: `test-results/gui-${page.context().browser().browserType().name()}-login.png`,
    fullPage: true,
    animations: "disabled",
  });
  await page.getByLabel("Username", { exact: true }).fill("owner");
  await page
    .getByLabel("Password", { exact: true })
    .fill("a local browser test password");
  if (session.setup_required)
    await page
      .getByLabel("Confirm password")
      .fill("a local browser test password");
  await page
    .getByRole("button", {
      name: session.setup_required ? "Create your archive" : "Sign in",
      exact: true,
    })
    .click();
  await expect(page.locator("#main")).toBeVisible();
}

test("extended-history drop selects actual files without opening a browser dialog", async ({
  page,
}) => {
  await login(page, "/import");
  const transfer = await page.evaluateHandle(() => {
    const data = new DataTransfer();
    data.items.add(
      new File(
        [
          JSON.stringify([
            {
              ts: "2025-06-01T12:03:00Z",
              ms_played: 180000,
              master_metadata_track_name: "Dropped song",
              master_metadata_album_artist_name: "Drop artist",
              master_metadata_album_album_name: "Drop album",
              spotify_track_uri: "spotify:track:0000000000000000000099",
            },
          ]),
        ],
        "Streaming_History_Audio_drop.json",
        { type: "application/json" },
      ),
    );
    return data;
  });
  await page
    .locator("#spotify-dropzone")
    .dispatchEvent("drop", { dataTransfer: transfer });
  await expect(
    page.getByText("Streaming_History_Audio_drop.json", { exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Import listening history", exact: true })
    .click();
  await expect(page.locator("#import-result")).toContainText(
    "Import complete",
    { timeout: 15000 },
  );
  await expect(page.locator("#import-result")).toContainText("1 added");
  await transfer.dispose();
});

test("listening-time explanation works with keyboard and touch-sized trigger", async ({
  page,
}) => {
  await login(page);
  const help = page
    .getByRole("button", { name: "How we calculate listening time" })
    .first();
  await help.focus();
  await expect(
    page.getByRole("region", { name: "How we calculate listening time" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(help).toHaveAttribute("aria-expanded", "false");
  await help.click();
  await expect(
    page.getByRole("region", { name: "How we calculate listening time" }),
  ).toContainText("Missing information remains unknown");
  const box = await help.boundingBox();
  expect(box.width).toBeGreaterThanOrEqual(44);
  expect(box.height).toBeGreaterThanOrEqual(44);
});

test("personal names, half-star ratings and custom images persist without changing source names", async ({
  page,
  baseURL,
  browserName,
}) => {
  await login(page);
  const dashboard = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  for (const kind of ["artists", "albums", "tracks"]) {
    const item = dashboard.rankings[kind][0];
    expect(item).toBeTruthy();
    await page.goto(`/music/${kind}/${item.id}/edit`);
    const label = kind === "artists" ? "Artist name" : "Title";
    await expect(page.getByLabel(label, { exact: true })).toBeVisible();
    if (kind === "artists")
      await page.screenshot({
        path: `test-results/gui-${browserName}-edit.png`,
        fullPage: true,
        animations: "disabled",
      });
    await page.getByLabel(label, { exact: true }).fill(`Personal ${kind}`);
    await page
      .getByLabel("Rating in stars", { exact: true })
      .selectOption("3.5");
    if (kind !== "tracks") {
      // Saving artwork must not discard an unsaved name or rating in the other form.
      await page.getByLabel("Upload an image").setInputFiles({
        name: "art.png",
        mimeType: "image/png",
        buffer: Buffer.from(
          "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=",
          "base64",
        ),
      });
      await page
        .getByRole("button", { name: "Save image", exact: true })
        .click();
      await expect(
        page.getByRole("button", { name: "Use Spotify image" }),
      ).toBeEnabled();
      await expect(page.getByLabel(label, { exact: true })).toHaveValue(
        `Personal ${kind}`,
      );
      await expect(page.getByLabel("Rating in stars")).toHaveValue("3.5");
      const image = await page.request.get(
        `/api/v1/artwork/${kind}/${item.id}`,
      );
      expect(image.status()).toBe(200);
      expect(image.headers()["content-type"]).toBe("image/png");
      await page
        .getByLabel("Or paste an image URL")
        .fill("https://127.0.0.1/private.png");
      await page
        .getByRole("button", { name: "Save image", exact: true })
        .click();
      await expect(page.getByRole("alert").first()).toContainText(
        "public internet",
      );
    }
    await page
      .getByRole("button", { name: "Save details", exact: true })
      .click();
    await expect(
      page.getByRole("heading", { name: `Personal ${kind}.`, exact: true }),
    ).toBeVisible();
    await expect(page.getByLabel("Rating in stars")).toHaveValue("3.5");
    await page.reload();
    await expect(page.getByLabel("Rating in stars")).toHaveValue("3.5");
    const details = await (
      await page.request.get(`/api/v1/library/${kind}/${item.id}`)
    ).json();
    expect(details.original_name).not.toBe(`Personal ${kind}`);
    await page.getByRole("link", { name: /Edit details/ }).click();
    await page.getByRole("button", { name: "Use original name" }).click();
    await page.getByRole("button", { name: "Clear rating" }).click();
    await page
      .getByRole("button", { name: "Save details", exact: true })
      .click();
    await expect(
      page.getByRole("heading", {
        name: `${details.original_name}.`,
        exact: true,
      }),
    ).toBeVisible();
    const session = await (await page.request.get("/api/session")).json();
    const bad = await page.request.post(`/api/v1/library/${kind}/${item.id}`, {
      headers: { Origin: baseURL, "X-CSRF-Token": session.csrf },
      data: { name: null, rating: 3.3 },
    });
    expect(bad.status()).toBe(400);
  }
});

test("release routes fit phone and tablet viewports in both orientations", async ({
  page,
  browserName,
}) => {
  test.setTimeout(120000);
  await login(page);
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  const paths = [
    "/dashboard",
    "/history",
    "/rankings/artists",
    "/rankings/albums",
    "/rankings/tracks",
    "/import",
    "/activities",
    "/settings/general",
    "/settings/appearance",
    "/settings/connections",
    "/settings/data",
    "/settings/api-keys",
  ];
  const archive = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  for (const kind of ["artists", "albums", "tracks"]) {
    const id = archive.rankings[kind][0].id;
    paths.push(`/music/${kind}/${id}`, `/music/${kind}/${id}/edit`);
  }
  for (const [width, height] of [
    [390, 844],
    [844, 390],
    [768, 1024],
    [1024, 768],
    [1440, 1000],
  ]) {
    await page.setViewportSize({ width, height });
    for (const path of paths) {
      await page.goto(path);
      await expect(page.locator("#main")).toBeVisible();
      if (path.startsWith("/music/")) {
        if (path.endsWith("/edit"))
          await expect(
            page.getByLabel(
              path.includes("/artists/") ? "Artist name" : "Title",
              { exact: true },
            ),
          ).toBeVisible();
        else await expect(page.locator(".music-heading h1")).toBeVisible();
      }
      const overflow = await page.evaluate(
        () => document.documentElement.scrollWidth > innerWidth + 1,
      );
      expect(overflow, `${path} at ${width}x${height}`).toBe(false);
      if (
        [
          "/dashboard",
          "/import",
          "/activities",
          "/settings/connections",
        ].includes(path)
      )
        await page.screenshot({
          path: `test-results/gui-${browserName}-${width}-${path.split("/").filter(Boolean).join("-")}.png`,
          fullPage: true,
          animations: "disabled",
        });
    }
  }
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.setViewportSize({ width: 390, height: 844 });
  await page
    .getByRole("button", { name: "Open navigation", exact: true })
    .click();
  await expect(
    page.getByRole("link", { name: "Overview", exact: true }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("button", { name: "Open navigation", exact: true }),
  ).toBeFocused();
  await page.evaluate(() => {
    localStorage.setItem("scrobbl:theme", JSON.stringify("dark"));
    localStorage.setItem("scrobbl:accent", JSON.stringify("#c5a3ee"));
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/dashboard");
  await expect(page.locator("#total-listens")).toBeVisible();
  await page.screenshot({
    path: `test-results/gui-${browserName}-dark-overview.png`,
    fullPage: true,
    animations: "disabled",
  });
  expect(errors).toEqual([]);
});

test("overview and sidebar share a single playback sample across navigation", async ({
  page,
}) => {
  await page.route("**/api/spotify/status", async (route) => {
    const response = await route.fetch();
    await route.fulfill({
      json: {
        ...(await response.json()),
        connected: true,
        now_playing_seconds: 86400,
      },
    });
  });
  let requests = 0;
  await page.route("**/api/spotify/now-playing", (route) => {
    requests++;
    return route.fulfill({
      json: {
        title: "Shared playback",
        artists: [{ id: "demo", name: "Shared artist" }],
        album: "Album",
        cover_url: null,
        spotify_url: "https://open.spotify.com/track/0000000000000000000001",
        duration_ms: 180000,
        progress_ms: 1000,
        observed_at_ms: 1,
        source_timestamp_ms: 1,
        requested_at_ms: 1,
        source_requested_at_ms: 1,
        response_received_at_ms: 1,
        served_at_ms: 1,
      },
    });
  });
  await login(page);
  await expect(page.locator(".sidebar-now")).toContainText("Shared playback");
  await expect(page.locator(".now-panel")).toContainText("Shared playback");
  expect(requests).toBe(1);
  await page.getByRole("link", { name: "Import history", exact: true }).click();
  await expect(page.locator(".sidebar-now")).toContainText("Shared playback");
  await page.getByRole("link", { name: "Overview", exact: true }).click();
  await expect(page.locator(".now-panel")).toContainText("Shared playback");
  expect(requests).toBe(1);
});

test("neutral tinted surfaces retain readable text for every palette and extreme custom accents", async ({
  page,
}) => {
  await login(page, "/settings/appearance");
  const ratios = await page.evaluate(() => {
    const root = document.documentElement;
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 1;
    const context = canvas.getContext("2d");
    const luminance = (color) => {
      context.clearRect(0, 0, 1, 1);
      context.fillStyle = color;
      context.fillRect(0, 0, 1, 1);
      const rgb = Array.from(context.getImageData(0, 0, 1, 1).data)
        .slice(0, 3)
        .map((v) => {
          const s = v / 255;
          return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
        });
      return rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
    };
    const contrast = (a, b) => {
      const x = luminance(a),
        y = luminance(b);
      return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
    };
    const original = root.style.getPropertyValue("--accent"),
      wasLight = root.classList.contains("theme-light");
    const result = [];
    for (const light of [false, true])
      for (const accent of [
        "#c9ee8b",
        "#91c9f7",
        "#c5a3ee",
        "#f7b68b",
        "#f099ad",
        "#64d9c3",
        "#000000",
        "#ffffff",
      ]) {
        root.classList.toggle("theme-light", light);
        root.style.setProperty("--accent", accent);
        // Resolve tokens through real computed styles; canvas converts oklab to sRGB.
        const panel = document.querySelector(".settings-form");
        const text = document.querySelector(".settings-form p");
        result.push({
          light,
          accent,
          ratio: contrast(
            getComputedStyle(text).color,
            getComputedStyle(panel).backgroundColor,
          ),
        });
      }
    root.classList.toggle("theme-light", wasLight);
    root.style.setProperty("--accent", original);
    return result;
  });
  for (const sample of ratios)
    expect(sample.ratio, JSON.stringify(sample)).toBeGreaterThanOrEqual(4.5);
});

test("ranking categories keep distinct identities during tab changes and back navigation", async ({
  page,
}) => {
  await login(page, "/rankings/artists?period=all");
  const archive = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  const expected = Object.fromEntries(
    ["artists", "albums", "tracks"].map((kind) => [
      kind,
      archive.rankings[kind][0].name,
    ]),
  );
  for (const kind of ["albums", "tracks", "artists", "tracks"]) {
    const response = page.waitForResponse((r) =>
      r.url().includes(`/api/v1/consultation/rankings/${kind}?`),
    );
    await page
      .getByRole("navigation", { name: "Ranking categories" })
      .getByRole("link", { name: new RegExp(`^${kind}`, "i") })
      .click();
    const data = await (await response).json();
    expect(data.items[0].name).toBe(expected[kind]);
    const rows = page.locator(
      "#main > .panel .rank-name a, #main > .rankings-panel .rank-name a",
    );
    await expect(rows.filter({ hasText: expected[kind] })).toBeVisible();
    for (const other of Object.keys(expected).filter((k) => k !== kind))
      await expect(rows.filter({ hasText: expected[other] })).toHaveCount(0);
    await expect(
      page
        .getByRole("navigation", { name: "Ranking categories" })
        .locator('[aria-current="page"]'),
    ).toHaveAttribute("href", new RegExp(`/rankings/${kind}`));
  }
  await page.goBack();
  await expect(
    page
      .locator("#main > .panel .rank-name a")
      .filter({ hasText: expected.artists }),
  ).toBeVisible();
});

test("connection cards show a verified check and a readable automatic wait", async ({
  page,
  browserName,
}) => {
  await login(page, "/settings/connections");
  const status = await (await page.request.get("/api/spotify/status")).json();
  await page.route("**/api/spotify/status", (route) =>
    route.fulfill({
      json: {
        ...status,
        configured: true,
        connected: true,
        connection: "verified",
        collection: "waiting",
        verified_at_ms: Date.now() - 60000,
        retry_at_ms: Date.now() + 35 * 60000,
        last_error: "Spotify request limit reached. Retrying in 2100 seconds.",
      },
    }),
  );
  await page.reload();
  await expect(page.locator("#connection-badge")).toContainText("Connected");
  await expect(page.locator("#connection-badge")).toHaveClass(
    /connection-verified/,
  );
  const summary = page.locator(".spotify-card > summary");
  await summary.click();
  await expect(
    page.getByRole("status").filter({ hasText: "Spotify requests are paused" }),
  ).toContainText("resume automatically");
  await expect(page.locator(".spotify-card > summary")).not.toContainText(
    "2100 seconds",
  );
  await page.locator(".guide > summary").first().click();
  await page.screenshot({
    path: `test-results/gui-feedback-connections-${browserName}.png`,
    fullPage: true,
    animations: "disabled",
  });
});

test("name groups can be separated and rejoined from the edit page", async ({
  page,
}) => {
  await login(page, "/rankings/artists?period=all");
  const archive = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  const item = archive.rankings.artists[0];
  await page.goto(`/music/artists/${item.id}/edit`);
  await expect(
    page.getByRole("heading", { name: "Grouped by name" }),
  ).toBeVisible();
  await page.locator(".group-members input[type=checkbox]").first().check();
  await page.getByRole("button", { name: "Separate selected entries" }).click();
  await expect(page).not.toHaveURL(new RegExp(item.id));
  await expect(
    page.getByRole("button", { name: "Rejoin selected by name" }),
  ).toBeVisible();
  await page.locator(".group-members input[type=checkbox]").first().check();
  await page.getByRole("button", { name: "Rejoin selected by name" }).click();
  await expect(page).toHaveURL(new RegExp(item.id));
});

test("yearly activity labels and full details remain readable without clipped tooltips", async ({
  page,
  browserName,
}) => {
  await page.emulateMedia({ colorScheme: "dark" });
  const values = [4738, 4581, 9469, 11761, 19815, 16559, 34462, 26764];
  await page.route("**/api/v1/consultation/activity?*", (route) =>
    route.fulfill({
      json: {
        grain: "year",
        mixed_week_starts: false,
        buckets: values.map((listens, index) => ({
          start: `${2019 + index}-01-01`,
          through: `${2019 + index}-12-31`,
          listens,
          actual_ms: listens * 180000,
          estimated_ms: 0,
          unknown_durations: 0,
          partial: index === 0 || index === 7,
        })),
      },
    }),
  );
  await login(page, "/dashboard?period=all&grain=year");
  await expect(page.locator(".chart-segment")).toHaveCount(8);
  await page.locator(".chart-segment").nth(6).hover();
  await expect(page.locator(".chart-readout")).toContainText("34,462");
  await expect(page.locator(".chart-readout")).toContainText(
    "2025-01-01 – 2025-12-31",
  );
  const inside = await page.locator(".chart-readout").evaluate((node) => {
    const box = node.getBoundingClientRect(),
      panel = node.closest("article").getBoundingClientRect();
    return (
      box.top >= panel.top &&
      box.bottom <= panel.bottom &&
      box.right <= panel.right
    );
  });
  expect(inside).toBe(true);
  await expect(
    page.locator(".chart-segment").nth(6).locator("small").first(),
  ).toHaveText("2025");
  await page.screenshot({
    path: `test-results/gui-activity-${browserName}.png`,
    fullPage: true,
    animations: "disabled",
  });
});
