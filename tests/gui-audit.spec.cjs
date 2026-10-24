const { chooseOption } = require("./controls.cjs");
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
              password: "a local browser test password 7!",
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
                      platform: "Android app",
                      conn_country: "IT",
                      shuffle: false,
                      offline: false,
                      skipped: false,
                      reason_end: "trackdone",
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
    .fill("a local browser test password 7!");
  if (session.setup_required)
    await page
      .getByLabel("Confirm password")
      .fill("a local browser test password 7!");
  await page
    .getByRole("button", {
      name: session.setup_required ? "Create your archive" : "Sign in",
      exact: true,
    })
    .click();
  await expect(page.locator("#main")).toBeVisible();
}

test("music listening patterns switch within one compact chart and header filters match action widths", async ({
  page,
  browserName,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await login(page);
  await page.route("**/api/v1/library/*/*", async (route) => {
    const response = await route.fetch();
    await route.fulfill({
      json: {
        ...(await response.json()),
        spotify_url: "https://open.spotify.com/track/0000000000000000000088",
      },
    });
  });
  const archive = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  for (const kind of ["artists", "albums", "tracks"]) {
    await page.goto(
      `/music/${kind}/${archive.rankings[kind][0].id}?period=all`,
    );
    const cover = page.locator(".music-heading > .cover");
    await expect(cover).toBeVisible();
    await expect
      .poll(() =>
        cover.evaluate((node) => {
          const main = document.querySelector("#main");
          return Math.abs(
            node.getBoundingClientRect().top -
              main.getBoundingClientRect().top -
              parseFloat(getComputedStyle(main).paddingTop),
          );
        }),
      )
      .toBeLessThan(1);
    await expect(page.locator(".music-personal > a")).toHaveCount(2);
    await expect
      .poll(() =>
        cover.evaluate((node) => {
          const controls = node.parentElement.querySelectorAll(
            ".music-period .select-control",
          );
          const title = node.parentElement.querySelector("h1");
          const inset = parseFloat(getComputedStyle(title).fontSize) / 4;
          return Math.max(
            ...Array.from(controls, (control) =>
              Math.abs(
                node.getBoundingClientRect().bottom -
                  control.getBoundingClientRect().bottom -
                  inset,
              ),
            ),
          );
        }),
      )
      .toBeLessThan(1);
    await expect
      .poll(async () => {
        const buttons = await page
          .locator(".music-personal > a")
          .evaluateAll((nodes) => nodes.map((n) => n.getBoundingClientRect()));
        const controls = await page
          .locator(".music-period .select-control")
          .evaluateAll((nodes) => nodes.map((n) => n.getBoundingClientRect()));
        return Math.max(
          ...buttons.map((b, i) => Math.abs(b.width - controls[i].width)),
          Math.abs(buttons[1].right - controls[1].right),
        );
      })
      .toBeLessThan(1);
    const chart = page.getByRole("article", {
      name: "Listening patterns",
      exact: true,
    });
    await expect(chart).toContainText("1 listens");
    await expect(chart.getByRole("listitem")).toHaveCount(24);
    expect((await chart.boundingBox()).height).toBeLessThan(450);
    await chooseOption(chart.getByRole("combobox"), "devices");
    await expect(chart.getByRole("listitem")).toHaveAccessibleName(
      "Android app: 1 listens",
    );
    await chooseOption(chart.getByRole("combobox"), "shuffle");
    await expect(chart.getByRole("listitem")).toHaveAccessibleName(
      "No: 1 listens",
    );
    await chooseOption(chart.getByRole("combobox"), "weekdays");
    await expect(chart.getByRole("listitem")).toHaveCount(7);
    if (kind === "tracks") {
      await page.evaluate(() => {
        if (document.activeElement instanceof HTMLElement)
          document.activeElement.blur();
        window.scrollTo(0, 0);
      });
      await page.screenshot({
        path: `test-results/gui-listening-patterns-${browserName}.png`,
        fullPage: true,
        animations: "disabled",
      });
    }
    await page.setViewportSize({ width: 390, height: 844 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.setViewportSize({ width: 1440, height: 1000 });
  }
});

test("owner account menu saves a private photo and password and moves sign out from the sidebar", async ({
  page,
  browserName,
}) => {
  await login(page);
  const trigger = page.getByRole("button", {
    name: "Account menu",
    exact: true,
  });
  await expect(
    page.locator(".sidebar-bottom").getByRole("button", { name: "Sign out" }),
  ).toHaveCount(0);
  await trigger.click();
  await expect(
    page.getByRole("navigation", { name: "Account actions" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(trigger).toBeFocused();
  await trigger.click();
  await page.screenshot({
    path: `test-results/gui-account-menu-${browserName}.png`,
    fullPage: true,
    animations: "disabled",
  });
  await page
    .getByRole("link", { name: "Account settings", exact: true })
    .click();
  await expect(page.getByLabel("Username", { exact: true })).toHaveValue(
    "owner",
  );
  await page
    .getByLabel("Display name", { exact: true })
    .fill("My music profile");
  await page.getByRole("button", { name: "Save profile", exact: true }).click();
  await expect(page.getByLabel("Profile photo", { exact: true })).toBeEnabled();
  const largePhoto = Buffer.from(
    await page.evaluate(() => {
      const canvas = document.createElement("canvas");
      canvas.width = 1024;
      canvas.height = 1024;
      const context = canvas.getContext("2d");
      const image = context.createImageData(1024, 1024);
      for (let i = 0; i < image.data.length; i += 65536)
        crypto.getRandomValues(image.data.subarray(i, i + 65536));
      context.putImageData(image, 0, 0);
      return canvas.toDataURL("image/png").split(",")[1];
    }),
    "base64",
  );
  expect(largePhoto.length).toBeGreaterThan(1024 * 1024);
  await page.getByLabel("Profile photo", { exact: true }).setInputFiles({
    name: "profile.png",
    mimeType: "image/png",
    buffer: largePhoto,
  });
  await expect(page.locator(".account-avatar.large img")).toBeVisible();
  await expect(trigger.locator("img")).toBeVisible();
  await expect
    .poll(() => trigger.locator("img").evaluate((image) => image.naturalWidth))
    .toBe(512);
  await page.reload();
  await expect(page.getByLabel("Display name", { exact: true })).toHaveValue(
    "My music profile",
  );
  await expect(trigger.locator("img")).toBeVisible();
  await page.screenshot({
    path: `test-results/gui-account-${browserName}.png`,
    fullPage: true,
    animations: "disabled",
  });
  await page
    .getByLabel("Current password", { exact: true })
    .fill("wrong password");
  await page
    .getByLabel("New password", { exact: true })
    .fill("new personal music pass 9!");
  await page
    .getByLabel("Confirm new password", { exact: true })
    .fill("new personal music pass 9!");
  await page
    .getByRole("button", { name: "Change password", exact: true })
    .click();
  await expect(
    page.getByText("The current password is incorrect.", { exact: true }),
  ).toBeVisible();
  await page
    .getByLabel("Current password", { exact: true })
    .fill("a local browser test password 7!");
  await page
    .getByRole("button", { name: "Change password", exact: true })
    .click();
  await expect(
    page.getByLabel("Current password", { exact: true }),
  ).toHaveValue("");
  await page.getByRole("button", { name: "Remove photo", exact: true }).click();
  await expect(trigger.locator("img")).toHaveCount(0);
  await trigger.click();
  await page.getByRole("button", { name: "Sign out", exact: true }).click();
  await expect(page.getByLabel("Password", { exact: true })).toBeVisible();
  await page.getByLabel("Username", { exact: true }).fill("owner");
  await page
    .getByLabel("Password", { exact: true })
    .fill("new personal music pass 9!");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(trigger).toBeVisible();
});

test("application controls restart the disposable app and stop only its process", async ({
  page,
}) => {
  await login(page, "/settings/application");
  const original = await (await page.request.get("/api/v1/application")).json();
  await page
    .getByRole("button", { name: "Restart application", exact: true })
    .first()
    .click();
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(
    (await (await page.request.get("/api/v1/application")).json()).boot_id,
  ).toBe(original.boot_id);
  await page
    .getByRole("button", { name: "Restart application", exact: true })
    .first()
    .click();
  await page.getByRole("button", { name: "Restart now", exact: true }).click();
  await expect(page.getByRole("dialog")).toContainText(
    "scrobbl.rs has restarted",
    { timeout: 30000 },
  );
  const restarted = await (
    await page.request.get("/api/v1/application")
  ).json();
  expect(restarted.boot_id).not.toBe(original.boot_id);
  expect(
    (await (await page.request.get("/api/dashboard?period=all")).json()).totals
      .listens,
  ).toBe(1);
  await page.getByRole("button", { name: "Close", exact: true }).click();
  await page
    .getByRole("button", { name: "Stop application", exact: true })
    .first()
    .click();
  await page.getByRole("button", { name: "Stop now", exact: true }).click();
  await expect(page.getByRole("dialog")).toContainText("Stop accepted");
  await expect
    .poll(async () => {
      try {
        return (await page.request.get("/healthz", { timeout: 1000 })).ok();
      } catch {
        return false;
      }
    })
    .toBe(false);
});

test("ranking metrics share type size and emphasize the chosen comparison measure", async ({
  page,
  browserName,
}) => {
  await login(page);
  async function verify(byTime) {
    const rows = page.locator(".rank-metrics");
    await expect(rows.first()).toBeVisible();
    for (const row of await rows.all()) {
      await expect(row.locator(".rank-separator")).toHaveText("·");
      await expect(row.locator("strong")).toHaveClass(
        byTime ? "rank-time" : "rank-plays",
      );
      const styles = await row.evaluate((node) => {
        const plays = getComputedStyle(node.querySelector(".rank-plays"));
        const time = getComputedStyle(node.querySelector(".rank-time"));
        return {
          playsSize: plays.fontSize,
          timeSize: time.fontSize,
          playsWeight: Number(plays.fontWeight),
          timeWeight: Number(time.fontWeight),
        };
      });
      expect(styles.playsSize).toBe(styles.timeSize);
      expect(byTime ? styles.timeWeight : styles.playsWeight).toBeGreaterThan(
        byTime ? styles.playsWeight : styles.timeWeight,
      );
    }
  }
  await verify(false);
  await chooseOption(page.getByLabel("Rank by", { exact: true }), "time");
  await verify(true);
  await page.screenshot({
    path: `test-results/gui-rank-metrics-${browserName}.png`,
    fullPage: true,
    animations: "disabled",
  });
  for (const kind of ["artists", "albums", "tracks"]) {
    await page.goto(`/rankings/${kind}?sort=time`);
    await verify(true);
    await page.getByRole("button", { name: "Grid", exact: true }).click();
    await verify(true);
    await chooseOption(page.getByLabel("Rank by", { exact: true }), "plays");
    await verify(false);
  }
});

test("album tracks scroll inside their card without pagination or missing later results", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await login(page);
  const archive = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  let seed;
  await page.route(
    "**/api/v1/consultation/rankings/tracks?*",
    async (route) => {
      const response = await route.fetch();
      const data = await response.json();
      const pageNumber = Number(
        new URL(route.request().url()).searchParams.get("page"),
      );
      seed ??= data.items[0];
      await route.fulfill({
        json: {
          ...data,
          total: 40,
          page: pageNumber,
          page_size: 20,
          items: Array.from({ length: 20 }, (_, i) => ({
            ...seed,
            id: `album-track-${(pageNumber - 1) * 20 + i}`,
            name: `Album track ${(pageNumber - 1) * 20 + i + 1}`,
            position: (pageNumber - 1) * 20 + i + 1,
          })),
        },
      });
    },
  );
  await page.goto(
    `/music/albums/${archive.rankings.albums[0].id}?tracks_page=2`,
  );
  const card = page.getByRole("article", { name: "tracks rankings" });
  await expect(card.locator(".ranking-row")).toHaveCount(40);
  await expect(
    card.getByRole("button", { name: "Next", exact: true }),
  ).toHaveCount(0);
  await expect(card.getByText("Results per page", { exact: true })).toHaveCount(
    0,
  );
  for (const view of ["List", "Grid"]) {
    await card.getByRole("button", { name: view, exact: true }).click();
    const results = card.locator(".ranking-results");
    await results.scrollIntoViewIfNeeded();
    await expect
      .poll(() =>
        results.evaluate((node) => node.scrollHeight > node.clientHeight),
      )
      .toBe(true);
    await results.evaluate((node) => {
      node.scrollTop = node.scrollHeight;
    });
    await expect(
      card.getByText("Album track 40", { exact: true }),
    ).toBeInViewport();
  }
});

test("review controls share a row and automatic review collapses with aligned animated content", async ({
  page,
  browserName,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await login(page, "/review");
  const automatic = page.getByRole("region", {
    name: "Automatic duplicate review",
  });
  const summary = automatic.locator(":scope > summary");
  await expect(automatic).not.toHaveAttribute("open");
  await expect(summary).toContainText("0 matching pairs");
  const collapsed = await automatic.boundingBox();
  expect(collapsed.height).toBeLessThan(80);
  const views = await page
    .getByRole("group", { name: "Review views" })
    .boundingBox();
  const refresh = await page
    .getByRole("button", { name: "Refresh review", exact: true })
    .boundingBox();
  expect(
    Math.abs(views.y + views.height / 2 - refresh.y - refresh.height / 2),
  ).toBeLessThan(1);
  const timings = await summary.evaluate((node) => {
    node.click();
    return node.parentElement
      .getAnimations()
      .map((animation) => animation.effect.getTiming().duration);
  });
  expect(timings).toContain(160);
  await expect(
    automatic.locator(".automatic-review-body > p").first(),
  ).toBeVisible();
  await expect
    .poll(async () => (await automatic.boundingBox()).height)
    .toBeGreaterThan(collapsed.height + 80);
  const positions = await automatic.evaluate((node) =>
    [
      node.querySelector("summary h2"),
      node.querySelector(".automatic-review-body > p"),
      node.querySelector(".review-actions [role=status]"),
    ].map((element) => element.getBoundingClientRect().x),
  );
  expect(Math.max(...positions) - Math.min(...positions)).toBeLessThan(1);
  await summary.click();
  await expect(automatic).not.toHaveAttribute("open");
  await page.screenshot({
    path: `test-results/gui-review-compact-${browserName}.png`,
    fullPage: true,
    animations: "disabled",
  });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await summary.focus();
  await page.keyboard.press("Enter");
  await expect(automatic).toHaveAttribute("open");
  expect(await automatic.evaluate((node) => node.getAnimations().length)).toBe(
    0,
  );
  await page.keyboard.press("Enter");
  await expect(automatic).not.toHaveAttribute("open");
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: `test-results/gui-review-compact-mobile-${browserName}.png`,
    fullPage: true,
    animations: "disabled",
  });
});

test("music page back preserves source filters across child filters reload and forward navigation", async ({
  page,
  browserName,
}) => {
  await login(page);
  const archive = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  const artistURL = `/music/artists/${archive.rankings.artists[0].id}?period=all&sort=time`;
  await page.goto(artistURL);
  await page
    .getByRole("article", { name: "albums rankings" })
    .getByRole("link", { name: "Seed album", exact: true })
    .click();
  await expect(page).toHaveURL(/\/music\/albums\//);
  const backBox = await page
    .getByRole("button", { name: "Back", exact: true })
    .boundingBox();
  const coverBox = await page.locator(".music-heading > .cover").boundingBox();
  const headingBox = await page.locator(".music-heading").boundingBox();
  expect(Math.abs(backBox.y - coverBox.y)).toBeLessThan(1);
  expect(
    Math.abs(backBox.x + backBox.width - headingBox.x - headingBox.width),
  ).toBeLessThan(1);
  await page.screenshot({
    path: `test-results/gui-music-back-right-${browserName}.png`,
    fullPage: true,
    animations: "disabled",
  });
  const albumURL = page.url();
  await chooseOption(
    page.locator(".music-period [role=combobox]").first(),
    "year",
  );
  await page.reload();
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await expect(page).toHaveURL(
    new RegExp(artistURL.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + "$"),
  );
  await expect(page.locator(".music-heading h1")).toContainText("Seed artist");
  await page.goForward();
  await expect(page).toHaveURL(albumURL);
  await page
    .getByRole("article", { name: "tracks rankings" })
    .getByRole("link", { name: "Seed song", exact: true })
    .click();
  await expect(page).toHaveURL(/\/music\/tracks\//);
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await expect(page).toHaveURL(albumURL);
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await expect(page.locator(".music-heading h1")).toContainText("Seed artist");
  await page.goto(`/music/albums/${archive.rankings.albums[0].id}?period=all`);
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await expect(page).toHaveURL(/\/rankings\/albums\?period=all$/);
});

test("report and duplicate review links use the shared action button style", async ({
  page,
}) => {
  await login(page, "/history?period=all");
  const review = page.getByRole("link", {
    name: "Review possible duplicates",
    exact: true,
  });
  await expect(review).toHaveClass(/button secondary/);
  const status = await (await page.request.get("/api/spotify/status")).json();
  await page.route("**/api/spotify/status", (route) =>
    route.fulfill({
      json: { ...status, configured: true, connected: true, review_count: 470 },
    }),
  );
  await page.goto("/settings/connections");
  await page.locator(".spotify-card > summary").click();
  const reports = page.getByRole("link", {
    name: "Collection and import reports",
    exact: true,
  });
  const monitor = page.locator(".connection-monitor").filter({ has: reports });
  if ((await monitor.getAttribute("open")) === null)
    await monitor.locator(":scope > summary").click();
  for (const link of [reports, review]) {
    await expect(link).toBeVisible();
    await expect(link).toHaveClass(/button secondary/);
    await expect(link).toHaveCSS("text-decoration-line", "none");
  }
});

test("custom dropdowns support keyboard and mobile categories while grid controls stay stable", async ({
  page,
  browserName,
}) => {
  await login(page, "/settings/appearance");
  const theme = page.getByRole("combobox", { name: "Theme", exact: true });
  await theme.focus();
  await theme.press("Enter");
  await expect(
    page.getByRole("listbox", { name: "Theme options" }),
  ).toBeVisible();
  await theme.press("End");
  await theme.press("Enter");
  await expect(theme).toHaveAttribute("data-value", "dark");
  await theme.press("Enter");
  await theme.press("Home");
  await theme.press("Escape");
  await expect(theme).toHaveAttribute("data-value", "dark");
  await expect(theme).toBeFocused();
  await theme.press("Enter");
  await page.getByRole("heading", { name: "Appearance", exact: true }).click();
  await expect(theme).toHaveAttribute("aria-expanded", "false");
  expect(await page.locator("select").count()).toBe(0);
  await page.setViewportSize({ width: 390, height: 844 });
  const category = page.getByRole("combobox", { name: "Settings section" });
  await expect(category).toBeVisible();
  await expect(
    page.getByRole("navigation", { name: "Settings sections" }),
  ).not.toBeVisible();
  await chooseOption(category, "connections");
  await expect(page).toHaveURL(/settings\/connections/);
  await page.screenshot({
    path: `test-results/gui-ninth-settings-mobile-${browserName}.png`,
    fullPage: true,
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/rankings/albums?period=all");
  const panel = page.locator(".rankings-panel").first();
  const toolbar = page.locator(".ranking-toolbar").first();
  const before = await toolbar.boundingBox();
  await page.getByRole("button", { name: "Grid", exact: true }).first().click();
  const after = await toolbar.boundingBox();
  expect(Math.abs(before.height - after.height)).toBeLessThan(1);
  const search = toolbar.getByRole("textbox", { name: "Search rankings" });
  await expect(search).toBeVisible();
  const arrow = page.getByRole("button", { name: /Collapse sidebar/ });
  if (await arrow.count()) {
    const icon = arrow.locator(".material-icons-outlined"),
      initial = await icon.boundingBox();
    await arrow.hover();
    await expect
      .poll(async () => Math.abs((await icon.boundingBox()).x - initial.x))
      .toBeLessThan(0.1);
  }
  await page.screenshot({
    path: `test-results/gui-ninth-grid-${browserName}.png`,
    fullPage: true,
  });
});

test("custom categories and long option lists respond to touch without leaving the viewport", async ({
  page,
  browser,
  browserName,
  auditApp,
}) => {
  await login(page, "/settings/general");
  const touchContext = await browser.newContext({
    hasTouch: true,
    viewport: { width: 390, height: 640 },
    baseURL: auditApp,
    storageState: await page.context().storageState(),
  });
  try {
    const touch = await touchContext.newPage();
    await touch.goto("/settings/general");
    const category = touch.getByRole("combobox", {
      name: "Settings section",
      exact: true,
    });
    await category.tap();
    const options = touch.getByRole("listbox", {
      name: "Settings section options",
      exact: true,
    });
    await expect(options).toBeVisible();
    await touch.screenshot({
      path: `test-results/gui-ninth-dropdown-touch-${browserName}.png`,
      fullPage: false,
    });
    await options
      .getByRole("option", { name: "Appearance", exact: true })
      .tap();
    await expect(touch).toHaveURL(/settings\/appearance/);
    await expect(category).toHaveAttribute("data-value", "appearance");
    await category.tap();
    await touch.getByRole("heading", { name: "Settings & connections." }).tap();
    await expect(category).toHaveAttribute("aria-expanded", "false");
    await touch.goto("/settings/general");
    const zone = touch.getByRole("combobox", {
      name: "Time zone",
      exact: true,
    });
    await zone.fill("Europe/");
    await touch
      .getByRole("listbox", { name: "Time zone options", exact: true })
      .getByRole("option", { name: "Europe/Rome", exact: true })
      .tap();
    await expect(zone).toHaveValue("Europe/Rome");
    await touch.goto("/dashboard?period=all");
    const period = touch.getByRole("combobox", { name: "Period", exact: true });
    await period.tap();
    const list = touch.getByRole("listbox", {
      name: "Period options",
      exact: true,
    });
    const bounds = await list.boundingBox();
    expect(bounds.x).toBeGreaterThanOrEqual(0);
    expect(bounds.y).toBeGreaterThanOrEqual(0);
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(390.01);
    expect(bounds.y + bounds.height).toBeLessThanOrEqual(640.01);
    await list.getByRole("option", { name: "Custom dates", exact: true }).tap();
    await expect(period).toHaveAttribute("data-value", "custom");
    expect(
      await touch.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
  } finally {
    await touchContext.close();
  }
});

test("imports queue all missing artwork on the server with persistent progress and controls", async ({
  page,
  browser,
  auditApp,
  browserName,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await login(page, "/activities");
  let job;
  await expect
    .poll(async () => {
      const jobs = await (await page.request.get("/api/v1/activities")).json();
      job = jobs.find((item) => item.kind === "artwork");
      return job?.state;
    })
    .toBe("waiting");
  expect(job.expected_rows).toBe(2);
  await page.goto(`/activities/${job.id}`);
  await expect(
    page.getByRole("heading", {
      name: "Download library artwork",
      exact: true,
    }),
  ).toBeVisible();
  await expect(page.locator(".activity-detail")).toContainText(
    "2 images checked",
  );
  await expect(page.locator(".activity-detail")).toContainText(
    "Album · Seed album",
  );
  await expect(page.locator(".activity-detail")).toContainText(
    "Artist · Seed artist",
  );
  await expect(
    page.getByRole("button", { name: "Cancel downloads", exact: true }),
  ).toBeVisible();
  await page.screenshot({
    path: `test-results/gui-ninth-artwork-queue-${browserName}.png`,
    fullPage: true,
    animations: "disabled",
  });
  await page
    .getByRole("button", { name: "Cancel downloads", exact: true })
    .click();
  await expect(page.locator(".activity-detail .state-badge")).toContainText(
    "Cancelled",
  );
  await page
    .getByRole("button", { name: "Resume downloads", exact: true })
    .click();
  const state = await page.context().storageState();
  await page.close();
  const reopened = await browser.newContext({
    baseURL: auditApp,
    storageState: state,
  });
  try {
    const resumed = await reopened.newPage();
    await resumed.goto(`/activities/${job.id}`);
    await expect(resumed.locator(".activity-detail")).toContainText(
      "0 of 2 images checked",
    );
    expect(
      (await (await resumed.request.get(`/api/v1/activities/${job.id}`)).json())
        .members,
    ).toHaveLength(2);
  } finally {
    await reopened.close();
  }
});

test("activity automatically fits dense daily and weekly data without horizontal scrolling", async ({
  page,
  browserName,
}) => {
  let grain = "day";
  const make = () =>
    Array.from({ length: 160 }, (_, i) => {
      const start = new Date(
        Date.UTC(2020, 0, 1 + i * (grain === "week" ? 7 : 1)),
      )
        .toISOString()
        .slice(0, 10);
      return {
        start,
        through: start,
        listens: (i % 7) + 1,
        actual_ms: ((i % 7) + 1) * 60000,
        estimated_ms: 0,
        unknown_durations: 0,
        partial: false,
      };
    });
  await page.route("**/api/v1/consultation/activity?*", (route) => {
    expect(new URL(route.request().url()).searchParams.has("grain")).toBe(
      false,
    );
    return route.fulfill({
      json: {
        grain,
        buckets: make(),
        mixed_week_starts: false,
        partial: false,
      },
    });
  });
  await login(page, "/dashboard?period=all&grain=day");
  await expect(page.getByRole("combobox", { name: "Group by" })).toHaveCount(0);
  for (const kind of ["day", "week"]) {
    grain = kind;
    await page.goto(`/dashboard?period=all&grain=${kind}`);
    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: 900 });
      const bars = page.locator(".chart-segment");
      await expect.poll(() => bars.count()).toBeGreaterThan(0);
      await expect
        .poll(() => bars.count())
        .toBeLessThanOrEqual(width === 390 ? 5 : 18);
      expect(
        await page
          .locator(".chart-scroll")
          .evaluate((el) => el.scrollWidth - el.clientWidth),
      ).toBeLessThanOrEqual(1);
      const total = await page
        .locator(".chart-bar-value")
        .evaluateAll((nodes) =>
          nodes.reduce((sum, node) => sum + Number(node.textContent), 0),
        );
      expect(total).toBe(make().reduce((sum, b) => sum + b.listens, 0));
      if (kind === "week")
        await expect(bars.first().locator("small").first()).toContainText(
          /202[0-3]/,
        );
    }
  }
  await page.screenshot({
    path: `test-results/gui-ninth-auto-chart-mobile-${browserName}.png`,
    fullPage: true,
  });
});

test("duplicate indicators preserve history density and the primary divider stays anchored", async ({
  page,
}) => {
  await page.route("**/api/history?*", async (route) => {
    const data = await (await route.fetch()).json();
    if (data.items.length)
      data.items = [
        data.items[0],
        { ...data.items[0], id: "duplicate-copy", needs_review: true },
      ];
    return route.fulfill({ json: data });
  });
  await login(page, "/history?period=all");
  const rows = page.locator(".listen-row");
  await expect(rows).toHaveCount(2);
  await expect
    .poll(async () =>
      Math.abs(
        (await rows.nth(0).boundingBox()).height -
          (await rows.nth(1).boundingBox()).height,
      ),
    )
    .toBeLessThan(1.01);
  const warning = rows
    .nth(1)
    .getByRole("link", { name: "Possible duplicate — review record" });
  await warning.hover();
  await expect(
    page.getByRole("tooltip", { name: "Possible duplicate" }),
  ).toBeVisible();
  await page.screenshot({
    path: "test-results/gui-ninth-history-warning.png",
    fullPage: true,
  });
  const archive = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  await page.goto("/music/albums/" + archive.rankings.albums[0].id);
  const row = page.locator(".listen-context-albums").first();
  const divider = () =>
    row.evaluate((el) => {
      const main = el.querySelector(".listen-main");
      return (main || el).getBoundingClientRect().bottom + (main ? 10 : 0);
    });
  await row.scrollIntoViewIfNeeded();
  const before = await divider();
  await row.locator("summary").click();
  await expect(row.locator("dl")).toBeVisible();
  await expect
    .poll(async () => Math.abs((await divider()) - before))
    .toBeLessThan(1);
});

test("history artwork and type follow metadata height and shrink again", async ({
  page,
}) => {
  await login(page, "/history?period=all");
  const row = page.locator(".listen-row").first();
  const sizes = () =>
    row.evaluate((el) => ({
      artwork: el.querySelector(".cover").getBoundingClientRect().width,
      type: parseFloat(
        getComputedStyle(el.querySelector(".listen-heading > a")).fontSize,
      ),
    }));
  const compact = await sizes();
  await row.locator(".listen-date").evaluate((el) => {
    for (let i = 0; i < 3; i++) {
      const line = document.createElement("small");
      line.className = "extra-evidence";
      line.textContent = "Possible duplicate";
      el.append(line);
    }
  });
  await expect
    .poll(async () => (await sizes()).artwork)
    .toBeGreaterThan(compact.artwork + 8);
  expect((await sizes()).type).toBeGreaterThan(compact.type);
  await row
    .locator(".extra-evidence")
    .evaluateAll((elements) => elements.forEach((el) => el.remove()));
  await expect
    .poll(async () => (await sizes()).artwork)
    .toBeCloseTo(compact.artwork, 0);
  await expect(page.getByText("Private archive", { exact: true })).toHaveCount(
    0,
  );
});

test("monthly charts compact quieter ranges without losing listens or drilldown", async ({
  page,
  browserName,
}) => {
  const buckets = Array.from({ length: 84 }, (_, i) => {
    const start = new Date(Date.UTC(2019, i, 1)).toISOString().slice(0, 10);
    const through = new Date(Date.UTC(2019, i + 1, 0))
      .toISOString()
      .slice(0, 10);
    const listens = i % 13 === 0 ? 0 : i === 60 ? 600 : i % 7 === 0 ? 90 : 4;
    return {
      start,
      through,
      listens,
      actual_ms: listens * 60000,
      estimated_ms: 0,
      unknown_durations: 0,
      partial: false,
    };
  });
  await page.route("**/api/v1/consultation/activity?*", (route) =>
    route.fulfill({
      json: {
        grain: "month",
        buckets,
        mixed_week_starts: false,
        partial: false,
      },
    }),
  );
  await page.setViewportSize({ width: 1440, height: 1000 });
  await login(page, "/dashboard?period=all&grain=month");
  const bars = page.locator(".chart-segment");
  await expect.poll(() => bars.count()).toBeLessThanOrEqual(18);
  await expect.poll(() => bars.count()).toBeGreaterThan(0);
  const total = await bars.evaluateAll((elements) =>
    elements.reduce(
      (sum, el) =>
        sum +
        Number(
          el.querySelector(".chart-bar-value").textContent.replaceAll(",", ""),
        ),
      0,
    ),
  );
  expect(total).toBe(buckets.reduce((sum, b) => sum + b.listens, 0));
  await expect(
    page.getByRole("link", {
      name: "2024-01-01 – 2024-01-31: 600 listens",
      exact: true,
    }),
  ).toBeVisible();
  expect(
    await page
      .locator(".chart-scroll")
      .evaluate((el) => el.scrollWidth - el.clientWidth),
  ).toBeLessThanOrEqual(1);
  await page.screenshot({
    path: `test-results/gui-eighth-chart-${browserName}.png`,
    fullPage: true,
    animations: "disabled",
  });
  const grouped = page.locator('.chart-segment[href*="period=custom"]').first();
  const href = new URL(await grouped.getAttribute("href"), page.url());
  const from = href.searchParams.get("from"),
    to = href.searchParams.get("to");
  await grouped.click();
  await expect(page).toHaveURL(new RegExp("from=" + from + ".*to=" + to));
  await page.goto("/dashboard?period=all&grain=month");
  await page.setViewportSize({ width: 390, height: 844 });
  await expect.poll(() => bars.count()).toBeLessThanOrEqual(5);
  expect(
    await page
      .locator(".chart-scroll")
      .evaluate((el) => el.scrollWidth > el.clientWidth + 1),
  ).toBe(false);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
});

test("completed Spotify collection runs collapse into dated reports that fill the page", async ({
  page,
  browserName,
}) => {
  const jobs = Array.from({ length: 40 }, (_, i) => ({
    id: `collection-${i}`,
    kind: "spotify_collection",
    state: "completed",
    filename: "Spotify background collection",
    created_at_ms: Date.UTC(2026, 9, 23, 12, i),
    attempt: 1,
    processed: 2,
    added: 1,
    reconciled: 0,
    duplicates: 1,
    invalid: 0,
    skipped: 0,
    ambiguous: 0,
    retry_at_ms: null,
    error_code: null,
    expected_rows: null,
    phase: "finished",
    outcome: "complete",
    inventory_complete: true,
    originals: 1,
    variants: 0,
    below_threshold: 0,
    target_timezone: null,
    target_week_start: null,
  }));
  await page.route("**/api/v1/activities", (route) =>
    route.fulfill({ json: jobs }),
  );
  await page.route("**/api/v1/activities/collection-*", (route) =>
    route.fulfill({
      json: {
        activity: jobs[0],
        issues: [],
        members: [],
        source_parameters: null,
      },
    }),
  );
  await page.setViewportSize({ width: 1680, height: 1000 });
  await login(page, "/activities");
  await expect(page.locator(".collection-runs")).toContainText(
    "40 completed runs",
  );
  await expect(page.locator(".collection-run").first()).toBeHidden();
  await expect(page.locator(".activity-row")).toHaveCount(0);
  await page.locator(".collection-runs > summary").click();
  await expect(page.locator(".collection-run").first()).toContainText(
    "23 Oct 2026",
  );
  await page.screenshot({
    path: `test-results/gui-eighth-activities-${browserName}.png`,
    fullPage: true,
    animations: "disabled",
  });
  await page.locator(".collection-run").first().click();
  await expect(page.locator(".activity-detail")).toBeVisible();
  const widths = await page
    .locator(".activity-detail")
    .evaluate((el) => [
      el.getBoundingClientRect().width,
      el.closest("main").clientWidth,
    ]);
  expect(widths[1] - widths[0]).toBeLessThan(4);
  await page.screenshot({
    path: `test-results/gui-eighth-report-${browserName}.png`,
    fullPage: true,
    animations: "disabled",
  });
});

test("duplicate review supports an explicit choice, trash and undo from actual source imports", async ({
  page,
  browserName,
}) => {
  await login(page, "/import");
  const seedListen = (
    await (await page.request.get("/api/history?period=all&status=all")).json()
  ).items[0];
  await page
    .getByLabel("ListenBrainz username for files (optional)")
    .fill("review-sample");
  await page.getByLabel("ListenBrainz history files").setInputFiles({
    name: "review.jsonl",
    mimeType: "application/json",
    buffer: Buffer.from(
      JSON.stringify({
        listened_at: Math.floor(seedListen.played_at_ms / 1000),
        track_metadata: {
          artist_name: "Seed artist",
          track_name: "Seed song",
          release_name: "Seed album",
        },
      }),
    ),
  });
  await page
    .getByRole("button", { name: "Upload ListenBrainz files", exact: true })
    .click();
  await expect
    .poll(
      async () =>
        (await (await page.request.get("/api/v1/review")).json()).total,
    )
    .toBe(2);
  await page.goto("/history?status=review&period=all");
  await page
    .getByRole("link", { name: "Review possible duplicates", exact: true })
    .click();
  await expect(page.locator(".review-record")).toHaveCount(2);
  await expect(
    page.getByRole("button", { name: /Keep selected/ }),
  ).toBeDisabled();
  await page
    .getByRole("radio", { name: "Keep spotify_import record", exact: true })
    .check();
  await page.screenshot({
    path: `test-results/gui-eighth-duplicates-${browserName}.png`,
    fullPage: true,
    animations: "disabled",
  });
  await page.getByRole("button", { name: /Keep selected/ }).click();
  await expect(
    page.getByRole("heading", { name: "All caught up" }),
  ).toBeVisible();
  expect(
    (
      await (
        await page.request.get("/api/history?period=all&status=all")
      ).json()
    ).total,
  ).toBe(1);
  await page.getByRole("button", { name: /Trash/ }).click();
  await expect(
    page.getByRole("heading", { name: "Trash", exact: true }),
  ).toBeVisible();
  await expect(page.locator(".trash-record")).toHaveCount(1);
  await expect(page.locator(".trash-record")).toContainText("Seed song");
  await page.screenshot({
    path: `test-results/gui-trash-${browserName}.png`,
    fullPage: true,
    animations: "disabled",
  });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: `test-results/gui-trash-mobile-${browserName}.png`,
    fullPage: true,
    animations: "disabled",
  });
  await page.reload();
  await page.getByRole("button", { name: /Trash/ }).click();
  await page.getByRole("button", { name: /Restore decision/ }).click();
  await expect(page.getByText("Trash is empty", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: /Comparisons/ }).click();
  await expect(page.locator(".review-record")).toHaveCount(2);
  await page
    .getByRole("radio", { name: "Keep spotify_import record", exact: true })
    .check();
  await page.getByRole("button", { name: /Keep selected/ }).click();
  await page.getByText("Review decisions · Undo", { exact: true }).click();
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  await expect(page.locator(".review-record")).toHaveCount(2);
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: `test-results/gui-eighth-duplicates-mobile-${browserName}.png`,
    fullPage: true,
    animations: "disabled",
  });
  const automatic = page.getByRole("region", {
    name: "Automatic duplicate review",
  });
  await automatic.locator(":scope > summary").click();
  await expect(automatic).toContainText("1 matching pair");
  await automatic
    .getByRole("button", { name: "Resolve 1 pair automatically", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "All caught up" }),
  ).toBeVisible();
  await expect(automatic.getByRole("button")).toBeDisabled();
  await page.getByText("Review decisions · Undo", { exact: true }).click();
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  await expect(page.locator(".review-record")).toHaveCount(2);
  await page
    .getByRole("button", { name: "Keep all as separate listens", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "All caught up" }),
  ).toBeVisible();
  expect(
    (
      await (
        await page.request.get("/api/history?period=all&status=all")
      ).json()
    ).total,
  ).toBe(2);
});

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
    const fourthStar = page.locator(".star-button").nth(3);
    await fourthStar.click({ position: { x: 8, y: 20 } });
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
      await expect(page.getByLabel("Rating in stars")).toHaveText("3.5 / 5");
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
    await expect(page.getByLabel("Rating in stars")).toHaveText("3.5 / 5");
    await page.reload();
    await expect(page.getByLabel("Rating in stars")).toHaveText("3.5 / 5");
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
    "/settings/account",
    "/settings/application",
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

test("sidebar retains one playback sample while overview omits the duplicate", async ({
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
  await expect(page.locator(".now-panel")).toHaveCount(0);
  expect(requests).toBe(1);
  await page.getByRole("link", { name: "Import history", exact: true }).click();
  await expect(page.locator(".sidebar-now")).toContainText("Shared playback");
  await page.getByRole("link", { name: "Overview", exact: true }).click();
  await expect(page.locator(".now-panel")).toHaveCount(0);
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
  const identity = await page.locator(".connection-identity").boundingBox();
  const badge = await page.locator("#connection-badge").boundingBox();
  expect(
    Math.abs(identity.y + identity.height / 2 - badge.y - badge.height / 2),
  ).toBeLessThanOrEqual(1);
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

test("music charts start independently and back navigation restores the visited page while refreshing", async ({
  page,
}) => {
  await login(page);
  const archive = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  const album = archive.rankings.albums[0];
  let release;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  let hold = true;
  await page.route("**/api/v1/consultation/music/albums/*", async (route) => {
    if (hold) await gate;
    await route.continue();
  });
  try {
    await page.goto(`/music/albums/${album.id}`);
    await expect(page.locator(".chart-bars")).toBeVisible({ timeout: 1000 });
  } finally {
    hold = false;
    release();
  }
  await expect(page.locator(".music-heading h1")).toContainText(album.name);
  await page.locator(".music-heading .page-heading a").first().click();
  await expect(page.locator(".music-heading h1")).toContainText("Seed artist");
  await page.unroute("**/api/v1/consultation/music/albums/*");
  await page.route("**/api/v1/consultation/music/albums/*", async (route) => {
    await new Promise((resolve) => setTimeout(resolve, 1500));
    await route.continue();
  });
  await page.goBack();
  await expect(page.locator(".music-heading h1")).toContainText(album.name, {
    timeout: 500,
  });
  await expect(page.locator(".chart-bars")).toBeVisible({ timeout: 500 });
});

test("history filters collapse without clearing the selection and music history keeps listen details", async ({
  page,
  browserName,
}) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await page.addInitScript(() =>
    localStorage.setItem("scrobbl:accent", JSON.stringify("#c5a3ee")),
  );
  await login(page, "/history?period=all&source=spotify_import");
  const titleBox = await page
    .getByRole("heading", { name: "Listening history." })
    .boundingBox();
  for (const control of [
    page.getByRole("button", { name: "Filters", exact: true }),
    page.getByRole("link", { name: "Review possible duplicates", exact: true }),
  ]) {
    const box = await control.boundingBox();
    expect(box.y).toBeGreaterThanOrEqual(titleBox.y);
    expect(box.y).toBeLessThan(titleBox.y + titleBox.height);
  }
  await page.screenshot({
    path: `test-results/gui-history-heading-actions-${browserName}.png`,
    fullPage: true,
    animations: "disabled",
  });
  const disclosure = page.locator(".history-filter-disclosure");
  await expect(disclosure).not.toHaveAttribute("open");
  await page.getByRole("button", { name: "Filters", exact: true }).click();
  await expect(page.getByLabel("Source", { exact: true })).toHaveAttribute(
    "data-value",
    "spotify_import",
  );
  await page.getByRole("button", { name: "Filters", exact: true }).click();
  await expect(page.getByLabel("Source", { exact: true })).toBeHidden();
  await expect(page).toHaveURL(/source=spotify_import/);
  await page.reload();
  await expect(disclosure).not.toHaveAttribute("open");
  const archive = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  for (const kind of ["tracks", "albums", "artists"]) {
    const item = archive.rankings[kind][0];
    await page.goto(`/music/${kind}/${item.id}?period=all`);
    await expect(page.locator(".music-heading h1")).toContainText(item.name);
    await expect(page.locator(".listen-row").first()).toBeVisible();
    if (kind !== "artists") {
      const row = page.locator(`.listen-context-${kind}`).first();
      await expect(row.locator(".listen-album")).toHaveCount(0);
      await expect(row.locator(".listen-title")).toHaveCount(
        kind === "tracks" ? 0 : 1,
      );
      if (kind === "albums")
        await expect(row.locator(".listen-title .listen-artists")).toHaveCount(
          0,
        );
      await row.locator("summary").click();
      await expect(row).toContainText("Playback timestamp (UTC)");
      await expect(row).toContainText("Archived on");
      await expect(row).toContainText("Counted listen");
    }
    for (const control of await page
      .locator("#main [role=combobox]:visible")
      .all()) {
      // Gecko can report 43.99988 for a 44 CSS-pixel control. Allow only
      // subpixel layout precision, not a smaller interaction target.
      expect((await control.boundingBox()).height).toBeGreaterThanOrEqual(
        44 - 0.01,
      );
    }
    const cover = await page.locator(".music-heading > .cover").boundingBox();
    expect(cover.width).toBeGreaterThanOrEqual(160);
    await page.screenshot({
      path: `test-results/gui-feedback-${kind}-${browserName}.png`,
      fullPage: true,
      animations: "disabled",
    });
  }
  await page.goto("/import");
  await expect(
    page.getByRole("heading", { name: "Import your export" }),
  ).toBeVisible();
  await expect(page.getByLabel("Public ListenBrainz username")).toBeHidden();
  await page.screenshot({
    path: `test-results/gui-feedback-listenbrainz-upload-${browserName}.png`,
    fullPage: true,
    animations: "disabled",
  });
  await page
    .getByRole("button", { name: "Fetch public listens", exact: true })
    .click();
  await page.getByLabel("Public ListenBrainz username").fill("public-listener");
  await page.getByRole("button", { name: "Import files", exact: true }).click();
  await page
    .getByRole("button", { name: "Fetch public listens", exact: true })
    .click();
  await expect(page.getByLabel("Public ListenBrainz username")).toHaveValue(
    "public-listener",
  );
  await page.screenshot({
    path: `test-results/gui-feedback-listenbrainz-${browserName}.png`,
    fullPage: true,
    animations: "disabled",
  });
  await page.goto("/activities");
  await page.locator(".activity-row-heading a").first().click();
  const back = page.getByRole("link", { name: "All activities", exact: true });
  await expect(back).toBeVisible();
  expect((await back.boundingBox()).height).toBeGreaterThanOrEqual(44);
  await page.screenshot({
    path: `test-results/gui-feedback-activity-detail-${browserName}.png`,
    fullPage: true,
    animations: "disabled",
  });
  await back.click();
  await expect(page.locator(".activity-row").first()).toBeVisible();
});

test("visible artwork waits in the client while music data remains available", async ({
  page,
}) => {
  let release;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  let requests = 0;
  await page.route("**/api/v1/artwork/**", async (route) => {
    if (new URL(route.request().url()).searchParams.has("cached"))
      return route.fulfill({ status: 404, body: "" });
    requests++;
    await gate;
    await route.fulfill({ status: 404, body: "" });
  });
  try {
    await login(page);
    await expect(page.locator(".chart-bars")).toBeVisible();
    await page.locator(".rankings-grid").scrollIntoViewIfNeeded();
    await expect.poll(() => requests).toBeGreaterThan(0);
    await page.waitForTimeout(150);
    expect(
      requests,
      "cold images must not occupy all browser HTTP connections",
    ).toBe(1);
  } finally {
    release();
  }
  await page.getByRole("link", { name: "Import history", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Import your export" }),
  ).toBeVisible();
});

test("artist autocomplete matches artist names rather than track titles", async ({
  page,
}) => {
  await login(page, "/history?period=all");
  const response = await page.request.get(
    "/api/v1/consultation/choices/artists?q=song",
  );
  expect(await response.json()).toEqual([]);
});

test("selected filters and source-entry checkboxes stay compact", async ({
  page,
}) => {
  await login(page, "/history?period=all");
  await page.getByRole("button", { name: "Filters", exact: true }).click();
  await page.getByLabel("Find artist").fill("Seed");
  await page
    .getByRole("button", { name: "Seed artist · Seed song", exact: true })
    .click();
  const chip = page.locator(".entity-field").first();
  await expect(page.getByLabel("Find artist")).toHaveValue("Seed artist");
  expect((await chip.boundingBox()).height).toBeLessThanOrEqual(44);
  const archive = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  await page.goto(`/music/artists/${archive.rankings.artists[0].id}/edit`);
  const checkbox = page.locator(".group-members input").first();
  await expect(checkbox).toBeVisible();
  const box = await checkbox.boundingBox();
  expect(box.width).toBeLessThanOrEqual(24);
  expect(box.height).toBeLessThanOrEqual(24);
});

test("page size lives below results while filters are collapsed", async ({
  page,
}) => {
  await login(page, "/history?period=all");
  await expect(page.locator("#history-list .listen-row")).toHaveCount(1);
  await expect(page.getByLabel("Results per page")).toBeVisible();
  const size = await page.getByLabel("Results per page").boundingBox();
  const row = await page
    .locator("#history-list .listen-row")
    .last()
    .boundingBox();
  expect(size.y).toBeGreaterThan(row.y + row.height);
});

test("stars preview half ratings and track history uses compact rows", async ({
  page,
}) => {
  await login(page);
  const archive = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  await page.goto(`/music/tracks/${archive.rankings.tracks[0].id}`);
  const star = page.locator(".star-button").nth(3);
  await expect(star).toBeVisible();
  const rect = await star.boundingBox();
  await page.mouse.move(rect.x + rect.width * 0.25, rect.y + rect.height / 2);
  await expect(star.locator(".material-icons-outlined")).toHaveText(
    "star_half",
  );
  await expect(page.locator(".star-button").nth(2).locator("span")).toHaveText(
    "star",
  );
  await page.mouse.move(0, 0);
  await expect(star.locator("span")).toHaveText("star_outline");
  await expect(
    page.getByRole("combobox", { name: "Rating in stars" }),
  ).toHaveCount(0);
  const row = page.locator(".listen-context-tracks").first();
  await expect(row).toBeVisible();
  expect((await row.boundingBox()).height).toBeLessThanOrEqual(100);
});

test("music layouts fill wide viewports and album and track views switch independently", async ({
  page,
  browserName,
}) => {
  await login(page);
  const archive = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  for (const width of [1440, 2400]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const kind of ["artists", "tracks"]) {
      const id = archive.rankings[kind][0].id;
      await page.goto(`/music/${kind}/${id}/edit`);
      await expect(page.locator(".details-editor")).toBeVisible();
      const main = await page.locator("#main").boundingBox();
      const layout = await page.locator(".edit-layout").boundingBox();
      expect(layout.width).toBeGreaterThan(main.width * 0.95);
      if (kind === "tracks") {
        expect(
          (await page.locator(".details-editor").boundingBox()).width,
        ).toBeGreaterThan(main.width * 0.95);
        expect(
          (await page.locator(".details-editor").boundingBox()).height,
        ).toBeLessThan(440);
      }
      const back = page.locator(".back-link");
      expect(
        await back.evaluate((e) => getComputedStyle(e).textDecorationLine),
      ).toBe("none");
      await page.screenshot({
        path: `test-results/gui-${browserName}-${width}-compact-edit-${kind}.png`,
        fullPage: true,
        animations: "disabled",
      });
    }
  }
  await page.goto(`/music/artists/${archive.rankings.artists[0].id}`);
  const albums = page.getByRole("article", { name: "albums rankings" });
  const tracks = page.getByRole("article", { name: "tracks rankings" });
  await albums.getByRole("button", { name: "List", exact: true }).click();
  await tracks.getByRole("button", { name: "Grid", exact: true }).click();
  await expect(albums.locator(".ranking-row")).toHaveCount(1);
  await expect(tracks.locator(".music-card")).toHaveCount(1);
  await page.reload();
  await expect(
    albums.getByRole("button", { name: "List", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(
    tracks.getByRole("button", { name: "Grid", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.screenshot({
    path: `test-results/gui-${browserName}-compact-artist.png`,
    fullPage: true,
    animations: "disabled",
  });
});
test("password field fills the form and visibility control stays compact", async ({
  page,
}) => {
  await page.goto("/");
  const input = page.getByLabel("Password", { exact: true });
  await expect(input).toBeVisible();
  const field = await input.boundingBox();
  const form = await page.locator(".auth-card form").boundingBox();
  expect(field.width).toBeGreaterThan(form.width * 0.75);
  expect(
    (
      await page
        .getByRole("button", { name: "Show password", exact: true })
        .boundingBox()
    ).width,
  ).toBeLessThanOrEqual(48);
});
test("cached artwork bypasses a slow cold image", async ({ page }) => {
  let release;
  const gate = new Promise((resolve) => (release = resolve));
  let first;
  const cached = new Set();
  await page.route("**/api/v1/artwork/**", async (route) => {
    const url = new URL(route.request().url());
    if (!first) first = url.pathname;
    if (url.searchParams.has("cached")) {
      if (url.pathname === first)
        return route.fulfill({ status: 404, body: "" });
      cached.add(url.pathname);
      return route.fulfill({
        contentType: "image/png",
        body: Buffer.from(
          "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aTwAAAABJRU5ErkJggg==",
          "base64",
        ),
      });
    }
    if (url.pathname === first) await gate;

    await route.fulfill({ status: 404, body: "" });
  });
  try {
    await login(page);
    await expect.poll(() => cached.size, { timeout: 2000 }).toBeGreaterThan(0);
  } finally {
    release();
  }
});
test("password setup uses the requested policy and matching feedback", async ({
  page,
  browserName,
}) => {
  await page.route("**/api/session", (route) =>
    route.fulfill({
      json: {
        authenticated: false,
        setup_required: true,
        username_required: true,
      },
    }),
  );
  await page.goto("/");
  await page.getByLabel("Username", { exact: true }).fill("owner");
  const password = page.getByLabel("Password", { exact: true });
  const confirm = page.getByLabel("Confirm password");
  const submit = page.getByRole("button", {
    name: "Create your archive",
    exact: true,
  });
  for (const value of [
    "M7!pL9$",
    "MppLL9xx",
    "MppLL!xx",
    "7777799!",
    "Abc!7mRp",
    "M7!123xy",
    "Qwe!7mRp",
  ]) {
    await password.fill(value);
    await confirm.fill(value);
    await expect(submit).toBeDisabled();
  }
  await password.fill("M7!pL9$x");
  await confirm.fill("M7!pL9$x");
  await expect(submit).toBeEnabled();
  await expect(page.locator(".password-match")).toHaveText(
    "check_circlePasswords match",
  );
  expect(
    await page
      .locator(".password-match")
      .evaluate((e) => getComputedStyle(e).fontSize),
  ).toBe(
    await page
      .locator(".password-guidance li")
      .first()
      .evaluate((e) => getComputedStyle(e).fontSize),
  );
  await page.screenshot({
    path: `test-results/gui-${browserName}-password-policy.png`,
    fullPage: true,
    animations: "disabled",
  });
});

test("copy actions show a nearby check without success toasts and report failures inline", async ({
  page,
  browserName,
}) => {
  await page.addInitScript(() =>
    Object.defineProperty(navigator, "clipboard", {
      value: {
        writeText: async (text) => {
          if (window.failClipboard) throw new Error("denied");
          window.copiedText = text;
        },
      },
    }),
  );
  await login(page, "/settings/api-keys");
  const copy = page.getByRole("button", { name: "Copy API URL", exact: true });
  await copy.click();
  await expect(copy.locator("..").getByRole("status")).toHaveText(
    "checkCopied",
  );
  await expect(page.locator(".toast")).toHaveCount(0);
  expect(await page.evaluate(() => window.copiedText)).toContain("/api/v1");
  await page.getByLabel("Key name", { exact: true }).fill("Copy feedback test");
  await page
    .getByRole("button", { name: "Create read-only key", exact: true })
    .click();
  const keyCopy = page.getByRole("button", { name: "Copy key", exact: true });
  await keyCopy.click();
  await expect(keyCopy.locator("..").getByRole("status")).toContainText(
    "Copied",
  );
  await expect(
    page.locator(".toast").filter({ hasText: "Key copied." }),
  ).toHaveCount(0);
  await page.evaluate(() => (window.failClipboard = true));
  await keyCopy.click();
  await expect(keyCopy.locator("..").getByRole("alert")).toContainText(
    "Could not copy",
  );
  await page.evaluate(() => (window.failClipboard = false));
  await page.screenshot({
    path: `test-results/gui-${browserName}-api-spacing.png`,
    mask: [page.getByLabel("New key secret", { exact: true })],
    fullPage: true,
    animations: "disabled",
  });
  await page.goto("/settings/connections");
  await page.locator(".spotify-card").evaluate((e) => (e.open = true));
  await page
    .locator(".guide")
    .nth(1)
    .evaluate((e) => (e.open = true));
  const redirect = page.getByRole("button", {
    name: "Copy redirect URI",
    exact: true,
  });
  await redirect.click();
  await expect(redirect.locator("..").getByRole("status")).toContainText(
    "Copied",
  );
  await expect(page.locator(".toast")).toHaveCount(0);
});

test("music artwork and metrics stay proportionate and the full album card opens", async ({
  page,
  browserName,
}) => {
  await login(page);
  const archive = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  const artist = archive.rankings.artists[0];
  await page.route("**/api/v1/library/artists/*", async (route) => {
    const response = await route.fetch();
    await route.fulfill({
      json: {
        ...(await response.json()),
        spotify_url: "https://open.spotify.com/artist/0000000000000000000088",
      },
    });
  });
  await page.goto(`/music/artists/${artist.id}?period=all`);
  for (const width of [390, 768, 1440, 2400]) {
    await page.setViewportSize({ width, height: 1000 });
    await expect(page.locator(".music-heading h1")).toHaveText(
      artist.name + ".",
    );
    await expect(page.getByText("Your rating", { exact: true })).toHaveCount(0);
    await expect
      .poll(
        async () =>
          (await page.locator(".music-heading > .cover").boundingBox())
            ?.width || 0,
      )
      .toBeGreaterThanOrEqual(width < 400 ? 112 : 160);
    for (const stat of await page.locator(".stat").all())
      expect((await stat.boundingBox()).height).toBeLessThan(165);
    const spotify = page.locator(".spotify-attribution");
    await expect(spotify).toBeVisible();
    expect(
      await spotify.evaluate((e) => getComputedStyle(e).textDecorationLine),
    ).toBe("none");
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth),
      JSON.stringify(
        await page.locator("body *").evaluateAll((nodes) =>
          nodes
            .filter((node) => {
              const box = node.getBoundingClientRect();
              return box.width && box.right > innerWidth + 1;
            })
            .map((node) => ({
              tag: node.tagName,
              class: node.className,
              right: node.getBoundingClientRect().right,
              width: node.getBoundingClientRect().width,
            }))
            .slice(-8),
        ),
      ),
    ).toBeLessThanOrEqual(width);
    await page.screenshot({
      path: `test-results/gui-${browserName}-${width}-balanced-artist.png`,
      fullPage: true,
      animations: "disabled",
    });
  }
  const panel = page.getByRole("article", { name: "albums rankings" });
  await panel.getByRole("button", { name: "Grid", exact: true }).click();
  const card = panel.locator(".music-card").first();
  await card.click({ position: { x: 30, y: 60 } });
  await expect(page).toHaveURL(/\/music\/albums\//);
});

test("sidebar playback card displays artwork, progress and remains usable collapsed", async ({
  page,
  browserName,
}) => {
  await page.route("**/api/spotify/status", async (route) => {
    const response = await route.fetch();
    await route.fulfill({
      json: {
        ...(await response.json()),
        connected: true,
        connection: "verified",
      },
    });
  });
  await page.route("**/api/spotify/now-playing", (route) =>
    route.fulfill({
      json: {
        title: "A song with a long title for the sidebar",
        artists: [{ id: "artist", name: "An artist with a long name" }],
        album: "Album",
        cover_url: null,
        spotify_url: "https://open.spotify.com/track/0000000000000000000088",
        duration_ms: 180000,
        progress_ms: 45000,
        response_received_at_ms: Date.now(),
        observed_at_ms: Date.now(),
      },
    }),
  );
  await login(page);
  const widget = page.locator(".sidebar-now");
  await expect(widget.getByRole("link")).toContainText("A song");
  await expect(widget.getByRole("progressbar")).toBeVisible();
  const lefts = await widget
    .locator(".playback-copy")
    .locator(":scope > small, :scope > a, :scope > span")
    .evaluateAll((nodes) =>
      nodes.map((node) => node.getBoundingClientRect().left),
    );
  expect(Math.max(...lefts) - Math.min(...lefts)).toBeLessThanOrEqual(1);
  const collapse = page.getByRole("button", {
    name: "Collapse sidebar",
    exact: true,
  });
  await collapse.hover();
  await expect(collapse).toHaveCSS("box-shadow", "none");
  await expect(page.locator(".sidebar .wordmark > .accent")).toHaveText(".");
  expect((await widget.boundingBox()).height).toBeLessThan(155);
  const compactHeight = (await widget.boundingBox()).height;
  expect(
    await widget.evaluate((node) => {
      const cover = node
        .querySelector(":scope > .cover")
        .getBoundingClientRect();
      const copy = node.querySelector(".playback-copy").getBoundingClientRect();
      const style = getComputedStyle(node);
      return Math.abs(
        Math.min(cover.top, copy.top) -
          node.getBoundingClientRect().top -
          parseFloat(style.borderTopWidth) -
          parseFloat(style.paddingTop),
      );
    }),
  ).toBeLessThan(1);
  await widget
    .getByRole("button", { name: "Expand now playing artwork", exact: true })
    .click();
  await expect
    .poll(async () => (await widget.boundingBox()).height)
    .toBeGreaterThan(compactHeight + 100);
  await widget
    .getByRole("button", { name: "Collapse now playing artwork", exact: true })
    .click();
  await expect
    .poll(async () => (await widget.boundingBox()).height)
    .toBeLessThan(compactHeight + 1);
  await page.screenshot({
    path: `test-results/gui-${browserName}-sidebar-playback.png`,
    fullPage: true,
    animations: "disabled",
  });
  await page
    .getByRole("button", { name: "Collapse sidebar", exact: true })
    .click();
  await expect(widget.getByRole("progressbar")).toBeHidden();
});
test("first star accepts half a star and album listen rows stay compact", async ({
  page,
}) => {
  await login(page);
  const archive = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  await page.goto(`/music/albums/${archive.rankings.albums[0].id}`);
  const first = page.locator(".star-button").first();
  await expect(first).toBeVisible();
  const box = await first.boundingBox();
  await first.hover({ position: { x: box.width * 0.25, y: box.height / 2 } });
  await expect(first.locator(".material-icons-outlined")).toHaveText(
    "star_half",
  );
  await first.click({ position: { x: box.width * 0.25, y: box.height / 2 } });
  await expect(page.getByLabel("Rating in stars", { exact: true })).toHaveText(
    "0.5 / 5",
  );
  await page.reload();
  await expect(page.getByLabel("Rating in stars", { exact: true })).toHaveText(
    "0.5 / 5",
  );
  const row = page.locator(".listen-context-albums").first();
  expect((await row.boundingBox()).height).toBeLessThanOrEqual(100);
});
test("compact music toolbar, adjustable grids, aligned collections and collage PNG export", async ({
  page,
  browserName,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.route("**/api/v1/artwork/**", (route) =>
    route.fulfill({
      contentType: "image/svg+xml",
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="600" height="600"><rect width="600" height="600" fill="#687f9c"/><circle cx="300" cy="300" r="170" fill="#d8c78c"/></svg>',
    }),
  );
  await page.route("**/api/v1/consultation/rankings/*?*", async (route) => {
    const response = await route.fetch(),
      data = await response.json();
    if (data.items.length)
      data.items = Array.from({ length: 20 }, (_, index) => ({
        ...data.items[0],
        id: `${data.items[0].id}-${index}`,
        name: `${data.items[0].name} ${index + 1}`,
        position: index + 1,
        plays: 40 - index,
        listened_ms: (40 - index) * 180000,
      }));
    data.total = data.items.length;
    await route.fulfill({ json: data });
  });
  await login(page);
  const archive = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  await page.goto(`/music/artists/${archive.rankings.artists[0].id}`);
  await expect(
    page.locator(".music-heading").getByLabel("Period", { exact: true }),
  ).toBeVisible();
  const albums = page.getByRole("article", { name: "albums rankings" }),
    tracks = page.getByRole("article", { name: "tracks rankings" });
  await albums.getByRole("button", { name: "Grid", exact: true }).click();
  const small = (await albums.locator(".music-card").first().boundingBox())
    .width;
  expect(small).toBeGreaterThanOrEqual(140);
  expect(small).toBeLessThan(180);
  const gridBox = await albums.locator(".music-grid").boundingBox();
  const firstRow = await albums.locator(".music-card").evaluateAll((nodes) => {
    const y = nodes[0].getBoundingClientRect().y;
    return nodes
      .map((node) => node.getBoundingClientRect())
      .filter((box) => Math.abs(box.y - y) < 1)
      .map((box) => ({ x: box.x, width: box.width }));
  });
  const lastCell = firstRow.at(-1);
  expect(
    Math.abs(lastCell.x + lastCell.width - gridBox.x - gridBox.width),
  ).toBeLessThan(1);
  await albums
    .getByRole("button", { name: "Larger cards", exact: true })
    .click();
  await expect(
    albums.getByRole("status", { name: "Card size", exact: true }),
  ).toHaveText("Medium");
  expect(
    (await albums.locator(".music-card").first().boundingBox()).width,
  ).toBeGreaterThan(small + 25);
  await page.reload();
  await expect(
    albums.getByRole("status", { name: "Card size", exact: true }),
  ).toHaveText("Medium");
  await tracks.getByRole("button", { name: "List", exact: true }).click();
  const ab = await albums.boundingBox(),
    tb = await tracks.boundingBox();
  expect(Math.abs(ab.y + ab.height - tb.y - tb.height)).toBeLessThanOrEqual(2);
  const last = tracks.locator(".ranking-row").last();
  expect(
    parseFloat(await last.evaluate((e) => getComputedStyle(e).paddingBottom)),
  ).toBeGreaterThanOrEqual(10);
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({
    path: `test-results/gui-${browserName}-music-density.png`,
    fullPage: true,
    animations: "disabled",
  });
  await albums.getByRole("button", { name: "Collage", exact: true }).click();
  const collage = albums.getByRole("region", { name: "Album collage" });
  await expect(collage.locator(".collage-tile")).toHaveCount(16);
  await expect.poll(() => collage.locator("img").count()).toBe(16);
  const cells = await collage.locator(".collage-tile").evaluateAll((nodes) =>
    nodes.map((e) => ({
      x: parseFloat(e.style.left) / 100,
      y: parseFloat(e.style.top) / 100,
      w: parseFloat(e.style.width) / 100,
      h: parseFloat(e.style.height) / 100,
    })),
  );
  expect(cells.reduce((sum, t) => sum + t.w * t.h, 0)).toBeCloseTo(1, 5);
  for (const cell of await collage.locator(".collage-tile").all()) {
    const box = await cell.boundingBox();
    expect(Math.abs(box.width - box.height)).toBeLessThan(1);
  }
  await expect(
    albums.getByRole("button", { name: "Collage", exact: true }),
  ).toHaveCSS("outline-style", "solid");

  for (let i = 0; i < cells.length; i++)
    for (let j = i + 1; j < cells.length; j++) {
      const a = cells[i],
        b = cells[j];
      expect(
        Math.max(0, Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x)) *
          Math.max(0, Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y)),
      ).toBeLessThan(0.000001);
    }
  const readyDownload = page.waitForEvent("download");
  await collage
    .getByRole("button", { name: "Download PNG", exact: true })
    .click();
  const download = await readyDownload;
  expect(download.suggestedFilename()).toBe("scrobbl-albums-collage.png");
  const png = await require("node:fs/promises").readFile(await download.path());
  expect(png.subarray(1, 4).toString()).toBe("PNG");
  expect(png.readUInt32BE(16)).toBe(1200);
  const preview = await collage.locator(".collage-preview").boundingBox();
  expect(png.readUInt32BE(20)).toBe(
    Math.round((preview.height / preview.width) * 1200) + 120,
  );
  await page.screenshot({
    path: `test-results/gui-${browserName}-album-collage.png`,
    fullPage: true,
    animations: "disabled",
  });
  await page.goto("/rankings/artists?period=all");
  await page.getByRole("button", { name: "Collage", exact: true }).click();
  await expect(
    page
      .getByRole("region", { name: "Artist collage" })
      .locator(".collage-tile"),
  ).toHaveCount(16);
});

test("now playing opens the archived track locally and handles tracks awaiting collection", async ({
  page,
}) => {
  let trackId = null;
  await page.route("**/api/spotify/status", async (route) => {
    const response = await route.fetch();
    await route.fulfill({
      json: {
        ...(await response.json()),
        connected: true,
        connection: "verified",
      },
    });
  });
  await page.route("**/api/spotify/now-playing", (route) =>
    route.fulfill({
      json: {
        track_id: trackId,
        title: "Seed song",
        artists: [{ id: "artist", name: "Seed artist" }],
        album: "Seed album",
        cover_url: null,
        spotify_url: "https://open.spotify.com/track/0000000000000000000088",
        duration_ms: 180000,
        progress_ms: 45000,
        response_received_at_ms: Date.now(),
        observed_at_ms: Date.now(),
      },
    }),
  );
  await login(page);
  const archive = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  trackId = archive.rankings.tracks[0].id;
  await page.reload();
  const link = page
    .locator(".sidebar-now")
    .getByRole("link", { name: "Seed song", exact: true });
  await expect(link).toHaveAttribute("href", /\/music\/tracks\//);
  await link.click();
  await expect(page.locator(".music-heading")).toContainText(
    "Track by Seed artist",
  );
  await page.goto(`/music/albums/${archive.rankings.albums[0].id}`);
  await expect(page.locator(".music-heading")).toContainText(
    "Album by Seed artist",
  );
  trackId = null;
  await page.reload();
  await page
    .locator(".sidebar-now")
    .getByRole("link", { name: "Seed song", exact: true })
    .click();
  await expect(page).toHaveURL(/\/music\/now-playing/);
  await expect(
    page.getByRole("heading", { name: "Playing now", exact: true }),
  ).toBeVisible();
  expect(
    (await (await page.request.get("/api/dashboard?period=all")).json()).totals
      .records,
  ).toBe(archive.totals.records);
});
test("music header and history disclosure keep their geometry", async ({
  page,
}) => {
  await login(page);
  const archive = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  for (const kind of ["albums", "tracks"]) {
    await page.goto(
      `/music/${kind}/${archive.rankings[kind][0].id}?period=all`,
    );
    await expect(page.locator(".music-heading h1")).toBeVisible();
    const row = page.locator(`.listen-context-${kind}`).first();
    const arrow = row.locator("summary");
    await arrow.scrollIntoViewIfNeeded();
    const before = await arrow.boundingBox();
    await arrow.click();
    await expect(row.locator("details")).toHaveAttribute("open");
    const after = await arrow.boundingBox();
    expect(Math.abs(after.x - before.x)).toBeLessThan(1);
    expect(Math.abs(after.y - before.y)).toBeLessThan(1);
    await page.mouse.click(
      before.x + before.width / 2,
      before.y + before.height / 2,
    );
    await expect(row.locator("details")).not.toHaveAttribute("open");
    const heading = page.locator(".music-heading");
    await heading.scrollIntoViewIfNeeded();
    await expect
      .poll(() =>
        heading.evaluate((node) => {
          const cover = node
            .querySelector(":scope > .cover")
            .getBoundingClientRect();
          const title = node
            .querySelector(".page-heading")
            .getBoundingClientRect();
          const filters = node
            .querySelector(".music-period")
            .getBoundingClientRect();
          const inset =
            parseFloat(getComputedStyle(node.querySelector("h1")).fontSize) / 4;
          return Math.max(
            Math.abs(cover.top - title.top),
            Math.abs(cover.bottom - filters.bottom - inset),
          );
        }),
      )
      .toBeLessThan(1);
    await page.screenshot({
      path: "test-results/gui-sixth-" + kind + ".png",
      fullPage: true,
      animations: "disabled",
    });
    await page.setViewportSize({ width: 390, height: 844 });
    await arrow.scrollIntoViewIfNeeded();
    const mobile = await arrow.boundingBox();
    await arrow.click();
    const expanded = await arrow.boundingBox();
    expect(Math.abs(expanded.x - mobile.x)).toBeLessThan(1);
    expect(Math.abs(expanded.y - mobile.y)).toBeLessThan(1);
    await arrow.click();
    await page.setViewportSize({ width: 1280, height: 900 });
  }
  await page.goto("/activities");
  await expect(page.locator(".activity-link")).toHaveAttribute(
    "aria-current",
    "page",
  );
});

test("overview charts and activity reports have compact responsive layouts", async ({
  page,
  browserName,
}) => {
  const running = {
    id: "demo-import",
    kind: "spotify_import",
    filename: "Streaming history · October",
    state: "running",
    phase: "processing",
    attempt: 1,
    processed: 1250,
    expected_rows: 5000,
    added: 1040,
    duplicates: 210,
    reconciled: 0,
    invalid: 0,
    skipped: 0,
    ambiguous: 0,
    originals: 1250,
    variants: 0,
    below_threshold: 0,
    outcome: null,
    inventory_complete: true,
    retry_at_ms: null,
    error_code: null,
    target_timezone: null,
    target_week_start: null,
  };
  await page.route("**/api/v1/activities", async (route) => {
    const existing = await (await route.fetch()).json();
    return route.fulfill({ json: [running, ...existing] });
  });
  await page.route("**/api/v1/activities/demo-import", (route) =>
    route.fulfill({
      json: {
        activity: running,
        source_parameters: { account: "Your Spotify account" },
        members: [],
        issues: [],
      },
    }),
  );
  await page.route("**/api/dashboard?*", async (route) => {
    const data = await (await route.fetch()).json();
    for (const kind of ["artists", "albums", "tracks"]) {
      const first = data.rankings[kind][0];
      data.rankings[kind] = ["First", "Second", "Third", "Fourth", "Fifth"].map(
        (label, i) => ({
          ...first,
          id: first.id + "-" + i,
          name: label + " " + kind.slice(0, -1),
          position: i + 1,
          plays: 50 - i * 9,
          actual_ms: 0,
          listened_ms: (5 - i) * 180000,
          estimated_ms: (5 - i) * 180000,
        }),
      );
    }
    return route.fulfill({ json: data });
  });
  await login(page);
  await expect(page.locator(".now-panel")).toHaveCount(0);
  await expect(page.locator(".ranking-meter")).toHaveCount(15);
  const first = page.locator(".overview-rankings article").first();
  const title = await first.locator(".panel-title").boundingBox();
  const item = await first.locator(".ranking-row").first().boundingBox();
  expect(item.y - title.y - title.height).toBeGreaterThanOrEqual(12);
  const bars = first.locator(".ranking-meter > span");
  await expect(bars.last()).toHaveAttribute("style", "width: 28%;");
  await chooseOption(
    page.getByRole("combobox", { name: "Rank by", exact: true }),
    "time",
  );
  await expect(bars.last()).toHaveAttribute("style", "width: 20%;");
  await chooseOption(
    page.getByRole("combobox", { name: "Rank by", exact: true }),
    "plays",
  );
  await first.locator(".ranking-row").first().hover();
  await page.screenshot({
    path: "test-results/gui-sixth-overview-" + browserName + ".png",
    fullPage: true,
    animations: "disabled",
  });
  await page.goto("/activities");
  await expect(page.locator(".activity-link")).toHaveAttribute(
    "aria-current",
    "page",
  );
  await expect(page.locator(".activity-link")).toHaveCSS(
    "background-color",
    await page
      .locator(".sidebar nav a")
      .first()
      .evaluate((node) => {
        const probe = document.createElement("span");
        probe.style.backgroundColor = "var(--accent)";
        node.append(probe);
        const color = getComputedStyle(probe).backgroundColor;
        probe.remove();
        return color;
      }),
  );
  await expect(
    page.locator(".sidebar-job").first().getByRole("progressbar"),
  ).toHaveAttribute("value", "1250");
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 1000 });
    await expect(page.locator(".activity-row").first()).toContainText(
      "Streaming history",
    );
    await page.screenshot({
      path:
        "test-results/gui-sixth-activities-" +
        width +
        "-" +
        browserName +
        ".png",
      fullPage: true,
      animations: "disabled",
    });
    await page.locator(".activity-row-heading a").first().click();
    await expect(page.locator(".activity-detail")).toBeVisible();
    await expect(
      page.locator(".activity-detail .activity-progress progress"),
    ).toHaveAttribute("value", "1250");
    await expect(page.locator(".activity-results")).toContainText("1,040");
    await expect(page.locator(".activity-diagnostics")).not.toHaveAttribute(
      "open",
    );
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth),
    ).toBeLessThanOrEqual(width);
    await page.screenshot({
      path:
        "test-results/gui-sixth-activity-detail-" +
        width +
        "-" +
        browserName +
        ".png",
      fullPage: true,
      animations: "disabled",
    });
    await page
      .getByRole("link", { name: "All activities", exact: true })
      .click();
  }
});

test("period notices do not move controls and music artwork stays square", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await login(page);
  const controls = page.locator(".overview-heading .filters [role=combobox]");
  const before = await controls.evaluateAll((nodes) =>
    nodes.map((node) => ({
      x: node.getBoundingClientRect().x,
      y: node.getBoundingClientRect().y,
    })),
  );
  await chooseOption(controls.first(), "month");
  const after = await controls.evaluateAll((nodes) =>
    nodes.map((node) => ({
      x: node.getBoundingClientRect().x,
      y: node.getBoundingClientRect().y,
    })),
  );
  for (let i = 0; i < before.length; i++) {
    expect(Math.abs(after[i].x - before[i].x)).toBeLessThan(1);
    expect(Math.abs(after[i].y - before[i].y)).toBeLessThan(1);
  }
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({
    path: "test-results/gui-seventh-period.png",
    fullPage: true,
    animations: "disabled",
  });
  const archive = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  for (const kind of ["artists", "albums", "tracks"]) {
    await page.goto("/music/" + kind + "/" + archive.rankings[kind][0].id);
    const cover = page.locator(".music-heading > .cover");
    await expect(cover).toBeVisible();
    await expect
      .poll(async () => {
        const box = await cover.boundingBox();
        return box ? Math.abs(box.width - box.height) : Infinity;
      })
      .toBeLessThan(1);
    await expect(cover).toHaveCSS("object-fit", "contain");
    if (kind === "albums") {
      await page.evaluate(() => window.scrollTo(0, 0));
      await page.screenshot({
        path: "test-results/gui-seventh-square-header.png",
        fullPage: true,
        animations: "disabled",
      });
    }
  }
});

test("music artwork is square from the first frame through delayed image loading", async ({
  page,
}) => {
  await login(page);
  const archive = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  await page.addInitScript(() => {
    window.artworkFrames = [];
    const sample = () => {
      const node = document.querySelector(".music-heading > .cover");
      if (node) {
        const box = node.getBoundingClientRect();
        window.artworkFrames.push({
          tag: node.tagName,
          width: box.width,
          height: box.height,
        });
      }
      requestAnimationFrame(sample);
    };
    requestAnimationFrame(sample);
  });
  await page.route("**/api/v1/artwork/**", async (route) => {
    await new Promise((resolve) => setTimeout(resolve, 300));
    await route.fulfill({
      contentType: "image/svg+xml",
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="600" height="200"><rect width="600" height="200" fill="#7c9bac"/></svg>',
    });
  });
  for (const width of [1440, 768, 390]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const kind of ["artists", "albums", "tracks"]) {
      await page.goto("/music/" + kind + "/" + archive.rankings[kind][0].id);
      await expect(page.locator(".music-heading > img")).toBeVisible();
      await expect
        .poll(() =>
          page.evaluate(
            () => document.querySelector(".music-heading > img")?.complete,
          ),
        )
        .toBe(true);
      const frames = await page.evaluate(() => window.artworkFrames);
      expect(frames.some((frame) => frame.tag === "SPAN")).toBe(true);
      expect(
        frames.filter((frame) => Math.abs(frame.width - frame.height) > 1),
      ).toEqual([]);
    }
  }
});

test("history collaborators share a baseline and album edits have equal panels", async ({
  page,
}) => {
  await page.route("**/api/history?*", async (route) => {
    const response = await route.fetch(),
      data = await response.json();
    data.items = data.items.map((item) => ({
      ...item,
      artists: [
        { id: "first", name: "First artist" },
        { id: "second", name: "Second artist" },
      ],
    }));
    await route.fulfill({ json: data });
  });
  await login(page, "/history?period=all");
  const artists = page.locator(".listen-title small").first().getByRole("link");
  await expect(artists).toHaveCount(2);
  const first = await artists.first().boundingBox(),
    second = await artists.last().boundingBox();
  expect(Math.abs(first.y - second.y)).toBeLessThan(1);
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({
    path: "test-results/gui-seventh-collaborators.png",
    fullPage: true,
    animations: "disabled",
  });
  const archive = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  await page.goto("/music/albums/" + archive.rankings.albums[0].id + "/edit");
  const details = await page.locator(".details-editor").boundingBox(),
    artwork = await page.locator(".artwork-editor").boundingBox();
  expect(Math.abs(details.height - artwork.height)).toBeLessThan(1);
  await expect(
    page.getByRole("link", { name: "Back to album", exact: true }),
  ).toHaveClass(/button/);
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({
    path: "test-results/gui-seventh-album-edit.png",
    fullPage: true,
    animations: "disabled",
  });
});

test("artwork downloads appear in activities and continue across navigation", async ({
  page,
  browserName,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  let release;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  const saved = new Set();
  let downloads = 0;
  const svg =
    '<svg xmlns="http://www.w3.org/2000/svg" width="600" height="300"><rect width="600" height="300" fill="#7c9bac"/><circle cx="300" cy="150" r="130" fill="#e8cb8b"/></svg>';
  await page.route("**/api/v1/artwork/**", async (route) => {
    const url = new URL(route.request().url());
    if (url.searchParams.has("cached"))
      return route.fulfill(
        saved.has(url.pathname)
          ? { contentType: "image/svg+xml", body: svg }
          : { status: 404, body: "" },
      );
    downloads++;
    await gate;
    saved.add(url.pathname);
    await route.fulfill({ contentType: "image/svg+xml", body: svg });
  });
  try {
    await login(page, "/rankings/albums?period=all");
    await expect(page.locator(".sidebar .artwork-work")).toContainText(
      "Downloading artwork",
    );
    await page.locator(".sidebar .artwork-work").click();
    const work = page.getByRole("article", {
      name: "Artwork downloads",
      exact: true,
    });
    await expect(work).toContainText("1 remaining");
    await expect(work.getByRole("progressbar")).toHaveAttribute("value", "0");
    await page.evaluate(() => window.scrollTo(0, 0));
    await page.screenshot({
      path: "test-results/gui-seventh-artwork-" + browserName + ".png",
      fullPage: true,
      animations: "disabled",
    });
    release();
    await expect(work).toContainText("1 saved");
    await expect(work).toContainText("Complete");
    await expect(page.locator(".sidebar .artwork-work")).toHaveCount(0);
    await page.goto("/rankings/albums?period=all");
    await expect(
      page.locator(".music-card img, .ranking-row img").first(),
    ).toBeVisible();
    expect(downloads).toBe(1);
  } finally {
    release();
  }
});

test("album evidence aligns with closed columns and disclosures animate", async ({
  page,
  browserName,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await login(page);
  const archive = await (
    await page.request.get("/api/dashboard?period=all")
  ).json();
  await page.goto("/music/albums/" + archive.rankings.albums[0].id);
  const row = page.locator(".listen-context-albums").first();
  const dateX = (await row.locator(".listen-date").boundingBox()).x;
  const archiveX = (await row.locator(".listen-archive").boundingBox()).x;
  await expect(row.locator(".listen-artists")).toHaveCount(0);
  const motion = await row.locator("summary").evaluate((summary) => {
    summary.click();
    const body = summary.parentElement.querySelector("dl");
    const animation = body.getAnimations()[0];
    if (!animation) return null;
    animation.pause();
    animation.currentTime = 0;
    const start = body.getBoundingClientRect().height;
    animation.currentTime = 80;
    const middle = body.getBoundingClientRect().height;
    animation.finish();
    return {
      start,
      middle,
      end: body.getBoundingClientRect().height,
      duration: animation.effect.getTiming().duration,
    };
  });
  expect(motion.duration).toBe(160);
  expect(motion.middle).toBeGreaterThan(motion.start);
  expect(motion.middle).toBeLessThan(motion.end);
  expect(
    Math.abs((await row.locator("dl > div").nth(0).boundingBox()).x - dateX),
  ).toBeLessThan(1);
  expect(
    Math.abs((await row.locator("dl > div").nth(4).boundingBox()).x - archiveX),
  ).toBeLessThan(1);
  await row.scrollIntoViewIfNeeded();
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({
    path: "test-results/gui-seventh-evidence-" + browserName + ".png",
    fullPage: true,
    animations: "disabled",
  });
  await page.emulateMedia({ reducedMotion: "reduce" });
  const reduced = await row.locator("summary").evaluate((summary) => {
    summary.click();
    return {
      open: summary.parentElement.open,
      animations: summary.parentElement.querySelector("dl").getAnimations()
        .length,
    };
  });
  expect(reduced).toEqual({ open: false, animations: 0 });
});

test("weighted collages retain square tiles across sizes and exports", async ({
  page,
  browserName,
}) => {
  await page.setViewportSize({ width: 1440, height: 1600 });
  await page.route("**/api/v1/artwork/**", (route) => {
    const path = new URL(route.request().url()).pathname;
    const hue =
      [...path].reduce((sum, char) => sum + char.charCodeAt(0), 0) % 360;
    return route.fulfill({
      contentType: "image/svg+xml",
      body:
        '<svg xmlns="http://www.w3.org/2000/svg" width="600" height="600"><rect width="600" height="600" fill="hsl(' +
        hue +
        ',40%,36%)"/><circle cx="300" cy="300" r="210" fill="hsl(' +
        (hue + 75) +
        ',60%,72%)"/></svg>',
    });
  });
  await page.route(
    "**/api/v1/consultation/rankings/albums?*",
    async (route) => {
      const data = await (await route.fetch()).json();
      const source = data.items[0];
      data.items = Array.from({ length: 25 }, (_, i) => ({
        ...source,
        id: "mosaic-" + i,
        name: "Album " + (i + 1),
        position: i + 1,
        cover_url: "/api/v1/artwork/albums/mosaic-" + i,
        plays: Math.max(1, Math.round(400 / 2 ** (i / 2))),
        listened_ms: Math.max(1, Math.round(400 / 2 ** (i / 2))) * 180000,
      }));
      data.total = 25;
      return route.fulfill({ json: data });
    },
  );
  await login(page, "/rankings/albums?period=all");
  await page.getByRole("button", { name: "Collage", exact: true }).click();
  const collage = page.getByRole("region", {
    name: "Album collage",
    exact: true,
  });
  for (const count of [9, 16, 25]) {
    await chooseOption(collage.getByRole("combobox"), String(count));
    await expect(collage.locator(".collage-tile")).toHaveCount(count);
    const boxes = await collage.locator(".collage-tile").evaluateAll((nodes) =>
      nodes.map((node) => {
        const box = node.getBoundingClientRect();
        return { x: box.x, y: box.y, w: box.width, h: box.height };
      }),
    );
    expect(boxes[0].w).toBeGreaterThan(boxes.at(-1).w * 1.5);
    for (let i = 0; i < boxes.length; i++) {
      expect(Math.abs(boxes[i].w - boxes[i].h)).toBeLessThan(1);
      for (let j = i + 1; j < boxes.length; j++)
        expect(
          Math.max(
            0,
            Math.min(boxes[i].x + boxes[i].w, boxes[j].x + boxes[j].w) -
              Math.max(boxes[i].x, boxes[j].x),
          ) *
            Math.max(
              0,
              Math.min(boxes[i].y + boxes[i].h, boxes[j].y + boxes[j].h) -
                Math.max(boxes[i].y, boxes[j].y),
            ),
        ).toBeLessThan(1);
    }
  }
  await expect.poll(() => collage.locator("img").count()).toBe(25);
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({
    path: "test-results/gui-seventh-weighted-collage-" + browserName + ".png",
    fullPage: true,
    animations: "disabled",
  });
  const download = page.waitForEvent("download");
  await collage
    .getByRole("button", { name: "Download PNG", exact: true })
    .click();
  const file = await download;
  const png = await require("node:fs/promises").readFile(await file.path());
  const box = await collage.locator(".collage-preview").boundingBox();
  expect(png.readUInt32BE(16)).toBe(1200);
  expect(png.readUInt32BE(20)).toBe(
    Math.round((box.height / box.width) * 1200) + 120,
  );
});
