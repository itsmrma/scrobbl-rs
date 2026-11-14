const { test: base, expect } = require("@playwright/test");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const { mkdtemp, rm } = require("node:fs/promises");
const { tmpdir } = require("node:os");
const { join, resolve } = require("node:path");
const net = require("node:net");

const test = base.extend({
  polishApp: async ({ browser }, use) => {
    const directory = await mkdtemp(join(tmpdir(), "scrobbl-gui-polish-"));
    const socket = net.createServer();
    socket.listen(0, "127.0.0.1");
    await once(socket, "listening");
    const port = socket.address().port;
    await new Promise((resolve) => socket.close(resolve));
    const baseURL = `http://127.0.0.1:${port}`;
    const child = spawn(
      resolve(
        process.env.CARGO_TARGET_DIR || "target",
        "debug",
        "scrobbl-rs" + (process.platform === "win32" ? ".exe" : ""),
      ),
      [],
      {
        windowsHide: true,
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
    child.stdout.on("data", (data) => (logs += data));
    child.stderr.on("data", (data) => (logs += data));
    const context = await browser.newContext({ baseURL });
    try {
      await expect
        .poll(
          async () => {
            if (child.exitCode !== null) throw new Error(logs);
            try {
              return (await context.request.get("/api/session")).status();
            } catch {
              return 0;
            }
          },
          { timeout: 20000 },
        )
        .toBe(200);
      await use({ page: await context.newPage(), baseURL });
    } finally {
      await context.close().catch(() => {});
      if (child.exitCode === null) {
        const stopped = once(child, "exit");
        child.kill("SIGINT");
        await stopped;
      }
      await rm(directory, { recursive: true, force: true });
    }
  },
});

const password = "a local browser test password 7!";

test("gui polish fixes: registration match, account menu, app storage, ai download, artwork selection, time info, search", async ({
  polishApp: { page },
}) => {
  await page.goto("/");

  // 1. Registration password match alignment & styling
  await page.getByLabel("Username", { exact: true }).fill("owner");
  await page.getByLabel("Password", { exact: true }).fill(password);
  await page.getByLabel("Confirm password", { exact: true }).fill(password);
  const matchNote = page.locator(".password-match.requirement-met");
  await expect(matchNote).toBeVisible();
  await expect(matchNote).toHaveText(/Passwords match/);
  const matchStyle = await matchNote.evaluate((el) => {
    const cs = getComputedStyle(el);
    return {
      display: cs.display,
      alignItems: cs.alignItems,
      gap: cs.gap,
    };
  });
  expect(matchStyle.display).toBe("flex");
  expect(matchStyle.alignItems).toBe("center");
  expect(parseFloat(matchStyle.gap)).toBeGreaterThanOrEqual(6);

  await page.getByRole("button", { name: "Create your archive" }).click();
  await expect(
    page.getByRole("heading", { name: /Your listening, at a glance/ }),
  ).toBeVisible();

  // Import sample listens via ListenBrainz JSONL so search, music page, and time breakdown have data.
  const csrf = await page.evaluate(async () => {
    const res = await fetch("/api/session");
    return (await res.json()).csrf;
  });
  const sampleRows = [
    JSON.stringify({
      listened_at: 1750000000,
      track_metadata: {
        artist_name: "Test Artist",
        release_name: "Test Album",
        track_name: "Sample Track",
        additional_info: { duration_ms: 180000 },
      },
    }),
  ].join("\n");
  const importRes = await page.evaluate(
    async ({ csrfToken, content }) => {
      const form = new FormData();
      form.append("account", "owner");
      form.append(
        "files",
        new Blob([content], { type: "application/jsonl" }),
        "listens.jsonl",
      );
      const res = await fetch("/api/v1/listenbrainz/upload", {
        method: "POST",
        headers: { "X-CSRF-Token": csrfToken },
        body: form,
      });
      return res.status;
    },
    { csrfToken: csrf, content: sampleRows },
  );
  expect(importRes).toBe(200);
  await expect
    .poll(
      async () => {
        const info = await (
          await page.request.get("/api/v1/application")
        ).json();
        return info.listens;
      },
      { timeout: 10000 },
    )
    .toBeGreaterThanOrEqual(1);

  await page.reload();

  // 2. TimeHelp (i) contains Actual & Estimated; .stat does not show .time-composition
  await expect(page.locator(".stat .time-composition")).toHaveCount(0);
  await page
    .getByRole("button", { name: "How we calculate listening time" })
    .click();
  const explanation = page.locator(".time-explanation");
  await expect(explanation).toBeVisible();
  await expect(
    explanation.locator(".time-explanation-breakdown"),
  ).toContainText("Actual");
  await expect(
    explanation.locator(".time-explanation-breakdown"),
  ).toContainText("Estimated");

  // 3. AccountMenu spacing between icon and links + stacked role label
  await page.getByRole("button", { name: "Account menu", exact: true }).click();
  const accountLink = page.getByRole("link", {
    name: "Account settings",
    exact: true,
  });
  await expect(accountLink).toBeVisible();
  const linkGap = await accountLink.evaluate((el) =>
    parseFloat(getComputedStyle(el).gap),
  );
  expect(linkGap).toBeGreaterThanOrEqual(8);
  const roleDisplay = await page
    .locator(".account-menu-heading > div")
    .evaluate((el) => getComputedStyle(el).display);
  expect(roleDisplay).toBe("grid");

  // 4. Application settings storage breakdown
  await page
    .getByRole("link", { name: "Application settings", exact: true })
    .click();
  await expect(page.getByText("Listens & catalog data")).toBeVisible();
  await expect(page.getByText("AI models on disk")).toBeVisible();
  await expect(page.getByText("Total app data")).toBeVisible();

  // 5. AI Settings: download enabled while AI off, no inline progress bar, speed & ETA rendered
  await page.route("**/api/v1/ai/catalog", async (route) => {
    const res = await route.fetch();
    const json = await res.json();
    if (json.models?.length) {
      json.models[0].download_job = {
        id: "job-1",
        state: "running",
        processed_bytes: 541 * 1024 * 1024,
        expected_bytes: 1200 * 1024 * 1024,
        bytes_per_second: 12 * 1024 * 1024,
        eta_seconds: 55,
        cancel_requested: false,
        error_code: null,
      };
    }
    await route.fulfill({ status: 200, json });
  });
  await page.goto("/settings/ai");
  await expect(page.locator(".ai-inline-progress")).toHaveCount(0);
  await expect(page.locator(".ai-progress-head")).toContainText("12.0 MB/s");
  await expect(page.locator(".ai-progress-head")).toContainText("55s left");
  const secondaryDownload = page
    .getByRole("button", { name: "Download", exact: true })
    .first();
  if (await secondaryDownload.count()) {
    await expect(secondaryDownload).toBeEnabled();
  }
  await page.unroute("**/api/v1/ai/catalog");

  // 6. Search autocomplete + debounced auto-search
  await page.goto("/search");
  const searchInput = page.getByPlaceholder("Artist, album or track name");
  await searchInput.fill("Test");
  await expect(page.locator(".search-autocomplete")).toBeVisible({
    timeout: 5000,
  });
  await expect(page.locator(".search-autocomplete")).toContainText(
    "Test Artist",
  );
  await expect(page).toHaveURL(/\/search\?q=Test/, { timeout: 5000 });

  // 7. Music page Edit button single space + Choose artwork selects actual current candidate
  await page
    .getByRole("link", { name: /Test Album/i })
    .first()
    .click();
  const editLink = page.getByRole("link", { name: "Edit details & image" });
  await expect(editLink).toBeVisible();
  expect(await editLink.locator("> span").last().textContent()).toBe(
    "Edit details & image",
  );

  await page.route("**/api/v1/artwork/**/candidates*", (route) =>
    route.fulfill({
      status: 200,
      json: {
        target_kind: "albums",
        target_id: "alb-1",
        target_name: "Test Album",
        default_query: "Test Album",
        query: "Test Album",
        current_image_url: "https://example.org/current-2.jpg",
        current_provider: "itunes",
        candidates: [
          {
            id: "cand-1",
            title: "Wrong First",
            subtitle: "Deezer",
            provider: "deezer",
            provider_label: "Deezer",
            preview_url: "https://example.org/other-1.jpg",
            image_url: "https://example.org/other-1.jpg",
            source_url: "https://example.org/1",
            credit: {
              provider: "deezer",
              source_url: "https://example.org/1",
              image_url: "https://example.org/other-1.jpg",
              author: null,
              license: null,
              license_url: null,
              credit: "Deezer",
            },
          },
          {
            id: "cand-2",
            title: "Actual Saved",
            subtitle: "iTunes",
            provider: "itunes",
            provider_label: "Apple iTunes",
            preview_url: "https://example.org/current-2.jpg",
            image_url: "https://example.org/current-2.jpg",
            source_url: "https://example.org/2",
            credit: {
              provider: "itunes",
              source_url: "https://example.org/2",
              image_url: "https://example.org/current-2.jpg",
              author: null,
              license: null,
              license_url: null,
              credit: "Apple iTunes",
            },
          },
        ],
        warnings: [],
      },
    }),
  );
  await page.getByRole("button", { name: "Change artwork" }).click();
  const firstCand = page.locator(".image-candidate-card").nth(0);
  const secondCand = page.locator(".image-candidate-card").nth(1);
  await expect(firstCand).toHaveAttribute("aria-selected", "false");
  await expect(secondCand).toHaveAttribute("aria-selected", "true");
  await expect(secondCand).toContainText("Current");
});
