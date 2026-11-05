import { syncServerArtworkJobs, useArtworkActivity } from "./artwork-activity";
import { useDisclosureMotion } from "./disclosure";
import { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import type { Session } from "./api";
import { api, setCsrf } from "./api";
import type { Activity, Preferences, SpotifyStatus } from "./generated";
import { Activities, ActivityMenu } from "./activities";
import {
  ImportPage,
  Now,
  NowPlayingProvider,
  NowPlayingPage,
  LibraryToolsMenu,
} from "./library";
import { HistoryPage, Overview, RankingsPage, MusicPage } from "./consultation";
import { AskPage } from "./ask";
import { kinds } from "./selection";
import type { Kind } from "./selection";
import { Link, navigate, useLocation, useSearch } from "./navigation";
import {
  isBool,
  isWidth,
  useAppearance,
  useBrowserPreference,
} from "./preferences";
import { Settings } from "./settings";
import { SpotifyConnectReturn } from "./spotify-connect";
import { MusicEdit } from "./music-edit";
import { ReviewPage } from "./review";
import { Heading, Icon, number, Toasts, Wordmark } from "./ui";
import type { Style } from "./ui";
import "./style.css";
import "./dashboard.css";
import { Auth, UsernameSetup } from "./account";
import { AccountMenu } from "./account-settings";
import { SearchPage } from "./search";
import { StartupScreen } from "./startup-screen";

function Shell({
  logout,
  initialPreferences = null,
}: {
  logout: () => void;
  initialPreferences?: Preferences | null;
}) {
  useDisclosureMotion();
  const artworkWork = useArtworkActivity();
  const path = useLocation(),
    search = useSearch(),
    appearance = useAppearance();
  const [preferences, setPreferences] = useState<Preferences | null>(
      initialPreferences,
    ),
    [loadError, setLoadError] = useState("");
  const [status, setStatus] = useState<SpotifyStatus | null>(null),
    [statusError, setStatusError] = useState("");
  const [jobs, setJobs] = useState<Activity[]>([]),
    [jobsError, setJobsError] = useState("");
  const [jobsLoading, setJobsLoading] = useState(true);
  const [previewBoot, setPreviewBoot] = useState(
    () => new URLSearchParams(location.search).get("boot") === "preview",
  );
  const [collapsed, setCollapsed] = useBrowserPreference(
      "sidebar-collapsed",
      false,
      isBool,
    ),
    [width, setWidth] = useBrowserPreference("sidebar-width", 248, isWidth),
    [mobileOpen, setMobileOpen] = useState(false);
  const drag = useRef<{ x: number; width: number } | null>(null);
  const [mobile, setMobile] = useState(
    () => matchMedia("(max-width: 900px)").matches,
  );
  const sidebar = useRef<HTMLElement>(null);
  const menu = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    const params = new URLSearchParams(search);
    if (params.get("boot") === "preview") {
      setPreviewBoot(true);
    }
  }, [search]);

  useEffect(() => {
    const openPreview = () => setPreviewBoot(true);
    window.addEventListener("preview-startup-screen", openPreview);
    return () =>
      window.removeEventListener("preview-startup-screen", openPreview);
  }, []);

  useEffect(() => {
    const query = matchMedia("(max-width: 900px)");
    const change = () => setMobile(query.matches);
    query.addEventListener("change", change);
    return () => query.removeEventListener("change", change);
  }, []);

  useEffect(() => {
    if (!mobile || !mobileOpen) return;
    const prior = document.activeElement;
    const controls = () =>
      Array.from(
        sidebar.current?.querySelectorAll<HTMLElement>(
          "a[href],button:not([disabled])",
        ) || [],
      ).filter((element) => element.getClientRects().length > 0);
    controls()[0]?.focus();
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        setMobileOpen(false);
      }
      if (event.key === "Tab") {
        const items = controls(),
          first = items[0],
          last = items.at(-1);
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last?.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first?.focus();
        }
      }
    };
    document.addEventListener("keydown", key);
    return () => {
      document.removeEventListener("keydown", key);
      if (menu.current?.getClientRects().length) menu.current.focus();
      else if (prior instanceof HTMLElement) prior.focus();
    };
  }, [mobileOpen, mobile]);

  const alive = useRef(true);
  const lastCalendar = useRef<string | null>(null);
  const lastArtwork = useRef("");

  async function refresh() {
    await Promise.all([
      api
        .spotify()
        .then((data) => {
          if (alive.current) {
            setStatus(data);
            setStatusError("");
          }
        })
        .catch((e) => {
          if (alive.current) setStatusError(e.message);
        }),
      api
        .activities()
        .then(async (data) => {
          if (!alive.current) return;
          setJobs(data);
          setJobsLoading(false);
          setJobsError("");
          syncServerArtworkJobs(data);
          const artwork = JSON.stringify(
            data
              .filter((job) => job.kind === "artwork")
              .map((job) => [job.id, job.processed, job.state]),
          );
          if (artwork !== lastArtwork.current) {
            lastArtwork.current = artwork;
            window.dispatchEvent(new Event("artwork-cache-changed"));
          }
          const calendar = JSON.stringify(
            data
              .filter((j) => j.kind === "calendar")
              .map((j) => [j.id, j.state]),
          );
          if (lastCalendar.current === null) {
            lastCalendar.current = calendar;
          } else if (calendar !== lastCalendar.current) {
            lastCalendar.current = calendar;
            const p = await api.preferences();
            if (alive.current) setPreferences(p);
          }
        })
        .catch((e) => {
          if (alive.current) {
            setJobsError(e.message);
            setJobsLoading(false);
          }
        }),
    ]);
  }

  useEffect(() => {
    alive.current = true;
    if (!initialPreferences) {
      void api
        .preferences()
        .then((p) => {
          if (alive.current) {
            setPreferences(p);
            setLoadError("");
          }
        })
        .catch((e) => {
          if (alive.current) setLoadError(e.message);
        });
    }
    let polling = false;
    const poll = async () => {
      if (polling) return;
      polling = true;
      await refresh();
      polling = false;
    };
    void poll();
    const timer = setInterval(() => {
      if (!document.hidden) void poll();
    }, 3000);
    const wake = () => {
      if (!document.hidden) void poll();
    };
    document.addEventListener("visibilitychange", wake);
    return () => {
      alive.current = false;
      clearInterval(timer);
      document.removeEventListener("visibilitychange", wake);
    };
  }, []);

  useEffect(() => {
    setMobileOpen(false);
    window.scrollTo(0, 0);
  }, [path]);

  const page =
    path === "/" || path === "/dashboard" ? "dashboard" : path.split("/")[1];
  const settingsSection = [
    "account",
    "application",
    "general",
    "appearance",
    "connections",
    "images",
    "data",
    "api-keys",
  ].includes(path.split("/")[2])
    ? path.split("/")[2]
    : "general";
  const title =
    (
      {
        dashboard: "Overview",
        search: "Search",
        ask: "Ask",
        history: "Listening history",
        review: "Review duplicates",
        import: "Import history",
        settings: settingsSection === "api-keys" ? "API keys" : "Settings",
        activities: "Activities",
        rankings: "Rankings",
        music: "Music",
      } as Record<string, string>
    )[page] || "Page not found";

  useEffect(() => {
    document.title = `${title} · ${preferences?.name || "scrobbl.rs"}`;
  }, [title, preferences?.name]);

  if (!preferences) {
    return (
      <StartupScreen
        stage="preferences"
        detail="Loading instance preferences, timezone calendar, and custom layout…"
        error={loadError || undefined}
        onReload={() => location.reload()}
      />
    );
  }

  if (previewBoot) {
    return (
      <StartupScreen
        stage="ready"
        instanceName={preferences.name}
        preview
        onDismiss={() => {
          setPreviewBoot(false);
          const next = new URLSearchParams(location.search);
          if (next.has("boot")) {
            next.delete("boot");
            navigate(
              `${location.pathname}${next.size ? `?${next.toString()}` : ""}`,
            );
          }
        }}
      />
    );
  }

  const nav = [
    ["/dashboard", "dashboard", "Overview", "space_dashboard"],
    ["/search", "search", "Search", "search"],
    ["/ask", "ask", "Ask", "forum"],
    ["/history", "history", "Listening history", "history"],
    ["/rankings/artists", "rankings", "Rankings", "leaderboard"],
    ["/import", "import", "Import history", "file_upload"],
    ["/settings/general", "settings", "Settings", "settings"],
  ];

  const serverArtworkSync = artworkWork.serverSync;
  const showArtworkPill = !!serverArtworkSync || artworkWork.pending > 0;

  return (
    <NowPlayingProvider status={status} statusError={statusError}>
      <div
        className={`app-layout ${collapsed ? "collapsed" : ""} ${mobileOpen ? "mobile-open" : ""}`}
        style={{ "--sidebar-width": `${width}px` } as Style}
      >
        <a className="skip-link" href="#main">
          Skip to content
        </a>
        {mobileOpen && (
          <button
            className="sidebar-backdrop"
            aria-label="Close navigation"
            onClick={() => setMobileOpen(false)}
          />
        )}
        <aside
          ref={sidebar}
          inert={mobile && !mobileOpen}
          className="sidebar"
          id="navigation-panel"
        >
          <div className="sidebar-heading">
            <Link to="/dashboard" className="brand">
              <img src="/favicon.svg" alt="" width="30" height="30" />
              <span className="nav-copy">
                <Wordmark name={preferences.name} />
              </span>
            </Link>
            <button
              className="btn btn-icon icon-button desktop-collapse"
              aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
              aria-expanded={!collapsed}
              onClick={() => setCollapsed(!collapsed)}
            >
              <Icon>{collapsed ? "chevron_right" : "chevron_left"}</Icon>
            </button>
            <button
              className="btn btn-icon icon-button mobile-close"
              aria-label="Close navigation"
              onClick={() => setMobileOpen(false)}
            >
              <Icon>close</Icon>
            </button>
          </div>
          <p className="sidebar-section-label nav-copy">Your library</p>
          <nav aria-label="Main navigation">
            {nav.map(([to, key, label, icon]) => (
              <Link
                key={to}
                to={to}
                aria-label={label}
                className={
                  page === key || (page === "review" && key === "history")
                    ? "active"
                    : ""
                }
                aria-current={
                  page === key || (page === "review" && key === "history")
                    ? "page"
                    : undefined
                }
              >
                <Icon>{icon}</Icon>
                <span className="nav-copy">{label}</span>
                <span className="collapsed-label">{label}</span>
              </Link>
            ))}
          </nav>
          <div className="sidebar-bottom">
            <Now
              status={status}
              paused={(collapsed && !mobile) || (mobile && !mobileOpen)}
            />
          </div>
        </aside>
        {!collapsed && (
          <div
            className="sidebar-resizer"
            role="separator"
            tabIndex={0}
            aria-label="Sidebar width"
            aria-orientation="vertical"
            aria-controls="navigation-panel"
            aria-valuemin={216}
            aria-valuemax={360}
            aria-valuenow={width}
            onKeyDown={(e) => {
              if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(e.key))
                return;
              e.preventDefault();
              setWidth(
                e.key === "Home"
                  ? 216
                  : e.key === "End"
                    ? 360
                    : Math.max(
                        216,
                        Math.min(
                          360,
                          width + (e.key === "ArrowLeft" ? -16 : 16),
                        ),
                      ),
              );
            }}
            onPointerDown={(e) => {
              drag.current = { x: e.clientX, width };
              e.currentTarget.setPointerCapture(e.pointerId);
            }}
            onPointerMove={(e) => {
              if (drag.current)
                setWidth(
                  Math.max(
                    216,
                    Math.min(
                      360,
                      drag.current.width + e.clientX - drag.current.x,
                    ),
                  ),
                );
            }}
            onPointerUp={() => {
              drag.current = null;
            }}
            onPointerCancel={() => {
              drag.current = null;
            }}
          />
        )}
        <div className="main-area">
          <header className="topbar">
            <button
              ref={menu}
              className="btn btn-icon icon-button mobile-menu"
              aria-label="Open navigation"
              aria-expanded={mobileOpen}
              aria-controls="navigation-panel"
              onClick={() => setMobileOpen(true)}
            >
              <Icon>menu</Icon>
            </button>
            <span className="topbar-breadcrumb">Your library / {title}</span>
            <div className="topbar-actions">
              {showArtworkPill && (
                <Link
                  to="/activities"
                  className={`artwork-sync-pill ${serverArtworkSync?.paused ? "is-paused" : ""}`}
                  title="Open artwork sync activity"
                >
                  <Icon>
                    {serverArtworkSync?.paused ? "pause_circle" : "downloading"}
                  </Icon>
                  <span className="artwork-sync-pill-text">
                    {serverArtworkSync
                      ? serverArtworkSync.paused
                        ? "Artwork paused"
                        : serverArtworkSync.expected
                          ? `Artwork ${number.format(serverArtworkSync.processed)}/${number.format(serverArtworkSync.expected)}`
                          : "Syncing artwork"
                      : `Artwork (${number.format(artworkWork.pending)})`}
                  </span>
                </Link>
              )}
              <LibraryToolsMenu />
              <ActivityMenu
                jobs={jobs}
                work={artworkWork}
                error={jobsError}
                loading={jobsLoading}
              />
              <AccountMenu logout={logout} />
            </div>
          </header>
          <main id="main" tabIndex={-1} inert={mobile && mobileOpen}>
            {page === "search" && (
              <section className="page-container">
                <SearchPage />
              </section>
            )}
            <section className="page-container" hidden={page !== "ask"}>
              <AskPage preferences={preferences} />
            </section>
            <section className="page-container" hidden={page !== "dashboard"}>
              <Overview
                preferences={preferences}
                visible={page === "dashboard"}
                status={status}
              />
            </section>
            <section className="page-container" hidden={page !== "history"}>
              <HistoryPage
                preferences={preferences}
                visible={page === "history"}
              />
            </section>
            {page === "review" && (
              <section className="page-container">
                <ReviewPage preferences={preferences} refresh={refresh} />
              </section>
            )}
            <section className="page-container" hidden={page !== "import"}>
              <ImportPage
                refresh={refresh}
                visible={page === "import"}
                preferences={preferences}
              />
            </section>
            <section className="page-container" hidden={page !== "settings"}>
              <Settings
                section={settingsSection}
                preferences={preferences}
                saved={setPreferences}
                appearance={appearance}
                status={status}
                refresh={refresh}
                statusError={statusError}
                visible={page === "settings"}
              />
            </section>
            <section className="page-container" hidden={page !== "activities"}>
              <Activities
                loading={jobsLoading}
                jobs={jobs}
                artworkWork={artworkWork}
                error={jobsError}
                path={path}
                preferences={preferences}
                refresh={refresh}
              />
            </section>
            {page === "rankings" &&
              kinds.includes(path.split("/")[2] as Kind) && (
                <section className="page-container">
                  <RankingsPage
                    status={status}
                    preferences={preferences}
                    kind={path.split("/")[2] as Kind}
                    key={path}
                  />
                </section>
              )}
            {path === "/music/now-playing" && (
              <section className="page-container">
                <NowPlayingPage />
              </section>
            )}
            {page === "music" &&
              kinds.includes(path.split("/")[2] as Kind) &&
              path.split("/")[3] && (
                <section className="page-container">
                  {path.split("/")[4] === "edit" ? (
                    <MusicEdit
                      kind={path.split("/")[2] as Kind}
                      id={decodeURIComponent(path.split("/")[3])}
                      key={path}
                    />
                  ) : (
                    <MusicPage
                      preferences={preferences}
                      kind={path.split("/")[2] as Kind}
                      id={decodeURIComponent(path.split("/")[3])}
                      key={path}
                    />
                  )}
                </section>
              )}
            {![
              "rankings",
              "music",
              "dashboard",
              "search",
              "ask",
              "history",
              "review",
              "import",
              "settings",
              "activities",
            ].includes(page) && (
              <section className="page-container">
                <Heading title="Page not found" />
                <Link to="/dashboard" className="btn btn-secondary secondary">
                  Back to overview
                </Link>
              </section>
            )}
          </main>
          <footer>
            <div className="footer-meta">
              <a href="/licenses" target="_blank" rel="noreferrer">
                Licenses
              </a>
              <span className="footer-separator" aria-hidden="true">
                ·
              </span>
              <span>{preferences.name}</span>
            </div>
            <span className="footer-tagline">Made for the music you keep.</span>
          </footer>
        </div>
      </div>
    </NowPlayingProvider>
  );
}

function App({
  initialSession = null,
  initialPreferences = null,
  initialError = "",
}: {
  initialSession?: Session | null;
  initialPreferences?: Preferences | null;
  initialError?: string;
}) {
  const [session, setSession] = useState<Session | null>(initialSession),
    [bootPreferences, setBootPreferences] = useState<Preferences | null>(
      initialPreferences,
    ),
    [error, setError] = useState(initialError);

  async function reloadSession() {
    const data = await api.session();
    setCsrf(data.csrf);
    if (data.authenticated && !data.username_setup_required) {
      try {
        const prefs = await api.preferences();
        setBootPreferences(prefs);
      } catch {
        setBootPreferences(null);
      }
    } else {
      setBootPreferences(null);
    }
    setError("");
    setSession(data);
  }

  useEffect(() => {
    if (!initialSession) {
      void reloadSession().catch((e) =>
        setError(e instanceof Error ? e.message : String(e)),
      );
    }
    const expired = () =>
      void reloadSession().catch((e) =>
        setError(e instanceof Error ? e.message : String(e)),
      );
    window.addEventListener("session-expired", expired);
    return () => window.removeEventListener("session-expired", expired);
  }, []);

  return (
    <Toasts>
      {!session ? (
        <StartupScreen
          stage="session"
          error={error || undefined}
          onReload={() => location.reload()}
        />
      ) : session.authenticated ? (
        session.username_setup_required ? (
          <UsernameSetup done={reloadSession} />
        ) : (
          <Shell
            initialPreferences={bootPreferences}
            logout={() => {
              navigate("/dashboard");
              void reloadSession();
            }}
          />
        )
      ) : (
        <Auth
          setup={session.setup_required}
          usernameRequired={!!session.username_required}
          done={reloadSession}
        />
      )}
    </Toasts>
  );
}

async function mount() {
  const rootEl = document.getElementById("root")!;
  const statusEl = rootEl.querySelector(".pre-splash-status");
  const barEl = rootEl.querySelector<HTMLElement>(".pre-splash-bar");

  try {
    let res = await fetch("/api/boot-status", { cache: "no-store" });
    if (res.ok) {
      let data = (await res.json()) as { ready?: boolean; stage?: string };
      while (data.ready === false) {
        if (data.stage && statusEl) {
          statusEl.textContent =
            data.stage === "migrating" || data.stage === "opening_database"
              ? "Verifying SQLite WAL archive…"
              : data.stage === "starting_workers"
                ? "Starting background workers…"
                : "Starting scrobbl.rs…";
          if (barEl) {
            barEl.style.width =
              data.stage === "starting_workers"
                ? "84%"
                : data.stage === "migrating" ||
                    data.stage === "opening_database"
                  ? "58%"
                  : "32%";
          }
        }
        await new Promise((resolve) => setTimeout(resolve, 450));
        res = await fetch("/api/boot-status", { cache: "no-store" });
        if (res.ok) {
          data = (await res.json()) as { ready?: boolean; stage?: string };
        } else {
          break;
        }
      }
    }
  } catch {
    // Proceed to mount so React handles connectivity or error states
  }

  const spotifyReturn = location.pathname.match(
    /^\/spotify\/connect-result\/([a-f0-9]{64})$/,
  );
  if (spotifyReturn) {
    createRoot(rootEl).render(
      <SpotifyConnectReturn attempt={spotifyReturn[1]} />,
    );
    return;
  }

  let initialSession: Session | null = null;
  let initialPreferences: Preferences | null = null;
  let initialError = "";
  try {
    initialSession = await api.session();
    setCsrf(initialSession.csrf);
    if (
      initialSession.authenticated &&
      !initialSession.username_setup_required
    ) {
      try {
        initialPreferences = await api.preferences();
      } catch {
        // Shell will retry and surface loadError if needed
      }
    }
  } catch (e) {
    initialError = e instanceof Error ? e.message : String(e);
  }

  createRoot(rootEl).render(
    <App
      initialSession={initialSession}
      initialPreferences={initialPreferences}
      initialError={initialError}
    />,
  );
}

void mount();
