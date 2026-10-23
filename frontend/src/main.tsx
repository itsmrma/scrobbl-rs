import { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import type { Session } from "./api";
import { api, setCsrf } from "./api";
import type { Activity, Preferences, SpotifyStatus } from "./generated";
import { Activities, activityName, isActive } from "./activities";
import { ImportPage, Now, NowPlayingProvider } from "./library";
import { HistoryPage, Overview, RankingsPage, MusicPage } from "./consultation";
import { kinds } from "./selection";
import type { Kind } from "./selection";
import { Link, navigate, useLocation } from "./navigation";
import {
  isBool,
  isWidth,
  useAppearance,
  useBrowserPreference,
} from "./preferences";
import { Settings } from "./settings";
import { SpotifyConnectReturn } from "./spotify-connect";
import { MusicEdit } from "./music-edit";
import {
  ArtworkContext,
  ErrorText,
  Heading,
  Icon,
  Toasts,
  useToast,
  Wordmark,
} from "./ui";
import type { Style } from "./ui";
import "./style.css";
import { Auth, UsernameSetup } from "./account";

function Shell({ logout }: { logout: () => void }) {
  const path = useLocation(),
    appearance = useAppearance(),
    toast = useToast();
  const [preferences, setPreferences] = useState<Preferences | null>(null),
    [loadError, setLoadError] = useState("");
  const [status, setStatus] = useState<SpotifyStatus | null>(null),
    [statusError, setStatusError] = useState("");
  const [jobs, setJobs] = useState<Activity[]>([]),
    [jobsError, setJobsError] = useState("");
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
      // WebKit does not focus a button on pointer activation. Return explicitly
      // to the drawer trigger rather than whichever control happened to precede it.
      if (menu.current?.getClientRects().length) menu.current.focus();
      else if (prior instanceof HTMLElement) prior.focus();
    };
  }, [mobileOpen, mobile]);
  const alive = useRef(true);
  const lastCalendar = useRef("");
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
          setJobsError("");
          const calendar = JSON.stringify(
            data
              .filter((j) => j.kind === "calendar")
              .map((j) => [j.id, j.state]),
          );
          if (calendar !== lastCalendar.current) {
            lastCalendar.current = calendar;
            const p = await api.preferences();
            if (alive.current) setPreferences(p);
          }
        })
        .catch((e) => {
          if (alive.current) setJobsError(e.message);
        }),
    ]);
  }
  useEffect(() => {
    alive.current = true;
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
    "general",
    "appearance",
    "connections",
    "data",
    "api-keys",
  ].includes(path.split("/")[2])
    ? path.split("/")[2]
    : "general";
  const title =
    (
      {
        dashboard: "Overview",
        history: "Listening history",
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
  const activeJobs = jobs.filter(isActive);
  if (!preferences)
    return (
      <main className="loading">
        <p role="status">Opening your archive…</p>
        <ErrorText>{loadError}</ErrorText>
        {loadError && (
          <button className="secondary" onClick={() => location.reload()}>
            Reload
          </button>
        )}
      </main>
    );
  const nav = [
    ["/dashboard", "dashboard", "Overview", "space_dashboard"],
    ["/history", "history", "Listening history", "history"],
    ["/rankings/artists", "rankings", "Rankings", "leaderboard"],
    ["/import", "import", "Import history", "file_upload"],
    ["/settings/general", "settings", "Settings", "settings"],
  ];
  return (
    <NowPlayingProvider status={status}>
      <ArtworkContext
        value={{
          connected: !!status?.connected,
          retryAt: status?.retry_at_ms || null,
        }}
      >
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
                className="icon-button desktop-collapse"
                aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
                aria-expanded={!collapsed}
                onClick={() => setCollapsed(!collapsed)}
              >
                <Icon>{collapsed ? "chevron_right" : "chevron_left"}</Icon>
              </button>
              <button
                className="icon-button mobile-close"
                aria-label="Close navigation"
                onClick={() => setMobileOpen(false)}
              >
                <Icon>close</Icon>
              </button>
            </div>
            <p className="eyebrow nav-copy">YOUR LIBRARY</p>
            <nav aria-label="Main navigation">
              {nav.map(([to, key, label, icon]) => (
                <Link
                  key={to}
                  to={to}
                  aria-label={label}
                  className={page === key ? "active" : ""}
                  aria-current={page === key ? "page" : undefined}
                >
                  <Icon>{icon}</Icon>
                  <span className="nav-copy">{label}</span>
                  <span className="collapsed-label">{label}</span>
                </Link>
              ))}
            </nav>
            <div className="sidebar-activities">
              <Link
                to="/activities"
                className="activity-link"
                aria-label={`Activities: ${activeJobs.length} active`}
              >
                <Icon>pending_actions</Icon>
                <span className="nav-copy">
                  Activities <span className="pill">{activeJobs.length}</span>
                </span>
                <span className="collapsed-label">
                  Activities: {activeJobs.length} active
                </span>
                {collapsed && (
                  <span className="activity-count" aria-hidden="true">
                    {activeJobs.length}
                  </span>
                )}
              </Link>
              <div className="nav-copy">
                <p className="subtle" role="status">
                  {jobsError ||
                    (activeJobs.length
                      ? `${activeJobs.length} in progress`
                      : "No activities running")}
                </p>
                {activeJobs.slice(0, 3).map((job) => (
                  <Link
                    to={`/activities/${job.id}`}
                    key={job.id}
                    className="sidebar-job"
                  >
                    <strong>{activityName(job)}</strong>
                    <small>
                      {job.state} · {job.processed} processed
                    </small>
                  </Link>
                ))}
                {jobs.some((j) => j.state === "failed") && (
                  <Link to="/activities" className="warning">
                    Review failed activities
                  </Link>
                )}
              </div>
            </div>
            <div className="sidebar-bottom">
              <Now status={status} compact />
              <button
                className="secondary"
                aria-label="Sign out"
                onClick={async () => {
                  try {
                    await api.logout();
                    setCsrf(null);
                    logout();
                  } catch (e) {
                    toast(e instanceof Error ? e.message : String(e), true);
                  }
                }}
              >
                <Icon>logout</Icon>
                <span className="nav-copy">Sign out</span>
              </button>
              <p className="subtle nav-copy">Made for the music you keep.</p>
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
                className="icon-button mobile-menu"
                aria-label="Open navigation"
                aria-expanded={mobileOpen}
                aria-controls="navigation-panel"
                onClick={() => setMobileOpen(true)}
              >
                <Icon>menu</Icon>
              </button>
              <span>Your library / {title}</span>
              <Link to="/activities" className="mobile-activities">
                <Icon>pending_actions</Icon>Activities ({activeJobs.length})
              </Link>
              <span className="private-label">
                <Icon>lock</Icon>Private archive
              </span>
            </header>
            <main id="main" tabIndex={-1} inert={mobile && mobileOpen}>
              <section hidden={page !== "dashboard"}>
                <Overview
                  preferences={preferences}
                  visible={page === "dashboard"}
                  status={status}
                />
              </section>
              <section hidden={page !== "history"}>
                <HistoryPage
                  preferences={preferences}
                  visible={page === "history"}
                />
              </section>
              <section hidden={page !== "import"}>
                <ImportPage refresh={refresh} />
              </section>
              <section hidden={page !== "settings"}>
                <Settings
                  section={settingsSection}
                  preferences={preferences}
                  saved={setPreferences}
                  appearance={appearance}
                  status={status}
                  refresh={refresh}
                  statusError={statusError}
                />
              </section>
              <section hidden={page !== "activities"}>
                <Activities
                  jobs={jobs}
                  error={jobsError}
                  path={path}
                  preferences={preferences}
                  refresh={refresh}
                />
              </section>
              {page === "rankings" &&
                kinds.includes(path.split("/")[2] as Kind) && (
                  <RankingsPage
                    status={status}
                    preferences={preferences}
                    kind={path.split("/")[2] as Kind}
                    key={path}
                  />
                )}
              {page === "music" &&
                kinds.includes(path.split("/")[2] as Kind) &&
                path.split("/")[3] &&
                (path.split("/")[4] === "edit" ? (
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
                ))}
              {![
                "rankings",
                "music",
                "dashboard",
                "history",
                "import",
                "settings",
                "activities",
              ].includes(page) && (
                <>
                  <Heading title="Page not found" />
                  <Link to="/dashboard">Back to overview</Link>
                </>
              )}
            </main>
            <footer>
              <a href="/licenses" target="_blank" rel="noreferrer">
                Licenses
              </a>
              {" · "}
              {preferences.name}
              <span>Made for the music you keep.</span>
            </footer>
          </div>
        </div>
      </ArtworkContext>
    </NowPlayingProvider>
  );
}
function App() {
  const [session, setSession] = useState<Session | null>(null),
    [error, setError] = useState("");
  useEffect(() => {
    void api
      .session()
      .then((data) => {
        setCsrf(data.csrf);
        setSession(data);
      })
      .catch((e) => setError(e.message));
    const expired = () =>
      void api
        .session()
        .then((data) => {
          setCsrf(data.csrf);
          setSession(data);
        })
        .catch((e) => setError(e.message));
    window.addEventListener("session-expired", expired);
    return () => window.removeEventListener("session-expired", expired);
  }, []);
  async function reloadSession() {
    const data = await api.session();
    setCsrf(data.csrf);
    setSession(data);
  }
  return (
    <Toasts>
      {session ? (
        session.authenticated ? (
          session.username_setup_required ? (
            <UsernameSetup done={reloadSession} />
          ) : (
            <Shell
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
        )
      ) : (
        <main className="loading">
          <p>Opening your archive…</p>
          <ErrorText>{error}</ErrorText>
        </main>
      )}
    </Toasts>
  );
}
const spotifyReturn = location.pathname.match(
  /^\/spotify\/connect-result\/([a-f0-9]{64})$/,
);
createRoot(document.getElementById("root")!).render(
  spotifyReturn ? <SpotifyConnectReturn attempt={spotifyReturn[1]} /> : <App />,
);
