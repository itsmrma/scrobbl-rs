const { test: base, expect } = require("@playwright/test");
const { chooseOption } = require("./controls.cjs");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const http = require("node:http");
const { mkdtemp, rm } = require("node:fs/promises");
const { tmpdir } = require("node:os");
const { join, resolve } = require("node:path");
const net = require("node:net");

const password = "ask free browser password 7!";

const test = base.extend({
  harness: async ({ browser }, use) => {
    const directory = await mkdtemp(join(tmpdir(), "scrobbl-ask-free-"));

    // Start an in-process mock OpenAI-compatible server on loopback.
    const seen = [];
    let nextReply = "";
    const mockServer = http.createServer((req, res) => {
      let raw = "";
      req.on("data", (chunk) => (raw += chunk));
      req.on("end", () => {
        seen.push(JSON.parse(raw));
        res.writeHead(200, { "content-type": "application/json" });
        res.end(
          JSON.stringify({
            choices: [{ message: { role: "assistant", content: nextReply } }],
          }),
        );
      });
    });
    mockServer.listen(0, "127.0.0.1");
    await once(mockServer, "listening");
    const mockPort = mockServer.address().port;

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
        .poll(
          async () => {
            if (child.exitCode !== null) throw new Error(logs);
            try {
              return (await context.request.get("/healthz")).status();
            } catch {
              return 0;
            }
          },
          { timeout: 20000 },
        )
        .toBe(200);

      await page.goto("/");
      await page.getByLabel("Username", { exact: true }).fill("owner");
      await page.getByLabel("Password", { exact: true }).fill(password);
      await page.getByLabel("Confirm password").fill(password);
      await page.getByRole("button", { name: "Create your archive" }).click();
      await expect(page.locator("#total-listens")).toHaveText("0");

      const rows = [
        {
          ts: "2024-01-10T10:00:00Z",
          ms_played: 240000,
          master_metadata_track_name: "Jóga",
          master_metadata_album_artist_name: "SecretArchiveArtist",
          master_metadata_album_album_name: "Homogenic",
          spotify_track_uri: "spotify:track:0000000000000000000001",
        },
        {
          ts: "2024-01-11T10:00:00Z",
          ms_played: 240000,
          master_metadata_track_name: "Jóga",
          master_metadata_album_artist_name: "SecretArchiveArtist",
          master_metadata_album_album_name: "Homogenic",
          spotify_track_uri: "spotify:track:0000000000000000000001",
        },
        {
          ts: "2024-02-12T10:00:00Z",
          ms_played: 200000,
          master_metadata_track_name: "Creep",
          master_metadata_album_artist_name: "Radiohead",
          master_metadata_album_album_name: "Pablo Honey",
          spotify_track_uri: "spotify:track:0000000000000000000002",
        },
        {
          ts: "2024-02-13T11:30:00Z",
          ms_played: 150000,
          master_metadata_track_name:
            '<img src=x onerror="window.__askXss=1">\'; DROP TABLE listens;--',
          master_metadata_album_artist_name: "LiteralArtist",
          master_metadata_album_album_name: "LiteralAlbum",
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
            name: "ask-free.json",
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

      await use({
        page,
        mockUrl: `http://127.0.0.1:${mockPort}/v1`,
        seen,
        setReply: (value) => {
          nextReply = JSON.stringify(value);
        },
      });
      expect(errors).toEqual([]);
    } finally {
      await context.close();
      child.kill();
      mockServer.close();
      await rm(directory, { recursive: true, force: true }).catch(() => {});
    }
  },
});

test("a free question with remote AI shows the computed sentence, a table and the SQL", async ({
  harness: { page, mockUrl, seen, setReply },
}) => {
  await page.goto("/settings/ai");
  await page.getByLabel("Enable AI").check();
  await chooseOption(page.getByLabel("AI engine"), "remote");
  await expect(page.getByLabel("Answer mode")).toHaveAttribute(
    "data-value",
    "sql",
  );
  await page.getByLabel("Server address").fill(mockUrl);
  await page.getByLabel("Model name").fill("test-sql-model");
  await page.getByRole("button", { name: "Save AI settings" }).click();
  await expect(page.getByText("AI settings saved.")).toBeVisible();

  const validSql =
    "SELECT t.title AS track, COUNT(*) AS plays FROM listens l JOIN tracks t ON t.id = l.track_id WHERE l.counted = 1 GROUP BY t.id ORDER BY plays DESC, t.title ASC";
  setReply({
    sql: validSql,
    title: "Top tracks",
    computed: "Counted plays per track across all time.",
    answer_kind: "table",
    columns: [
      { name: "track", format: "track" },
      { name: "plays", format: "count" },
    ],
  });

  await page.goto("/ask");
  await page
    .getByLabel("Your question", { exact: true })
    .fill("What are my top tracks?");
  await page.getByRole("button", { name: "Ask", exact: true }).click();

  const card = page.locator(".ask-result");
  await expect(card).toBeVisible();
  await expect(card).toContainText("Top tracks");
  await expect(card).toContainText(
    "Computed: Counted plays per track across all time.",
  );
  await expect(card).toContainText("Projection as of ");

  const table = card.locator(".ask-table");
  await expect(table.locator("thead th")).toHaveText(["track", "plays"]);
  await expect(table.locator("tbody tr")).toHaveCount(3);
  await expect(table.locator("tbody tr").nth(0).locator("td")).toHaveText([
    "Jóga",
    "2",
  ]);
  await expect(table.locator("tbody tr").nth(1).locator("td")).toHaveText([
    '<img src=x onerror="window.__askXss=1">\'; DROP TABLE listens;--',
    "1",
  ]);
  await expect(table.locator("tbody tr").nth(2).locator("td")).toHaveText([
    "Creep",
    "1",
  ]);

  // SQL disclosure starts collapsed and reveals the editable SQL when opened.
  await expect(card.locator(".ask-sql-code")).toBeHidden();
  await card.getByText("Show SQL").click();
  await expect(card.locator(".ask-sql-code")).toBeVisible();
  await expect(card.locator(".ask-sql-code")).toHaveValue(validSql);

  // Confirm no archive values were sent to the mock server.
  expect(seen.length).toBe(1);
  const sentText = JSON.stringify(seen[0]);
  expect(sentText).not.toContain("SecretArchiveArtist");
  expect(sentText).not.toContain("Jóga");
  expect(sentText).not.toContain("Creep");

  // Unsafe SQL returned by the model is rejected with a rule code and no data.
  setReply({
    sql: "SELECT value FROM settings",
    title: "Unsafe",
    computed: "Attempted to read settings.",
    answer_kind: "table",
    columns: [],
  });
  await page
    .getByLabel("Your question", { exact: true })
    .fill("Show me settings");
  await page.getByRole("button", { name: "Ask", exact: true }).click();

  await expect(page.getByText("I couldn't answer that")).toBeVisible();
  await expect(page.locator(".notice")).toContainText("unauthorized_table");
  await expect(page.locator(".ask-table")).toHaveCount(0);

  // Storage settings reports Ask projection disk size while AI is enabled, and 0 KB after disabling AI.
  await page.goto("/settings/application");
  const projRow = page.locator("dl.connection-facts div", {
    hasText: "Ask projection on disk",
  });
  await expect(projRow).toBeVisible();
  await expect(projRow).not.toContainText("0 KB");

  await page.goto("/settings/ai");
  await page.getByLabel("Enable AI").uncheck();
  await page.getByRole("button", { name: "Save AI settings" }).click();
  await expect(page.getByText("AI settings saved.")).toBeVisible();

  await page.goto("/settings/application");
  await expect(
    page.locator("dl.connection-facts div", {
      hasText: "Ask projection on disk",
    }),
  ).toContainText("0 KB");
});

test("free results render value, table, series, ranking, entity/history links, pagination and editable SQL across themes and mobile", async ({
  harness: { page, mockUrl, setReply },
}) => {
  await page.goto("/settings/ai");
  await page.getByLabel("Enable AI").check();
  await chooseOption(page.getByLabel("AI engine"), "remote");
  await page.getByLabel("Server address").fill(mockUrl);
  await page.getByLabel("Model name").fill("test-sql-model");
  await page.getByRole("button", { name: "Save AI settings" }).click();
  await expect(page.getByText("AI settings saved.")).toBeVisible();

  await page.goto("/ask");

  // 1. Value answer with exact history filter and projection as-of time.
  setReply({
    sql: "SELECT SUM(listened_ms) AS total_ms FROM listens WHERE counted = 1",
    title: "Total listening time",
    computed: "Known listening time for all counted plays.",
    answer_kind: "value",
    columns: [{ name: "total_ms", format: "duration_ms" }],
  });
  await page
    .getByLabel("Your question", { exact: true })
    .fill("How long did I listen?");
  await page.getByRole("button", { name: "Ask", exact: true }).click();
  const card = page.locator(".ask-result");
  await expect(card.locator(".ask-result-value")).toHaveText("13m");
  await expect(card).toContainText("Projection as of");
  await expect(
    card.getByRole("link", { name: "See these listens" }),
  ).toHaveAttribute("href", "/history?period=all&status=counted");

  // 2. Series answer (bar chart) with unmappable filter (local_hour) omits history link.
  setReply({
    sql: "SELECT local_date AS day, COUNT(*) AS plays FROM listens WHERE counted = 1 AND local_hour >= 0 GROUP BY local_date ORDER BY local_date ASC",
    title: "Plays by day",
    computed: "Counted plays by day for hours >= 0.",
    answer_kind: "series",
    columns: [
      { name: "day", format: "date" },
      { name: "plays", format: "count" },
    ],
  });
  await page.getByLabel("Your question", { exact: true }).fill("Plays by day");
  await page.getByRole("button", { name: "Ask", exact: true }).click();
  await expect(
    card.locator(".ask-chart .chart-bars .chart-segment"),
  ).toHaveCount(4);
  await expect(card.locator(".ask-chart")).toContainText("10 Jan 2024");
  await expect(
    card.getByRole("link", { name: "See these listens" }),
  ).toHaveCount(0);

  // 3. Ranking answer with entity links and literal HTML+SQL track title (no XSS).
  setReply({
    sql: "SELECT t.title AS track, COUNT(*) AS plays FROM listens l JOIN tracks t ON t.id = l.track_id WHERE l.counted = 1 GROUP BY t.id ORDER BY plays DESC, t.title ASC",
    title: "Top tracks",
    computed: "Top tracks by counted plays.",
    answer_kind: "ranking",
    columns: [
      { name: "track", format: "track" },
      { name: "plays", format: "count" },
    ],
  });
  await page.getByLabel("Your question", { exact: true }).fill("Top tracks");
  await page.getByRole("button", { name: "Ask", exact: true }).click();
  const segments = card.locator(".ask-chart .chart-segment.ask-chart-link");
  await expect(segments).toHaveCount(3);
  await expect(segments.nth(1)).toContainText(
    '<img src=x onerror="window.__askXss=1">\'; DROP TABLE listens;--',
  );
  expect(await page.evaluate(() => window.__askXss)).toBeUndefined();

  // Clicking the entity link opens the right track music page.
  await segments.nth(0).click();
  await expect(page).toHaveURL(/\/music\/tracks\//);
  await expect(page.getByRole("heading", { name: "Jóga." })).toBeVisible();
  await page.goBack();
  await expect(card).toBeVisible();

  // 4. Table answer with entity links, datetime, duration_ms, percent, then editable SQL -> pagination + truncation + mobile/themes + rejection.
  setReply({
    sql: "SELECT ar.name AS artist, al.name AS album, t.title AS track, l.played_at_ms AS when_ms, l.listened_ms AS dur_ms, ROUND(100.0 * l.counted, 1) AS pct FROM listens l JOIN tracks t ON t.id = l.track_id LEFT JOIN albums al ON al.id = t.album_id JOIN track_artists ta ON ta.track_id = t.id JOIN artists ar ON ar.id = ta.artist_id WHERE l.counted = 1 ORDER BY l.played_at_ms ASC",
    title: "All counted listens",
    computed:
      "Counted listens with artist, album, track, timestamp and duration.",
    answer_kind: "table",
    columns: [
      { name: "artist", format: "artist" },
      { name: "album", format: "album" },
      { name: "track", format: "track" },
      { name: "when_ms", format: "datetime" },
      { name: "dur_ms", format: "duration_ms" },
      { name: "pct", format: "percent" },
    ],
  });
  await page
    .getByLabel("Your question", { exact: true })
    .fill("List all listens");
  await page.getByRole("button", { name: "Ask", exact: true }).click();
  const table = card.locator(".ask-table");
  await expect(table.locator("tbody tr")).toHaveCount(4);
  await expect(table.locator("tbody tr").nth(0).locator("td a")).toHaveCount(3);
  await expect(
    table.locator("tbody tr").nth(0).locator("td").nth(4),
  ).toHaveText("4m");
  await expect(
    table.locator("tbody tr").nth(0).locator("td").nth(5),
  ).toHaveText("100%");

  await expect(card.locator(".ask-sql-code")).toBeHidden();
  await card.getByText("Show SQL").click();
  const editor = card.locator(".ask-sql-code");
  await expect(editor).toBeVisible();

  await editor.fill(
    "WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < 1050) SELECT n AS idx, ROUND(n * 0.1, 1) AS pct FROM seq",
  );
  await card.getByRole("button", { name: "Run" }).click();
  await expect(card).toContainText("result truncated");
  await expect(card.locator(".ask-table tbody tr")).toHaveCount(100);
  await expect(card.locator(".pagination")).toContainText("Page 1 of 10");
  await card.getByRole("button", { name: "Next" }).click();
  await expect(card.locator(".pagination")).toContainText("Page 2 of 10");

  // Mobile width (<= 600px) and both themes (dark & light) have no horizontal overflow.
  await page.setViewportSize({ width: 375, height: 812 });
  for (const theme of ["dark", "light"]) {
    await page.evaluate((nextTheme) => {
      document.documentElement.classList.toggle(
        "theme-light",
        nextTheme === "light",
      );
    }, theme);
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth > window.innerWidth,
    );
    expect(overflow, `horizontal overflow in ${theme} theme at 375px`).toBe(
      false,
    );
  }
  await page.setViewportSize({ width: 1280, height: 800 });

  // Forbidden edit in SQL editor is rejected with its sandbox rule.
  await card.locator(".ask-sql-code").fill("SELECT value FROM settings");
  await card.getByRole("button", { name: "Run" }).click();
  await expect(page.getByText("I couldn't answer that")).toBeVisible();
  await expect(page.locator(".notice")).toContainText("unauthorized_table");
});
