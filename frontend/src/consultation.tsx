import { Select } from "./select";
import { SpotifyNotice } from "./spotify-status";
import {
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type {
  Dashboard,
  DashboardLayout,
  History,
  Preferences,
  Ranking,
  SpotifyStatus,
  Totals,
  WidgetConfig,
  MusicPage as MusicPageData,
} from "./generated";
import { api } from "./api";
import { useLoad } from "./load";
import { ListenList } from "./listens";
import { Collage } from "./collage";
import { ArtworkCredits } from "./image-credit";
import { compactBuckets } from "./chart-buckets";
import { MusicActions } from "./music-edit";
import {
  ListeningPatterns,
  ListeningPatternsWidget,
} from "./listening-patterns";
import { NowPlayingWidget } from "./library";
import { BackButton, Link, navigate, useSearch } from "./navigation";
import { Cover, duration, ErrorText, Heading, Icon, number } from "./ui";
import type { Style } from "./ui";
import {
  ChartSkeleton,
  ListenListSkeleton,
  MusicPageSkeleton,
  RankingsSkeleton,
  SkeletonBlock,
  SkeletonList,
} from "./skeleton";
import {
  periods,
  periodBadgeLabel,
  isBool,
  useBrowserPreference,
} from "./preferences";
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
      onPointerOver={(event) => setHover(event.pointerType === "mouse")}
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
function Stats({
  totals,
  ids = false,
  periodOverride,
  loading = false,
  embedded = false,
}: {
  totals: Totals;
  loading?: boolean;
  embedded?: boolean;
  ids?: boolean;
  periodOverride?: string | null;
}) {
  return (
    <>
      <div className="stats-grid">
        <article className={embedded ? "stat" : "panel stat"}>
          <span>
            Total listens
            {periodOverride && (
              <span
                className="pill widget-period-badge"
                data-period={periodOverride}
              >
                {periodBadgeLabel(periodOverride)}
              </span>
            )}
          </span>
          <strong id={ids ? "total-listens" : undefined}>
            {loading ? (
              <SkeletonBlock width="60%" height={34} />
            ) : (
              number.format(totals.listens)
            )}
          </strong>
        </article>
        <article className={embedded ? "stat" : "panel stat"}>
          <span>
            Listening time <TimeHelp />
          </span>
          <strong id={ids ? "total-time" : undefined}>
            {loading ? (
              <SkeletonBlock width="60%" height={34} />
            ) : (
              timeValue(totals)
            )}
          </strong>
          {loading ? (
            <small className="time-composition">
              <SkeletonBlock width="80%" height={14} />
            </small>
          ) : (
            <Composition value={totals} />
          )}
        </article>
        <article className={embedded ? "stat" : "panel stat"}>
          <span>Different tracks</span>
          <strong id={ids ? "total-tracks" : undefined}>
            {loading ? (
              <SkeletonBlock width="60%" height={34} />
            ) : (
              number.format(totals.tracks)
            )}
          </strong>
        </article>
      </div>
      {(embedded || totals.unknown_durations > 0) && (
        <p className={embedded ? "warning widget-totals-note" : "warning"}>
          {totals.unknown_durations > 0 && (
            <>
              Missing durations limit the completeness of listening-time totals
              and rankings.
            </>
          )}
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
        <Select
          aria-label="Period"
          value={params.get("period") || "all"}
          onChange={(value) =>
            update({
              period: value,
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
        </Select>
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
        <label className="period-rank-filter">
          Rank by
          <Select
            aria-label="Rank by"
            value={params.get("sort") || "plays"}
            onChange={(value) => update({ sort: value })}
          >
            <option value="plays">Listens</option>
            <option value="time">Listening time</option>
          </Select>
        </label>
      )}
      {["today", "week", "month", "year"].includes(
        params.get("period") || "",
      ) && (
        <span className="period-notice">
          Current period · incomplete, through now
        </span>
      )}
    </div>
  );
}
function Pager({
  total,
  page,
  pageSize,
  change,
  list,
  resize,
}: {
  total: number;
  page: number;
  pageSize: number;
  change: (page: number) => void;
  list: ReturnType<typeof useListPreferences>;
  resize: () => void;
}) {
  return (
    <div className="list-footer">
      <PageSize list={list} changed={resize} />
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
          {number.format(total)} {total === 1 ? "result" : "results"}
        </span>
        <button
          className="secondary"
          disabled={page * pageSize >= total}
          onClick={() => change(page + 1)}
        >
          Next
        </button>
      </div>
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
    <label className="page-size">
      Results per page
      <Select
        aria-label="Results per page"
        value={list.size}
        onChange={(value) => {
          list.setSize(Number(value));
          changed();
        }}
      >
        {[10, 25, 50, 100].map((n) => (
          <option value={n} key={n}>
            {n}
          </option>
        ))}
      </Select>
    </label>
  );
}
function RankingRows({
  items,
  kind,
  params,
  grid = false,
  cardSize = 140,
  bars = false,
}: {
  items: Ranking[];
  kind: Kind;
  params: URLSearchParams;
  grid?: boolean;
  cardSize?: number;
  bars?: boolean;
}) {
  const byTime = params.get("sort") === "time";
  const Plays = byTime ? "span" : "strong";
  const Time = byTime ? "strong" : "span";
  const metric = (row: Ranking) => (byTime ? row.listened_ms : row.plays);
  const maximum = Math.max(1, ...items.map(metric));
  return items.length ? (
    <ol
      className={`${grid ? "music-grid" : "ranking-list"} category-${kind}`}
      style={{ "--music-card-size": `${cardSize}px` } as Style}
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
            <Link
              className="music-item-link"
              to={musicLink(kind, item.id, params)}
            >
              {item.name}
            </Link>
            <small>
              {kind === "artists"
                ? item.subtitle?.endsWith(" tracks")
                  ? `${item.subtitle.replace(" tracks", "")} different tracks listened`
                  : item.subtitle
                    ? `1 track listened (${item.subtitle})`
                    : "Artist"
                : item.subtitle || (kind === "albums" ? "Album" : "Track")}
            </small>
          </div>
          {bars && (
            <span className="ranking-meter" aria-hidden="true">
              <span
                style={{
                  width: (100 * metric(item)) / maximum + "%",
                }}
              />
            </span>
          )}
          <div className="rank-value">
            <span className="rank-metrics">
              <Plays className="rank-plays">
                {number.format(item.plays)} <small>listens</small>
              </Plays>
              <span className="rank-separator" aria-hidden="true">
                ·
              </span>
              <Time className="rank-time">{timeValue(item)}</Time>
            </span>
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
  scrolling = false,
}: {
  kind: Kind;
  params: URLSearchParams;
  visible?: boolean;
  showView?: boolean;
  title?: string;
  version?: number;
  standalone?: boolean;
  scrolling?: boolean;
}) {
  const list = useListPreferences(kind === "albums" ? "grid" : "list", kind);
  const cardSizes = [100, 140, 220];
  const [savedCardSize, setCardSize] = useBrowserPreference(
    `card-size-${kind}`,
    140,
    (value): value is number =>
      typeof value === "number" && [100, 140, 180, 220].includes(value),
  );
  const cardSize = savedCardSize === 180 ? 220 : savedCardSize;
  useEffect(() => {
    if (savedCardSize === 180) setCardSize(220);
  }, [savedCardSize, setCardSize]);
  const [collage, setCollage] = useState(false);
  const search = useSearch();
  const [draft, setDraft] = useState(params.get("q") || "");
  useEffect(() => setDraft(params.get("q") || ""), [search]);
  const pageKey = `${kind}_page`;
  const query = new URLSearchParams(params);
  query.set(
    "page",
    scrolling ? "1" : params.get(pageKey) || params.get("page") || "1",
  );
  query.set("page_size", scrolling ? "100" : String(list.size));
  const result = useLoad(
    async () => {
      const first = await api.rankings(kind, query);
      if (!scrolling) return first;
      const items = [...first.items];
      for (let page = 2; items.length < first.total; page++) {
        const next = new URLSearchParams(query);
        next.set("page", String(page));
        const batch = await api.rankings(kind, next);
        if (!batch.items.length) break;
        items.push(...batch.items);
      }
      return { ...first, items };
    },
    `rankings:${scrolling ? "scroll" : "page"}:${query}:${kind}:${version}`,
    visible,
    undefined,
    true,
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
      className={`panel padded ranking-panel ${standalone ? "rankings-panel" : ""} ${scrolling ? "scrolling-rankings" : ""}`}
      aria-label={`${kind} rankings`}
    >
      <div className="ranking-panel-heading">
        <h2>{standalone ? `Your ${kind}` : title || `Top ${kind}`}</h2>
        {result.data && (
          <span className="result-count">
            {number.format(result.data.total)}{" "}
            {result.data.total === 1 ? kind.slice(0, -1) : kind}
          </span>
        )}
      </div>
      {showView && (
        <div className="ranking-toolbar">
          <form
            className="search-inline"
            onSubmit={(event) => {
              event.preventDefault();
              update({ q: draft, page: "1", [pageKey]: "1" });
            }}
          >
            <input
              aria-label="Search rankings"
              placeholder="Search rankings"
              value={draft}
              onChange={(event) => setDraft(event.target.value)}
              maxLength={500}
            />
            <button className="secondary" aria-label="Search">
              <Icon>search</Icon>
            </button>
          </form>
          <div className="view-toggle" role="group" aria-label="Ranking view">
            <button
              className="secondary"
              aria-pressed={!collage && list.view === "list"}
              onClick={() => {
                list.setView("list");
                setCollage(false);
              }}
            >
              <Icon>view_list</Icon>List
            </button>
            <button
              className="secondary"
              aria-pressed={!collage && list.view === "grid"}
              onClick={() => {
                list.setView("grid");
                setCollage(false);
              }}
            >
              <Icon>grid_view</Icon>Grid
            </button>
            {kind !== "tracks" && (
              <button
                className="secondary collage-toggle"
                aria-pressed={collage}
                onClick={() => setCollage(!collage)}
              >
                <Icon>dashboard</Icon>Collage
              </button>
            )}
          </div>
          <div
            className={`card-size-control ${collage || list.view !== "grid" ? "inactive" : ""}`}
            aria-hidden={collage || list.view !== "grid"}
            inert={collage || list.view !== "grid"}
            role="group"
            aria-label="Card size"
          >
            <button
              className="icon-button"
              aria-label="Smaller cards"
              disabled={cardSize === 100}
              onClick={() =>
                setCardSize(
                  cardSizes[Math.max(0, cardSizes.indexOf(cardSize) - 1)],
                )
              }
            >
              <Icon>remove</Icon>
            </button>
            <output aria-label="Card size">
              {cardSize === 100
                ? "Small"
                : cardSize === 140
                  ? "Medium"
                  : "Large"}
            </output>
            <button
              className="icon-button"
              aria-label="Larger cards"
              disabled={cardSize === 220}
              onClick={() =>
                setCardSize(
                  cardSizes[
                    Math.min(
                      cardSizes.length - 1,
                      cardSizes.indexOf(cardSize) + 1,
                    )
                  ],
                )
              }
            >
              <Icon>add</Icon>
            </button>
          </div>
        </div>
      )}
      <ErrorText>{result.error}</ErrorText>
      {result.data ? (
        <>
          {collage && kind !== "tracks" ? (
            <Collage items={result.data.items} kind={kind} params={params} />
          ) : (
            <div className="ranking-results">
              <RankingRows
                cardSize={cardSize}
                items={result.data.items}
                kind={kind}
                params={params}
                grid={(!showView && kind === "albums") || list.view === "grid"}
              />
            </div>
          )}
          {!scrolling && (
            <Pager
              list={list}
              resize={reset}
              total={result.data.total}
              page={result.data.page}
              pageSize={result.data.page_size}
              change={(n) => update({ [pageKey]: String(n) }, false)}
            />
          )}
        </>
      ) : (
        !result.error && (
          <RankingsSkeleton
            grid={
              collage ||
              (!showView && kind === "albums") ||
              list.view === "grid"
            }
          />
        )
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
const periodFormatters = new Map<string, Intl.DateTimeFormat>();
function periodLabel(start: string, grain: string, full = false) {
  const date = new Date(`${start}T00:00:00Z`);
  if (Number.isNaN(date.getTime())) return start;
  if (grain === "year") return String(date.getUTCFullYear());
  const key = grain === "month" ? "month" : full ? "day:full" : "day";
  let formatter = periodFormatters.get(key);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat("en-GB", {
      timeZone: "UTC",
      ...(grain === "month"
        ? { month: "short", year: "numeric" }
        : {
            day: "numeric",
            month: "short",
            ...(full ? { year: "numeric" } : {}),
          }),
    });
    periodFormatters.set(key, formatter);
  }
  const label = formatter.format(date);
  return grain === "week" ? `Week of ${label}` : label;
}
const compactNumber = new Intl.NumberFormat("en-GB", {
  notation: "compact",
  maximumFractionDigits: 1,
});
function Chart({
  params,
  visible = true,
  version = 0,
  periodOverride,
}: {
  params: URLSearchParams;
  visible?: boolean;
  version?: number;
  periodOverride?: string | null;
}) {
  const chart = useRef<HTMLElement>(null);
  const [barLimit, setBarLimit] = useState(18);
  useEffect(() => {
    const observer = new ResizeObserver((entries) => {
      const width = entries[0]?.contentRect.width || 0;
      if (!width) return;
      setBarLimit(Math.max(3, Math.min(18, Math.floor(width / 76))));
    });
    if (chart.current) observer.observe(chart.current);
    return () => observer.disconnect();
  }, []);
  const chartParams = new URLSearchParams(params);
  chartParams.delete("grain");
  const result = useLoad(
    () => api.chart(chartParams),
    `chart:${chartParams}:${version}`,
    visible,
    undefined,
    true,
  );
  const [inspected, setInspected] = useState<string | null>(null);
  const byTime = params.get("sort") === "time";
  const buckets = useMemo(
    () =>
      result.data &&
      (result.data.grain === "month" || result.data.buckets.length > barLimit)
        ? compactBuckets(result.data.buckets, barLimit)
        : (result.data?.buckets || []).map((b) => ({ ...b, periods: 1 })),
    [result.data, barLimit],
  );
  const grouped = buckets.some((b) => b.periods > 1);
  const spansYears =
    !!buckets.length &&
    buckets[0].start.slice(0, 4) !==
      buckets[buckets.length - 1].through.slice(0, 4);
  const bucketLabel = (b: (typeof buckets)[number], full = spansYears) =>
    b.periods > 1
      ? `${periodLabel(b.start, result.data!.grain === "week" ? "day" : result.data!.grain, full)} – ${periodLabel(b.through, result.data!.grain === "week" ? "day" : result.data!.grain, full)}`
      : periodLabel(b.start, result.data!.grain, full);
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
    <article className="panel padded activity-chart chart-compact" ref={chart}>
      <div className="panel-title">
        <h2>
          Listening activity
          {periodOverride && (
            <span
              className="pill widget-period-badge"
              data-period={periodOverride}
            >
              {periodBadgeLabel(periodOverride)}
            </span>
          )}
        </h2>
      </div>
      <p className="subtle">
        Your archived listening over time. Select a bar to explore that period.
      </p>
      <div className="widget-chart-content">
        {grouped && (
          <p className="subtle">
            Quieter periods are combined to fit this view; each bar shows its
            exact date range.
          </p>
        )}
        {inspectedBucket && (
          <div className="chart-readout" aria-live="polite" aria-atomic="true">
            <div>
              <small>
                {inspected ? "Period details" : "Most active period"}
              </small>
              <strong>
                {bucketLabel(inspectedBucket, true)}
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
            segments can overlap in calendar dates; each listen belongs to
            exactly one segment.
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
                if (b.periods > 1) {
                  next.delete("bucket");
                  next.delete("bucket_start");
                  next.set("period", "custom");
                  next.set("from", b.start);
                  next.set("to", b.through);
                }
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
                      {bucketLabel(b)}
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
        {!result.data && !result.error && <ChartSkeleton embedded />}
      </div>
      <p className="subtle">
        * Partial period. Each listen belongs to its assigned date.
      </p>
    </article>
  );
}
export const DEFAULT_V1_LAYOUT: DashboardLayout = {
  version: 0,
  widgets: [
    { id: "now_playing", type: "now_playing", colSpan: 6 },
    { id: "totals", type: "totals", colSpan: 6 },
    { id: "top_artists", type: "top_artists", colSpan: 2 },
    { id: "top_albums", type: "top_albums", colSpan: 2 },
    { id: "top_tracks", type: "top_tracks", colSpan: 2 },
    { id: "activity_chart", type: "activity_chart", colSpan: 6 },
    { id: "listening_patterns", type: "listening_patterns", colSpan: 3 },
    { id: "recent_listens", type: "recent_listens", colSpan: 3 },
  ],
};

const CATALOG_ITEMS = [
  {
    type: "totals",
    name: "Riepilogo Totali",
    description:
      "Ascolti totali, brani distinti e tempo riprodotto effettivo o stimato.",
    defaultSpan: 6 as const,
  },
  {
    type: "unified_rankings",
    name: "Classifiche Congiunte",
    description:
      "Le classifiche dei brani, artisti e album più ascoltati nel periodo.",
    defaultSpan: 6 as const,
  },
  {
    type: "top_artists",
    name: "Top Artisti",
    description: "Gli artisti più ascoltati nel periodo con avatar e volumi.",
    defaultSpan: 2 as const,
  },
  {
    type: "top_albums",
    name: "Top Album",
    description: "Gli album con il maggior numero di ascolti nel periodo.",
    defaultSpan: 2 as const,
  },
  {
    type: "top_tracks",
    name: "Top Brani",
    description: "I brani più riprodotti nel periodo selezionato.",
    defaultSpan: 2 as const,
  },
  {
    type: "activity_chart",
    name: "Grafico Attività",
    description:
      "Istogramma temporale del volume di ascolti nel corso del tempo.",
    defaultSpan: 6 as const,
  },
  {
    type: "recent_listens",
    name: "Ascolti Recenti",
    description:
      "Elenco cronologico degli ascolti recenti registrati nell'archivio.",
    defaultSpan: 6 as const,
  },
  {
    type: "listening_patterns",
    name: "Pattern di ascolto",
    description:
      "Distribuzione degli ascolti per ora del giorno e giorno della settimana.",
    defaultSpan: 3 as const,
  },
  {
    type: "now_playing",
    name: "In riproduzione",
    description: "Brano in ascolto live e avanzamento traccia",
    defaultSpan: 3 as const,
  },
  {
    type: "discovery_ratio",
    name: "Rapporto scoperte",
    description: "Confronto tra brani ascoltati per la prima volta e riascolti",
    defaultSpan: 3 as const,
  },
  {
    type: "history",
    name: "Cronologia ascolti",
    description: "Elenco cronologico dettagliato con filtri e ricerca.",
    defaultSpan: 6 as const,
  },
];

export function TotalsWidget({
  totals,
  ids = true,
  periodOverride,
  loading = false,
}: {
  totals?: Totals;
  ids?: boolean;
  periodOverride?: string | null;
  loading?: boolean;
}) {
  return (
    <article className="panel padded widget-totals" aria-busy={loading}>
      <Stats
        totals={
          totals || {
            records: 0,
            listens: 0,
            tracks: 0,
            actual_ms: 0,
            estimated_ms: 0,
            unknown_durations: 0,
          }
        }
        ids={ids && !loading}
        periodOverride={periodOverride}
        loading={loading}
        embedded
      />
    </article>
  );
}

export function UnifiedRankingsWidget({
  rankings,
  params,
  periodOverride,
  loading = false,
}: {
  rankings?: Dashboard["rankings"];
  params: URLSearchParams;
  periodOverride?: string | null;
  loading?: boolean;
}) {
  return (
    <div className="widget-unified-rankings">
      <TopArtistsWidget
        items={rankings?.artists}
        params={params}
        periodOverride={periodOverride}
        loading={loading}
      />
      <TopAlbumsWidget
        items={rankings?.albums}
        params={params}
        periodOverride={periodOverride}
        loading={loading}
      />
      <TopTracksWidget
        items={rankings?.tracks}
        params={params}
        periodOverride={periodOverride}
        loading={loading}
      />
    </div>
  );
}

export function TopArtistsWidget({
  items,
  params,
  loading = false,
  periodOverride,
}: {
  items?: Ranking[];
  params: URLSearchParams;
  loading?: boolean;
  periodOverride?: string | null;
}) {
  return (
    <div className="rankings-grid overview-rankings">
      <article className="panel padded widget-top-artists overview-ranking-panel">
        <div className="panel-title">
          <h2>
            Top artists
            {periodOverride && (
              <span
                className="pill widget-period-badge"
                data-period={periodOverride}
              >
                {periodBadgeLabel(periodOverride)}
              </span>
            )}
          </h2>
          <Link to={`/rankings/artists?${context(params)}`}>
            View all artists
          </Link>
        </div>
        <div className="widget-ranking-content">
          {loading ? (
            <RankingsSkeleton count={5} label="Loading top artists…" />
          ) : items && items.length > 0 ? (
            <RankingRows bars items={items} kind="artists" params={params} />
          ) : (
            <p className="empty">No listens recorded in this period.</p>
          )}
        </div>
      </article>
    </div>
  );
}

export function TopAlbumsWidget({
  items,
  params,
  loading = false,
  periodOverride,
}: {
  items?: Ranking[];
  params: URLSearchParams;
  loading?: boolean;
  periodOverride?: string | null;
}) {
  return (
    <div className="overview-rankings">
      <article className="panel padded widget-top-albums overview-ranking-panel">
        <div className="panel-title">
          <h2>
            Top albums
            {periodOverride && (
              <span
                className="pill widget-period-badge"
                data-period={periodOverride}
              >
                {periodBadgeLabel(periodOverride)}
              </span>
            )}
          </h2>
          <Link to={`/rankings/albums?${context(params)}`}>
            View all albums
          </Link>
        </div>
        <div className="widget-ranking-content">
          {loading ? (
            <RankingsSkeleton count={5} label="Loading top albums…" />
          ) : items && items.length > 0 ? (
            <RankingRows bars items={items} kind="albums" params={params} />
          ) : (
            <p className="empty">No listens recorded in this period.</p>
          )}
        </div>
      </article>
    </div>
  );
}

export function TopTracksWidget({
  items,
  params,
  loading = false,
  periodOverride,
}: {
  items?: Ranking[];
  params: URLSearchParams;
  loading?: boolean;
  periodOverride?: string | null;
}) {
  return (
    <div className="overview-rankings">
      <article className="panel padded widget-top-tracks overview-ranking-panel">
        <div className="panel-title">
          <h2>
            Top tracks
            {periodOverride && (
              <span
                className="pill widget-period-badge"
                data-period={periodOverride}
              >
                {periodBadgeLabel(periodOverride)}
              </span>
            )}
          </h2>
          <Link to={`/rankings/tracks?${context(params)}`}>
            View all tracks
          </Link>
        </div>
        <div className="widget-ranking-content">
          {loading ? (
            <RankingsSkeleton count={5} label="Loading top tracks…" />
          ) : items && items.length > 0 ? (
            <RankingRows bars items={items} kind="tracks" params={params} />
          ) : (
            <p className="empty">No listens recorded in this period.</p>
          )}
        </div>
      </article>
    </div>
  );
}

export function ActivityChartWidget({
  params,
  visible,
  version,
  periodOverride,
}: {
  params: URLSearchParams;
  visible: boolean;
  version: number;
  periodOverride?: string | null;
}) {
  return (
    <Chart
      params={params}
      visible={visible}
      version={version}
      periodOverride={periodOverride}
    />
  );
}

export function RecentListensWidget({
  recent,
  preferences,
  params,
  periodOverride,
  loading = false,
}: {
  recent?: Dashboard["recent"];
  loading?: boolean;
  preferences: Preferences;
  params: URLSearchParams;
  periodOverride?: string | null;
}) {
  const needsFetch = recent === undefined;
  const historyParams = useMemo(() => {
    if (!needsFetch) return null;
    const hp = new URLSearchParams(params);
    hp.set("page", "1");
    hp.set("page_size", "8");
    return hp;
  }, [needsFetch, params]);
  const isPreview =
    params.get("track") === "preview" || params.get("artist") === "preview";
  const fetched = useLoad(
    () => api.history(historyParams!),
    `recent-widget:${historyParams?.toString()}:${preferences.version}`,
    needsFetch && !isPreview,
  );
  const items = recent !== undefined ? recent : (fetched.data?.items || []);
  const isLoading = loading || (needsFetch && fetched.loading);

  return (
    <article className="panel padded widget-recent-listens">
      <div className="panel-title">
        <h2>
          Recent listening
          {periodOverride && (
            <span
              className="pill widget-period-badge"
              data-period={periodOverride}
            >
              {periodBadgeLabel(periodOverride)}
            </span>
          )}
        </h2>
        <Link to={`/history?${context(params)}`}>View history</Link>
      </div>
      <div className="widget-recent-content">
        {isLoading ? (
          <ListenListSkeleton count={8} />
        ) : (
          <ListenList items={items} preferences={preferences} />
        )}
      </div>
    </article>
  );
}

export function DiscoveryRatioWidget({
  params,
  version,
  periodOverride,
}: {
  params: URLSearchParams;
  version: number;
  periodOverride?: string | null;
}) {
  const result = useLoad(
    () => api.discoveryRatio(params),
    `discovery-ratio:${params}:${version}`,
    true,
    undefined,
    true,
  );

  return (
    <article
      className="panel padded widget-discovery-ratio"
      aria-label="Nuove scoperte vs Riascolti"
    >
      <div className="panel-title">
        <h2>
          Nuove scoperte vs Riascolti
          {periodOverride && (
            <span
              className="pill widget-period-badge"
              data-period={periodOverride}
            >
              {periodBadgeLabel(periodOverride)}
            </span>
          )}
        </h2>
      </div>
      <p className="subtle">
        Confronto tra brani ascoltati per la prima volta e riascolti.
      </p>
      <div className="widget-discovery-content">
        {result.error && <ErrorText>{result.error}</ErrorText>}
        {!result.data && !result.error && (
          <div className="skeleton-container" role="status" aria-busy="true">
            <span className="visually-hidden">
              Caricamento rapporto scoperte…
            </span>
            <div className="discovery-ratio-skeleton">
              <div className="skeleton-metrics">
                <SkeletonBlock width="40%" height={38} borderRadius={4} />
                <SkeletonBlock width="25%" height={14} borderRadius={4} />
              </div>
              <SkeletonBlock
                width="100%"
                height={12}
                borderRadius={6}
                style={{ margin: "14px 0 16px" }}
              />
              <div className="skeleton-legend">
                <SkeletonBlock width="35%" height={14} borderRadius={4} />
                <SkeletonBlock width="35%" height={14} borderRadius={4} />
              </div>
            </div>
          </div>
        )}
        {result.data && result.data.total === 0 && (
          <div className="empty" role="status">
            <p>No listening data in this period.</p>
          </div>
        )}
        {result.data && result.data.total > 0 && (
          <div className="discovery-ratio-content">
            <div className="discovery-ratio-metrics">
              <div className="stat">
                <span className="stat-number tabular-nums">
                  {Math.round(result.data.discovery_percent)}%
                </span>
                <span className="stat-label">nuove scoperte</span>
              </div>
              <div className="stat-secondary">
                <span className="tabular-nums">
                  {100 - Math.round(result.data.discovery_percent)}%
                </span>{" "}
                <span className="subtle">riascolti</span>
              </div>
            </div>

            <div
              className="discovery-ratio-bar"
              role="progressbar"
              aria-valuenow={Math.round(result.data.discovery_percent)}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-label={`Rapporto scoperte: ${Math.round(result.data.discovery_percent)}% nuove scoperte, ${100 - Math.round(result.data.discovery_percent)}% riascolti`}
            >
              <div
                className="discovery-ratio-segment discoveries"
                style={{ width: `${result.data.discovery_percent}%` }}
                title={`Nuove scoperte: ${result.data.discoveries} (${result.data.discovery_percent.toFixed(1)}%)`}
              />
              <div
                className="discovery-ratio-segment repeats"
                style={{ width: `${100 - result.data.discovery_percent}%` }}
                title={`Riascolti: ${result.data.repeats} (${(100 - result.data.discovery_percent).toFixed(1)}%)`}
              />
            </div>

            <div className="discovery-ratio-legend">
              <div className="discovery-legend-item">
                <span
                  className="legend-indicator discoveries"
                  aria-hidden="true"
                />
                <span className="legend-label">Nuove scoperte:</span>
                <span className="legend-count tabular-nums">
                  {result.data.discoveries.toLocaleString()}
                </span>
                <span className="legend-percent tabular-nums">
                  ({result.data.discovery_percent.toFixed(1)}%)
                </span>
              </div>
              <div className="discovery-legend-item">
                <span className="legend-indicator repeats" aria-hidden="true" />
                <span className="legend-label">Riascolti:</span>
                <span className="legend-count tabular-nums">
                  {result.data.repeats.toLocaleString()}
                </span>
                <span className="legend-percent tabular-nums">
                  ({(100 - result.data.discovery_percent).toFixed(1)}%)
                </span>
              </div>
            </div>
          </div>
        )}
      </div>
    </article>
  );
}

function TotalsSkeleton() {
  return <TotalsWidget loading ids={false} />;
}

function OverriddenDashboardWidget({
  widget,
  widgetParams,
  preferences,
  visible,
  status,
}: {
  widget: WidgetConfig;
  widgetParams: URLSearchParams;
  preferences: Preferences;
  visible: boolean;
  status: SpotifyStatus | null;
}) {
  const queryParams = useMemo(() => {
    const p = new URLSearchParams(widgetParams);
    p.set("widgets", widget.type);
    return p;
  }, [widgetParams.toString(), widget.type]);

  const result = useLoad<Dashboard>(
    () => api.dashboard(queryParams),
    `dashboard-override:${widget.id}:${queryParams.toString()}:${preferences.version}`,
    visible,
    (status?.sync_seconds || 120) * 1000,
  );

  const loading = !result.data && !result.error;

  if (result.error && !result.data)
    return (
      <article className="panel padded widget-read-error">
        <ErrorText>{result.error}</ErrorText>
      </article>
    );

  switch (widget.type) {
    case "totals":
      return result.data?.totals ? (
        <TotalsWidget
          totals={result.data.totals}
          ids
          periodOverride={widget.periodOverride}
        />
      ) : (
        <TotalsSkeleton />
      );
    case "unified_rankings":
      return (
        <UnifiedRankingsWidget
          rankings={result.data?.rankings}
          params={widgetParams}
          periodOverride={widget.periodOverride}
          loading={loading}
        />
      );
    case "top_artists":
      return (
        <TopArtistsWidget
          items={result.data?.rankings?.artists}
          params={widgetParams}
          loading={loading}
          periodOverride={widget.periodOverride}
        />
      );
    case "top_albums":
      return (
        <TopAlbumsWidget
          items={result.data?.rankings?.albums}
          params={widgetParams}
          loading={loading}
          periodOverride={widget.periodOverride}
        />
      );
    case "top_tracks":
      return (
        <TopTracksWidget
          items={result.data?.rankings?.tracks}
          params={widgetParams}
          loading={loading}
          periodOverride={widget.periodOverride}
        />
      );
    case "recent_listens":
      return (
        <RecentListensWidget
          recent={result.data?.recent}
          loading={loading}
          preferences={preferences}
          params={widgetParams}
          periodOverride={widget.periodOverride}
        />
      );
    default:
      return null;
  }
}

const OVERRIDE_PERIOD_OPTIONS: [string, string][] = [
  ["", "Eredita globale"],
  ...(periods as [string, string][]),
];

function isTimeSensitiveWidget(type: string): boolean {
  return type !== "now_playing";
}

function getWidgetParams(
  params: URLSearchParams,
  periodOverride?: string | null,
): URLSearchParams {
  const wp = new URLSearchParams(params);
  if (periodOverride) {
    wp.set("period", periodOverride);
    wp.delete("from");
    wp.delete("to");
    wp.delete("bucket");
    wp.delete("bucket_start");
  }
  return wp;
}

function reorderWidgets(widgets: WidgetConfig[], from: number, to: number) {
  if (to < 0 || to >= widgets.length || from === to) return widgets;
  const copy = [...widgets];
  const [item] = copy.splice(from, 1);
  copy.splice(to, 0, item);
  return copy;
}

export type TemplateKind = "home" | "artist" | "album" | "track";

export const DEFAULT_ARTIST_LAYOUT: DashboardLayout = {
  version: 0,
  widgets: [
    { id: "totals", type: "totals", colSpan: 6 },
    { id: "activity_chart", type: "activity_chart", colSpan: 6 },
    { id: "listening_patterns", type: "listening_patterns", colSpan: 6 },
    { id: "top_albums", type: "top_albums", colSpan: 3 },
    { id: "top_tracks", type: "top_tracks", colSpan: 3 },
    { id: "history", type: "history", colSpan: 6 },
  ],
};

export const DEFAULT_TRACK_LAYOUT: DashboardLayout = {
  version: 0,
  widgets: [
    { id: "totals", type: "totals", colSpan: 6 },
    { id: "activity_chart", type: "activity_chart", colSpan: 6 },
    { id: "listening_patterns", type: "listening_patterns", colSpan: 6 },
    { id: "history", type: "history", colSpan: 6 },
  ],
};

export function isWidgetAllowedForTemplate(
  type: string,
  template: TemplateKind,
): boolean {
  switch (template) {
    case "home":
      return [
        "totals",
        "unified_rankings",
        "top_artists",
        "top_albums",
        "top_tracks",
        "activity_chart",
        "recent_listens",
        "listening_patterns",
        "now_playing",
        "discovery_ratio",
      ].includes(type);
    case "artist":
      return [
        "totals",
        "activity_chart",
        "listening_patterns",
        "top_albums",
        "top_tracks",
        "history",
        "recent_listens",
        "discovery_ratio",
      ].includes(type);
    case "album":
      return [
        "totals",
        "activity_chart",
        "listening_patterns",
        "top_tracks",
        "history",
        "recent_listens",
        "discovery_ratio",
      ].includes(type);
    case "track":
      return [
        "totals",
        "activity_chart",
        "listening_patterns",
        "history",
        "recent_listens",
      ].includes(type);
  }
}

export function useLayoutEditor({
  template,
  defaultLayout,
  initialEditing = false,
  onSaved,
}: {
  template: TemplateKind;
  defaultLayout: DashboardLayout;
  initialEditing?: boolean;
  onSaved?: (saved: DashboardLayout) => void;
}) {
  const layoutResult = useLoad<DashboardLayout>(
    () => api.dashboardLayout(template === "home" ? undefined : template),
    `dashboard-layout:${template}`,
    true,
  );

  const [editing, setEditing] = useState(initialEditing);
  const [layout, setLayout] = useState<DashboardLayout>(defaultLayout);
  const [savedLayout, setSavedLayout] = useState<DashboardLayout | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [catalogOpen, setCatalogOpen] = useState(false);

  useEffect(() => {
    if (layoutResult.data) {
      setLayout(layoutResult.data);
      setSavedLayout(layoutResult.data);
    }
  }, [layoutResult.data]);

  const isDirty = useMemo(() => {
    if (!savedLayout) return false;
    return JSON.stringify(layout) !== JSON.stringify(savedLayout);
  }, [layout, savedLayout]);

  useEffect(() => {
    if (!editing || !isDirty) return;
    const handleBeforeUnload = (e: BeforeUnloadEvent) => {
      e.preventDefault();
      e.returnValue = "";
    };
    window.addEventListener("beforeunload", handleBeforeUnload);
    return () => window.removeEventListener("beforeunload", handleBeforeUnload);
  }, [editing, isDirty]);

  const handleStartEditing = () => {
    setEditing(true);
    setSaveError(null);
  };

  const handleCancel = () => {
    if (isDirty) {
      if (
        !window.confirm(
          "Ci sono modifiche non salvate al template. Vuoi davvero annullare?",
        )
      ) {
        return;
      }
    }
    setEditing(false);
    setLayout(savedLayout || defaultLayout);
    setSaveError(null);
  };

  const handleReset = () => {
    setLayout({
      version: layout.version,
      widgets: JSON.parse(JSON.stringify(defaultLayout.widgets)),
    });
  };

  const handleSave = async () => {
    setSaving(true);
    setSaveError(null);
    try {
      const saved = await api.saveDashboardLayout(
        layout,
        template === "home" ? undefined : template,
      );
      setLayout(saved);
      setSavedLayout(saved);
      setEditing(false);
      onSaved?.(saved);
    } catch (e: any) {
      setSaveError(e.message || "Failed to save dashboard layout.");
    } finally {
      setSaving(false);
    }
  };

  const cycleSpan = (id: string) => {
    setLayout((prev) => {
      const spans: (2 | 3 | 4 | 6)[] = [2, 3, 4, 6];
      return {
        ...prev,
        widgets: prev.widgets.map((w) => {
          if (w.id !== id) return w;
          const next = spans[(spans.indexOf(w.colSpan) + 1) % spans.length];
          return { ...w, colSpan: next };
        }),
      };
    });
  };

  const removeWidget = (id: string) => {
    setLayout((prev) => ({
      ...prev,
      widgets: prev.widgets.filter((w) => w.id !== id),
    }));
  };

  const moveWidget = (index: number, delta: number) => {
    setLayout((prev) => ({
      ...prev,
      widgets: reorderWidgets(prev.widgets, index, index + delta),
    }));
  };

  const addWidget = (item: (typeof CATALOG_ITEMS)[number]) => {
    let span: 2 | 3 | 4 | 6 = item.defaultSpan;
    if (
      template === "artist" &&
      (item.type === "top_albums" || item.type === "top_tracks")
    ) {
      span = 3;
    }
    setLayout((prev) => ({
      ...prev,
      widgets: [
        ...prev.widgets,
        {
          id: item.type,
          type: item.type,
          colSpan: span,
        },
      ],
    }));
    setCatalogOpen(false);
  };

  return {
    editing,
    setEditing,
    layout,
    setLayout,
    savedLayout,
    saving,
    saveError,
    catalogOpen,
    setCatalogOpen,
    isDirty,
    handleStartEditing,
    handleCancel,
    handleReset,
    handleSave,
    cycleSpan,
    removeWidget,
    moveWidget,
    addWidget,
    layoutResult,
  };
}

export function DashboardEditToolbar({
  template,
  saving,
  saveError,
  onAddWidget,
  onReset,
  onCancel,
  onSave,
}: {
  template: TemplateKind;
  saving: boolean;
  saveError: string | null;
  onAddWidget: () => void;
  onReset: () => void;
  onCancel: () => void;
  onSave: () => void;
}) {
  const ariaLabel =
    template === "home"
      ? "Personalizza dashboard"
      : template === "track"
        ? "Personalizza template brano"
        : template === "album"
          ? "Personalizza template album"
          : "Personalizza template artista";
  return (
    <div
      className="dashboard-edit-toolbar panel padded"
      role="toolbar"
      aria-label={ariaLabel}
    >
      <div className="dashboard-edit-actions">
        <button
          type="button"
          className="button secondary"
          disabled={saving}
          onClick={onAddWidget}
        >
          <Icon>add</Icon> Aggiungi widget
        </button>
        <button
          type="button"
          className="button secondary"
          disabled={saving}
          onClick={onReset}
        >
          <Icon>restart_alt</Icon> Ripristina predefinito
        </button>
        <button
          type="button"
          className="button secondary"
          disabled={saving}
          onClick={onCancel}
        >
          Annulla
        </button>
        <button
          type="button"
          className="button primary"
          onClick={onSave}
          disabled={saving}
        >
          <Icon>check</Icon> Salva modifiche
        </button>
      </div>
      {template === "artist" && (
        <p className="dashboard-edit-notice">
          <Icon>info</Icon> Modifica del template artista: le modifiche verranno
          applicate a tutte le pagine artista dell'archivio.
        </p>
      )}
      {template === "track" && (
        <p className="dashboard-edit-notice">
          <Icon>info</Icon> Modifica del template brano: le modifiche verranno
          applicate a tutte le pagine brano dell'archivio.
        </p>
      )}
      {saving && (
        <span className="visually-hidden" role="status">
          Salvataggio…
        </span>
      )}
      {saveError && <ErrorText>{saveError}</ErrorText>}
    </div>
  );
}

export function WidgetCatalogDialog({
  open,
  onClose,
  template,
  widgets,
  onAdd,
}: {
  open: boolean;
  onClose: () => void;
  template: TemplateKind;
  widgets: WidgetConfig[];
  onAdd: (item: (typeof CATALOG_ITEMS)[number]) => void;
}) {
  const catalogDialogRef = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    const dialog = catalogDialogRef.current;
    if (!dialog) return;
    if (open) {
      if (!dialog.open) {
        dialog.showModal();
      }
    } else {
      if (dialog.open) {
        dialog.close();
      }
    }
  }, [open]);

  const availableItems = CATALOG_ITEMS.filter(
    (item) =>
      isWidgetAllowedForTemplate(item.type, template) &&
      !widgets.some((w) => w.type === item.type),
  );

  return (
    <dialog
      ref={catalogDialogRef}
      className="modal widget-catalog-dialog"
      aria-label="Aggiungi widget"
      onClose={onClose}
    >
      <div className="dialog-header">
        <h2>Aggiungi widget</h2>
        <button
          type="button"
          className="icon-button"
          aria-label="Chiudi catalogo"
          onClick={onClose}
        >
          <Icon>close</Icon>
        </button>
      </div>
      {availableItems.length === 0 ? (
        <p>Tutti i widget disponibili sono già presenti nella dashboard.</p>
      ) : (
        <div className="catalog-items-grid">
          {availableItems.map((item) => (
            <div key={item.type} className="panel padded catalog-item-card">
              <div className="catalog-item-info">
                <h3>{item.name}</h3>
                <p>{item.description}</p>
                <small>
                  Dimensione:{" "}
                  {template === "artist" &&
                  (item.type === "top_albums" || item.type === "top_tracks")
                    ? 3
                    : item.defaultSpan}{" "}
                  colonne
                </small>
              </div>
              <button
                type="button"
                className="button secondary"
                aria-label={`Aggiungi ${item.name}`}
                onClick={() => onAdd(item)}
              >
                <Icon>add</Icon> Aggiungi
              </button>
            </div>
          ))}
        </div>
      )}
    </dialog>
  );
}

export function WidgetEditControls({
  widget,
  index,
  totalWidgets,
  saving,
  onMove,
  onCycleSpan,
  onRemove,
  onChangePeriod,
}: {
  widget: WidgetConfig;
  index: number;
  totalWidgets: number;
  saving: boolean;
  onMove: (index: number, delta: number) => void;
  onCycleSpan: (id: string) => void;
  onRemove: (id: string) => void;
  onChangePeriod?: (period: string | undefined) => void;
}) {
  return (
    <div className="widget-edit-controls">
      <button
        type="button"
        className="widget-control widget-drag-handle"
        disabled={saving}
        title="Trascina per riordinare"
        aria-label="Sposta widget"
        draggable={!saving}
        onDragStart={(e) => {
          e.dataTransfer.setData("text/plain", String(index));
          e.dataTransfer.effectAllowed = "move";
        }}
      >
        <Icon>drag_indicator</Icon>
      </button>
      <button
        type="button"
        className="widget-control widget-move-up"
        title="Sposta prima"
        aria-label="Sposta widget prima"
        disabled={saving || index === 0}
        onClick={() => onMove(index, -1)}
      >
        <Icon>arrow_upward</Icon>
      </button>
      <button
        type="button"
        className="widget-control widget-move-down"
        title="Sposta dopo"
        aria-label="Sposta widget dopo"
        disabled={saving || index === totalWidgets - 1}
        onClick={() => onMove(index, 1)}
      >
        <Icon>arrow_downward</Icon>
      </button>
      <button
        type="button"
        className="widget-control widget-span-button"
        title={`Larghezza: ${widget.colSpan} colonne. Clicca per modificare.`}
        aria-label={`Larghezza: ${widget.colSpan} colonne`}
        disabled={saving}
        onClick={() => onCycleSpan(widget.id)}
      >
        <Icon>view_column</Icon>
        <span>{widget.colSpan}/6</span>
      </button>
      {isTimeSensitiveWidget(widget.type) && onChangePeriod && (
        <div className="widget-period-override-control">
          <Select
            aria-label="Periodo widget"
            disabled={saving}
            value={widget.periodOverride || ""}
            onChange={(val) => onChangePeriod(val || undefined)}
          >
            {OVERRIDE_PERIOD_OPTIONS.map(([val, label]) => (
              <option key={val} value={val}>
                {label}
              </option>
            ))}
          </Select>
        </div>
      )}
      <button
        type="button"
        className="widget-control widget-remove-button"
        title="Rimuovi widget"
        aria-label="Rimuovi widget"
        disabled={saving}
        onClick={() => onRemove(widget.id)}
      >
        <Icon>close</Icon>
      </button>
    </div>
  );
}

function OverriddenArtistTotalsWidget({
  kind,
  id,
  widgetParams,
  periodOverride,
}: {
  kind: Kind;
  id: string;
  widgetParams: URLSearchParams;
  periodOverride?: string;
}) {
  const isPreview = id === "preview";
  const result = useLoad(
    () => api.music(kind, id, widgetParams),
    `music-totals-override:${kind}:${id}:${widgetParams.toString()}`,
    !isPreview,
  );
  if (isPreview) {
    return (
      <TotalsWidget
        totals={{
          records: 0,
          listens: 0,
          tracks: 0,
          actual_ms: 0,
          estimated_ms: 0,
          unknown_durations: 0,
        }}
        ids
        periodOverride={periodOverride}
      />
    );
  }
  if (result.error && !result.data) {
    return (
      <article className="panel padded widget-read-error">
        <ErrorText>{result.error}</ErrorText>
      </article>
    );
  }
  if (!result.data) return <TotalsSkeleton />;
  return (
    <TotalsWidget
      totals={result.data.totals}
      ids
      periodOverride={periodOverride}
    />
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
  const layoutResult = useLoad<DashboardLayout>(
    () => api.dashboardLayout(),
    "dashboard-layout",
    visible,
  );

  const [editing, setEditing] = useState(false);
  const [layout, setLayout] = useState<DashboardLayout>(DEFAULT_V1_LAYOUT);
  const [savedLayout, setSavedLayout] = useState<DashboardLayout | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [catalogOpen, setCatalogOpen] = useState(false);
  const editButtonRef = useRef<HTMLButtonElement>(null);
  const catalogDialogRef = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    const dialog = catalogDialogRef.current;
    if (!dialog) return;
    if (catalogOpen) {
      if (!dialog.open) {
        dialog.showModal();
      }
    } else {
      if (dialog.open) {
        dialog.close();
      }
    }
  }, [catalogOpen]);

  useEffect(() => {
    if (layoutResult.data) {
      setLayout(layoutResult.data);
      setSavedLayout(layoutResult.data);
    }
  }, [layoutResult.data]);

  const requestedWidgets = useMemo(() => {
    return Array.from(
      new Set(
        layout.widgets.filter((w) => !w.periodOverride).map((w) => w.type),
      ),
    ).join(",");
  }, [layout.widgets]);

  const dashboardParams = useMemo(() => {
    const p = new URLSearchParams(params);
    if (requestedWidgets) {
      p.set("widgets", requestedWidgets);
    }
    return p;
  }, [params, requestedWidgets]);

  const result = useLoad<Dashboard>(
    () => api.dashboard(dashboardParams),
    `${dashboardParams}:${preferences.version}`,
    visible && requestedWidgets.length > 0,
    (status?.sync_seconds || 120) * 1000,
  );

  const handleStartEditing = () => {
    setEditing(true);
    setSaveError(null);
  };

  const handleCancel = () => {
    setEditing(false);
    editButtonRef.current?.focus();
    setLayout(savedLayout || DEFAULT_V1_LAYOUT);
    setSaveError(null);
  };

  const handleReset = () => {
    setLayout({
      version: layout.version,
      widgets: JSON.parse(JSON.stringify(DEFAULT_V1_LAYOUT.widgets)),
    });
  };

  const handleSave = async () => {
    setSaving(true);
    setSaveError(null);
    try {
      const saved = await api.saveDashboardLayout(layout);
      setLayout(saved);
      setSavedLayout(saved);
      setEditing(false);
      editButtonRef.current?.focus();
    } catch (e: any) {
      setSaveError(e.message || "Failed to save dashboard layout.");
    } finally {
      setSaving(false);
    }
  };

  const cycleSpan = (id: string) => {
    setLayout((prev) => {
      const spans: (2 | 3 | 4 | 6)[] = [2, 3, 4, 6];
      return {
        ...prev,
        widgets: prev.widgets.map((w) => {
          if (w.id !== id) return w;
          const next = spans[(spans.indexOf(w.colSpan) + 1) % spans.length];
          return { ...w, colSpan: next };
        }),
      };
    });
  };

  const removeWidget = (id: string) => {
    setLayout((prev) => ({
      ...prev,
      widgets: prev.widgets.filter((w) => w.id !== id),
    }));
  };

  const moveWidget = (index: number, delta: number) => {
    setLayout((prev) => ({
      ...prev,
      widgets: reorderWidgets(prev.widgets, index, index + delta),
    }));
  };

  const addWidget = (item: (typeof CATALOG_ITEMS)[number]) => {
    setLayout((prev) => ({
      ...prev,
      widgets: [
        ...prev.widgets,
        {
          id: item.type,
          type: item.type,
          colSpan: item.defaultSpan,
        },
      ],
    }));
    setCatalogOpen(false);
  };

  const renderWidgetContent = (widget: WidgetConfig) => {
    const widgetParams = getWidgetParams(params, widget.periodOverride);
    if (widget.periodOverride) {
      switch (widget.type) {
        case "activity_chart":
          return (
            <ActivityChartWidget
              params={widgetParams}
              visible={visible}
              version={preferences.version}
              periodOverride={widget.periodOverride}
            />
          );
        case "listening_patterns":
          return (
            <ListeningPatternsWidget
              params={widgetParams}
              version={preferences.version}
              periodOverride={widget.periodOverride}
            />
          );
        case "discovery_ratio":
          return (
            <DiscoveryRatioWidget
              params={widgetParams}
              version={preferences.version}
              periodOverride={widget.periodOverride}
            />
          );
        case "now_playing":
          return <NowPlayingWidget status={status} />;
        default:
          return (
            <OverriddenDashboardWidget
              widget={widget}
              widgetParams={widgetParams}
              preferences={preferences}
              visible={visible}
              status={status}
            />
          );
      }
    }

    if (
      result.error &&
      !result.data &&
      [
        "totals",
        "unified_rankings",
        "top_artists",
        "top_albums",
        "top_tracks",
        "recent_listens",
      ].includes(widget.type)
    )
      return (
        <article className="panel padded widget-read-error">
          <ErrorText>{result.error}</ErrorText>
        </article>
      );
    const loading = !result.data && !result.error;
    switch (widget.type) {
      case "totals":
        return result.data?.totals ? (
          <TotalsWidget totals={result.data.totals} ids />
        ) : (
          <TotalsSkeleton />
        );
      case "unified_rankings":
        return (
          <UnifiedRankingsWidget
            rankings={result.data?.rankings}
            params={params}
            loading={loading}
          />
        );
      case "top_artists":
        return (
          <TopArtistsWidget
            items={result.data?.rankings?.artists}
            params={params}
            loading={loading}
          />
        );
      case "top_albums":
        return (
          <TopAlbumsWidget
            items={result.data?.rankings?.albums}
            params={params}
            loading={loading}
          />
        );
      case "top_tracks":
        return (
          <TopTracksWidget
            items={result.data?.rankings?.tracks}
            params={params}
            loading={loading}
          />
        );
      case "activity_chart":
        return (
          <ActivityChartWidget
            params={params}
            visible={visible}
            version={preferences.version}
          />
        );
      case "recent_listens":
        return (
          <RecentListensWidget
            recent={result.data?.recent}
            loading={loading}
            preferences={preferences}
            params={params}
          />
        );
      case "listening_patterns":
        return (
          <ListeningPatternsWidget
            params={params}
            version={preferences.version}
          />
        );
      case "now_playing":
        return <NowPlayingWidget status={status} />;
      case "discovery_ratio":
        return (
          <DiscoveryRatioWidget params={params} version={preferences.version} />
        );
      default:
        return null;
    }
  };

  if (!visible) return null;

  return (
    <>
      <div className="overview-heading">
        <Heading title="Your listening, at a glance">
          The songs, the repeats, the moments. All yours.
        </Heading>
        <div className="overview-heading-actions">
          <button
            ref={editButtonRef}
            aria-pressed={editing}
            aria-controls="dashboard-grid"
            type="button"
            className="button secondary"
            disabled={saving}
            onClick={handleStartEditing}
          >
            <Icon>tune</Icon> Personalizza
          </button>
          <PeriodFilters selection={selection} />
        </div>
      </div>

      {editing && (
        <div
          className="dashboard-edit-toolbar panel padded"
          role="toolbar"
          aria-label="Personalizza dashboard"
        >
          <div className="dashboard-edit-actions">
            <button
              type="button"
              className="button secondary"
              disabled={saving}
              onClick={() => setCatalogOpen(true)}
            >
              <Icon>add</Icon> Aggiungi widget
            </button>
            <button
              type="button"
              className="button secondary"
              disabled={saving}
              onClick={handleReset}
            >
              <Icon>restart_alt</Icon> Ripristina predefinito
            </button>
            <button
              type="button"
              className="button secondary"
              disabled={saving}
              onClick={handleCancel}
            >
              Annulla
            </button>
            <button
              type="button"
              className="button primary"
              onClick={handleSave}
              disabled={saving}
            >
              <Icon>check</Icon> Salva modifiche
            </button>
          </div>
          {saving && (
            <span className="visually-hidden" role="status">
              Salvataggio…
            </span>
          )}
          {saveError && <ErrorText>{saveError}</ErrorText>}
        </div>
      )}

      <dialog
        ref={catalogDialogRef}
        className="modal widget-catalog-dialog"
        aria-label="Aggiungi widget"
        onClose={() => setCatalogOpen(false)}
      >
        <div className="dialog-header">
          <h2>Aggiungi widget</h2>
          <button
            type="button"
            className="icon-button"
            aria-label="Chiudi catalogo"
            onClick={() => setCatalogOpen(false)}
          >
            <Icon>close</Icon>
          </button>
        </div>
        {CATALOG_ITEMS.filter(
          (item) => !layout.widgets.some((w) => w.type === item.type),
        ).length === 0 ? (
          <p>Tutti i widget disponibili sono già presenti nella dashboard.</p>
        ) : (
          <div className="catalog-items-grid">
            {CATALOG_ITEMS.filter(
              (item) => !layout.widgets.some((w) => w.type === item.type),
            ).map((item) => (
              <div key={item.type} className="panel padded catalog-item-card">
                <div className="catalog-item-info">
                  <h3>{item.name}</h3>
                  <p>{item.description}</p>
                  <small>Dimensione: {item.defaultSpan} colonne</small>
                </div>
                <button
                  type="button"
                  className="button secondary"
                  onClick={() => addWidget(item)}
                >
                  <Icon>add</Icon> Aggiungi
                </button>
              </div>
            ))}
          </div>
        )}
      </dialog>

      <ErrorText>{result.error || layoutResult.error}</ErrorText>
      <div
        id="dashboard-grid"
        className={`dashboard-grid${editing ? " editing" : ""}${result.loading && !result.data ? " skeleton-container" : ""}`}
        aria-busy={result.loading && !result.data}
      >
        {result.loading && !result.data && (
          <span className="visually-hidden" role="status">
            Loading your archive…
          </span>
        )}
        {layout.widgets.map((w, index) => (
          <div
            key={w.id}
            className={`dashboard-widget span-${w.colSpan}${editing ? " editing" : ""}`}
            data-widget-id={w.id}
            data-widget-type={w.type}
            draggable={editing && !saving}
            onDragStart={(e) => {
              if (!editing || saving) return;
              e.dataTransfer.setData("text/plain", String(index));
              e.dataTransfer.effectAllowed = "move";
            }}
            onDragOver={(e) => {
              if (!editing || saving) return;
              e.preventDefault();
              e.dataTransfer.dropEffect = "move";
            }}
            onDrop={(e) => {
              if (!editing || saving) return;
              e.preventDefault();
              const sourceIndex = parseInt(
                e.dataTransfer.getData("text/plain"),
                10,
              );
              if (!isNaN(sourceIndex) && sourceIndex !== index) {
                setLayout((prev) => ({
                  ...prev,
                  widgets: reorderWidgets(prev.widgets, sourceIndex, index),
                }));
              }
            }}
          >
            {editing && (
              <div className="widget-edit-controls">
                <button
                  type="button"
                  className="widget-control widget-drag-handle"
                  disabled={saving}
                  title="Trascina per riordinare"
                  aria-label="Sposta widget"
                  draggable={editing && !saving}
                  onDragStart={(e) => {
                    e.dataTransfer.setData("text/plain", String(index));
                    e.dataTransfer.effectAllowed = "move";
                  }}
                >
                  <Icon>drag_indicator</Icon>
                </button>
                <button
                  type="button"
                  className="widget-control widget-move-up"
                  title="Sposta prima"
                  aria-label="Sposta widget prima"
                  disabled={saving || index === 0}
                  onClick={() => moveWidget(index, -1)}
                >
                  <Icon>arrow_upward</Icon>
                </button>
                <button
                  type="button"
                  className="widget-control widget-move-down"
                  title="Sposta dopo"
                  aria-label="Sposta widget dopo"
                  disabled={saving || index === layout.widgets.length - 1}
                  onClick={() => moveWidget(index, 1)}
                >
                  <Icon>arrow_downward</Icon>
                </button>
                <button
                  type="button"
                  className="widget-control widget-span-button"
                  title={`Larghezza: ${w.colSpan} colonne. Clicca per modificare.`}
                  aria-label={`Larghezza: ${w.colSpan} colonne`}
                  disabled={saving}
                  onClick={() => cycleSpan(w.id)}
                >
                  <Icon>view_column</Icon>
                  <span>{w.colSpan}/6</span>
                </button>
                {isTimeSensitiveWidget(w.type) && (
                  <div className="widget-period-override-control">
                    <Select
                      aria-label="Periodo widget"
                      disabled={saving}
                      value={w.periodOverride || ""}
                      onChange={(val) => {
                        setLayout((prev) => ({
                          ...prev,
                          widgets: prev.widgets.map((item) =>
                            item.id === w.id
                              ? { ...item, periodOverride: val || undefined }
                              : item,
                          ),
                        }));
                      }}
                    >
                      {OVERRIDE_PERIOD_OPTIONS.map(([val, label]) => (
                        <option key={val} value={val}>
                          {label}
                        </option>
                      ))}
                    </Select>
                  </div>
                )}
                <button
                  type="button"
                  className="widget-control widget-remove-button"
                  title="Rimuovi widget"
                  aria-label="Rimuovi widget"
                  disabled={saving}
                  onClick={() => removeWidget(w.id)}
                >
                  <Icon>close</Icon>
                </button>
              </div>
            )}
            {renderWidgetContent(w)}
          </div>
        ))}
      </div>
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
      <Heading title="Rankings">
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
  const [query, setQuery] = useState("");
  const [focused, setFocused] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const container = useRef<HTMLDivElement>(null);
  const optionsId = useId();
  useEffect(() => {
    const timer = setTimeout(() => setQuery(search.trim()), 150);
    return () => clearTimeout(timer);
  }, [search]);
  const options = useLoad(
    () => api.choices(kind, query),
    `${kind}:${query}`,
    !!query && !id,
  );
  const chosen = useLoad(
    () => api.music(kind, id, new URLSearchParams()),
    `${kind}:${id}`,
    !!id,
  );
  const [selectedName, setSelectedName] = useState("");
  const showOptions = focused && !id && !!query && query === search.trim();
  return (
    <div
      className="entity-filter"
      ref={container}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget))
          setFocused(false);
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          input.current?.focus();
          setFocused(false);
        }
        if (["ArrowDown", "ArrowUp"].includes(event.key)) {
          event.preventDefault();
          setFocused(true);
          const buttons = Array.from(
            container.current?.querySelectorAll<HTMLButtonElement>(
              ".entity-options button",
            ) || [],
          );
          const current = buttons.indexOf(
            document.activeElement as HTMLButtonElement,
          );
          const next =
            current < 0
              ? event.key === "ArrowDown"
                ? 0
                : buttons.length - 1
              : (current +
                  (event.key === "ArrowDown" ? 1 : -1) +
                  buttons.length) %
                buttons.length;
          buttons[next]?.focus();
        }
      }}
    >
      <label>
        {kind === "artists" ? "Artist" : "Album"}
        <span className="entity-field">
          <input
            ref={input}
            type="search"
            aria-label={kind === "artists" ? "Find artist" : "Find album"}
            placeholder={`Find ${kind}`}
            autoComplete="off"
            maxLength={500}
            aria-expanded={showOptions}
            aria-controls={optionsId}
            aria-autocomplete="list"
            readOnly={!!id}
            value={
              id
                ? chosen.data?.name || selectedName || "Selected identity"
                : search
            }
            onFocus={() => setFocused(true)}
            onChange={(event) => setSearch(event.target.value)}
          />
          {id && (
            <button
              type="button"
              className="icon-button"
              aria-label={`Clear ${kind}`}
              onClick={() => {
                change("");
                setSearch("");
                setQuery("");
                setSelectedName("");
                input.current?.focus();
              }}
            >
              <Icon>close</Icon>
            </button>
          )}
        </span>
      </label>
      <ErrorText>{options.error || (id ? chosen.error : "")}</ErrorText>
      {showOptions && (
        <ul
          id={optionsId}
          className="entity-options"
          aria-label={`${kind} matches`}
        >
          {options.data ? (
            <>
              {options.data.map((item) => (
                <li key={item.id}>
                  <button
                    type="button"
                    className="secondary"
                    aria-label={`${item.name}${item.subtitle ? ` · ${item.subtitle}` : ""}`}
                    // Safari does not focus buttons on pointer down. Keep the
                    // input focused until the selection click has been handled.
                    onPointerDown={(event) => event.preventDefault()}
                    onClick={() => {
                      setSelectedName(item.name);
                      change(item.id);
                      setSearch("");
                      setQuery("");
                      input.current?.focus();
                      setFocused(false);
                    }}
                  >
                    <strong>{item.name}</strong>
                    {item.subtitle && <small>{item.subtitle}</small>}
                  </button>
                </li>
              ))}
              {!options.data.length && <li>No matches</li>}
              {options.data.length === 50 && (
                <li>
                  Showing 50 matches. Refine your search to narrow the choices.
                </li>
              )}
            </>
          ) : (
            !options.error && (
              <li>
                <SkeletonList count={3} label="Finding matches…" />
              </li>
            )
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
    <div className="track-scope-chip">
      <span className="track-scope-label">Track:</span>
      <Link to={musicLink("tracks", id)}>
        {result.data?.name || "Selected track"}
      </Link>
      <button
        type="button"
        className="track-scope-clear"
        aria-label="Clear track filter"
        title="Clear track filter"
        onClick={clear}
      >
        <Icon>close</Icon>
      </button>
    </div>
  );
}
export function HistoryPage({
  preferences,
  visible,
  params: override,
  embedded = false,
  musicKind,
}: {
  preferences: Preferences;
  visible: boolean;
  params?: URLSearchParams;
  embedded?: boolean;
  musicKind?: Kind;
}) {
  const selection = useSelection(visible),
    list = useListPreferences();
  const params = new URLSearchParams(override || selection.params);
  const [filtersOpen, setFiltersOpen] = useBrowserPreference(
    "history-filters-open",
    false,
    isBool,
  );
  const filterAnimation = useRef<Animation | null>(null);
  const filterDetails = useRef<HTMLDetailsElement | null>(null);
  const filtersId = useId();
  useEffect(() => () => filterAnimation.current?.cancel(), []);
  const [draft, setDraft] = useState(params.get("q") || "");
  const search = useSearch();
  useLayoutEffect(() => {
    if (visible) setDraft(params.get("q") || "");
  }, [search, visible]);
  params.set("page_size", String(list.size));
  if (embedded) params.set("page", params.get("history_page") || "1");
  const result = useLoad<History>(
    () => api.history(params),
    `history:${params}:${preferences.version}`,
    visible,
    undefined,
    true,
  );
  const change = (patch: Record<string, string>) =>
    selection.update({
      ...patch,
      history_page: "1",
      artists_page: "1",
      albums_page: "1",
      tracks_page: "1",
    });
  function toggleFilters() {
    const details = filterDetails.current;
    if (!details) return;
    const form = details.querySelector("form")!;
    const opening = !filtersOpen;
    const from = details.open ? form.getBoundingClientRect().height : 0;
    filterAnimation.current?.cancel();
    details.open = true;
    const to = opening ? form.scrollHeight : 0;
    setFiltersOpen(opening);
    details.open = true;
    form.style.overflow = "hidden";
    const animation = form.animate(
      [
        { height: `${from}px`, opacity: opening ? 0 : 1 },
        { height: `${to}px`, opacity: opening ? 1 : 0 },
      ],
      {
        duration: matchMedia("(prefers-reduced-motion: reduce)").matches
          ? 0
          : 160,
        easing: "ease-out",
      },
    );
    filterAnimation.current = animation;
    animation.finished
      .then(() => {
        details.open = opening;
        form.style.overflow = "";
        filterAnimation.current = null;
      })
      .catch(() => {});
  }
  if (!visible) return null;
  return (
    <div className={`history-section ${embedded ? "embedded-history" : ""}`}>
      {!embedded && (
        <div className="history-heading">
          <Heading title="Listening history">
            Search and combine period, artist, album and source filters.
          </Heading>
          <div className="history-actions">
            <Link className="button secondary" to="/review">
              <Icon>rule</Icon>Review possible duplicates
            </Link>
            <button
              className="secondary"
              aria-expanded={filtersOpen}
              aria-controls={filtersId}
              onClick={toggleFilters}
            >
              <Icon>tune</Icon>Filters
              <Icon>{filtersOpen ? "expand_less" : "expand_more"}</Icon>
            </button>
          </div>
        </div>
      )}
      {!embedded && params.get("track") && (
        <TrackScope
          id={params.get("track")!}
          clear={() => change({ track: "" })}
        />
      )}
      <details
        id={filtersId}
        className="history-filter-disclosure panel"
        ref={(node) => {
          filterDetails.current = node;
          if (node && !filterAnimation.current) node.open = filtersOpen;
        }}
      >
        <summary
          onClick={(event) => {
            event.preventDefault();
            toggleFilters();
          }}
        >
          <Icon>tune</Icon>
          <span>
            <strong>Filters</strong>
            <small>
              {[
                params.get("period") && params.get("period") !== "all"
                  ? params.get("period")
                  : "All time",
                params.get("q"),
                params.get("source")?.replaceAll("_", " "),
                params.get("artist") ? "Artist selected" : "",
                params.get("album") ? "Album selected" : "",
                params.get("status") && params.get("status") !== "counted"
                  ? params.get("status")
                  : "Counted listens",
              ]
                .filter(Boolean)
                .join(" · ")}
            </small>
          </span>
          <span className="filter-toggle-label">
            {filtersOpen ? "Hide filters" : "Show filters"}
          </span>
          <Icon>expand_more</Icon>
        </summary>
        <form
          className="history-filters padded"
          aria-label="Listening history filters"
          onSubmit={(e) => {
            e.preventDefault();
            change({ q: draft });
          }}
        >
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
              <Select
                aria-label="Source"
                value={params.get("source") || ""}
                onChange={(value) => change({ source: value })}
              >
                <option value="">All sources</option>
                <option value="spotify_import">Spotify import</option>
                <option value="spotify_api">Spotify collection</option>
                <option value="listenbrainz">ListenBrainz</option>
              </Select>
            </label>
            <label>
              Include
              <Select
                aria-label="Include"
                id={embedded ? undefined : "history-status"}
                value={params.get("status") || "counted"}
                onChange={(value) => change({ status: value })}
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
              </Select>
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
          <p className="subtle">
            Listens below 30 seconds of actual playback remain archived and are
            excluded from valid-listen statistics. Estimated or unknown time
            alone does not exclude a listen.
          </p>
        </form>
      </details>
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
      <ErrorText>{result.error}</ErrorText>
      {result.data ? (
        <article className="panel">
          <div className="panel-title history-result-heading">
            <h2 id={embedded ? undefined : "history-total"}>
              {number.format(result.data.total)}{" "}
              {result.data.total === 1 ? "record" : "records"}
            </h2>
          </div>
          <ListenList
            id={embedded ? undefined : "history-list"}
            items={result.data.items}
            preferences={preferences}
            context={musicKind}
          />
          <Pager
            list={list}
            resize={() => change({})}
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
        !result.error && <ListenListSkeleton count={8} />
      )}
    </div>
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
  const isPreview = id === "preview";
  const searchParams = new URLSearchParams(location.search);
  const isInitialEdit =
    searchParams.get("edit") === "template" || searchParams.get("edit") === "1";

  const selection = useSelection(true),
    params = scoped(selection.params, kind, id);

  const isMusicTemplate = kind === "artists" || kind === "tracks";
  const templateKind: TemplateKind = kind === "tracks" ? "track" : "artist";
  const defaultLayout =
    kind === "tracks" ? DEFAULT_TRACK_LAYOUT : DEFAULT_ARTIST_LAYOUT;

  const layoutEditor = useLayoutEditor({
    template: templateKind,
    defaultLayout,
    initialEditing: isInitialEdit && isMusicTemplate,
  });

  const dummyTotals: Totals = {
    records: 0,
    listens: 0,
    tracks: 0,
    actual_ms: 0,
    estimated_ms: 0,
    unknown_durations: 0,
  };
  const dummyPreviewPage: MusicPageData = {
    id: "preview",
    name: kind === "tracks" ? "Brano di anteprima" : "Artista di anteprima",
    kind,
    cover_url: null,
    album_id: null,
    album: null,
    artists: [],
    totals: dummyTotals,
  };

  const apiResult = useLoad(
    () => api.music(kind, id, params),
    `music:${kind}:${id}:${params}:${preferences.version}`,
    !isPreview,
    undefined,
    true,
  );

  const result = isPreview
    ? { data: dummyPreviewPage, error: null, loading: false }
    : apiResult;

  const introduction = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const content = introduction.current;
    const header = content?.parentElement;
    if (!content || !header) return;
    const resize = () => {
      const size = Math.min(
        content.getBoundingClientRect().height,
        header.clientWidth * 0.42,
      );
      const value = Math.round(size) + "px";
      if (header.style.getPropertyValue("--music-artwork-size") !== value)
        header.style.setProperty("--music-artwork-size", value);
    };
    let frame = 0;
    const observer = new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(resize);
    });
    observer.observe(content);
    observer.observe(header);
    resize();
    return () => {
      observer.disconnect();
      cancelAnimationFrame(frame);
    };
  }, [result.data?.id]);

  if (!isMusicTemplate) {
    return (
      <>
        <ErrorText>{result.error}</ErrorText>
        {result.data ? (
          <>
            <div className="music-heading">
              <Cover url={result.data.cover_url} large>
                <ArtworkCredits url={result.data.cover_url} />
              </Cover>
              <div className="music-header-content" ref={introduction}>
                <div className="music-title-row">
                  <Heading title={result.data.name}>
                    {kind === "albums"
                      ? "Album by "
                      : kind === "tracks"
                        ? "Track by "
                        : ""}
                    {result.data.artists.map(
                      (a: { id: string; name: string }, i: number) => (
                        <span key={a.id}>
                          {i > 0 && ", "}
                          <Link to={musicLink("artists", a.id, params)}>
                            {a.name}
                          </Link>
                        </span>
                      ),
                    )}
                    {result.data.album_id && (
                      <>
                        {" "}
                        ·{" "}
                        <Link
                          to={musicLink("albums", result.data.album_id, params)}
                        >
                          {result.data.album}
                        </Link>
                      </>
                    )}
                  </Heading>
                  <BackButton
                    fallback={`/rankings/${kind}?${context(selection.params)}`}
                  />
                </div>
                <MusicActions kind={kind} id={id} />
                <div className="music-period">
                  <PeriodFilters selection={selection} />
                </div>
              </div>
            </div>
            <Stats totals={result.data.totals} />
          </>
        ) : (
          !result.error && <MusicPageSkeleton />
        )}
        <Chart params={params} version={preferences.version} />
        <ListeningPatterns params={params} version={preferences.version} />
        <div>
          <RankingsPanel
            kind="tracks"
            params={params}
            title="Tracks in this period"
            version={preferences.version}
            scrolling={kind === "albums"}
          />
        </div>
        <h2>Listening history</h2>
        <HistoryPage
          preferences={preferences}
          visible
          params={params}
          embedded
          musicKind={kind}
        />
      </>
    );
  }

  // --- Music Page (Artist / Track) Consultation with Shared Grid Engine ---
  const renderMusicWidgetContent = (widget: WidgetConfig) => {
    const widgetParams = getWidgetParams(params, widget.periodOverride);
    switch (widget.type) {
      case "totals":
        if (widget.periodOverride) {
          return (
            <OverriddenArtistTotalsWidget
              kind={kind}
              id={id}
              widgetParams={widgetParams}
              periodOverride={widget.periodOverride}
            />
          );
        }
        return result.data?.totals ? (
          <TotalsWidget totals={result.data.totals} ids />
        ) : (
          <TotalsSkeleton />
        );
      case "activity_chart":
        return <Chart params={widgetParams} version={preferences.version} />;
      case "listening_patterns":
        return (
          <ListeningPatterns
            params={widgetParams}
            version={preferences.version}
          />
        );
      case "top_albums":
        return (
          <RankingsPanel
            kind="albums"
            params={widgetParams}
            version={preferences.version}
            title="Albums in this period"
          />
        );
      case "top_tracks":
        return (
          <RankingsPanel
            kind="tracks"
            params={widgetParams}
            title="Tracks in this period"
            version={preferences.version}
          />
        );
      case "history":
        return (
          <div className="music-history-widget-container">
            <h2>Listening history</h2>
            <HistoryPage
              preferences={preferences}
              visible
              params={widgetParams}
              embedded
              musicKind={kind}
            />
          </div>
        );
      case "recent_listens":
        return (
          <RecentListensWidget
            recent={undefined}
            loading={false}
            preferences={preferences}
            params={widgetParams}
            periodOverride={widget.periodOverride}
          />
        );
      case "discovery_ratio":
        return (
          <DiscoveryRatioWidget
            params={widgetParams}
            version={preferences.version}
            periodOverride={widget.periodOverride}
          />
        );
      default:
        return null;
    }
  };

  return (
    <>
      <ErrorText>{result.error}</ErrorText>
      {result.data ? (
        <div className="music-heading">
          <Cover url={result.data.cover_url} large>
            <ArtworkCredits url={result.data.cover_url} />
          </Cover>
          <div className="music-header-content" ref={introduction}>
            <div className="music-title-row">
              <Heading title={result.data.name}>
                {kind === "tracks" && "Track by "}
                {result.data.artists.map(
                  (a: { id: string; name: string }, i: number) => (
                    <span key={a.id}>
                      {i > 0 && ", "}
                      <Link to={musicLink("artists", a.id, params)}>
                        {a.name}
                      </Link>
                    </span>
                  ),
                )}
                {result.data.album_id && (
                  <>
                    {" "}
                    ·{" "}
                    <Link
                      to={musicLink("albums", result.data.album_id, params)}
                    >
                      {result.data.album}
                    </Link>
                  </>
                )}
              </Heading>
              <BackButton
                fallback={`/rankings/${kind}?${context(selection.params)}`}
              />
            </div>
            {!isPreview && <MusicActions kind={kind} id={id} />}
            <div className="music-period">
              {!layoutEditor.editing && (
                <button
                  type="button"
                  className="button secondary"
                  aria-label={
                    kind === "tracks"
                      ? "Personalizza template brano"
                      : "Personalizza template artista"
                  }
                  onClick={() => layoutEditor.handleStartEditing()}
                >
                  <Icon>tune</Icon>{" "}
                  {kind === "tracks"
                    ? "Personalizza template brano"
                    : "Personalizza template artista"}
                </button>
              )}
              <PeriodFilters selection={selection} />
            </div>
          </div>
        </div>
      ) : (
        !result.error && <MusicPageSkeleton />
      )}

      {layoutEditor.editing && (
        <DashboardEditToolbar
          template={templateKind}
          saving={layoutEditor.saving}
          saveError={layoutEditor.saveError}
          onAddWidget={() => layoutEditor.setCatalogOpen(true)}
          onReset={layoutEditor.handleReset}
          onCancel={layoutEditor.handleCancel}
          onSave={layoutEditor.handleSave}
        />
      )}

      <WidgetCatalogDialog
        open={layoutEditor.catalogOpen}
        onClose={() => layoutEditor.setCatalogOpen(false)}
        template={templateKind}
        widgets={layoutEditor.layout.widgets}
        onAdd={layoutEditor.addWidget}
      />

      <div
        id="dashboard-grid"
        className={`dashboard-grid${layoutEditor.editing ? " editing" : ""}`}
      >
        {layoutEditor.layout.widgets.length === 0 ? (
          <div className="dashboard-empty-notice panel padded">
            <p>
              Nessun widget configurato per questo template. Clicca su
              &quot;Aggiungi widget&quot; per personalizzare la pagina.
            </p>
          </div>
        ) : (
          layoutEditor.layout.widgets.map((w, index) => (
            <div
              key={w.id}
              className={`dashboard-widget span-${w.colSpan}${layoutEditor.editing ? " editing" : ""}`}
              data-widget-id={w.id}
              data-widget-type={w.type}
              draggable={layoutEditor.editing && !layoutEditor.saving}
              onDragStart={(e) => {
                if (!layoutEditor.editing || layoutEditor.saving) return;
                e.dataTransfer.setData("text/plain", String(index));
                e.dataTransfer.effectAllowed = "move";
              }}
              onDragOver={(e) => {
                if (!layoutEditor.editing || layoutEditor.saving) return;
                e.preventDefault();
                e.dataTransfer.dropEffect = "move";
              }}
              onDrop={(e) => {
                if (!layoutEditor.editing || layoutEditor.saving) return;
                e.preventDefault();
                const sourceIndex = parseInt(
                  e.dataTransfer.getData("text/plain"),
                  10,
                );
                if (!isNaN(sourceIndex) && sourceIndex !== index) {
                  layoutEditor.setLayout((prev) => ({
                    ...prev,
                    widgets: reorderWidgets(prev.widgets, sourceIndex, index),
                  }));
                }
              }}
            >
              {layoutEditor.editing && (
                <WidgetEditControls
                  widget={w}
                  index={index}
                  totalWidgets={layoutEditor.layout.widgets.length}
                  saving={layoutEditor.saving}
                  onMove={layoutEditor.moveWidget}
                  onCycleSpan={layoutEditor.cycleSpan}
                  onRemove={layoutEditor.removeWidget}
                  onChangePeriod={(val) => {
                    layoutEditor.setLayout((prev) => ({
                      ...prev,
                      widgets: prev.widgets.map((item) =>
                        item.id === w.id
                          ? { ...item, periodOverride: val }
                          : item,
                      ),
                    }));
                  }}
                />
              )}
              {renderMusicWidgetContent(w)}
            </div>
          ))
        )}
      </div>
    </>
  );
}
