const { test: base, expect } = require("@playwright/test");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const { mkdtemp, rm } = require("node:fs/promises");
const { tmpdir } = require("node:os");
const { join, resolve } = require("node:path");
const net = require("node:net");

const password = "ask context browser password 7!";

const readyStatus = {
  enabled: true,
  runtime_available: true,
  model_ready: true,
  model_loading: false,
};

function firstPlayQuery({ kind, id, name }) {
  return {
    metric: "first_play",
    entity: { kind, id, name },
    period: "all_time",
    year: null,
    month: null,
    weekday: null,
    hour: null,
    source: null,
    group_by: null,
    sort_by: "metric",
    order: "desc",
    limit: null,
  };
}

function interpretationChips({ kind, name }) {
  return [
    { key: "metric", label: "Metric", value: "First play" },
    {
      key: "entity",
      label: kind === "artist" ? "Artist" : "Album",
      value: name,
    },
    { key: "period", label: "Period", value: "All time" },
  ];
}

function firstPlayResult({ kind, id, name, trackId }) {
  const playedAt = Date.parse("2024-01-05T20:00:00Z");
  return {
    metric: "first_play",
    scalar: playedAt,
    time: null,
    rows: [],
    first: {
      played_at_ms: playedAt,
      title: "Opening track",
      artist: name,
      album: null,
      track_id: trackId,
    },
    last: null,
    streak: null,
    milestones: [],
    total: 1,
    links: [
      {
        label: kind === "artist" ? "View artist" : "Open album",
        href: `/music/${kind === "artist" ? "artists" : "albums"}/${id}`,
      },
      {
        label: "Played on 2024-01-05",
        href: "/history?from=2024-01-05&to=2024-01-05",
      },
    ],
  };
}

function okResponse({ kind, id, name, trackId }) {
  return {
    status: "ok",
    question: "",
    query: firstPlayQuery({ kind, id, name }),
    chips: interpretationChips({ kind, name }),
    result: firstPlayResult({ kind, id, name, trackId }),
    message: null,
    candidates: [],
    runtime_available: true,
  };
}

const test = base.extend({
  archive: async ({ browser }, use) => {
    const directory = await mkdtemp(join(tmpdir(), "scrobbl-ask-context-"));
    const socket = net.createServer();
    socket.listen(0, "127.0.0.1");
    await once(socket, "listening");
    const port = socket.address().port;
    await new Promise((resolveClose) => socket.close(resolveClose));
    const child = spawn(
      resolve(
        process.env.CARGO_TARGET_DIR || "target",
        "debug",
        "scrobbl-rs" + (process.platform === "win32" ? ".exe" : ""),
      ),
      [],
      {
        env: {
          ...process.env,
          SCROBBL_PORT: String(port),
          SCROBBL_BIND: "127.0.0.1",
          SCROBBL_DATA_DIR: directory,
          SCROBBL_EXTERNAL_ORIGIN: `http://127.0.0.1:${port}`,
          SPOTIFY_CLIENT_ID: "",
          SPOTIFY_CLIENT_SECRET: "",
        },
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
    let logs = "";
    child.stdout.on("data", (data) => (logs += data));
    child.stderr.on("data", (data) => (logs += data));
    const context = await browser.newContext({
      baseURL: `http://127.0.0.1:${port}`,
    });
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    try {
      await expect
        .poll(async () => {
          if (child.exitCode !== null) throw new Error(logs);
          try {
            return (await context.request.get("/healthz")).status();
          } catch {
            return 0;
          }
        })
        .toBe(200);
      await page.goto("/");
      await page.getByLabel("Username", { exact: true }).fill("owner");
      await page.getByLabel("Password", { exact: true }).fill(password);
      await page.getByLabel("Confirm password").fill(password);
      await page.getByRole("button", { name: "Create your archive" }).click();
      await expect(page.locator("#total-listens")).toHaveText("0");

      const rows = [
        {
          ts: "2024-01-05T20:00:00Z",
          ms_played: 180000,
          master_metadata_track_name: "Opening track",
          master_metadata_album_artist_name: "Context artist",
          master_metadata_album_album_name: "Context album",
          spotify_track_uri: "spotify:track:0000000000000000000001",
        },
        {
          ts: "2024-02-10T21:00:00Z",
          ms_played: 200000,
          master_metadata_track_name: "Second track",
          master_metadata_album_artist_name: "Context artist",
          master_metadata_album_album_name: "Context album",
          spotify_track_uri: "spotify:track:0000000000000000000002",
        },
        {
          ts: "2024-03-15T22:00:00Z",
          ms_played: 210000,
          master_metadata_track_name: "Other track",
          master_metadata_album_artist_name: "Other artist",
          master_metadata_album_album_name: "Other album",
          spotify_track_uri: "spotify:track:0000000000000000000003",
        },
      ];
      const session = await (await page.request.get("/api/session")).json();
      const upload = await page.request.post("/api/import/spotify", {
        headers: {
          "x-csrf-token": session.csrf,
          origin: new URL(page.url()).origin,
        },
        multipart: {
          files: {
            name: "ask-context.json",
            mimeType: "application/json",
            buffer: Buffer.from(JSON.stringify(rows)),
          },
        },
      });
      expect(upload.ok(), await upload.text()).toBeTruthy();
      const { job_id } = await upload.json();
      await expect
        .poll(
          async () =>
            (
              await (
                await page.request.get(`/api/v1/activities/${job_id}`)
              ).json()
            ).activity.state,
        )
        .toBe("completed");

      const dashboard = await (
        await page.request.get("/api/dashboard?period=all")
      ).json();
      const rankings = dashboard.rankings;
      const artist = rankings.artists.find(
        (item) => item.name === "Context artist",
      );
      const album = rankings.albums.find(
        (item) => item.name === "Context album",
      );
      const track = rankings.tracks.find(
        (item) => item.name === "Opening track",
      );
      expect(artist).toBeTruthy();
      expect(album).toBeTruthy();
      expect(track).toBeTruthy();

      await use({ page, context, artist, album, track });
      expect(errors).toEqual([]);
    } finally {
      await context.close().catch(() => {});
      if (child.exitCode === null) {
        const exited = once(child, "exit");
        child.kill("SIGTERM");
        await exited;
      }
      await rm(directory, { recursive: true, force: true });
    }
  },
});

test("the contextual Ask panel appears on artist and album pages, never on tracks", async ({
  archive: { page, artist, album, track },
}) => {
  await page.goto(`/music/artists/${artist.id}?period=all`);
  await expect(page.locator(".music-heading h1")).toBeVisible();
  const panel = page.locator(".ask-context");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText("Ask about this artist");
  await expect(panel).toContainText(artist.name);

  await panel.getByRole("button", { name: /Ask about this artist/ }).click();
  await expect(panel.getByText("AI is off")).toBeVisible();
  await expect(
    panel.getByRole("link", { name: "Open AI settings" }),
  ).toHaveAttribute("href", "/settings/ai");

  await page.goto(`/music/albums/${album.id}?period=all`);
  await expect(page.locator(".music-heading h1")).toBeVisible();
  await expect(page.locator(".ask-context")).toContainText(
    "Ask about this album",
  );

  await page.goto(`/music/tracks/${track.id}?period=all`);
  await expect(page.locator(".music-heading h1")).toBeVisible();
  await expect(page.locator(".ask-context")).toHaveCount(0);
});

test("asking without repeating the name sends the page context and shows the entity chip and result links", async ({
  archive: { page, artist, album, track },
}) => {
  await page.route("**/api/v1/ask/status", (route) =>
    route.fulfill({ json: readyStatus }),
  );
  const requests = [];
  await page.route("**/api/v1/ask", async (route) => {
    if (route.request().method() !== "POST") return route.continue();
    const body = route.request().postDataJSON();
    requests.push(body);
    const context = body.context;
    await route.fulfill({
      json: okResponse({
        kind: context.kind,
        id: context.id,
        name: context.kind === "artist" ? artist.name : album.name,
        trackId: track.id,
      }),
    });
  });

  await page.goto(`/music/artists/${artist.id}?period=all`);
  await expect(page.locator(".music-heading h1")).toBeVisible();
  let panel = page.locator(".ask-context");
  await panel.getByRole("button", { name: /Ask about this artist/ }).click();
  await panel
    .getByLabel("Your question", { exact: true })
    .fill("When did I first listen?");
  await panel.getByRole("button", { name: "Ask", exact: true }).click();

  await expect(panel.locator(".ask-result")).toBeVisible();
  expect(requests).toHaveLength(1);
  expect(requests[0]).toEqual({
    question: "When did I first listen?",
    context: { kind: "artist", id: artist.id },
  });
  await expect(
    panel.locator(".ask-chip").filter({ hasText: artist.name }),
  ).toBeVisible();
  await expect(panel).toContainText(`Your first listen to ${artist.name}`);
  await expect(
    panel.getByRole("link", { name: "View artist" }),
  ).toHaveAttribute("href", new RegExp(`^/music/artists/${artist.id}`));
  await expect(
    panel.getByRole("link", { name: "Played on 2024-01-05" }),
  ).toBeVisible();

  await page.goto(`/music/albums/${album.id}?period=all`);
  await expect(page.locator(".music-heading h1")).toBeVisible();
  panel = page.locator(".ask-context");
  await panel.getByRole("button", { name: /Ask about this album/ }).click();
  await panel
    .getByLabel("Your question", { exact: true })
    .fill("When did I first listen?");
  await panel.getByRole("button", { name: "Ask", exact: true }).click();

  await expect(panel.locator(".ask-result")).toBeVisible();
  expect(requests).toHaveLength(2);
  expect(requests[1].context).toEqual({ kind: "album", id: album.id });
  await expect(
    panel.locator(".ask-chip").filter({ hasText: album.name }),
  ).toBeVisible();
  await expect(panel.getByRole("link", { name: "Open album" })).toHaveAttribute(
    "href",
    new RegExp(`^/music/albums/${album.id}`),
  );
});

test("an ambiguous entity name still renders candidates and choosing one re-runs the query", async ({
  archive: { page, album, track },
}) => {
  await page.route("**/api/v1/ask/status", (route) =>
    route.fulfill({ json: readyStatus }),
  );
  const candidate = {
    id: album.id,
    name: album.name,
    subtitle: "Context artist",
    kind: "album",
  };
  await page.route("**/api/v1/ask", (route) =>
    route.fulfill({
      json: {
        status: "ambiguous",
        question: "",
        query: firstPlayQuery({
          kind: "album",
          id: null,
          name: album.name,
        }),
        chips: interpretationChips({ kind: "album", name: album.name }),
        result: null,
        message: "I found more than one match. Choose the one you meant.",
        candidates: [
          candidate,
          {
            id: "other-context-album",
            name: album.name,
            subtitle: "Other artist",
            kind: "album",
          },
        ],
        runtime_available: true,
      },
    }),
  );
  let chosen;
  await page.route("**/api/v1/ask/query", async (route) => {
    chosen = route.request().postDataJSON();
    await route.fulfill({
      json: okResponse({
        kind: "album",
        id: candidate.id,
        name: candidate.name,
        trackId: track.id,
      }),
    });
  });

  await page.goto(`/music/albums/${album.id}?period=all`);
  await expect(page.locator(".music-heading h1")).toBeVisible();
  const panel = page.locator(".ask-context");
  await panel.getByRole("button", { name: /Ask about this album/ }).click();
  await panel
    .getByLabel("Your question", { exact: true })
    .fill("When did I first listen?");
  await panel.getByRole("button", { name: "Ask", exact: true }).click();

  await expect(
    panel.getByText("More than one match", { exact: true }),
  ).toBeVisible();
  const candidates = panel.locator(".ask-candidate");
  await expect(candidates).toHaveCount(2);
  await candidates.first().click();

  await expect(panel.locator(".ask-result")).toBeVisible();
  expect(chosen.query.entity).toEqual({
    kind: "album",
    name: album.name,
    id: album.id,
  });
});

test("the contextual panel stays usable at 390px without horizontal overflow", async ({
  archive: { page, album, track },
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.route("**/api/v1/ask/status", (route) =>
    route.fulfill({ json: readyStatus }),
  );
  await page.route("**/api/v1/ask", (route) =>
    route.fulfill({
      json: okResponse({
        kind: "album",
        id: album.id,
        name: album.name,
        trackId: track.id,
      }),
    }),
  );

  await page.goto(`/music/albums/${album.id}?period=all`);
  await expect(page.locator(".music-heading h1")).toBeVisible();
  const panel = page.locator(".ask-context");
  const toggle = panel.getByRole("button", { name: /Ask about this album/ });
  await expect(toggle).toBeVisible();
  const toggleBox = await toggle.boundingBox();
  expect(toggleBox.height).toBeGreaterThanOrEqual(44);

  await toggle.click();
  await panel
    .getByLabel("Your question", { exact: true })
    .fill("When did I first listen?");
  const ask = panel.getByRole("button", { name: "Ask", exact: true });
  const askBox = await ask.boundingBox();
  expect(askBox.height).toBeGreaterThanOrEqual(44);
  await ask.click();

  await expect(panel.locator(".ask-result")).toBeVisible();
  await expect(panel.getByRole("link", { name: "Open album" })).toBeVisible();

  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth > innerWidth + 1,
  );
  expect(overflow).toBe(false);
  const panelBox = await panel.boundingBox();
  expect(panelBox.x).toBeGreaterThanOrEqual(0);
  expect(panelBox.x + panelBox.width).toBeLessThanOrEqual(390.5);
});

test("the global Ask page and AI Settings stay usable at 390px without horizontal overflow", async ({
  archive: { page, album, track },
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.route("**/api/v1/ask/status", (route) =>
    route.fulfill({ json: readyStatus }),
  );
  await page.route("**/api/v1/ask", (route) =>
    route.fulfill({
      json: okResponse({
        kind: "album",
        id: album.id,
        name: album.name,
        trackId: track.id,
      }),
    }),
  );

  // 1. Global /ask page
  await page.goto("/ask");
  await expect(
    page.getByRole("heading", { name: /Ask/ }).first(),
  ).toBeVisible();
  const input = page.getByLabel("Your question", { exact: true });
  await expect(input).toBeVisible();
  await input.fill("How many plays?");
  const submit = page.getByRole("button", { name: "Ask", exact: true });
  const submitBox = await submit.boundingBox();
  expect(submitBox.height).toBeGreaterThanOrEqual(44);
  await submit.click();

  await expect(page.locator(".ask-result")).toBeVisible();
  const chip = page.locator(".ask-chip").first();
  await expect(chip).toBeVisible();
  const chipBox = await chip.boundingBox();
  expect(chipBox.height).toBeGreaterThanOrEqual(44);

  let overflow = await page.evaluate(
    () => document.documentElement.scrollWidth > innerWidth + 1,
  );
  expect(overflow).toBe(false);

  // 2. Settings AI page
  await page.goto("/settings/ai");
  await expect(page.locator(".ai-settings")).toBeVisible();
  const toggle = page.locator(".ai-toggle").first();
  await expect(toggle).toBeVisible();
  const toggleBox = await toggle.boundingBox();
  expect(toggleBox.height).toBeGreaterThanOrEqual(44);

  overflow = await page.evaluate(
    () => document.documentElement.scrollWidth > innerWidth + 1,
  );
  expect(overflow).toBe(false);
});

