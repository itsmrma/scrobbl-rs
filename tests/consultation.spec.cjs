const { test, expect } = require("@playwright/test");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const { mkdtemp, rm } = require("node:fs/promises");
const { tmpdir } = require("node:os");
const { join, resolve } = require("node:path");
const net = require("node:net");

test("consultation navigation, tied pages, persistent views, chart drilldown and accessible time help", async ({
  browser,
  browserName,
}) => {
  const directory = await mkdtemp(join(tmpdir(), "scrobbl-consultation-"));
  const socket = net.createServer();
  socket.listen(0, "127.0.0.1");
  await once(socket, "listening");
  const port = socket.address().port;
  await new Promise((resolve) => socket.close(resolve));
  const child = spawn(
    resolve(
      "target/debug/scrobbl-rs" + (process.platform === "win32" ? ".exe" : ""),
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
      .fill("consultation browser password");
    await page
      .getByLabel("Confirm password")
      .fill("consultation browser password");
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
    await ranking.getByLabel("Results per page").selectOption("10");
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
    await expect(page.getByLabel("Results per page")).toHaveValue("10");
    await page.getByLabel("Period", { exact: true }).selectOption("custom");
    await page.getByLabel("From", { exact: true }).fill("2025-06-02");
    await page.getByLabel("Through", { exact: true }).fill("2025-06-02");
    await expect(ranking).toContainText("No tracks with counted listens");
    await page.goto("/history?period=all");
    await expect(page.locator("#history-total")).toHaveText("135 records");
    await expect(page.getByLabel("Results per page")).toHaveValue("10");
    await page.getByLabel("Show listens below threshold too").check();
    await expect(page.locator("#history-total")).toHaveText("136 records");
    await page.getByLabel("Show listens below threshold too").uncheck();
    await page.getByLabel("Find artist").fill("Consultation track 010");
    await page
      .getByRole("button", {
        name: "Consultation artist · Consultation track 010",
        exact: true,
      })
      .click();
    await page
      .getByRole("combobox", { name: "Source", exact: true })
      .selectOption("spotify_api");
    await expect(page.locator("#history-total")).toHaveText("0 records");
    await page
      .getByRole("combobox", { name: "Source", exact: true })
      .selectOption("spotify_import");
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
    await page.getByLabel("Period", { exact: true }).selectOption("custom");
    await page.getByLabel("From", { exact: true }).fill("2025-06-01");
    await page.getByLabel("Through", { exact: true }).fill("2025-06-03");
    await page.getByLabel("Group by").selectOption("day");
    await expect(page.locator(".chart-segment:visible")).toHaveCount(3);
    await page.locator(".chart-segment:visible").first().click();
    await expect(page.locator("#history-total")).toHaveText("135 records");
    await expect(
      page.getByText("Chart segment:", { exact: false }),
    ).toBeVisible();
    await expect(page.getByLabel("Find artist")).toHaveAttribute(
      "placeholder",
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
});
