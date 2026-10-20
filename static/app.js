"use strict";
const $ = (id) => document.getElementById(id);
const number = new Intl.NumberFormat("en-GB");
const dateFormat = new Intl.DateTimeFormat("en-GB", {
  timeZone: "Europe/Rome",
  day: "2-digit",
  month: "short",
  year: "numeric",
  hour: "2-digit",
  minute: "2-digit",
  hourCycle: "h23",
});
let csrf = null,
  setupRequired = false,
  currentPage = "dashboard",
  historyPage = 1,
  historyTotal = 0;
let spotifyState = null,
  pollTimer = null,
  refreshTimer = null,
  generation = 0;
function node(tag, className, text) {
  const n = document.createElement(tag);
  if (className) n.className = className;
  if (text !== undefined && text !== null) n.textContent = String(text);
  return n;
}
function notice(message, error = false) {
  $("notice").textContent = message;
  $("notice").classList.toggle("error", error);
  $("notice").hidden = false;
}
function clearNotice() {
  $("notice").hidden = true;
}
function date(ms) {
  return ms ? dateFormat.format(new Date(ms)) : "Never";
}
function duration(ms, compact = false) {
  if (ms === null || ms === undefined) return "Unknown";
  const seconds = Math.floor(ms / 1000),
    hours = Math.floor(seconds / 3600),
    minutes = Math.floor(seconds / 60);
  if (compact)
    return hours
      ? `${number.format(hours)}h ${minutes % 60}m`
      : `${number.format(minutes)}m`;
  return `${minutes}:${String(seconds % 60).padStart(2, "0")}`;
}
async function api(path, options = {}) {
  const headers = { ...options.headers };
  if (options.body && !(options.body instanceof FormData))
    headers["Content-Type"] = "application/json";
  if (options.method && options.method !== "GET" && csrf)
    headers["X-CSRF-Token"] = csrf;
  const response = await fetch(path, {
    ...options,
    headers,
    credentials: "same-origin",
  });
  const data = await response
    .json()
    .catch(() => ({ error: "The server returned an unreadable response." }));
  if (!response.ok) {
    if (response.status === 401 && !path.endsWith("/login")) {
      csrf = null;
      showAuth();
    }
    throw new Error(data.error || `Request failed (${response.status}).`);
  }
  return data;
}
function post(path, body) {
  return api(path, {
    method: "POST",
    body: body instanceof FormData ? body : JSON.stringify(body || {}),
  });
}
function empty(target, title, message, link = false) {
  const box = node("div", "empty");
  box.append(node("strong", "", title), node("span", "", message));
  if (link) {
    const a = node("a", "", "Import your Spotify history ↗");
    a.href = "#import";
    box.append(node("br"), a);
  }
  target.replaceChildren(box);
}
function cover(url, large = false) {
  if (url && url.startsWith("https://i.scdn.co/")) {
    const img = node("img", `cover${large ? " large" : ""}`);
    img.src = url;
    img.alt = "";
    img.loading = "lazy";
    img.referrerPolicy = "no-referrer";
    img.addEventListener("error", () => img.replaceWith(cover(null, large)), {
      once: true,
    });
    return img;
  }
  return node("div", `cover placeholder${large ? " large" : ""}`, "♫");
}
function safeSpotifyLink(url, text) {
  if (url && url.startsWith("https://open.spotify.com/track/")) {
    const a = node("a", "", text);
    a.href = url;
    a.target = "_blank";
    a.rel = "noreferrer";
    return a;
  }
  return node("strong", "", text);
}
function renderList(target, items) {
  target.replaceChildren();
  if (!items.length) {
    empty(
      target,
      "Your archive is ready for its first listen",
      "Import your past or connect Spotify to start collecting.",
      true,
    );
    return;
  }
  items.forEach((item) => {
    const row = node("article", "listen-row");
    row.append(cover(item.cover_url));
    const title = node("div", "listen-title");
    title.append(
      safeSpotifyLink(item.spotify_url, item.title),
      node(
        "small",
        "",
        (item.artists || []).map((a) => a.name).join(", ") || "Unknown artist",
      ),
    );
    row.append(title);
    row.append(node("div", "listen-album", item.album || "Unknown album"));
    const stamp = node("div", "listen-date", date(item.played_at_ms));
    stamp.append(
      node(
        "small",
        "",
        item.sources.includes("spotify_import")
          ? item.sources.length > 1
            ? "Import + Spotify"
            : "Spotify import"
          : "Spotify collection",
      ),
    );
    if (item.needs_review)
      stamp.append(node("small", "warning", "Possible duplicate"));
    row.append(stamp);
    const length = node(
      "div",
      "listen-duration",
      `${item.duration_kind === "estimated" ? "≈ " : ""}${duration(item.listened_ms)}`,
    );
    length.append(
      node(
        "small",
        item.counted ? "" : "short-tag",
        !item.counted
          ? "Below threshold"
          : item.duration_kind === "actual"
            ? "Actual"
            : item.duration_kind === "estimated"
              ? "Estimated"
              : "Unknown duration",
      ),
    );
    row.append(length);
    target.append(row);
  });
}
function dashboardParams() {
  const p = new URLSearchParams({
    period: $("period").value,
    sort: $("ranking-sort").value,
  });
  if ($("period").value === "custom") {
    if ($("dashboard-from").value) p.set("from", $("dashboard-from").value);
    if ($("dashboard-to").value) p.set("to", $("dashboard-to").value);
  }
  return p;
}
async function loadDashboard() {
  const token = generation,
    data = await api(`/api/dashboard?${dashboardParams()}`);
  if (token !== generation || !csrf) return;
  $("total-listens").textContent = number.format(data.totals.listens);
  $("total-tracks").textContent = number.format(data.totals.tracks);
  $("total-time").textContent = duration(
    data.totals.actual_ms + data.totals.estimated_ms,
    true,
  );
  $("duration-detail").textContent =
    `${duration(data.totals.actual_ms, true)} actual · ${duration(data.totals.estimated_ms, true)} estimated${data.totals.unknown_durations ? ` · ${data.totals.unknown_durations} unknown` : ""}`;
  for (const kind of ["artists", "albums", "tracks"]) {
    const target = $(`top-${kind}`);
    target.replaceChildren();
    const items = data.rankings[kind] || [];
    if (!items.length) {
      const li = node("li", "empty", "Your favourites will appear here.");
      target.append(li);
      continue;
    }
    items.forEach((item, index) => {
      const row = node("li", "ranking-row");
      row.append(
        node("span", "rank-number", String(index + 1).padStart(2, "0")),
        cover(item.cover_url),
      );
      const name = node("div", "rank-name");
      name.append(node("strong", "", item.name));
      if (item.subtitle) name.append(node("small", "", item.subtitle));
      else
        name.append(
          node(
            "small",
            "",
            `${number.format(item.plays)} ${item.plays === 1 ? "listen" : "listens"}`,
          ),
        );
      row.append(
        name,
        node(
          "span",
          "rank-value",
          $("ranking-sort").value === "time"
            ? duration(item.listened_ms, true)
            : number.format(item.plays),
        ),
      );
      target.append(row);
    });
  }
  renderList($("recent-list"), data.recent);
}
function historyParams() {
  const p = new URLSearchParams({
    page: String(historyPage),
    status: $("history-status").value,
  });
  if ($("history-q").value.trim()) p.set("q", $("history-q").value.trim());
  if ($("history-from").value) p.set("from", $("history-from").value);
  if ($("history-to").value) p.set("to", $("history-to").value);
  return p;
}
async function loadHistory() {
  const token = generation,
    data = await api(`/api/history?${historyParams()}`);
  if (token !== generation || !csrf) return;
  historyTotal = data.total;
  renderList($("history-list"), data.items);
  $("history-total").textContent = `${number.format(data.total)} records`;
  $("page-number").textContent =
    `Page ${historyPage} of ${Math.max(1, Math.ceil(data.total / 50))}`;
  $("prev-page").disabled = historyPage <= 1;
  $("next-page").disabled = historyPage * 50 >= data.total;
}
function renderNow(value) {
  const target = $("now-content");
  target.replaceChildren();
  $("now-label").textContent = value ? "NOW PLAYING" : "NOT PLAYING";
  const copy = node("div");
  if (value) {
    target.append(cover(value.cover_url, true));
    const title = node("h3");
    title.append(safeSpotifyLink(value.spotify_url, value.title));
    copy.append(
      title,
      node("p", "", value.artists.map((a) => a.name).join(", ")),
    );
    $("now-footer").textContent =
      `${duration(value.progress_ms)} / ${duration(value.duration_ms)} · From Spotify`;
  } else {
    target.append(cover(null, true));
    copy.append(
      node("h3", "", "Your next favourite awaits"),
      node(
        "p",
        "",
        spotifyState?.connected
          ? "Nothing playing on Spotify right now."
          : "Connect Spotify to see what’s playing.",
      ),
    );
    $("now-footer").textContent = "Live while this page is open";
  }
  target.append(copy);
}
async function loadNow() {
  try {
    renderNow(await api("/api/spotify/now-playing"));
  } catch (e) {
    $("now-label").textContent = "SPOTIFY UNAVAILABLE";
    $("now-footer").textContent = "Check connection status in Settings";
  }
}
async function loadSpotify() {
  const token = generation,
    data = await api("/api/spotify/status");
  if (token !== generation || !csrf) return;
  spotifyState = data;
  $("redirect-uri").textContent = data.redirect_uri;
  $("connection-badge").textContent = data.connected
    ? "Connected"
    : data.configured
      ? "Ready to connect"
      : "App setup needed";
  $("credentials-form").hidden = data.credentials_from_env;
  $("connect-spotify").disabled = !data.configured;
  $("connect-spotify").textContent = data.connected
    ? "Reconnect Spotify ↗"
    : "Connect Spotify ↗";
  $("sync-spotify").disabled = !data.connected;
  $("disconnect-spotify").disabled = !data.connected;
  $("spotify-guide").open = !data.configured;
  $("last-sync").textContent = date(data.last_sync_ms);
  $("sync-interval").textContent =
    `Every ${data.sync_seconds} seconds · page can be closed`;
  $("retry-at").textContent =
    data.retry_at_ms > Date.now() ? date(data.retry_at_ms) : "No restriction";
  $("spotify-error").textContent = data.last_error || "";
  $("coverage-warning").textContent = data.coverage_warning || "";
  $("reconciliation-warning").textContent = data.review_count
    ? `${data.review_count} records have ambiguous matches. Use the Possible duplicates filter in Listening history.`
    : "";
  const jobs = $("import-jobs");
  jobs.replaceChildren();
  if (!data.imports.length) jobs.append(node("p", "subtle", "No imports yet."));
  data.imports.forEach((job) => {
    const row = node("div", "job");
    row.append(
      node("strong", "", job.filename),
      node("small", "", date(job.created_at_ms)),
      node(
        "span",
        "",
        `${job.report.added} added · ${job.report.reconciled} reconciled · ${job.report.duplicates} duplicates · ${job.report.invalid} invalid`,
      ),
    );
    if (job.report.ambiguous)
      row.append(
        node(
          "span",
          "warning",
          `${job.report.ambiguous} ambiguous matches kept separate`,
        ),
      );
    (job.report.errors || []).forEach((error) =>
      row.append(node("small", "form-error", error)),
    );
    jobs.append(row);
  });
}
function stopPolling() {
  clearInterval(pollTimer);
  clearInterval(refreshTimer);
  pollTimer = null;
  refreshTimer = null;
}
function showAuth() {
  generation++;
  stopPolling();
  $("app-view").hidden = true;
  $("auth-view").hidden = false;
  $("loading").hidden = true;
  $("auth-title").textContent = setupRequired
    ? "Make this space yours"
    : "Welcome back";
  $("auth-help").textContent = setupRequired
    ? "Set an owner password of at least 12 characters."
    : "Sign in to your listening archive.";
  $("auth-submit").textContent = setupRequired
    ? "Create your archive ↗"
    : "Sign in ↗";
  $("confirm-wrap").hidden = !setupRequired;
  $("confirm-password").required = setupRequired;
  $("password").autocomplete = setupRequired
    ? "new-password"
    : "current-password";
  $("password").minLength = setupRequired ? 12 : 1;
  $("password").focus();
}
async function showApp() {
  generation++;
  $("auth-view").hidden = true;
  $("app-view").hidden = false;
  $("loading").hidden = true;
  await loadSpotify();
  await route();
  stopPolling();
  pollTimer = setInterval(
    () => {
      if (!document.hidden && csrf) {
        loadSpotify().catch((e) => notice(e.message, true));
        if (currentPage === "dashboard") loadNow();
      }
    },
    (spotifyState?.now_playing_seconds || 30) * 1000,
  );
  refreshTimer = setInterval(
    () => {
      if (!document.hidden && csrf) {
        if (currentPage === "dashboard")
          loadDashboard().catch((e) => notice(e.message, true));
        if (currentPage === "history")
          loadHistory().catch((e) => notice(e.message, true));
      }
    },
    (spotifyState?.sync_seconds || 120) * 1000,
  );
}
async function route() {
  if (!csrf) return;
  window.scrollTo(0, 0);
  const page = location.hash.slice(1);
  currentPage = ["dashboard", "history", "import", "settings"].includes(page)
    ? page
    : "dashboard";
  for (const p of ["dashboard", "history", "import", "settings"])
    $(`page-${p}`).hidden = p !== currentPage;
  document.querySelectorAll("[data-page]").forEach((a) => {
    a.classList.toggle("active", a.dataset.page === currentPage);
    if (a.dataset.page === currentPage) a.setAttribute("aria-current", "page");
    else a.removeAttribute("aria-current");
  });
  $("breadcrumb").textContent =
    `Your library / ${{ dashboard: "Overview", history: "Listening history", import: "Import history", settings: "Settings & connections" }[currentPage]}`;
  document.title = `${{ dashboard: "Overview", history: "Listening history", import: "Import history", settings: "Settings" }[currentPage]} · scrobbl.rs`;
  clearNotice();
  if (currentPage === "dashboard") {
    await loadDashboard();
    await loadNow();
  }
  if (currentPage === "history") await loadHistory();
  if (currentPage === "settings") await loadSpotify();
}
function action(id, callback) {
  $(id).addEventListener("click", async () => {
    const b = $(id);
    b.disabled = true;
    clearNotice();
    try {
      await callback();
    } catch (e) {
      notice(e.message, true);
    } finally {
      b.disabled = false;
      if (
        spotifyState &&
        ["connect-spotify", "sync-spotify", "disconnect-spotify"].includes(id)
      ) {
        loadSpotify().catch(() => {});
      }
    }
  });
}
$("auth-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  $("auth-error").textContent = "";
  const button = $("auth-submit");
  button.disabled = true;
  try {
    if (setupRequired && $("password").value !== $("confirm-password").value)
      throw new Error("The passwords do not match.");
    const data = await post(setupRequired ? "/api/setup" : "/api/login", {
      password: $("password").value,
    });
    csrf = data.csrf;
    setupRequired = false;
    $("auth-form").reset();
    await showApp();
  } catch (e) {
    $("auth-error").textContent = e.message;
  } finally {
    button.disabled = false;
  }
});
action("logout", async () => {
  await post("/api/logout");
  csrf = null;
  setupRequired = false;
  $("auth-error").textContent = "";
  showAuth();
});
$("period").addEventListener("change", () => {
  $("dashboard-dates").hidden = $("period").value !== "custom";
  if ($("period").value !== "custom")
    loadDashboard().catch((e) => notice(e.message, true));
});
$("ranking-sort").addEventListener("change", () =>
  loadDashboard().catch((e) => notice(e.message, true)),
);
action("apply-period", loadDashboard);
$("history-filters").addEventListener("submit", (event) => {
  event.preventDefault();
  historyPage = 1;
  loadHistory().catch((e) => notice(e.message, true));
});
$("history-filters").addEventListener("reset", () => {
  historyPage = 1;
  setTimeout(() => loadHistory().catch((e) => notice(e.message, true)), 0);
});
action("prev-page", async () => {
  historyPage = Math.max(1, historyPage - 1);
  await loadHistory();
});
action("next-page", async () => {
  if (historyPage * 50 < historyTotal) historyPage++;
  await loadHistory();
});
$("import-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const button = $("import-submit"),
    target = $("import-result");
  button.disabled = true;
  button.textContent = "Importing… Keep this page open";
  target.hidden = true;
  clearNotice();
  try {
    const form = new FormData();
    for (const file of $("import-files").files) form.append("files", file);
    const data = await post("/api/import/spotify", form);
    target.replaceChildren();
    const summary = node("div", "import-summary");
    summary.append(
      node("h3", "", "Import complete"),
      node(
        "p",
        "",
        `${number.format(data.total.added)} added · ${number.format(data.total.reconciled)} reconciled · ${number.format(data.total.duplicates)} already present · ${data.total.skipped} non-music skipped · ${data.total.invalid} invalid`,
      ),
    );
    if (data.total.ambiguous)
      summary.append(
        node(
          "p",
          "warning",
          `${data.total.ambiguous} ambiguous matches were kept separate. Review the listening history.`,
        ),
      );
    const list = node("ul");
    for (const file of data.files) {
      list.append(
        node(
          "li",
          "",
          `${file.filename}: ${file.added} added, ${file.reconciled} reconciled, ${file.invalid} invalid`,
        ),
      );
      for (const error of file.errors)
        list.append(node("li", "form-error", error));
    }
    summary.append(list);
    const link = node("a", "button secondary", "Explore your archive ↗");
    link.href = "#dashboard";
    summary.append(link);
    target.append(summary);
    target.hidden = false;
    await loadSpotify();
  } catch (e) {
    notice(
      `${e.message} Files completed before this error remain saved; retrying is safe.`,
      true,
    );
  } finally {
    button.disabled = false;
    button.textContent = "Import listening history ↗";
  }
});
$("credentials-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  $("save-credentials").disabled = true;
  try {
    await post("/api/spotify/credentials", {
      client_id: $("client-id").value,
      client_secret: $("client-secret").value,
    });
    $("credentials-form").reset();
    await loadSpotify();
    notice(
      "Spotify app saved. Choose Connect Spotify to authorize your account.",
    );
  } catch (e) {
    notice(e.message, true);
  } finally {
    $("save-credentials").disabled = false;
  }
});
action("connect-spotify", async () => {
  const data = await post("/api/spotify/connect");
  const url = new URL(data.url);
  if (url.origin !== "https://accounts.spotify.com")
    throw new Error("Unexpected Spotify authorization URL.");
  window.location.assign(url.href);
});
action("sync-spotify", async () => {
  const data = await post("/api/spotify/sync");
  await loadSpotify();
  notice(
    `Sync complete: ${data.added} new listens, ${data.reconciled} reconciled, ${data.duplicates} already present.`,
  );
});
action("disconnect-spotify", async () => {
  await post("/api/spotify/disconnect");
  await loadSpotify();
  renderNow(null);
  notice("Spotify disconnected. Your archive has been kept.");
});
action("copy-redirect", async () => {
  await navigator.clipboard.writeText($("redirect-uri").textContent);
  notice("Redirect URI copied.");
});
const lightQuery = window.matchMedia("(prefers-color-scheme: light)");
function applyTheme() {
  const preference = $("theme").value;
  document.documentElement.classList.toggle(
    "theme-light",
    preference === "light" || (preference === "auto" && lightQuery.matches),
  );
}
try {
  $("theme").value = localStorage.getItem("scrobbl-theme") || "auto";
} catch {}
applyTheme();
$("theme").addEventListener("change", () => {
  try {
    localStorage.setItem("scrobbl-theme", $("theme").value);
  } catch {}
  applyTheme();
});
lightQuery.addEventListener("change", applyTheme);
window.addEventListener("hashchange", () =>
  route().catch((e) => notice(e.message, true)),
);
(async () => {
  try {
    const data = await api("/api/session");
    setupRequired = data.setup_required;
    if (data.authenticated) {
      csrf = data.csrf;
      await showApp();
    } else showAuth();
  } catch (e) {
    $("loading").textContent = e.message;
  }
})();
