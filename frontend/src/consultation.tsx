import { SpotifyNotice } from "./spotify-status";
import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import type {
  Dashboard,
  History,
  Preferences,
  Ranking,
  SpotifyStatus,
  Totals,
} from "./generated";
import { api } from "./api";
import { useLoad } from "./load";
import { ListenList } from "./listens";
import { Now } from "./library";
import { MusicActions } from "./music-edit";
import { Link, navigate, useSearch } from "./navigation";
import { Cover, duration, ErrorText, Heading, Icon, number } from "./ui";
import type { Style } from "./ui";
import { periods } from "./preferences";
import {
  context,
  kinds,
  musicLink,
  scoped,
  useListPreferences,
  useSelection,
} from "./selection";
import type { Kind } from "./selection";
type Selection = ReturnType<typeof useSelection>;

function TimeHelp() {
  const id = useId();
  const button = useRef<HTMLButtonElement>(null);
  const [position, setPosition] = useState<Style>();
  const [pinned, setPinned] = useState(false),
    [hover, setHover] = useState(false),
    [focus, setFocus] = useState(false);
  const open = pinned || hover || focus;
  useLayoutEffect(() => {
    if (!open) return;
    function place() {
      if (!button.current) return;
      const rect = button.current.getBoundingClientRect();
      const margin = 16,
        gap = 8;
      const width = Math.min(360, window.innerWidth - margin * 2);
      const above = Math.max(
        0,
        Math.min(rect.top, window.innerHeight) - margin,
      );
      const below = Math.max(
        0,
        window.innerHeight - Math.max(0, rect.bottom) - margin,
      );
      // Keep the panel beside its trigger, so it cannot intercept a closing tap.
      setPosition({
        left: Math.max(
          margin,
          Math.min(rect.left, window.innerWidth - width - margin),
        ),
        width,
        maxHeight: Math.max(0, Math.max(above, below) - gap),
        ...(below >= above
          ? { top: Math.max(0, rect.bottom) + gap }
          : {
              bottom:
                window.innerHeight -
                Math.min(rect.top, window.innerHeight) +
                gap,
            }),
      });
    }
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [open]);
  return (
    <span
      className="time-help"
      onPointerEnter={(event) => setHover(event.pointerType === "mouse")}
      onPointerLeave={() => setHover(false)}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          setPinned(false);
          setHover(false);
          setFocus(false);
        }
      }}
    >
      <button
        ref={button}
        type="button"
        className="info-button"
        aria-label="How we calculate listening time"
        aria-expanded={open}
        aria-controls={id}
        onFocus={() => setFocus(true)}
        onBlur={() => setFocus(false)}
        onClick={() => {
          setPinned(!pinned);
          setFocus(false);
          setHover(false);
        }}
      >
        <span aria-hidden="true">i</span>
      </button>
      {open && (
        <span
          id={id}
          role="region"
          aria-label="How we calculate listening time"
          className="time-explanation"
          style={position}
        >
          <strong>How we calculate listening time</strong>
          <span>
            Actual time is playback time reported by the source, rather than the
            full track length.
          </span>
          <span>
            Spotify recent history supplies track length as an estimate. A
            four-minute track can add four estimated minutes without proving a
            complete listen. Skips and partial playback can make this estimate
            too high.
          </span>
          <span>
            Now-playing progress is a current position, not an accumulated
            measurement of listening time.
          </span>
          <span>
            A valid reconciliation with an extended export can replace the
            estimate with actual time. Source matching still needs verification.
          </span>
          <span>
            ListenBrainz playback time is actual when provided; full track
            length is only an estimate. Missing information remains unknown,
            with no invented duration.
          </span>
        </span>
      )}
    </span>
  );
}
function timeValue(value: {
  actual_ms: number;
  estimated_ms: number;
  unknown_durations: number;
}) {
  return value.actual_ms + value.estimated_ms === 0 &&
    value.unknown_durations > 0
    ? "Time unavailable"
    : duration(value.actual_ms + value.estimated_ms, true);
}
function Composition({
  value,
}: {
  value: { actual_ms: number; estimated_ms: number; unknown_durations: number };
}) {
  return (
    <small className="time-composition">
      {duration(value.actual_ms, true)} actual ·{" "}
      {duration(value.estimated_ms, true)} estimated
      {value.unknown_durations > 0 &&
        ` · ${number.format(value.unknown_durations)} with unknown duration`}
    </small>
  );
}
function Stats({ totals, ids = false }: { totals: Totals; ids?: boolean }) {
  return (
    <>
      <div className="stats-grid">
        <article className="panel stat">
          <span>Total listens</span>
          <strong id={ids ? "total-listens" : undefined}>
            {number.format(totals.listens)}
          </strong>
        </article>
        <article className="panel stat">
          <span>
            Listening time <TimeHelp />
          </span>
          <strong id={ids ? "total-time" : undefined}>
            {timeValue(totals)}
          </strong>
          <Composition value={totals} />
        </article>
        <article className="panel stat">
          <span>Different tracks</span>
          <strong id={ids ? "total-tracks" : undefined}>
            {number.format(totals.tracks)}
          </strong>
        </article>
      </div>
      {totals.unknown_durations > 0 && (
        <p className="warning">
          Missing durations limit the completeness of listening-time totals and
          rankings.
        </p>
      )}
    </>
  );
}
function PeriodFilters({
  selection,
  sort = true,
}: {
  selection: Selection;
  sort?: boolean;
}) {
  const { params, update } = selection;
  return (
    <div className="filters">
      <label>
        Period
        <select
          aria-label="Period"
          value={params.get("period") || "all"}
          onChange={(e) =>
            update({
              period: e.target.value,
              from: "",
              to: "",
              bucket: "",
              bucket_start: "",
            })
          }
        >
          {periods.map(([v, t]) => (
            <option key={v} value={v}>
              {t}
            </option>
          ))}
          <option value="custom">Custom dates</option>
        </select>
      </label>
      {params.get("period") === "custom" && (
        <>
          <label>
            From
            <input
              type="date"
              value={params.get("from") || ""}
              onChange={(e) =>
                update({ from: e.target.value, bucket: "", bucket_start: "" })
              }
            />
          </label>
          <label>
            Through
            <input
              type="date"
              value={params.get("to") || ""}
              onChange={(e) =>
                update({ to: e.target.value, bucket: "", bucket_start: "" })
              }
            />
          </label>
        </>
      )}
      {sort && (
        <label>
          Rank by
          <select
            value={params.get("sort") || "plays"}
            onChange={(e) => update({ sort: e.target.value })}
          >
            <option value="plays">Listens</option>
            <option value="time">Listening time</option>
          </select>
        </label>
      )}
      {["today", "week", "month", "year"].includes(
        params.get("period") || "",
      ) && (
        <span className="pill">Current period · incomplete, through now</span>
      )}
    </div>
  );
}
function Pager({
  total,
  page,
  pageSize,
  change,
}: {
  total: number;
  page: number;
  pageSize: number;
  change: (page: number) => void;
}) {
  return (
    <div className="pagination">
      <button
        className="secondary"
        disabled={page <= 1}
        onClick={() => change(page - 1)}
      >
        Previous
      </button>
      <span>
        Page {page} of {Math.max(1, Math.ceil(total / pageSize))} ·{" "}
        {number.format(total)} results
      </span>
      <button
        className="secondary"
        disabled={page * pageSize >= total}
        onClick={() => change(page + 1)}
      >
        Next
      </button>
    </div>
  );
}
function PageSize({
  list,
  changed,
}: {
  list: ReturnType<typeof useListPreferences>;
  changed: () => void;
}) {
  return (
    <label>
      Results per page
      <select
        value={list.size}
        onChange={(e) => {
          list.setSize(Number(e.target.value));
          changed();
        }}
      >
        {[10, 25, 50, 100].map((n) => (
          <option value={n} key={n}>
            {n}
          </option>
        ))}
      </select>
    </label>
  );
}
function RankingRows({
  items,
  kind,
  params,
  grid = false,
}: {
  items: Ranking[];
  kind: Kind;
  params: URLSearchParams;
  grid?: boolean;
}) {
  return items.length ? (
    <ol
      className={`${grid ? "music-grid" : "ranking-list"} category-${kind}`}
      data-ranking-kind={kind}
    >
      {items.map((item) => (
        <li
          className={grid ? "music-card" : "ranking-row"}
          key={item.id}
          value={item.position}
        >
          <span
            className="rank-number"
            aria-label={`Position ${item.position}`}
          >
            {item.position}
          </span>
          <Cover url={item.cover_url} large={grid} />
          <div className="rank-name">
            <Link to={musicLink(kind, item.id, params)}>{item.name}</Link>
            <small>
              {kind === "artists"
                ? item.subtitle?.endsWith(" tracks")
                  ? item.subtitle
                  : "Artist"
                : item.subtitle || (kind === "albums" ? "Album" : "Track")}
            </small>
          </div>
          <div className="rank-value">
            <strong className="rank-plays">
              {number.format(item.plays)} <small>listens</small>
            </strong>
            <span className="rank-time">{timeValue(item)}</span>
            <Composition value={item} />
          </div>
        </li>
      ))}
    </ol>
  ) : (
    <p className="empty">No {kind} with counted listens in this selection.</p>
  );
}
function RankingsPanel({
  kind,
  params,
  visible = true,
  showView = true,
  title,
  version = 0,
  standalone = false,
}: {
  kind: Kind;
  params: URLSearchParams;
  visible?: boolean;
  showView?: boolean;
  title?: string;
  version?: number;
  standalone?: boolean;
}) {
  const list = useListPreferences();
  const search = useSearch();
  const [draft, setDraft] = useState(params.get("q") || "");
  useEffect(() => setDraft(params.get("q") || ""), [search]);
  const pageKey = `${kind}_page`;
  const query = new URLSearchParams(params);
  query.set("page", params.get(pageKey) || params.get("page") || "1");
  query.set("page_size", String(list.size));
  const result = useLoad(
    () => api.rankings(kind, query),
    `${query}:${kind}:${version}`,
    visible,
  );
  const update = (patch: Record<string, string>, resetPages = true) => {
    const next = new URLSearchParams(location.search);
    if (resetPages)
      for (const key of [
        "page",
        "history_page",
        "artists_page",
        "albums_page",
        "tracks_page",
      ])
        next.set(key, "1");
    for (const [key, value] of Object.entries(patch)) {
      if (value) next.set(key, value);
      else next.delete(key);
    }
    // Preserve the URL and browser history for return navigation.
    navigate(`${location.pathname}?${next}`);
  };
  const reset = () =>
    update({
      page: "1",
      artists_page: "1",
      albums_page: "1",
      tracks_page: "1",
    });
  return (
    <article
      className={`panel padded ${standalone ? "rankings-panel" : ""}`}
      aria-label={`${kind} rankings`}
    >
      <div className="ranking-panel-heading">
        <h2>{standalone ? `Your ${kind}` : title || `Top ${kind}`}</h2>
        {result.data && (
          <span className="result-count">
            {number.format(result.data.total)} {kind}
          </span>
        )}
      </div>
      {showView && (
        <div className="filters">
          <form
            className="search-inline"
            onSubmit={(e) => {
              e.preventDefault();
              update({ q: draft, page: "1", [pageKey]: "1" });
            }}
          >
            <label>
              Search rankings
              <input
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                maxLength={500}
              />
            </label>
            <button className="secondary">Search</button>
          </form>
        </div>
      )}
      <div className="filters">
        <PageSize list={list} changed={reset} />
        {showView && (
          <div className="view-toggle" role="group" aria-label="Ranking view">
            <button
              className="secondary"
              aria-pressed={list.view === "list"}
              onClick={() => list.setView("list")}
            >
              <Icon>view_list</Icon>List
            </button>
            <button
              className="secondary"
              aria-pressed={list.view === "grid"}
              onClick={() => list.setView("grid")}
            >
              <Icon>grid_view</Icon>Grid
            </button>
          </div>
        )}
      </div>
      <ErrorText>{result.error}</ErrorText>
      {result.data ? (
        <>
          <RankingRows
            items={result.data.items}
            kind={kind}
            params={params}
            grid={(!showView && kind === "albums") || list.view === "grid"}
          />
          <Pager
            total={result.data.total}
            page={result.data.page}
            pageSize={result.data.page_size}
            change={(n) => update({ [pageKey]: String(n) }, false)}
          />
        </>
      ) : (
        !result.error && <p role="status">Loading rankings…</p>
      )}
      {kind === "artists" && (
        <p className="subtle">
          Each structured artist receives the whole listen and its full time.
          Artist totals can exceed the archive total.
        </p>
      )}
    </article>
  );
}
function periodLabel(start: string, grain: string, full = false) {
  const date = new Date(`${start}T00:00:00Z`);
  if (Number.isNaN(date.getTime())) return start;
  if (grain === "year") return String(date.getUTCFullYear());
  const label = new Intl.DateTimeFormat("en-GB", {
    timeZone: "UTC",
    ...(grain === "month"
      ? { month: "short", year: "numeric" }
      : {
          day: "numeric",
          month: "short",
          ...(full ? { year: "numeric" } : {}),
        }),
  }).format(date);
  return grain === "week" ? `Week of ${label}` : label;
}
const compactNumber = new Intl.NumberFormat("en-GB", {
  notation: "compact",
  maximumFractionDigits: 1,
});
function Chart({
  params,
  visible = true,
  update,
  version = 0,
}: {
  params: URLSearchParams;
  visible?: boolean;
  update: Selection["update"];
  version?: number;
}) {
  const result = useLoad(
    () => api.chart(params),
    `${params}:${version}`,
    visible,
  );
  const [inspected, setInspected] = useState<string | null>(null);
  const byTime = params.get("sort") === "time";
  const buckets = result.data?.buckets || [];
  const max = Math.max(
    1,
    ...buckets.map((b) => (byTime ? b.actual_ms + b.estimated_ms : b.listens)),
  );
  const inspectedBucket =
    buckets.find((b) => b.start === inspected) ||
    buckets.reduce<(typeof buckets)[number] | undefined>(
      (best, b) =>
        !best ||
        (byTime
          ? b.actual_ms + b.estimated_ms > best.actual_ms + best.estimated_ms
          : b.listens > best.listens)
          ? b
          : best,
      undefined,
    );
  return (
    <article className="panel padded activity-chart">
      <div className="panel-title">
        <h2>Listening activity</h2>
        <label>
          Group by
          <select
            value={params.get("grain") || result.data?.grain || "day"}
            onChange={(e) => update({ grain: e.target.value }, false)}
          >
            {["day", "week", "month", "year"].map((v) => (
              <option key={v} value={v}>
                {v[0].toUpperCase() + v.slice(1)}
              </option>
            ))}
          </select>
        </label>
      </div>
      <p className="subtle">
        Your archived listening over time. Select a bar to explore that period.
      </p>
      {inspectedBucket && (
        <div className="chart-readout" aria-live="polite" aria-atomic="true">
          <div>
            <small>{inspected ? "Period details" : "Most active period"}</small>
            <strong>
              {periodLabel(inspectedBucket.start, result.data!.grain, true)}
              {inspectedBucket.partial && (
                <span className="partial-label">Partial period</span>
              )}
            </strong>
            <span>
              {inspectedBucket.start} – {inspectedBucket.through}
            </span>
          </div>
          <div>
            <strong>
              {byTime
                ? timeValue(inspectedBucket)
                : number.format(inspectedBucket.listens)}
            </strong>
            <span>{byTime ? "listening time" : "listens"}</span>
            {byTime && <Composition value={inspectedBucket} />}
          </div>
        </div>
      )}
      {byTime && (
        <p className="chart-legend">
          <span className="actual-key" />
          Actual <span className="estimated-key" />
          Estimated · Missing time is unavailable
        </p>
      )}
      {result.data?.mixed_week_starts && (
        <p className="warning">
          This selection contains different assigned week starts. Weekly
          segments can overlap in calendar dates; each listen belongs to exactly
          one segment.
        </p>
      )}
      <ErrorText>{result.error}</ErrorText>
      {!!buckets.length && (
        <div
          className="chart-scroll"
          tabIndex={0}
          aria-label="Listening activity segments"
        >
          <div className="chart-bars">
            {buckets.map((b) => {
              const next = new URLSearchParams(params);
              next.set("bucket", result.data!.grain);
              next.set("bucket_start", b.start);
              next.delete("page");
              const value = byTime
                ? timeValue(b)
                : `${number.format(b.listens)} listens`;
              const label = `${b.start}${b.start === b.through ? "" : ` – ${b.through}`}: ${value}${b.partial ? " · partial segment" : ""}`;
              const height = byTime
                ? ((b.actual_ms + b.estimated_ms) / max) * 100
                : (b.listens / max) * 100;
              return (
                <Link
                  key={b.start}
                  to={`/history?${next}`}
                  className="chart-segment"
                  aria-label={label}
                  onMouseEnter={() => setInspected(b.start)}
                  onFocus={() => setInspected(b.start)}
                >
                  <span className="bar-space">
                    <span
                      className={`bar ${b.partial ? "partial" : ""} ${byTime && b.unknown_durations > 0 && !b.actual_ms && !b.estimated_ms ? "unavailable" : ""}`}
                      style={{ height: `${Math.max(2, height)}%` } as Style}
                    >
                      {byTime ? (
                        <>
                          <span
                            className="actual-bar"
                            style={{ flex: b.actual_ms } as Style}
                          />
                          <span
                            className="estimated-bar"
                            style={{ flex: b.estimated_ms } as Style}
                          />
                        </>
                      ) : (
                        <span
                          className="actual-bar"
                          style={{ flex: 1 } as Style}
                        />
                      )}
                    </span>
                  </span>
                  <small>
                    {periodLabel(b.start, result.data!.grain)}
                    {b.partial ? " *" : ""}
                  </small>
                  <small className="chart-bar-value">
                    {byTime ? timeValue(b) : compactNumber.format(b.listens)}
                  </small>
                </Link>
              );
            })}
          </div>
        </div>
      )}
      {result.data && !buckets.length && (
        <p className="empty">No activity in this selection.</p>
      )}
      {!result.data && !result.error && <p role="status">Loading activity…</p>}
      <p className="subtle">
        * Partial period. Empty bars mean no archived listens. Each listen
        belongs to its assigned date.
      </p>
    </article>
  );
}
export function Overview({
  preferences,
  visible,
  status,
}: {
  preferences: Preferences;
  visible: boolean;
  status: SpotifyStatus | null;
}) {
  const selection = useSelection(visible, preferences.initial_period),
    { params } = selection;
  const result = useLoad<Dashboard>(
    () => api.dashboard(params),
    `${params}:${preferences.version}`,
    visible,
    (status?.sync_seconds || 120) * 1000,
  );
  if (!visible) return null;
  return (
    <>
      <Heading
        eyebrow="YOUR PERSONAL LISTENING ARCHIVE"
        title="Your listening, at a glance"
      >
        The songs, the repeats, the moments. All yours.
      </Heading>
      <PeriodFilters selection={selection} />
      <ErrorText>{result.error}</ErrorText>
      {result.data ? (
        <>
          <Stats totals={result.data.totals} ids />
          <Now visible={visible} status={status} />
          <div className="rankings-grid">
            {kinds.map((kind) => (
              <article className="panel padded" key={kind}>
                <div className="panel-title">
                  <h2>Top {kind}</h2>
                  <Link to={`/rankings/${kind}?${context(params)}`}>
                    View all {kind}
                  </Link>
                </div>
                <RankingRows
                  items={result.data!.rankings[kind]}
                  kind={kind}
                  params={params}
                />
              </article>
            ))}
          </div>
          <Chart
            params={params}
            visible={visible}
            update={selection.update}
            version={preferences.version}
          />
          <article className="panel">
            <div className="panel-title padded">
              <h2>Recent listening</h2>
              <Link to={`/history?${context(params)}`}>View history</Link>
            </div>
            <ListenList items={result.data.recent} preferences={preferences} />
          </article>
        </>
      ) : (
        !result.error && <p role="status">Loading your archive…</p>
      )}
    </>
  );
}
export function RankingsPage({
  kind,
  preferences,
  status,
}: {
  kind: Kind;
  preferences: Preferences;
  status: SpotifyStatus | null;
}) {
  const selection = useSelection(true);
  return (
    <>
      <Heading title="Rankings" eyebrow="YOUR LISTENING RANKED">
        {kind === "artists"
          ? "Your most played artists, combining listens across all their tracks."
          : kind === "albums"
            ? "Your most played albums, combining listens across their tracks."
            : "Your most played tracks, each ranked by its own listens."}
      </Heading>
      <nav className="ranking-categories" aria-label="Ranking categories">
        {kinds.map((k) => (
          <Link
            to={`/rankings/${k}?${context(selection.params)}`}
            key={k}
            aria-current={kind === k ? "page" : undefined}
          >
            <Icon>
              {{ artists: "person", albums: "album", tracks: "music_note" }[k]}
            </Icon>
            <span>
              <strong>{k[0].toUpperCase() + k.slice(1)}</strong>
              <small>
                {
                  {
                    artists: "All tracks per artist",
                    albums: "All tracks per album",
                    tracks: "Individual songs",
                  }[k]
                }
              </small>
            </span>
          </Link>
        ))}
      </nav>
      <SpotifyNotice status={status} preferences={preferences} />
      <div className="ranking-period">
        <PeriodFilters selection={selection} />
        <span className="ranking-help">
          About listening time <TimeHelp />
        </span>
      </div>
      <RankingsPanel
        kind={kind}
        params={selection.params}
        version={preferences.version}
        standalone
      />
    </>
  );
}
function EntityFilter({
  kind,
  id,
  change,
}: {
  kind: "artists" | "albums";
  id: string;
  change: (id: string) => void;
}) {
  const [search, setSearch] = useState("");
  const options = useLoad(
    () => api.choices(kind, search),
    `${kind}:${search}`,
    !!search && !id,
  );
  const chosen = useLoad(
    () => api.music(kind, id, new URLSearchParams()),
    `${kind}:${id}`,
    !!id,
  );
  return (
    <div className="entity-filter">
      <label>
        {kind === "artists" ? "Artist" : "Album"}
        <input
          type="search"
          aria-label={kind === "artists" ? "Find artist" : "Find album"}
          placeholder={
            id ? chosen.data?.name || "Selected identity" : `Find ${kind}`
          }
          value={search}
          onChange={(e) => {
            setSearch(e.target.value);
            if (id) change("");
          }}
        />
      </label>
      {id && (
        <span className="pill">
          {chosen.data?.name || "Selected"}
          <button
            type="button"
            className="icon-button"
            aria-label={`Clear ${kind}`}
            onClick={() => {
              change("");
              setSearch("");
            }}
          >
            <Icon>close</Icon>
          </button>
        </span>
      )}
      <ErrorText>{options.error || (id ? chosen.error : "")}</ErrorText>
      {!id && search && options.data && (
        <ul className="entity-options" aria-label={`${kind} matches`}>
          {options.data.map((item) => (
            <li key={item.id}>
              <button
                type="button"
                className="secondary"
                onClick={() => {
                  change(item.id);
                  setSearch("");
                }}
              >
                {item.name}
                {item.subtitle ? ` · ${item.subtitle}` : ""}
              </button>
            </li>
          ))}
          {!options.data.length && <li>No matches</li>}
          {options.data.length === 50 && (
            <li>
              Showing 50 matches. Refine your search to narrow the choices.
            </li>
          )}
        </ul>
      )}
    </div>
  );
}
function TrackScope({ id, clear }: { id: string; clear: () => void }) {
  const result = useLoad(
    () => api.music("tracks", id, new URLSearchParams()),
    id,
  );
  return (
    <p className="pill">
      Track:{" "}
      <Link to={musicLink("tracks", id)}>
        {result.data?.name || "Selected track"}
      </Link>
      <button type="button" className="secondary" onClick={clear}>
        Clear track filter
      </button>
    </p>
  );
}
export function HistoryPage({
  preferences,
  visible,
  params: override,
  embedded = false,
}: {
  preferences: Preferences;
  visible: boolean;
  params?: URLSearchParams;
  embedded?: boolean;
}) {
  const selection = useSelection(visible),
    list = useListPreferences();
  const params = new URLSearchParams(override || selection.params);
  const [draft, setDraft] = useState(params.get("q") || "");
  const search = useSearch();
  useEffect(() => {
    if (visible) setDraft(params.get("q") || "");
  }, [search, visible]);
  params.set("page_size", String(list.size));
  if (embedded) params.set("page", params.get("history_page") || "1");
  const result = useLoad<History>(
    () => api.history(params),
    `${params}:${preferences.version}`,
    visible,
  );
  const change = (patch: Record<string, string>) =>
    selection.update({
      ...patch,
      history_page: "1",
      artists_page: "1",
      albums_page: "1",
      tracks_page: "1",
    });
  if (!visible) return null;
  return (
    <>
      {!embedded && (
        <Heading title="Listening history" eyebrow="EVERY LISTEN HAS A STORY">
          Search and combine period, artist, album and source filters.
        </Heading>
      )}
      {!embedded && params.get("track") && (
        <TrackScope
          id={params.get("track")!}
          clear={() => change({ track: "" })}
        />
      )}
      <form
        className="history-filters panel padded"
        aria-label="Listening history filters"
        onSubmit={(e) => {
          e.preventDefault();
          change({ q: draft });
        }}
      >
        <div className="history-filter-heading">
          <h2>Find your listens</h2>
          <span>Combine filters to narrow your history</span>
        </div>
        {!embedded && <PeriodFilters selection={selection} sort={false} />}
        <div className="history-filter-grid">
          <label className="history-search">
            Search
            <input
              id="history-q"
              value={draft}
              placeholder="Artist, album or track"
              maxLength={500}
              onChange={(e) => setDraft(e.target.value)}
            />
          </label>
          {!embedded && (
            <>
              <EntityFilter
                kind="artists"
                id={params.get("artist") || ""}
                change={(id) => change({ artist: id })}
              />
              <EntityFilter
                kind="albums"
                id={params.get("album") || ""}
                change={(id) => change({ album: id })}
              />
            </>
          )}
          <label>
            Source
            <select
              aria-label="Source"
              value={params.get("source") || ""}
              onChange={(e) => change({ source: e.target.value })}
            >
              <option value="">All sources</option>
              <option value="spotify_import">Spotify import</option>
              <option value="spotify_api">Spotify collection</option>
              <option value="listenbrainz">ListenBrainz</option>
            </select>
          </label>
          <label>
            Include
            <select
              id={embedded ? undefined : "history-status"}
              value={params.get("status") || "counted"}
              onChange={(e) => change({ status: e.target.value })}
            >
              {[
                ["counted", "Counted listens"],
                ["all", "All records"],
                ["short", "Below threshold"],
                ["actual", "Actual time"],
                ["estimated", "Estimated / unknown time"],
                ["review", "Possible duplicates"],
              ].map(([v, t]) => (
                <option key={v} value={v}>
                  {t}
                </option>
              ))}
            </select>
          </label>
          <label className="checkbox-label">
            <input
              type="checkbox"
              checked={params.get("status") === "all"}
              onChange={(e) =>
                change({ status: e.target.checked ? "all" : "counted" })
              }
            />
            Show listens below threshold too
          </label>
          <PageSize list={list} changed={() => change({})} />
        </div>
        <div className="history-filter-actions">
          <button className="primary">
            <Icon>search</Icon>Search
          </button>
          <button
            type="button"
            className="secondary"
            onClick={() => {
              setDraft("");
              change({
                q: "",
                source: "",
                status: "",
                artist: "",
                album: "",
                track: "",
                bucket: "",
                bucket_start: "",
                from: "",
                to: "",
                period: "all",
              });
            }}
          >
            Reset
          </button>
        </div>
      </form>
      {params.get("bucket") && (
        <div className="segment-filter">
          <Icon>date_range</Icon>
          <span>
            <small>Chart segment:</small>
            <strong>
              {periodLabel(
                params.get("bucket_start") || "",
                params.get("bucket") || "",
                true,
              )}
            </strong>
          </span>
          <button
            className="icon-button"
            aria-label="Clear segment"
            onClick={() => change({ bucket: "", bucket_start: "" })}
          >
            <Icon>close</Icon>
          </button>
        </div>
      )}
      <p className="subtle">
        Listens below 30 seconds of actual playback remain archived and are
        excluded from valid-listen statistics. Estimated or unknown time alone
        does not exclude a listen.
      </p>
      <ErrorText>{result.error}</ErrorText>
      {result.data ? (
        <article className="panel">
          <div className="panel-title padded">
            <h2 id={embedded ? undefined : "history-total"}>
              {number.format(result.data.total)} records
            </h2>
          </div>
          <ListenList
            id={embedded ? undefined : "history-list"}
            items={result.data.items}
            preferences={preferences}
          />
          <Pager
            total={result.data.total}
            page={result.data.page}
            pageSize={result.data.page_size}
            change={(page) =>
              selection.update(
                { [embedded ? "history_page" : "page"]: String(page) },
                false,
              )
            }
          />
        </article>
      ) : (
        !result.error && <p role="status">Loading listening history…</p>
      )}
    </>
  );
}
export function MusicPage({
  preferences,
  kind,
  id,
}: {
  preferences: Preferences;
  kind: Kind;
  id: string;
}) {
  const selection = useSelection(true),
    params = scoped(selection.params, kind, id);
  const result = useLoad(
    () => api.music(kind, id, params),
    `${kind}:${id}:${params}:${preferences.version}`,
  );
  return (
    <>
      <PeriodFilters selection={selection} />
      <ErrorText>{result.error}</ErrorText>
      {result.data ? (
        <>
          <div className="music-heading">
            <Cover url={result.data.cover_url} large />
            <Heading
              title={result.data.name}
              eyebrow={kind.slice(0, -1).toUpperCase()}
            >
              {result.data.artists.map((a, i) => (
                <span key={a.id}>
                  {i > 0 && ", "}
                  <Link to={musicLink("artists", a.id, params)}>{a.name}</Link>
                </span>
              ))}
              {result.data.album_id && (
                <>
                  {" "}
                  ·{" "}
                  <Link to={musicLink("albums", result.data.album_id, params)}>
                    {result.data.album}
                  </Link>
                </>
              )}
            </Heading>
          </div>
          <MusicActions kind={kind} id={id} />
          <Stats totals={result.data.totals} />
          <Chart
            params={params}
            update={selection.update}
            version={preferences.version}
          />
          {kind === "artists" && (
            <RankingsPanel
              kind="albums"
              params={params}
              showView={false}
              version={preferences.version}
              title="Albums in this period"
            />
          )}
          {kind !== "tracks" && (
            <RankingsPanel
              kind="tracks"
              params={params}
              title="Tracks in this period"
              version={preferences.version}
            />
          )}
          <h2>Listening history</h2>
          <HistoryPage
            preferences={preferences}
            visible
            params={params}
            embedded
          />
        </>
      ) : (
        !result.error && <p role="status">Loading music page…</p>
      )}
    </>
  );
}
