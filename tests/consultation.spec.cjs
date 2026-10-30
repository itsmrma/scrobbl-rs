const { chooseOption, settleDisclosure } = require("./controls.cjs");
const { test: base, expect } = require("@playwright/test");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const { mkdtemp, rm } = require("node:fs/promises");
const { tmpdir } = require("node:os");
const { join, resolve } = require("node:path");
const net = require("node:net");

const test = base.extend({
  archive: async ({ browser }, use) => {
    const directory = await mkdtemp(join(tmpdir(), "scrobbl-consultation-"));
    const socket = net.createServer();
    socket.listen(0, "127.0.0.1");
    await once(socket, "listening");
    const port = socket.address().port;
    await new Promise((resolve) => socket.close(resolve));
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
    page.on("pageerror", (e) => errors.push(e.message));
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
      await page
        .getByLabel("Password", { exact: true })
        .fill("consultation browser password 7!");
      await page
        .getByLabel("Confirm password")
        .fill("consultation browser password 7!");
      await page.getByRole("button", { name: "Create your archive" }).click();
      await expect(page.locator("#total-listens")).toHaveText("0");
      const rows = [];
      for (let i = 0; i < 115; i++) {
        for (let n = 0; n < (i < 9 ? 3 : i < 11 ? 2 : 1); n++)
          rows.push({
            ts: new Date(Date.UTC(2025, 5, 1, 12, i * 5 + n)).toISOString(),
            ms_played: 60000 + i,
            master_metadata_track_name: `Consultation track ${String(i).padStart(3, "0")}`,
            master_metadata_album_artist_name: "Consultation artist",
            master_metadata_album_album_name: `Consultation album ${String(i).padStart(3, "0")}`,
            spotify_track_uri: `spotify:track:${String(i).padStart(22, "0")}`,
          });
      }
      rows.push({
        ts: "2025-06-02T12:00:00Z",
        ms_played: 29000,
        master_metadata_track_name: "Below threshold only",
        master_metadata_album_artist_name: "Consultation artist",
        master_metadata_album_album_name: "Hidden album",
        spotify_track_uri: "spotify:track:9999999999999999999999",
      });
      const session = await (await page.request.get("/api/session")).json();
      const upload = await page.request.post("/api/import/spotify", {
        headers: {
          "x-csrf-token": session.csrf,
          origin: new URL(page.url()).origin,
        },
        multipart: {
          files: {
            name: "consultation.json",
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
      await page.goto("/dashboard");
      await expect(page.locator("#total-listens")).toHaveText("135");
      await use({ page, context, port });
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

test.describe("consultation", () => {
  test("consultation navigation, tied pages, persistent views, chart drilldown and accessible time help", async ({
    archive: { page },
  }) => {
    const help = page.getByRole("button", {
      name: "How we calculate listening time",
      exact: true,
    });
    await help.hover();
    await expect(
      page.getByRole("region", { name: "How we calculate listening time" }),
    ).toContainText("four-minute track");
    await page
      .getByRole("heading", { name: "Your listening, at a glance." })
      .hover();
    await expect(help).toHaveAttribute("aria-expanded", "false");
    await help.focus();
    await expect(help).toHaveAttribute("aria-expanded", "true");
    await help.press("Escape");
    await expect(help).toHaveAttribute("aria-expanded", "false");
    await help.click();
    await expect(help).toHaveAttribute("aria-expanded", "true");
    await help.click();
    await expect(help).toHaveAttribute("aria-expanded", "false");
    await page
      .getByRole("link", { name: "View all tracks", exact: true })
      .click();
    await expect(
      page.getByRole("heading", { name: "Rankings." }),
    ).toBeVisible();
    const ranking = page.locator("article").filter({
      has: page.getByRole("heading", { name: "Your tracks", exact: true }),
    });
    await chooseOption(ranking.getByLabel("Results per page"), "10");
    await expect(ranking.locator(".ranking-row")).toHaveCount(10);
    await expect(ranking.locator(".rank-number").last()).toHaveText("10");
    await ranking.getByRole("button", { name: "Next", exact: true }).click();
    await expect(ranking.locator(".rank-number").first()).toHaveText("10");
    const firstName = await ranking
      .locator(".rank-name a:visible")
      .first()
      .textContent();
    const returnUrl = page.url();
    await ranking.locator(".rank-name a:visible").first().click();
    await expect(
      page.getByRole("heading", { name: firstName + ".", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Listening history", exact: true }),
    ).toBeVisible();
    await expect(page.locator(".chart-segment:visible")).toHaveCount(1);
    await page.goBack();
    await expect(page).toHaveURL(returnUrl);
    await expect(ranking.locator(".rank-name a:visible").first()).toHaveText(
      firstName,
    );
    await page.getByRole("button", { name: "Grid", exact: true }).click();
    await expect(ranking.locator(".music-card")).toHaveCount(10);
    await expect(
      ranking.getByRole("button", { name: "Grid", exact: true }),
    ).toHaveAttribute("aria-pressed", "true");
    await page.reload();
    await expect(
      page.getByRole("button", { name: "Grid", exact: true }),
    ).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByLabel("Results per page")).toHaveAttribute(
      "data-value",
      "10",
    );
    await chooseOption(page.getByLabel("Period", { exact: true }), "custom");
    await page.getByLabel("From", { exact: true }).fill("2025-06-02");
    await page.getByLabel("Through", { exact: true }).fill("2025-06-02");
    await expect(ranking).toContainText("No tracks with counted listens");
    await page.goto("/history?period=all");
    await page.getByRole("button", { name: "Filters", exact: true }).click();
    await settleDisclosure(page.locator(".history-filter-disclosure"));
    await expect(page.locator("#history-total")).toHaveText("135 records");
    await expect(page.getByLabel("Results per page")).toHaveAttribute(
      "data-value",
      "10",
    );
    await page.getByLabel("Show listens below threshold too").check();
    await expect(page.locator("#history-total")).toHaveText("136 records");
    await page.getByLabel("Show listens below threshold too").uncheck();
    await page.getByLabel("Find artist").fill("Consultation artist");
    await page
      .getByRole("button", {
        name: "Consultation artist · 116 tracks",
        exact: true,
      })
      .click();
    await chooseOption(
      page.getByRole("combobox", { name: "Source", exact: true }),
      "spotify_api",
    );
    await expect(page.locator("#history-total")).toHaveText("0 records");
    await chooseOption(
      page.getByRole("combobox", { name: "Source", exact: true }),
      "spotify_import",
    );
    // The source-name artist now combines all of its imported tracks.
    await expect(page.locator("#history-total")).toHaveText("135 records");
    await page.getByLabel("Find album").fill("Consultation album 010");
    await page
      .getByRole("button", {
        name: "Consultation album 010 · Consultation track 010",
        exact: true,
      })
      .click();
    await expect(page.locator("#history-total")).toHaveText("2 records");
    await page.locator("#history-list .listen-album a").first().click();
    await expect(
      page.getByRole("heading", { name: "Consultation album 010." }),
    ).toBeVisible();
    await expect(page.locator(".rank-name a:visible")).toHaveCount(1);
    await page.locator(".rank-name a:visible").click();
    await expect(
      page.getByRole("heading", { name: "Consultation track 010." }),
    ).toBeVisible();
    await page
      .getByRole("link", { name: "Consultation artist", exact: true })
      .first()
      .click();
    await expect(
      page.getByRole("heading", { name: "Consultation artist." }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Albums in this period" }),
    ).toBeVisible();
    await expect(
      page
        .locator("article")
        .filter({
          has: page.getByRole("heading", { name: "Albums in this period" }),
        })
        .locator(".music-card"),
    ).toHaveCount(10);
    await chooseOption(page.getByLabel("Period", { exact: true }), "custom");
    await page.getByLabel("From", { exact: true }).fill("2025-06-01");
    await page.getByLabel("Through", { exact: true }).fill("2025-06-03");
    await expect(page.getByRole("combobox", { name: "Group by" })).toHaveCount(
      0,
    );
    await expect(page.locator(".chart-segment:visible")).toHaveCount(3);
    await page.locator(".chart-segment:visible").first().click();
    await expect(page.locator("#history-total")).toHaveText("135 records");
    await expect(
      page.getByText("Chart segment:", { exact: false }),
    ).toBeVisible();
    await expect(page.getByLabel("Find artist")).toHaveAttribute(
      "value",
      "Consultation artist",
    );
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto("/rankings/albums?period=all");
    await expect(
      page.getByRole("heading", { name: "Rankings." }),
    ).toBeVisible();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: "test-results/consultation-mobile.png",
      fullPage: true,
    });
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.goto("/dashboard");
    await expect(page.locator("#total-listens")).toHaveText("135");
    await page.screenshot({
      path: "test-results/consultation-desktop.png",
      fullPage: true,
    });
  });

  test("dashboard customization persists layout, playback, rankings and listening patterns", async ({
    archive: { page },
  }) => {
    // Customization Edit Mode verification
    const customizeBtn = page.getByRole("button", { name: "Personalizza" });
    await expect(customizeBtn).toBeVisible();
    await customizeBtn.click();

    // Verify Edit Mode toolbar is displayed
    const editToolbar = page.getByRole("toolbar", {
      name: "Personalizza dashboard",
    });
    await expect(editToolbar).toBeVisible();
    await expect(
      editToolbar.getByRole("button", { name: "Aggiungi widget" }),
    ).toBeVisible();
    await expect(
      editToolbar.getByRole("button", { name: "Ripristina predefinito" }),
    ).toBeVisible();
    await expect(
      editToolbar.getByRole("button", { name: "Annulla" }),
    ).toBeVisible();
    await expect(
      editToolbar.getByRole("button", { name: "Salva modifiche" }),
    ).toBeVisible();

    // Verify 8-widget V1 default layout
    const initialNowPlaying = page.locator(
      ".dashboard-widget[data-widget-type='now_playing']",
    );
    await expect(initialNowPlaying).toHaveClass(/span-6/);
    const totalsWidget = page.locator(
      ".dashboard-widget[data-widget-type='totals']",
    );
    await expect(totalsWidget).toHaveClass(/span-6/);
    const initialTopArtists = page.locator(
      ".dashboard-widget[data-widget-type='top_artists']",
    );
    await expect(initialTopArtists).toHaveClass(/span-2/);
    const initialTopAlbums = page.locator(
      ".dashboard-widget[data-widget-type='top_albums']",
    );
    await expect(initialTopAlbums).toHaveClass(/span-2/);
    const initialTopTracks = page.locator(
      ".dashboard-widget[data-widget-type='top_tracks']",
    );
    await expect(initialTopTracks).toHaveClass(/span-2/);
    const initialActivityChart = page.locator(
      ".dashboard-widget[data-widget-type='activity_chart']",
    );
    await expect(initialActivityChart).toHaveClass(/span-6/);
    const initialPatterns = page.locator(
      ".dashboard-widget[data-widget-type='listening_patterns']",
    );
    await expect(initialPatterns).toHaveClass(/span-3/);
    const initialRecent = page.locator(
      ".dashboard-widget[data-widget-type='recent_listens']",
    );
    await expect(initialRecent).toHaveClass(/span-3/);

    // Verify widgets show dashed border and edit controls
    await expect(totalsWidget).toHaveClass(/editing/);
    const spanButton = totalsWidget.locator(".widget-span-button");
    await expect(spanButton).toBeVisible();

    // Cycle span (6 -> 2)
    await spanButton.click();
    await expect(totalsWidget).toHaveClass(/span-2/);

    // Remove now_playing and re-add from catalog
    await initialNowPlaying.locator(".widget-remove-button").click();
    await expect(initialNowPlaying).toHaveCount(0);

    // Test adding now_playing widget from catalog
    await editToolbar.getByRole("button", { name: "Aggiungi widget" }).click();
    const catalogDialog = page.locator(".widget-catalog-dialog");
    await expect(catalogDialog).toBeVisible();
    const nowPlayingCard = catalogDialog
      .locator(".catalog-item-card")
      .filter({ hasText: "In riproduzione" });
    await expect(nowPlayingCard).toBeVisible();
    await expect(
      nowPlayingCard.getByText("Brano in ascolto live e avanzamento traccia"),
    ).toBeVisible();
    await nowPlayingCard.getByRole("button", { name: "Aggiungi" }).click();
    await expect(catalogDialog).toBeHidden();

    const nowPlayingWidget = page.locator(
      ".dashboard-widget[data-widget-type='now_playing']",
    );
    await expect(nowPlayingWidget).toBeVisible();
    await expect(nowPlayingWidget).toHaveClass(/span-3/);
    await expect(nowPlayingWidget.getByText("In riproduzione")).toBeVisible();
    await expect(
      nowPlayingWidget.getByText("Nessuna riproduzione recente"),
    ).toBeVisible();

    // Cycle span (3 -> 4)
    const npSpanBtn = nowPlayingWidget.locator(".widget-span-button");
    await npSpanBtn.click();
    await expect(nowPlayingWidget).toHaveClass(/span-4/);

    // Save changes
    await editToolbar.getByRole("button", { name: "Salva modifiche" }).click();
    const alert = editToolbar.getByRole("alert");
    if (await alert.isVisible().catch(() => false))
      console.log("TOOLBAR ALERT WAS:", await alert.innerText());
    await expect(editToolbar).toBeHidden();

    // Verify persistence across page reload
    await page.reload();
    const totalsReloaded = page.locator(
      ".dashboard-widget[data-widget-type='totals']",
    );
    await expect(totalsReloaded).toHaveClass(/span-2/);
    const nowPlayingReloaded = page.locator(
      ".dashboard-widget[data-widget-type='now_playing']",
    );
    await expect(nowPlayingReloaded).toBeVisible();
    await expect(nowPlayingReloaded).toHaveClass(/span-4/);
    await expect(
      nowPlayingReloaded.getByText("Nessuna riproduzione recente"),
    ).toBeVisible();

    // Test active playback state in NowPlayingWidget
    await page.route("**/api/spotify/status", async (route) => {
      await route.fulfill({
        json: {
          configured: true,
          connected: true,
          credentials_from_env: false,
          redirect_uri: "http://127.0.0.1/callback",
          sync_seconds: 120,
          now_playing_seconds: 30,
          connection: "connected",
          collection: "active",
          last_attempt_ms: Date.now(),
          last_sync_ms: Date.now(),
          last_added: 0,
          verified_at_ms: Date.now(),
          last_error: null,
          now_playing_error: null,
          now_playing_timing: null,
          coverage_warning: null,
          retry_at_ms: null,
          review_count: 0,
        },
      });
    });
    await page.route("**/api/spotify/now-playing", async (route) => {
      await route.fulfill({
        json: {
          track_id: null,
          title: "Live Active Song",
          artists: [{ id: "artist_1", name: "Live Artist" }],
          album: "Live Album Name",
          cover_url: null,
          spotify_url: "https://open.spotify.com/track/xyz",
          duration_ms: 180000,
          progress_ms: 45000,
          observed_at_ms: Date.now(),
          source_timestamp_ms: null,
          requested_at_ms: Date.now(),
          source_requested_at_ms: Date.now(),
          response_received_at_ms: Date.now(),
          served_at_ms: Date.now(),
        },
      });
    });
    await page.reload();
    const activeWidget = page.locator(
      ".dashboard-widget[data-widget-type='now_playing']",
    );
    await expect(activeWidget.getByText("Live Active Song")).toBeVisible();
    await expect(activeWidget.getByText("Live Artist")).toBeVisible();
    await expect(activeWidget.getByText("Live Album Name")).toBeVisible();
    await expect(activeWidget.locator("progress")).toBeVisible();
    await expect(
      activeWidget.locator(".sync-status-badge.status-active"),
    ).toBeVisible();
    await expect(activeWidget.locator(".now-external-link")).toBeVisible();
    await page.unroute("**/api/spotify/now-playing");
    await page.unroute("**/api/spotify/status");

    // Open catalog and add Top Artists widget
    await page.getByRole("button", { name: "Personalizza" }).click();
    await expect(editToolbar).toBeVisible();
    const existingTopArtists = page.locator(
      ".dashboard-widget[data-widget-type='top_artists']",
    );
    await existingTopArtists.locator(".widget-remove-button").click();
    await expect(existingTopArtists).toHaveCount(0);

    await editToolbar.getByRole("button", { name: "Aggiungi widget" }).click();
    await expect(catalogDialog).toBeVisible();
    const topArtistsCard = catalogDialog
      .locator(".catalog-item-card")
      .filter({ hasText: "Top Artisti" });
    await expect(topArtistsCard).toBeVisible();
    await topArtistsCard.getByRole("button", { name: "Aggiungi" }).click();
    await expect(catalogDialog).toBeHidden();

    const topArtistsWidget = page.locator(
      ".dashboard-widget[data-widget-type='top_artists']",
    );
    await expect(topArtistsWidget).toBeVisible();
    await expect(topArtistsWidget).toHaveClass(/span-2/);

    // Save changes with added widget and verify persistence
    await editToolbar.getByRole("button", { name: "Salva modifiche" }).click();
    await expect(editToolbar).toBeHidden();
    await page.reload();
    const topArtistsReloaded = page.locator(
      ".dashboard-widget[data-widget-type='top_artists']",
    );
    await expect(topArtistsReloaded).toBeVisible();
    await expect(topArtistsReloaded.locator("h2")).toHaveText("Top artists");

    // Test adding listening_patterns widget from catalog (#120)
    await page.getByRole("button", { name: "Personalizza" }).click();
    await expect(editToolbar).toBeVisible();
    const existingPatterns = page.locator(
      ".dashboard-widget[data-widget-type='listening_patterns']",
    );
    await existingPatterns.locator(".widget-remove-button").click();
    await expect(existingPatterns).toHaveCount(0);

    await editToolbar.getByRole("button", { name: "Aggiungi widget" }).click();
    await expect(catalogDialog).toBeVisible();
    const patternCard = catalogDialog
      .locator(".catalog-item-card")
      .filter({ hasText: "Pattern di ascolto" });
    await expect(patternCard).toBeVisible();
    await patternCard.getByRole("button", { name: "Aggiungi" }).click();

    const patternsWidget = page.locator(
      ".dashboard-widget[data-widget-type='listening_patterns']",
    );
    await expect(patternsWidget).toBeVisible();
    await expect(patternsWidget).toHaveClass(/span-3/);

    // Save and verify rendered listening_patterns widget
    await editToolbar.getByRole("button", { name: "Salva modifiche" }).click();
    await expect(editToolbar).toBeHidden();
    await expect(
      patternsWidget.getByRole("heading", { name: "Pattern di ascolto" }),
    ).toBeVisible();
    await expect(patternsWidget.locator(".pattern-bars")).toBeVisible();
    await expect(
      patternsWidget.locator(".pattern-hours .pattern-row"),
    ).toHaveCount(24);

    // Switch view to weekdays
    await chooseOption(patternsWidget.getByRole("combobox"), "weekdays");
    await expect(
      patternsWidget.locator(".pattern-weekdays .pattern-row"),
    ).toHaveCount(7);

    // Switch view back to hours
    await chooseOption(patternsWidget.getByRole("combobox"), "hours");
    await expect(
      patternsWidget.locator(".pattern-hours .pattern-row"),
    ).toHaveCount(24);

    // Cycle span (3 -> 4 -> 6) and verify persistence
    await page.getByRole("button", { name: "Personalizza" }).click();
    await expect(editToolbar).toBeVisible();
    const patternSpanBtn = patternsWidget.locator(".widget-span-button");
    await patternSpanBtn.click(); // 3 -> 4
    await expect(patternsWidget).toHaveClass(/span-4/);
    await patternSpanBtn.click(); // 4 -> 6
    await expect(patternsWidget).toHaveClass(/span-6/);
    await editToolbar.getByRole("button", { name: "Salva modifiche" }).click();
    await expect(editToolbar).toBeHidden();

    await page.reload();
    await expect(
      page.locator(".dashboard-widget[data-widget-type='listening_patterns']"),
    ).toHaveClass(/span-6/);
  });

  test("discovery widget and local period overrides persist independently of the global period", async ({
    archive: { page },
  }) => {
    const editToolbar = page.getByRole("toolbar", {
      name: "Personalizza dashboard",
    });
    // Add DiscoveryRatioWidget from catalog (#122)
    await page.getByRole("button", { name: "Personalizza" }).click();
    await expect(editToolbar).toBeVisible();
    await editToolbar.getByRole("button", { name: "Aggiungi widget" }).click();
    const catalogDialog122 = page.locator(".widget-catalog-dialog");
    await expect(catalogDialog122).toBeVisible();
    const addDiscoveryBtn = catalogDialog122
      .locator(".catalog-item-card")
      .filter({ hasText: "Rapporto scoperte" })
      .getByRole("button", { name: "Aggiungi" });
    await expect(addDiscoveryBtn).toBeVisible();
    await addDiscoveryBtn.click();
    await expect(catalogDialog122).toBeHidden();

    // Verify discovery_ratio widget appears in dashboard with default span 3
    const discoveryWidget = page.locator(
      ".dashboard-widget[data-widget-type='discovery_ratio']",
    );
    await expect(discoveryWidget).toBeVisible();
    await expect(discoveryWidget).toHaveClass(/span-3/);

    // Save and verify rendering after save
    await editToolbar.getByRole("button", { name: "Salva modifiche" }).click();
    await expect(editToolbar).toBeHidden();
    await expect(
      discoveryWidget.getByRole("heading", {
        name: "Nuove scoperte vs Riascolti",
      }),
    ).toBeVisible();
    await expect(discoveryWidget.getByRole("progressbar")).toBeVisible();

    // Verify persistence across reload
    await page.reload();
    const discoveryReloaded = page.locator(
      ".dashboard-widget[data-widget-type='discovery_ratio']",
    );
    await expect(discoveryReloaded).toBeVisible();
    await expect(discoveryReloaded).toHaveClass(/span-3/);

    // Ticket #121: Override Locale del Periodo per Singolo Widget
    // 1. Enter edit mode and set periodOverride="30d" on discovery_ratio widget
    await page.getByRole("button", { name: "Personalizza" }).click();
    await expect(editToolbar).toBeVisible();

    const discoveryWidget121 = page.locator(
      ".dashboard-widget[data-widget-type='discovery_ratio']",
    );
    const discoveryPeriodSelect = discoveryWidget121.getByRole("combobox", {
      name: "Periodo widget",
    });
    await expect(discoveryPeriodSelect).toBeVisible();
    await chooseOption(discoveryPeriodSelect, "30d");

    // Save changes
    await editToolbar.getByRole("button", { name: "Salva modifiche" }).click();
    await expect(editToolbar).toBeHidden();

    // 2. Verify .pill badge appears indicating the locked period
    const discoveryBadge = discoveryWidget121.locator(
      ".pill.widget-period-badge",
    );
    await expect(discoveryBadge).toBeVisible();
    await expect(discoveryBadge).toHaveText("Last 30 days");
    await expect(discoveryBadge).toHaveAttribute("data-period", "30d");

    // For 30d (empty in test data), discovery_ratio shows empty state
    await expect(
      discoveryWidget121.getByText("No listening data in this period."),
    ).toBeVisible();

    // 3. Changing the global dashboard period filter does NOT alter the locked widget
    const globalPeriod = page.getByRole("combobox", { name: "Period" });
    await chooseOption(globalPeriod, "week");

    // Verify locked widget maintains its empty state and badge
    await expect(discoveryBadge).toBeVisible();
    await expect(
      discoveryWidget121.getByText("No listening data in this period."),
    ).toBeVisible();

    // Change global filter back to "all"
    await chooseOption(globalPeriod, "all");
    await expect(discoveryBadge).toBeVisible();
    await expect(
      discoveryWidget121.getByText("No listening data in this period."),
    ).toBeVisible();

    // 4. Persistence across page reload
    await page.reload();
    const discoveryAfterReload = page.locator(
      ".dashboard-widget[data-widget-type='discovery_ratio']",
    );
    const badgeAfterReload = discoveryAfterReload.locator(
      ".pill.widget-period-badge",
    );
    await expect(badgeAfterReload).toBeVisible();
    await expect(badgeAfterReload).toHaveText("Last 30 days");
    await expect(
      discoveryAfterReload.getByText("No listening data in this period."),
    ).toBeVisible();

    // 5. Test periodOverride on a dashboard-slice widget (e.g. totals)
    await page.getByRole("button", { name: "Personalizza" }).click();
    await expect(editToolbar).toBeVisible();
    const totalsWidget121 = page.locator(
      ".dashboard-widget[data-widget-type='totals']",
    );
    const totalsPeriodSelect = totalsWidget121.getByRole("combobox", {
      name: "Periodo widget",
    });
    await expect(totalsPeriodSelect).toBeVisible();
    await chooseOption(totalsPeriodSelect, "30d");
    await editToolbar.getByRole("button", { name: "Salva modifiche" }).click();
    await expect(editToolbar).toBeHidden();

    const totalsBadge = totalsWidget121.locator(".pill.widget-period-badge");
    await expect(totalsBadge).toBeVisible();
    await expect(totalsBadge).toHaveText("Last 30 days");
    // Under 30d, total listens is 0
    await expect(totalsWidget121.locator("#total-listens")).toHaveText("0");

    // Reload and verify persistence of totals override
    await page.reload();
    const totalsPeriodReloaded = page.locator(
      ".dashboard-widget[data-widget-type='totals']",
    );
    await expect(
      totalsPeriodReloaded.locator(".pill.widget-period-badge"),
    ).toHaveText("Last 30 days");
    await expect(totalsPeriodReloaded.locator("#total-listens")).toHaveText(
      "0",
    );

    // Reset to default
    await page.getByRole("button", { name: "Personalizza" }).click();
    await expect(editToolbar).toBeVisible();
    await editToolbar
      .getByRole("button", { name: "Ripristina predefinito" })
      .click();
    const totalsAfterReset = page.locator(
      ".dashboard-widget[data-widget-type='totals']",
    );
    await expect(totalsAfterReset).toHaveClass(/span-6/);
    await expect(
      page.locator(".dashboard-widget[data-widget-type='now_playing']"),
    ).toHaveClass(/span-6/);
    await expect(
      page.locator(".dashboard-widget[data-widget-type='top_artists']"),
    ).toHaveClass(/span-2/);
    await expect(
      page.locator(".dashboard-widget[data-widget-type='top_albums']"),
    ).toHaveClass(/span-2/);
    await expect(
      page.locator(".dashboard-widget[data-widget-type='top_tracks']"),
    ).toHaveClass(/span-2/);
    await expect(
      page.locator(".dashboard-widget[data-widget-type='activity_chart']"),
    ).toHaveClass(/span-6/);
    await expect(
      page.locator(".dashboard-widget[data-widget-type='listening_patterns']"),
    ).toHaveClass(/span-3/);
    await expect(
      page.locator(".dashboard-widget[data-widget-type='recent_listens']"),
    ).toHaveClass(/span-3/);
    await expect(
      page.locator(".dashboard-widget[data-widget-type='discovery_ratio']"),
    ).toHaveCount(0);
    await editToolbar.getByRole("button", { name: "Salva modifiche" }).click();
    await expect(editToolbar).toBeHidden();

    await page.reload();
    await expect(
      page.locator(".dashboard-widget[data-widget-type='totals']"),
    ).toHaveClass(/span-6/);
    await expect(
      page.locator(".dashboard-widget[data-widget-type='now_playing']"),
    ).toHaveClass(/span-6/);
    await expect(
      page.locator(".dashboard-widget[data-widget-type='top_artists']"),
    ).toHaveClass(/span-2/);
    await expect(
      page.locator(".dashboard-widget[data-widget-type='top_albums']"),
    ).toHaveClass(/span-2/);
    await expect(
      page.locator(".dashboard-widget[data-widget-type='top_tracks']"),
    ).toHaveClass(/span-2/);
    await expect(
      page.locator(".dashboard-widget[data-widget-type='activity_chart']"),
    ).toHaveClass(/span-6/);
    await expect(
      page.locator(".dashboard-widget[data-widget-type='listening_patterns']"),
    ).toHaveClass(/span-3/);
    await expect(
      page.locator(".dashboard-widget[data-widget-type='recent_listens']"),
    ).toHaveClass(/span-3/);
    await expect(
      page.locator(".dashboard-widget[data-widget-type='discovery_ratio']"),
    ).toHaveCount(0);
  });

  test("dashboard polish contains long names, aligns controls and preserves loading geometry", async ({
    archive: { page },
  }, testInfo) => {
    test.setTimeout(120000);
    await page.route("**/api/dashboard?*", async (route) => {
      const response = await route.fetch();
      const data = await response.json();
      for (const items of Object.values(data.rankings || {})) {
        for (const item of items) {
          item.name = "長いアーティスト名とアルバムのタイトル".repeat(12);
          item.subtitle = "A very long album and artist name ".repeat(15);
        }
      }
      for (const item of data.recent || []) {
        item.title = "長い曲名と長いアルバムタイトル".repeat(12);
        item.album = "Long album ".repeat(30);
        item.artists = item.artists.map((artist) => ({
          ...artist,
          name: "長いアーティスト名".repeat(20),
        }));
      }
      await route.fulfill({ response, json: data });
    });
    await page.reload();
    await expect(page.locator("#total-listens")).toHaveText("135");
    await expect(
      page.locator(".widget-patterns-content .pattern-row").first(),
    ).toBeVisible();
    const customize = page.getByRole("button", {
      name: "Personalizza",
      exact: true,
    });
    for (const width of [320, 390, 601, 768, 1440]) {
      await page.setViewportSize({ width, height: 1000 });
      for (const theme of ["light", "dark"]) {
        await page.evaluate((theme) => {
          document.documentElement.classList.toggle(
            "theme-light",
            theme === "light",
          );
          document.documentElement.style.setProperty("--accent", "#c5a3ee");
        }, theme);
        await customize.click();
        const failures = await page
          .locator(".dashboard-grid")
          .evaluate((grid) => {
            const failures = [];
            if (document.documentElement.scrollWidth > innerWidth)
              failures.push("page overflows");
            const cells = [
              ...grid.querySelectorAll(":scope > .dashboard-widget"),
            ];
            for (const cell of cells) {
              const rect = cell.getBoundingClientRect();
              const mainPanel = cell.querySelector(".overview-ranking-panel");
              if (
                mainPanel &&
                Math.abs(mainPanel.getBoundingClientRect().width - rect.width) >
                  1
              )
                failures.push("ranking does not fill slot");
              for (const panel of cell.querySelectorAll(
                ".panel, .widget-edit-controls",
              )) {
                const box = panel.getBoundingClientRect();
                if (box.bottom > rect.bottom + 1 || box.right > rect.right + 1)
                  failures.push(cell.dataset.widgetType + " escapes slot");
              }
              for (const button of cell.querySelectorAll(
                ".widget-control, .select-trigger",
              )) {
                const box = button.getBoundingClientRect();
                if (box.width < 44 || box.height < 44)
                  failures.push("small target");
              }
            }
            for (let i = 0; i < cells.length; i++)
              for (let j = i + 1; j < cells.length; j++) {
                const a = cells[i].getBoundingClientRect(),
                  b = cells[j].getBoundingClientRect();
                if (
                  a.left < b.right - 1 &&
                  a.right > b.left + 1 &&
                  a.top < b.bottom - 1 &&
                  a.bottom > b.top + 1
                )
                  failures.push("overlap");
              }
            return failures;
          });
        expect(failures).toEqual([]);
        for (const selector of [
          ".rank-value",
          ".rank-number",
          ".pattern-readout",
          ".chart-bar-value",
          ".time-composition",
        ]) {
          await expect(
            page.locator(".dashboard-widget " + selector).first(),
          ).toHaveCSS("font-variant-numeric", "tabular-nums");
        }
        await expect(
          page.locator(".widget-top-tracks .rank-name a").first(),
        ).toHaveCSS("text-overflow", "ellipsis");
        await page.screenshot({
          path: testInfo.outputPath(
            "dashboard-" + width + "-" + theme + ".png",
          ),
          fullPage: true,
        });
        await page
          .getByRole("button", { name: "Annulla", exact: true })
          .click();
        await expect(customize).toBeFocused();
      }
    }
    // Hold only the period read: the old, pending and empty states occupy the same slots.
    await page.setViewportSize({ width: 1440, height: 1000 });
    const heights = () =>
      page
        .locator(".dashboard-widget")
        .evaluateAll((cells) =>
          cells.map((cell) => Math.round(cell.getBoundingClientRect().height)),
        );
    const before = await heights();
    let release;
    const gate = new Promise((resolve) => (release = resolve));
    await page.route("**/api/dashboard?*", async (route) => {
      await gate;
      await route.continue();
    });
    try {
      await chooseOption(
        page.getByRole("combobox", { name: "Period", exact: true }),
        "30d",
      );
      await expect(page.locator(".dashboard-grid")).toHaveAttribute(
        "aria-busy",
        "true",
      );
      expect(await heights()).toEqual(before);
      release();
      await expect(page.locator("#total-listens")).toHaveText("0");
      expect(await heights()).toEqual(before);
    } finally {
      release();
    }
    await page.emulateMedia({ reducedMotion: "reduce" });
    await customize.click();
    await expect(page.locator(".dashboard-edit-toolbar")).toHaveCSS(
      "animation-name",
      "none",
    );
    await expect(page.locator(".dashboard-widget").first()).toHaveCSS(
      "transition-duration",
      "0s",
    );
    // The optional catalog widgets use the same anatomy at every supported span.
    for (const title of ["Classifiche Congiunte", "Rapporto scoperte"]) {
      await page
        .getByRole("button", { name: "Aggiungi widget", exact: true })
        .click();
      await page
        .locator(".catalog-item-card")
        .filter({
          has: page.getByRole("heading", { name: title, exact: true }),
        })
        .getByRole("button", { name: "Aggiungi", exact: true })
        .click();
    }
    for (const type of [
      "now_playing",
      "totals",
      "unified_rankings",
      "top_artists",
      "top_albums",
      "top_tracks",
      "activity_chart",
      "recent_listens",
      "listening_patterns",
      "discovery_ratio",
    ]) {
      const widget = page.locator(
        '.dashboard-widget[data-widget-type="' + type + '"]',
      );
      for (let i = 0; i < 4; i++) {
        await widget.locator(".widget-span-button").click();
        expect(
          await widget.evaluate((cell) => {
            const bounds = cell.getBoundingClientRect();
            return [
              ...cell.querySelectorAll(".panel, .widget-edit-controls"),
            ].every((panel) => {
              const rect = panel.getBoundingClientRect();
              return (
                rect.right <= bounds.right + 1 &&
                rect.bottom <= bounds.bottom + 1
              );
            });
          }),
          type,
        ).toBeTruthy();
      }
    }
    await page
      .getByRole("button", { name: "Aggiungi widget", exact: true })
      .click();
    await page.screenshot({
      path: testInfo.outputPath("widget-catalog.png"),
      fullPage: true,
    });
    await page.keyboard.press("Escape");
    await expect(page.locator(".widget-catalog-dialog")).toBeHidden();
  });

  test("listening time help remains accessible on touch screens and reports missing durations", async ({
    archive: { page, context, port },
    browser,
    browserName,
  }) => {
    if (browserName !== "firefox") {
      const touch = await browser.newContext({
        baseURL: `http://127.0.0.1:${port}`,
        storageState: await context.storageState(),
        hasTouch: true,
        viewport: { width: 390, height: 844 },
      });
      try {
        const touchPage = await touch.newPage();
        async function expectHelpAccessible(info) {
          await expect
            .poll(() =>
              info.evaluate((button) => {
                const panel = document.getElementById(
                  button.getAttribute("aria-controls"),
                );
                if (!panel) return false;
                const b = button.getBoundingClientRect();
                const p = panel.getBoundingClientRect();
                return (
                  (p.bottom <= b.top || p.top >= b.bottom) &&
                  p.top >= 0 &&
                  p.bottom <= innerHeight &&
                  p.left >= 0 &&
                  p.right <= innerWidth &&
                  document.elementFromPoint(
                    b.x + b.width / 2,
                    b.y + b.height / 2,
                  ) === button
                );
              }),
            )
            .toBe(true);
        }
        for (const height of [600, 844]) {
          await touchPage.setViewportSize({ width: 390, height });
          await touchPage.goto("/dashboard");
          const info = touchPage.getByRole("button", {
            name: "How we calculate listening time",
            exact: true,
          });
          // Touch devices can synthesize mouseover before delivering a click.
          await info.dispatchEvent("mouseover", { bubbles: true });
          await expect(info).toHaveAttribute("aria-expanded", "false");
          await info.tap();
          await expect(info).toHaveAttribute("aria-expanded", "true");
          // A fixed bottom panel covered the trigger on short WebKit viewports.
          await expectHelpAccessible(info);
          await info.tap();
          await expect(info).toHaveAttribute("aria-expanded", "false");
          await info.tap();
          await touchPage.setViewportSize({
            width: 390,
            height: height === 600 ? 844 : 600,
          });
          await info.scrollIntoViewIfNeeded();
          await expectHelpAccessible(info);
          await touchPage.evaluate(() => window.scrollBy(0, 80));
          await expectHelpAccessible(info);
          await info.tap();
          await expect(info).toHaveAttribute("aria-expanded", "false");
        }
      } finally {
        await touch.close();
      }
    }
    await page.route("**/api/dashboard?*", async (route) => {
      const response = await route.fetch();
      const data = await response.json();
      data.totals = {
        records: 1,
        listens: 1,
        tracks: 1,
        actual_ms: 0,
        estimated_ms: 0,
        unknown_durations: 1,
      };
      await route.fulfill({ json: data });
    });
    await page.reload();
    await expect(page.locator("#total-time")).toHaveText("Time unavailable");
    await expect(
      page.getByText("Missing durations limit the completeness", {
        exact: false,
      }),
    ).toBeVisible();
  });

  test("artist template customization persists layout, updates across artists, and isolates home", async ({
    archive: { page },
  }) => {
    // 1. Navigate to rankings and click an artist to open Artist consultation page
    await page.goto("/rankings/artists?period=all");
    const firstArtist = page.locator(".rank-name a").first();
    await expect(firstArtist).toBeVisible();
    const artistName = await firstArtist.innerText();
    await firstArtist.click();
    await expect(
      page.getByRole("heading", { name: artistName, exact: false }),
    ).toBeVisible();

    // 2. Verify external entity header components
    await expect(page.locator(".music-heading")).toBeVisible();
    await expect(page.locator(".music-personal")).toBeVisible();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
    await expect(page.getByLabel("Period", { exact: true })).toBeVisible();

    // 3. Verify default 6-widget artist template layout in .dashboard-grid
    const artistGrid = page.locator(".dashboard-grid");
    await expect(artistGrid).toBeVisible();

    const totalsWidget = artistGrid.locator(
      ".dashboard-widget[data-widget-type='totals']",
    );
    await expect(totalsWidget).toBeVisible();
    await expect(totalsWidget).toHaveClass(/span-6/);

    const activityWidget = artistGrid.locator(
      ".dashboard-widget[data-widget-type='activity_chart']",
    );
    await expect(activityWidget).toBeVisible();
    await expect(activityWidget).toHaveClass(/span-6/);

    const patternsWidget = artistGrid.locator(
      ".dashboard-widget[data-widget-type='listening_patterns']",
    );
    await expect(patternsWidget).toBeVisible();
    await expect(patternsWidget).toHaveClass(/span-6/);

    const topAlbumsWidget = artistGrid.locator(
      ".dashboard-widget[data-widget-type='top_albums']",
    );
    await expect(topAlbumsWidget).toBeVisible();
    await expect(topAlbumsWidget).toHaveClass(/span-3/);

    const topTracksWidget = artistGrid.locator(
      ".dashboard-widget[data-widget-type='top_tracks']",
    );
    await expect(topTracksWidget).toBeVisible();
    await expect(topTracksWidget).toHaveClass(/span-3/);

    const historyWidget = artistGrid.locator(
      ".dashboard-widget[data-widget-type='history']",
    );
    await expect(historyWidget).toBeVisible();
    await expect(historyWidget).toHaveClass(/span-6/);

    // 4. Click "Personalizza template artista" action to enter Edit mode
    const customizeBtn = page.getByRole("button", {
      name: "Personalizza template artista",
    });
    await expect(customizeBtn).toBeVisible();
    await customizeBtn.click();

    // 5. Verify toolbar
    const editToolbar = page.getByRole("toolbar", {
      name: "Personalizza template artista",
    });
    await expect(editToolbar).toBeVisible();
    await expect(
      editToolbar.getByText(/applicate a tutte le pagine artista/i),
    ).toBeVisible();

    // 6. Verify Catalog Modal does NOT show home-only widgets
    await editToolbar.getByRole("button", { name: "Aggiungi widget" }).click();
    const catalogDialog = page.locator(".widget-catalog-dialog");
    await expect(catalogDialog).toBeVisible();
    // home-only widgets must NOT be present
    await expect(catalogDialog.getByText("In riproduzione")).toHaveCount(0);
    await expect(catalogDialog.getByText("Top Artisti")).toHaveCount(0);
    await expect(catalogDialog.getByText("Classifiche Congiunte")).toHaveCount(
      0,
    );
    await catalogDialog
      .getByRole("button", { name: "Chiudi catalogo" })
      .click();
    await expect(catalogDialog).toBeHidden();

    // 7. Cycle span on top_albums (3 -> 4)
    const albumSpanBtn = topAlbumsWidget.locator(".widget-span-button");
    await albumSpanBtn.click();
    await expect(topAlbumsWidget).toHaveClass(/span-4/);

    // Remove top_tracks
    await topTracksWidget.locator(".widget-remove-button").click();
    await expect(
      artistGrid.locator(".dashboard-widget[data-widget-type='top_tracks']"),
    ).toHaveCount(0);

    // 8. Save changes
    await editToolbar.getByRole("button", { name: "Salva modifiche" }).click();
    await expect(editToolbar).toBeHidden();

    // 9. Reload page and assert persistence
    await page.reload();
    await expect(
      artistGrid.locator(".dashboard-widget[data-widget-type='top_albums']"),
    ).toHaveClass(/span-4/);
    await expect(
      artistGrid.locator(".dashboard-widget[data-widget-type='top_tracks']"),
    ).toHaveCount(0);

    // 10. Navigate to a second artist and verify identical template layout
    await page.goto("/rankings/artists?period=all");
    const artists = page.locator(".rank-name a");
    if ((await artists.count()) > 1) {
      const secondArtist = artists.nth(1);
      const secondArtistName = await secondArtist.innerText();
      await secondArtist.click();
      await expect(
        page.getByRole("heading", { name: secondArtistName, exact: false }),
      ).toBeVisible();
      const secondGrid = page.locator(".dashboard-grid");
      await expect(
        secondGrid.locator(".dashboard-widget[data-widget-type='top_albums']"),
      ).toHaveClass(/span-4/);
      await expect(
        secondGrid.locator(".dashboard-widget[data-widget-type='top_tracks']"),
      ).toHaveCount(0);
    }

    // 11. Verify Home dashboard layout is independent / untouched
    await page.goto("/dashboard");
    const homeGrid = page.locator(".dashboard-grid");
    await expect(
      homeGrid.locator(".dashboard-widget[data-widget-type='top_tracks']"),
    ).toBeVisible();
    await expect(
      homeGrid.locator(".dashboard-widget[data-widget-type='top_artists']"),
    ).toBeVisible();

    // 12. Verify Settings entry point for template customization
    await page.goto("/settings/appearance");
    await expect(
      page.getByRole("heading", { name: "Template pagine musicali" }),
    ).toBeVisible();
    const settingsCustomizeBtn = page.getByRole("button", {
      name: "Personalizza template artista",
    });
    await expect(settingsCustomizeBtn).toBeVisible();
    await settingsCustomizeBtn.click();
    // Navigates to artist page with editor toolbar open
    await expect(
      page.getByRole("toolbar", { name: "Personalizza template artista" }),
    ).toBeVisible();
  });

  test("artist template handles dirty cancel confirmation, 409 conflict preserving draft, and empty preview", async ({
    archive: { page },
  }) => {
    // 1. Open artist page
    await page.goto("/rankings/artists?period=all");
    const firstArtist = page.locator(".rank-name a").first();
    await firstArtist.click();

    // 2. Click "Personalizza template artista"
    await page
      .getByRole("button", { name: "Personalizza template artista" })
      .click();
    const editToolbar = page.getByRole("toolbar", {
      name: "Personalizza template artista",
    });
    await expect(editToolbar).toBeVisible();

    // Make a change to create dirty draft
    const artistGrid = page.locator(".dashboard-grid");
    const topAlbums = artistGrid.locator(
      ".dashboard-widget[data-widget-type='top_albums']",
    );
    await topAlbums.locator(".widget-span-button").click();

    // 3. Test Dirty confirmation on Cancel
    // When dialog is dismissed, stay in edit mode
    let dialogMessage = "";
    page.once("dialog", async (dialog) => {
      dialogMessage = dialog.message();
      await dialog.dismiss();
    });
    await editToolbar.getByRole("button", { name: "Annulla" }).click();
    expect(dialogMessage).toContain("modifiche non salvate");
    await expect(editToolbar).toBeVisible();

    // When dialog is accepted, exit edit mode and restore
    page.once("dialog", async (dialog) => {
      await dialog.accept();
    });
    await editToolbar.getByRole("button", { name: "Annulla" }).click();
    await expect(editToolbar).toBeHidden();

    // 4. Test 409 Concurrency Conflict preserving draft
    await page
      .getByRole("button", { name: "Personalizza template artista" })
      .click();
    await expect(editToolbar).toBeVisible();
    // Remove history widget
    await artistGrid
      .locator(".dashboard-widget[data-widget-type='history']")
      .locator(".widget-remove-button")
      .click();
    await expect(
      artistGrid.locator(".dashboard-widget[data-widget-type='history']"),
    ).toHaveCount(0);

    // Mock 409 Conflict from backend
    await page.route(
      "**/api/v1/dashboard/layout?template=artist",
      async (route) => {
        if (route.request().method() === "POST") {
          await route.fulfill({
            status: 409,
            json: {
              error:
                "Dashboard layout changed in another session. Reload before saving.",
            },
          });
        } else {
          await route.continue();
        }
      },
    );

    await editToolbar.getByRole("button", { name: "Salva modifiche" }).click();
    // Conflict message shown
    await expect(
      editToolbar.getByText("Dashboard layout changed in another session"),
    ).toBeVisible();
    // Draft preserved: editor is still open and history is still removed
    await expect(editToolbar).toBeVisible();
    await expect(
      artistGrid.locator(".dashboard-widget[data-widget-type='history']"),
    ).toHaveCount(0);

    await page.unroute("**/api/v1/dashboard/layout?template=artist");

    // 5. Test empty library preview from Settings
    await page.route(
      "**/api/v1/consultation/rankings/artists*",
      async (route) => {
        await route.fulfill({
          status: 200,
          json: { items: [], total: 0, page: 1, page_size: 20 },
        });
      },
    );

    await page.goto("/settings/appearance");
    await expect(
      page.getByText(
        "Nessun artista presente nell'archivio. Verrà usata un'anteprima vuota.",
      ),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "Personalizza template artista" })
      .click();

    // Navigates to /music/artists/preview?edit=template with editor open
    await expect(
      page.getByRole("toolbar", { name: "Personalizza template artista" }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Artista di anteprima" }),
    ).toBeVisible();
    await expect(page.locator(".dashboard-grid")).toBeVisible();
  });

  test("album template customization persists layout, updates across albums, renders Brani dell'album with proportional bars, and isolates home and artist", async ({
    archive: { page },
  }) => {
    // 1. Navigate to rankings and click an album to open Album consultation page
    await page.goto("/rankings/albums?period=all");
    const firstAlbum = page.locator(".rank-name a").first();
    await expect(firstAlbum).toBeVisible();
    const albumName = await firstAlbum.innerText();
    await firstAlbum.click();
    await expect(
      page.getByRole("heading", { name: albumName, exact: false }),
    ).toBeVisible();

    // 2. Verify external entity header components
    await expect(page.locator(".music-heading")).toBeVisible();
    await expect(page.locator(".music-personal")).toBeVisible();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
    await expect(page.getByLabel("Period", { exact: true })).toBeVisible();
    await expect(page.getByText("Album by")).toBeVisible();

    // 3. Verify default 5-widget album template layout in .dashboard-grid
    const albumGrid = page.locator(".dashboard-grid");
    await expect(albumGrid).toBeVisible();

    const totalsWidget = albumGrid.locator(
      ".dashboard-widget[data-widget-type='totals']",
    );
    await expect(totalsWidget).toBeVisible();
    await expect(totalsWidget).toHaveClass(/span-6/);

    const activityWidget = albumGrid.locator(
      ".dashboard-widget[data-widget-type='activity_chart']",
    );
    await expect(activityWidget).toBeVisible();
    await expect(activityWidget).toHaveClass(/span-6/);

    const patternsWidget = albumGrid.locator(
      ".dashboard-widget[data-widget-type='listening_patterns']",
    );
    await expect(patternsWidget).toBeVisible();
    await expect(patternsWidget).toHaveClass(/span-6/);

    const albumTracksWidget = albumGrid.locator(
      ".dashboard-widget[data-widget-type='top_tracks']",
    );
    await expect(albumTracksWidget).toBeVisible();
    await expect(albumTracksWidget).toHaveClass(/span-6/);

    const historyWidget = albumGrid.locator(
      ".dashboard-widget[data-widget-type='history']",
    );
    await expect(historyWidget).toBeVisible();
    await expect(historyWidget).toHaveClass(/span-6/);

    // 4. Verify widget "Brani dell'album" with proportional bars
    await expect(
      albumTracksWidget.getByRole("heading", { name: "Brani dell'album" }),
    ).toBeVisible();
    const bars = albumTracksWidget.locator(".ranking-meter");
    if ((await albumTracksWidget.locator(".ranking-row").count()) > 0) {
      await expect(bars.first()).toBeVisible();
    }

    // 5. Click "Personalizza template album" action to enter Edit mode
    const customizeBtn = page.getByRole("button", {
      name: "Personalizza template album",
    });
    await expect(customizeBtn).toBeVisible();
    await customizeBtn.click();

    // 6. Verify toolbar
    const editToolbar = page.getByRole("toolbar", {
      name: "Personalizza template album",
    });
    await expect(editToolbar).toBeVisible();
    await expect(
      editToolbar.getByText(/applicate a tutte le pagine album/i),
    ).toBeVisible();

    // 7. Verify Catalog Modal does NOT show forbidden widgets (top_albums, top_artists, now_playing, unified_rankings)
    await editToolbar.getByRole("button", { name: "Aggiungi widget" }).click();
    const catalogDialog = page.locator(".widget-catalog-dialog");
    await expect(catalogDialog).toBeVisible();
    await expect(catalogDialog.getByText("In riproduzione")).toHaveCount(0);
    await expect(catalogDialog.getByText("Top Artisti")).toHaveCount(0);
    await expect(catalogDialog.getByText("Top Album")).toHaveCount(0);
    await expect(catalogDialog.getByText("Classifiche Congiunte")).toHaveCount(
      0,
    );
    await catalogDialog
      .getByRole("button", { name: "Chiudi catalogo" })
      .click();
    await expect(catalogDialog).toBeHidden();

    // 8. Cycle span on albumTracksWidget (6 -> 2)
    const trackSpanBtn = albumTracksWidget.locator(".widget-span-button");
    await trackSpanBtn.click();
    await expect(albumTracksWidget).toHaveClass(/span-2/);

    // 9. Save changes
    await editToolbar.getByRole("button", { name: "Salva modifiche" }).click();
    await expect(editToolbar).toBeHidden();

    // 10. Reload page and assert persistence
    await page.reload();
    await expect(
      albumGrid.locator(".dashboard-widget[data-widget-type='top_tracks']"),
    ).toHaveClass(/span-2/);

    // 11. Navigate to a second album and verify identical template layout
    await page.goto("/rankings/albums?period=all");
    const albums = page.locator(".rank-name a");
    if ((await albums.count()) > 1) {
      const secondAlbum = albums.nth(1);
      const secondAlbumName = await secondAlbum.innerText();
      await secondAlbum.click();
      await expect(
        page.getByRole("heading", { name: secondAlbumName, exact: false }),
      ).toBeVisible();
      const secondGrid = page.locator(".dashboard-grid");
      await expect(
        secondGrid.locator(".dashboard-widget[data-widget-type='top_tracks']"),
      ).toHaveClass(/span-2/);
    }

    // 12. Verify Home and Artist dashboard layouts are independent / untouched
    await page.goto("/dashboard");
    const homeGrid = page.locator(".dashboard-grid");
    await expect(
      homeGrid.locator(".dashboard-widget[data-widget-type='top_tracks']"),
    ).toBeVisible();
    await expect(
      homeGrid.locator(".dashboard-widget[data-widget-type='top_albums']"),
    ).toBeVisible();

    await page.goto("/rankings/artists?period=all");
    const artist = page.locator(".rank-name a").first();
    await artist.click();
    const artistGrid = page.locator(".dashboard-grid");
    await expect(
      artistGrid.locator(".dashboard-widget[data-widget-type='top_albums']"),
    ).toBeVisible();

    // 13. Verify Settings entry point for template customization
    await page.goto("/settings/appearance");
    await expect(
      page.getByRole("heading", { name: "Template pagine musicali" }),
    ).toBeVisible();
    await page.getByLabel("Tipo di pagina").click();
    await page.getByRole("option", { name: "Pagina Album" }).click();
    const settingsCustomizeBtn = page.getByRole("button", {
      name: "Personalizza template album",
    });
    await expect(settingsCustomizeBtn).toBeVisible();
    await settingsCustomizeBtn.click();
    // Navigates to album page with editor toolbar open
    await expect(
      page.getByRole("toolbar", { name: "Personalizza template album" }),
    ).toBeVisible();
  });

  test("album template handles dirty cancel confirmation, 409 conflict preserving draft, empty album template, and empty library preview", async ({
    archive: { page },
  }) => {
    // 1. Open album page
    await page.goto("/rankings/albums?period=all");
    const firstAlbum = page.locator(".rank-name a").first();
    await firstAlbum.click();

    // 2. Click "Personalizza template album"
    await page
      .getByRole("button", { name: "Personalizza template album" })
      .click();
    const editToolbar = page.getByRole("toolbar", {
      name: "Personalizza template album",
    });
    await expect(editToolbar).toBeVisible();

    // Make a change to create dirty draft
    const albumGrid = page.locator(".dashboard-grid");
    const totals = albumGrid.locator(
      ".dashboard-widget[data-widget-type='totals']",
    );
    await totals.locator(".widget-span-button").click();

    // 3. Test Dirty confirmation on Cancel
    let dialogMessage = "";
    page.once("dialog", async (dialog) => {
      dialogMessage = dialog.message();
      await dialog.dismiss();
    });
    await editToolbar.getByRole("button", { name: "Annulla" }).click();
    expect(dialogMessage).toContain("modifiche non salvate");
    await expect(editToolbar).toBeVisible();

    page.once("dialog", async (dialog) => {
      await dialog.accept();
    });
    await editToolbar.getByRole("button", { name: "Annulla" }).click();
    await expect(editToolbar).toBeHidden();

    // 4. Test 409 Concurrency Conflict preserving draft
    await page
      .getByRole("button", { name: "Personalizza template album" })
      .click();
    await expect(editToolbar).toBeVisible();
    // Remove history widget
    await albumGrid
      .locator(".dashboard-widget[data-widget-type='history']")
      .locator(".widget-remove-button")
      .click();
    await expect(
      albumGrid.locator(".dashboard-widget[data-widget-type='history']"),
    ).toHaveCount(0);

    // Mock 409 Conflict from backend
    await page.route(
      "**/api/v1/dashboard/layout?template=album",
      async (route) => {
        if (route.request().method() === "POST") {
          await route.fulfill({
            status: 409,
            json: {
              error:
                "Dashboard layout changed in another session. Reload before saving.",
            },
          });
        } else {
          await route.continue();
        }
      },
    );

    await editToolbar.getByRole("button", { name: "Salva modifiche" }).click();
    // Conflict message shown
    await expect(
      editToolbar.getByText("Dashboard layout changed in another session"),
    ).toBeVisible();
    // Draft preserved: editor is still open and history is still removed
    await expect(editToolbar).toBeVisible();
    await expect(
      albumGrid.locator(".dashboard-widget[data-widget-type='history']"),
    ).toHaveCount(0);

    await page.unroute("**/api/v1/dashboard/layout?template=album");

    // 5. Test empty album template
    // Remove all remaining widgets
    for (let i = 0; i < 10; i++) {
      const removeBtns = albumGrid.locator(".widget-remove-button");
      if ((await removeBtns.count()) === 0) break;
      await removeBtns.first().click();
    }
    await expect(albumGrid.locator(".dashboard-widget")).toHaveCount(0);
    await editToolbar.getByRole("button", { name: "Salva modifiche" }).click();
    await expect(editToolbar).toBeHidden();

    // Reload and verify empty layout notice is displayed, while header and customize action remain
    await page.reload();
    await expect(page.locator(".dashboard-empty-notice")).toBeVisible();
    await expect(page.locator(".music-heading")).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Personalizza template album" }),
    ).toBeVisible();

    // 6. Test empty library preview from Settings
    await page.route(
      "**/api/v1/consultation/rankings/albums*",
      async (route) => {
        await route.fulfill({
          status: 200,
          json: { items: [], total: 0, page: 1, page_size: 20 },
        });
      },
    );

    await page.goto("/settings/appearance");
    await page.getByLabel("Tipo di pagina").click();
    await page.getByRole("option", { name: "Pagina Album" }).click();
    await expect(
      page.getByText(
        "Nessun album presente nell'archivio. Verrà usata un'anteprima vuota.",
      ),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "Personalizza template album" })
      .click();

    // Navigates to /music/albums/preview?edit=template with editor open
    await expect(
      page.getByRole("toolbar", { name: "Personalizza template album" }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Album di anteprima" }),
    ).toBeVisible();
    await expect(page.locator(".dashboard-grid")).toBeVisible();
  });

  test("track consultation page uses shared grid engine and independent track template layout", async ({
    archive: { page },
  }) => {
    // 1. Navigate to rankings and click a track to open Track consultation page
    await page.goto("/rankings/tracks?period=all");
    const firstTrack = page.locator(".rank-name a").first();
    await expect(firstTrack).toBeVisible();
    const trackName = await firstTrack.innerText();
    await firstTrack.click();
    await expect(
      page.getByRole("heading", { name: trackName, exact: false }),
    ).toBeVisible();

    // 2. Verify external entity header components
    await expect(page.locator(".music-heading")).toBeVisible();
    await expect(page.locator(".music-personal")).toBeVisible();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
    await expect(page.getByLabel("Period", { exact: true })).toBeVisible();

    // 3. Verify default 4-widget track template layout in .dashboard-grid
    const trackGrid = page.locator(".dashboard-grid");
    await expect(trackGrid).toBeVisible();

    const totalsWidget = trackGrid.locator(
      ".dashboard-widget[data-widget-type='totals']",
    );
    await expect(totalsWidget).toBeVisible();
    await expect(totalsWidget).toHaveClass(/span-6/);

    const activityWidget = trackGrid.locator(
      ".dashboard-widget[data-widget-type='activity_chart']",
    );
    await expect(activityWidget).toBeVisible();
    await expect(activityWidget).toHaveClass(/span-6/);

    const patternsWidget = trackGrid.locator(
      ".dashboard-widget[data-widget-type='listening_patterns']",
    );
    await expect(patternsWidget).toBeVisible();
    await expect(patternsWidget).toHaveClass(/span-6/);

    const historyWidget = trackGrid.locator(
      ".dashboard-widget[data-widget-type='history']",
    );
    await expect(historyWidget).toBeVisible();
    await expect(historyWidget).toHaveClass(/span-6/);

    // Verify top_albums and top_tracks are NOT present in track grid
    await expect(
      trackGrid.locator(".dashboard-widget[data-widget-type='top_albums']"),
    ).toHaveCount(0);
    await expect(
      trackGrid.locator(".dashboard-widget[data-widget-type='top_tracks']"),
    ).toHaveCount(0);

    // 4. Click "Personalizza template brano" action to enter Edit mode
    const customizeBtn = page.getByRole("button", {
      name: "Personalizza template brano",
    });
    await expect(customizeBtn).toBeVisible();
    await customizeBtn.click();

    // 5. Verify toolbar
    const editToolbar = page.getByRole("toolbar", {
      name: "Personalizza template brano",
    });
    await expect(editToolbar).toBeVisible();
    await expect(
      editToolbar.getByText(/applicate a tutte le pagine brano/i),
    ).toBeVisible();

    // 6. Verify Catalog Modal shows only compatible widgets
    await editToolbar.getByRole("button", { name: "Aggiungi widget" }).click();
    const catalogDialog = page.locator(".widget-catalog-dialog");
    await expect(catalogDialog).toBeVisible();

    // Incompatible widgets must NOT be present
    await expect(catalogDialog.getByText("In riproduzione")).toHaveCount(0);
    await expect(catalogDialog.getByText("Top Artisti")).toHaveCount(0);
    await expect(catalogDialog.getByText("Top Album")).toHaveCount(0);
    await expect(catalogDialog.getByText("Top Brani")).toHaveCount(0);
    await expect(catalogDialog.getByText("Classifiche Congiunte")).toHaveCount(
      0,
    );
    await expect(catalogDialog.getByText("Rapporto scoperte")).toHaveCount(0);

    // Compatible widget: Ascolti Recenti must be available
    const addRecentBtn = catalogDialog.getByRole("button", {
      name: /Ascolti Recenti/i,
    });
    await expect(addRecentBtn).toBeVisible();

    // 7. Add recent_listens widget to track template
    await addRecentBtn.click();
    await expect(catalogDialog).toBeHidden();

    const recentWidget = trackGrid.locator(
      ".dashboard-widget[data-widget-type='recent_listens']",
    );
    await expect(recentWidget).toBeVisible();

    // 8. Cycle span on totals (6 -> 2)
    const totalsSpanBtn = totalsWidget.locator(".widget-span-button");
    await totalsSpanBtn.click();
    await expect(totalsWidget).toHaveClass(/span-2/);

    // 9. Save changes
    await editToolbar.getByRole("button", { name: "Salva modifiche" }).click();
    await expect(editToolbar).toBeHidden();

    // 10. Reload page and assert persistence
    await page.reload();
    await expect(
      trackGrid.locator(".dashboard-widget[data-widget-type='totals']"),
    ).toHaveClass(/span-2/);
    await expect(
      trackGrid.locator(".dashboard-widget[data-widget-type='recent_listens']"),
    ).toBeVisible();

    // 11. Navigate to a second track and verify identical template layout
    await page.goto("/rankings/tracks?period=all");
    const tracks = page.locator(".rank-name a");
    if ((await tracks.count()) > 1) {
      const secondTrack = tracks.nth(1);
      const secondTrackName = await secondTrack.innerText();
      await secondTrack.click();
      await expect(
        page.getByRole("heading", { name: secondTrackName, exact: false }),
      ).toBeVisible();
      const secondGrid = page.locator(".dashboard-grid");
      await expect(
        secondGrid.locator(".dashboard-widget[data-widget-type='totals']"),
      ).toHaveClass(/span-2/);
      await expect(
        secondGrid.locator(
          ".dashboard-widget[data-widget-type='recent_listens']",
        ),
      ).toBeVisible();
    }

    // 12. Regression check: Home and Artist pages keep their independent layouts
    await page.goto("/dashboard");
    const homeGrid = page.locator(".dashboard-grid");
    await expect(
      homeGrid.locator(".dashboard-widget[data-widget-type='top_artists']"),
    ).toBeVisible();

    await page.goto("/rankings/artists?period=all");
    const artistLink = page.locator(".rank-name a").first();
    await artistLink.click();
    const artistGrid = page.locator(".dashboard-grid");
    await expect(
      artistGrid.locator(".dashboard-widget[data-widget-type='top_albums']"),
    ).toBeVisible();
  });

  test("track template handles dirty cancel, 409 conflict, empty layout, and settings entry", async ({
    archive: { page },
  }) => {
    // 1. Open track page
    await page.goto("/rankings/tracks?period=all");
    const firstTrack = page.locator(".rank-name a").first();
    await firstTrack.click();

    // 2. Click "Personalizza template brano"
    await page
      .getByRole("button", { name: "Personalizza template brano" })
      .click();
    const editToolbar = page.getByRole("toolbar", {
      name: "Personalizza template brano",
    });
    await expect(editToolbar).toBeVisible();

    // Make a change to create dirty draft
    const trackGrid = page.locator(".dashboard-grid");
    const totalsWidget = trackGrid.locator(
      ".dashboard-widget[data-widget-type='totals']",
    );
    await totalsWidget.locator(".widget-span-button").click();

    // 3. Test Dirty confirmation on Cancel
    let dialogMessage = "";
    page.once("dialog", async (dialog) => {
      dialogMessage = dialog.message();
      await dialog.dismiss();
    });
    await editToolbar.getByRole("button", { name: "Annulla" }).click();
    expect(dialogMessage).toContain("modifiche non salvate");
    await expect(editToolbar).toBeVisible();

    page.once("dialog", async (dialog) => {
      await dialog.accept();
    });
    await editToolbar.getByRole("button", { name: "Annulla" }).click();
    await expect(editToolbar).toBeHidden();

    // 4. Test 409 Concurrency Conflict preserving draft
    await page
      .getByRole("button", { name: "Personalizza template brano" })
      .click();
    await expect(editToolbar).toBeVisible();
    await trackGrid
      .locator(".dashboard-widget[data-widget-type='history']")
      .locator(".widget-remove-button")
      .click();
    await expect(
      trackGrid.locator(".dashboard-widget[data-widget-type='history']"),
    ).toHaveCount(0);

    // Mock 409 Conflict from backend
    await page.route(
      "**/api/v1/dashboard/layout?template=track",
      async (route) => {
        if (route.request().method() === "POST") {
          await route.fulfill({
            status: 409,
            json: {
              error:
                "Dashboard layout changed in another session. Reload before saving.",
            },
          });
        } else {
          await route.continue();
        }
      },
    );

    await editToolbar.getByRole("button", { name: "Salva modifiche" }).click();
    await expect(
      editToolbar.getByText("Dashboard layout changed in another session"),
    ).toBeVisible();
    await expect(editToolbar).toBeVisible();
    await expect(
      trackGrid.locator(".dashboard-widget[data-widget-type='history']"),
    ).toHaveCount(0);

    await page.unroute("**/api/v1/dashboard/layout?template=track");

    // 5. Test empty template saving and reset
    page.once("dialog", async (dialog) => {
      await dialog.accept();
    });
    await editToolbar.getByRole("button", { name: "Annulla" }).click();
    await expect(editToolbar).toBeHidden();

    // Re-enter edit mode
    await page
      .getByRole("button", { name: "Personalizza template brano" })
      .click();
    // Remove all remaining widgets
    const removeButtons = trackGrid.locator(".widget-remove-button");
    while ((await removeButtons.count()) > 0) {
      await removeButtons.first().click();
    }
    await expect(trackGrid.locator(".dashboard-widget")).toHaveCount(0);

    await editToolbar.getByRole("button", { name: "Salva modifiche" }).click();
    await expect(editToolbar).toBeHidden();

    // Reload: empty template notice is displayed
    await page.reload();
    await expect(
      page.getByText(
        'Nessun widget configurato per questo template. Clicca su "Aggiungi widget" per personalizzare la pagina.',
      ),
    ).toBeVisible();

    // Restore defaults
    await page
      .getByRole("button", { name: "Personalizza template brano" })
      .click();
    await editToolbar
      .getByRole("button", { name: "Ripristina predefinito" })
      .click();
    await editToolbar.getByRole("button", { name: "Salva modifiche" }).click();
    await expect(editToolbar).toBeHidden();
    await expect(trackGrid.locator(".dashboard-widget")).toHaveCount(4);

    // 6. Test Settings entry point for track template customization
    await page.goto("/settings/appearance");
    await expect(
      page.getByRole("heading", { name: "Template pagine musicali" }),
    ).toBeVisible();

    await chooseOption(
      page.getByRole("combobox", { name: "Tipo di pagina" }),
      "track",
    );
    await expect(
      page.getByRole("combobox", { name: "Brano di anteprima" }),
    ).toBeVisible();
    const settingsCustomizeBtn = page.getByRole("button", {
      name: "Personalizza template brano",
    });
    await expect(settingsCustomizeBtn).toBeVisible();
    await settingsCustomizeBtn.click();

    // Navigates to track page with editor toolbar open
    await expect(
      page.getByRole("toolbar", { name: "Personalizza template brano" }),
    ).toBeVisible();

    // 7. Test empty track library preview from Settings
    await page.route(
      "**/api/v1/consultation/rankings/tracks*",
      async (route) => {
        await route.fulfill({
          status: 200,
          json: { items: [], total: 0, page: 1, page_size: 20 },
        });
      },
    );

    await page.goto("/settings/appearance");
    await chooseOption(
      page.getByRole("combobox", { name: "Tipo di pagina" }),
      "track",
    );
    await expect(
      page.getByText(
        "Nessun brano presente nell'archivio. Verrà usata un'anteprima vuota.",
      ),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "Personalizza template brano" })
      .click();

    await expect(
      page.getByRole("toolbar", { name: "Personalizza template brano" }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Brano di anteprima" }),
    ).toBeVisible();
    await expect(page.locator(".dashboard-grid")).toBeVisible();
  });
  test("music templates support listening_calendar and listening_milestones, with scoped discovery_ratio and home catalog isolation", async ({
    archive: { page },
  }) => {
    // 1. Home template catalog MUST NOT contain listening_calendar or listening_milestones
    await page.goto("/dashboard");
    const homeCustomizeBtn = page.getByRole("button", {
      name: "Personalizza",
    });
    await expect(homeCustomizeBtn).toBeVisible();
    await homeCustomizeBtn.click();

    const homeToolbar = page.getByRole("toolbar", {
      name: "Personalizza dashboard",
    });
    await expect(homeToolbar).toBeVisible();
    await homeToolbar.getByRole("button", { name: "Aggiungi widget" }).click();

    const homeCatalog = page.locator(".widget-catalog-dialog");
    await expect(homeCatalog).toBeVisible();
    await expect(homeCatalog.getByText("Calendario ascolti")).toHaveCount(0);
    await expect(homeCatalog.getByText("Traguardi di ascolto")).toHaveCount(0);
    await homeCatalog.getByRole("button", { name: "Chiudi catalogo" }).click();
    await homeToolbar.getByRole("button", { name: "Annulla" }).click();

    // 2. Artist template: add listening_calendar, listening_milestones, and discovery_ratio
    await page.goto("/rankings/artists?period=all");
    const firstArtist = page.locator(".rank-name a").first();
    await expect(firstArtist).toBeVisible();
    const artistName = await firstArtist.innerText();
    await firstArtist.click();
    await expect(
      page.getByRole("heading", { name: artistName, exact: false }),
    ).toBeVisible();

    const artistGrid = page.locator(".dashboard-grid");
    const artistCustomizeBtn = page.getByRole("button", {
      name: "Personalizza template artista",
    });
    await expect(artistCustomizeBtn).toBeVisible();
    await artistCustomizeBtn.click();

    const artistToolbar = page.getByRole("toolbar", {
      name: "Personalizza template artista",
    });
    await expect(artistToolbar).toBeVisible();

    // Open catalog on Artist template
    await artistToolbar
      .getByRole("button", { name: "Aggiungi widget" })
      .click();
    const artistCatalog = page.locator(".widget-catalog-dialog");
    await expect(artistCatalog).toBeVisible();

    // Verify all 3 advanced widgets are present in Artist catalog
    const addCalendarBtn = artistCatalog.getByRole("button", {
      name: /Calendario ascolti/i,
    });
    await expect(addCalendarBtn).toBeVisible();
    const addMilestonesBtn = artistCatalog.getByRole("button", {
      name: /Traguardi di ascolto/i,
    });
    await expect(addMilestonesBtn).toBeVisible();
    const addDiscoveryBtn = artistCatalog.getByRole("button", {
      name: /Rapporto scoperte/i,
    });
    await expect(addDiscoveryBtn).toBeVisible();

    // Add Calendario ascolti
    await addCalendarBtn.click();
    await expect(artistCatalog).toBeHidden();
    const calendarWidget = artistGrid.locator(
      ".dashboard-widget[data-widget-type='listening_calendar']",
    );
    await expect(calendarWidget).toBeVisible();

    // Verify calendar has period override select in edit mode
    await expect(
      calendarWidget.getByRole("combobox", { name: "Periodo widget" }),
    ).toBeVisible();

    // Add Traguardi di ascolto
    await artistToolbar
      .getByRole("button", { name: "Aggiungi widget" })
      .click();
    await addMilestonesBtn.click();
    await expect(artistCatalog).toBeHidden();
    const milestonesWidget = artistGrid.locator(
      ".dashboard-widget[data-widget-type='listening_milestones']",
    );
    await expect(milestonesWidget).toBeVisible();

    // Verify milestones widget displays 'Tutto lo storico' badge
    await expect(
      milestonesWidget.locator(".widget-period-badge.all-time"),
    ).toHaveText("Tutto lo storico");

    // Verify milestones widget has NO period override selector
    await expect(
      milestonesWidget.getByRole("combobox", { name: "Periodo widget" }),
    ).toHaveCount(0);

    // Add Rapporto scoperte
    await artistToolbar
      .getByRole("button", { name: "Aggiungi widget" })
      .click();
    await addDiscoveryBtn.click();
    await expect(artistCatalog).toBeHidden();
    const discoveryWidget = artistGrid.locator(
      ".dashboard-widget[data-widget-type='discovery_ratio']",
    );
    await expect(discoveryWidget).toBeVisible();

    // Save artist template
    await artistToolbar
      .getByRole("button", { name: "Salva modifiche" })
      .click();
    await expect(artistToolbar).toBeHidden();

    // Reload and assert all 3 widgets persist on Artist page
    await page.reload();
    await expect(
      artistGrid.locator(
        ".dashboard-widget[data-widget-type='listening_calendar']",
      ),
    ).toBeVisible();
    await expect(
      artistGrid.locator(
        ".dashboard-widget[data-widget-type='listening_milestones']",
      ),
    ).toBeVisible();
    await expect(
      artistGrid.locator(
        ".dashboard-widget[data-widget-type='discovery_ratio']",
      ),
    ).toBeVisible();

    // 3. Track template: verify discovery_ratio is EXCLUDED from catalog, while calendar & milestones are available
    await page.goto("/rankings/tracks?period=all");
    const firstTrack = page.locator(".rank-name a").first();
    await expect(firstTrack).toBeVisible();
    await firstTrack.click();

    const trackGrid = page.locator(".dashboard-grid");
    const trackCustomizeBtn = page.getByRole("button", {
      name: "Personalizza template brano",
    });
    await expect(trackCustomizeBtn).toBeVisible();
    await trackCustomizeBtn.click();

    const trackToolbar = page.getByRole("toolbar", {
      name: "Personalizza template brano",
    });
    await expect(trackToolbar).toBeVisible();

    await trackToolbar.getByRole("button", { name: "Aggiungi widget" }).click();
    const trackCatalog = page.locator(".widget-catalog-dialog");
    await expect(trackCatalog).toBeVisible();

    // Discovery ratio MUST NOT be present in Track catalog
    await expect(trackCatalog.getByText("Rapporto scoperte")).toHaveCount(0);

    // Calendar and Milestones MUST be present
    const addTrackCalBtn = trackCatalog.getByRole("button", {
      name: /Calendario ascolti/i,
    });
    await expect(addTrackCalBtn).toBeVisible();
    const addTrackMsBtn = trackCatalog.getByRole("button", {
      name: /Traguardi di ascolto/i,
    });
    await expect(addTrackMsBtn).toBeVisible();

    // Add both to track
    await addTrackCalBtn.click();
    await trackToolbar.getByRole("button", { name: "Aggiungi widget" }).click();
    await addTrackMsBtn.click();

    // Save track template
    await trackToolbar.getByRole("button", { name: "Salva modifiche" }).click();
    await expect(trackToolbar).toBeHidden();

    // Reload and assert widgets persist on Track page
    await page.reload();
    await expect(
      trackGrid.locator(
        ".dashboard-widget[data-widget-type='listening_calendar']",
      ),
    ).toBeVisible();
    await expect(
      trackGrid.locator(
        ".dashboard-widget[data-widget-type='listening_milestones']",
      ),
    ).toBeVisible();
    // Discovery ratio still absent on track
    await expect(
      trackGrid.locator(
        ".dashboard-widget[data-widget-type='discovery_ratio']",
      ),
    ).toHaveCount(0);
  });
});
