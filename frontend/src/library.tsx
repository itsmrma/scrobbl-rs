import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type {
  Dashboard,
  History,
  ActivityDetail,
  ImportLimits,
  Listen,
  NowPlaying,
  Preferences,
  SpotifyStatus,
} from "./generated";
import { api } from "./api";
import {
  Cover,
  date,
  duration,
  ErrorText,
  Heading,
  number,
  SpotifyLink,
  useToast,
} from "./ui";
import { periods } from "./preferences";
import { Link } from "./navigation";

function useLoad<T>(
  load: () => Promise<T>,
  key: string,
  enabled = true,
  interval?: number,
) {
  const [data, setData] = useState<T | null>(null),
    [error, setError] = useState("");
  useEffect(() => {
    if (!enabled) return;
    setData(null);
    setError("");
    let active = true,
      busy = false;
    const refresh = async () => {
      if (busy) return;
      busy = true;
      try {
        const result = await load();
        if (active) {
          setData(result);
          setError("");
        }
      } catch (e) {
        if (active) setError(String(e instanceof Error ? e.message : e));
      } finally {
        busy = false;
      }
    };
    void refresh();
    const wake = () => {
      if (!document.hidden) void refresh();
    };
    document.addEventListener("visibilitychange", wake);
    const timer = interval
      ? setInterval(() => {
          if (!document.hidden) void refresh();
        }, interval)
      : undefined;
    return () => {
      active = false;
      clearInterval(timer);
      document.removeEventListener("visibilitychange", wake);
    };
    // The serialized key defines the request. Callers avoid request-function identity as a dependency.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, enabled, interval]);
  return { data, error };
}
export function ListenList({
  items,
  preferences,
  id,
}: {
  items: Listen[];
  preferences: Preferences;
  id?: string;
}) {
  return (
    <div id={id}>
      {!items.length ? (
        <div className="empty">
          <strong>Your archive is ready for its first listen</strong>
          <span>Import your past or connect Spotify to start collecting.</span>
          <Link to="/import">Import your Spotify history</Link>
        </div>
      ) : (
        items.map((item) => (
          <article className="listen-row" key={item.id}>
            <Cover url={item.cover_url} />
            <div className="listen-title">
              <SpotifyLink url={item.spotify_url}>{item.title}</SpotifyLink>
              <small>
                {item.artists.map((a) => a.name).join(", ") || "Unknown artist"}
              </small>
            </div>
            <div className="listen-album">{item.album || "Unknown album"}</div>
            <div className="listen-date">
              {date(item.played_at_ms, preferences)}
              <small>
                {item.sources
                  .join(" + ")
                  .replaceAll("spotify_import", "Spotify import")
                  .replaceAll("spotify_api", "Spotify collection")}
              </small>
              {item.needs_review && (
                <small className="warning">Possible duplicate</small>
              )}
            </div>
            <div className="listen-duration">
              {item.duration_kind === "estimated" && "≈ "}
              {duration(item.listened_ms)}
              <small>
                {!item.counted
                  ? "Below threshold"
                  : item.duration_kind === "actual"
                    ? "Actual"
                    : item.duration_kind === "estimated"
                      ? "Estimated"
                      : "Unknown duration"}
              </small>
            </div>
          </article>
        ))
      )}
    </div>
  );
}
function Now({
  visible,
  status,
}: {
  visible: boolean;
  status: SpotifyStatus | null;
}) {
  const [browserTiming, setBrowserTiming] = useState<number | null>(null);
  const [browserCommit, setBrowserCommit] = useState<number | null>(null);
  const requestStart = useRef(0);
  const { data, error } = useLoad<NowPlaying | null>(
    async () => {
      const start = performance.now();
      requestStart.current = start;
      const sample = await api.nowPlaying();
      setBrowserTiming(performance.now() - start);
      return sample;
    },
    "now",
    visible,
    (status?.now_playing_seconds || 30) * 1000,
  );
  useLayoutEffect(() => {
    if (data) setBrowserCommit(performance.now() - requestStart.current);
  }, [data]);
  return (
    <article className="panel now-panel">
      <p className="eyebrow">
        {error ? "SPOTIFY UNAVAILABLE" : data ? "NOW PLAYING" : "NOT PLAYING"}
      </p>
      <div className="now-content">
        <Cover url={data?.cover_url} large />
        <div>
          <h3>
            {data ? (
              <SpotifyLink url={data.spotify_url}>{data.title}</SpotifyLink>
            ) : (
              "Your next favourite awaits"
            )}
          </h3>
          <p>
            {data?.artists.map((a) => a.name).join(", ") ||
              (status?.connected
                ? "Nothing playing on Spotify right now."
                : "Connect Spotify to see what’s playing.")}
          </p>
        </div>
      </div>
      <p className="subtle">
        {error ||
          (data
            ? `${duration(data.progress_ms)} / ${duration(data.duration_ms)} · From Spotify`
            : "Live while this page is open")}
      </p>
      {data && (
        <details>
          <summary>Sample timing</summary>
          <p>
            Source playback-state change:{" "}
            {data.source_timestamp_ms
              ? new Date(data.source_timestamp_ms).toISOString()
              : "not reported"}
            . This is a state-change timestamp, not a measure of Spotify
            propagation delay.
          </p>
          <p>
            Server queue: {data.source_requested_at_ms - data.requested_at_ms}{" "}
            ms | provider request:{" "}
            {data.response_received_at_ms - data.source_requested_at_ms} ms |
            cached sample age when served:{" "}
            {Math.max(0, data.served_at_ms - data.response_received_at_ms)} ms |
            browser request/response:{" "}
            {browserTiming === null ? "unknown" : Math.round(browserTiming)} ms.
          </p>
          <p>
            Browser request through render commit:{" "}
            {browserCommit === null ? "unknown" : Math.round(browserCommit)} ms.
          </p>
          <p>
            Polled every {status?.now_playing_seconds || 30} seconds while
            visible; source delay requires comparison with actual playback.
          </p>
        </details>
      )}
      <small>
        Now playing is separate from background collection.{" "}
        <Link to="/settings/connections">Check collection status</Link>.
      </small>
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
  const [period, setPeriod] = useState(preferences.initial_period),
    [sort, setSort] = useState("plays"),
    [from, setFrom] = useState(""),
    [to, setTo] = useState("");
  const chosen = useRef(false);
  useEffect(() => {
    if (!chosen.current) setPeriod(preferences.initial_period);
  }, [preferences.initial_period]);
  const params = new URLSearchParams({
    period,
    sort,
    ...(period === "custom" ? { from, to } : {}),
  });
  const { data, error } = useLoad<Dashboard>(
    () => api.dashboard(params),
    params.toString(),
    visible,
    (status?.sync_seconds || 120) * 1000,
  );
  return (
    <>
      <Heading
        eyebrow="YOUR PERSONAL LISTENING ARCHIVE"
        title="Your listening, at a glance"
      >
        The songs, the repeats, the moments. All yours.
      </Heading>
      <div className="filters">
        <label>
          Period
          <select
            aria-label="Period"
            id="period"
            value={period}
            onChange={(e) => {
              chosen.current = true;
              setPeriod(e.target.value);
            }}
          >
            {periods.map(([v, text]) => (
              <option key={v} value={v}>
                {text}
              </option>
            ))}
            <option value="custom">Custom dates</option>
          </select>
        </label>
        {period === "custom" && (
          <>
            <label>
              From
              <input
                type="date"
                value={from}
                onChange={(e) => setFrom(e.target.value)}
              />
            </label>
            <label>
              Through
              <input
                type="date"
                value={to}
                onChange={(e) => setTo(e.target.value)}
              />
            </label>
          </>
        )}
        <label>
          Rank by
          <select value={sort} onChange={(e) => setSort(e.target.value)}>
            <option value="plays">Listens</option>
            <option value="time">Listening time</option>
          </select>
        </label>
      </div>
      <ErrorText>{error}</ErrorText>
      {data ? (
        <>
          <div className="stats-grid">
            <article className="panel stat">
              <span>Total listens</span>
              <strong id="total-listens">
                {number.format(data.totals.listens)}
              </strong>
            </article>
            <article className="panel stat">
              <span>
                Listening time{" "}
                <abbr title="Actual time is reported by a source. Estimated time uses track length and does not prove a full play. Unknown time is not zero.">
                  i
                </abbr>
              </span>
              <strong id="total-time">
                {duration(
                  data.totals.actual_ms + data.totals.estimated_ms,
                  true,
                )}
              </strong>
              <small>
                {duration(data.totals.actual_ms, true)} actual ·{" "}
                {duration(data.totals.estimated_ms, true)} estimated ·{" "}
                {data.totals.unknown_durations} unknown
              </small>
            </article>
            <article className="panel stat">
              <span>Different tracks</span>
              <strong id="total-tracks">
                {number.format(data.totals.tracks)}
              </strong>
            </article>
          </div>
          <Now visible={visible} status={status} />
          <div className="rankings-grid">
            {(["artists", "albums", "tracks"] as const).map((kind) => (
              <article className="panel padded" key={kind}>
                <h2>Top {kind}</h2>
                <ol className="ranking-list">
                  {data.rankings[kind].length ? (
                    data.rankings[kind].map((item, index) => (
                      <li className="ranking-row" key={item.id}>
                        <span className="rank-number">
                          {String(index + 1).padStart(2, "0")}
                        </span>
                        <Cover url={item.cover_url} />
                        <div className="rank-name">
                          <strong>{item.name}</strong>
                          <small>
                            {item.subtitle ||
                              `${number.format(item.plays)} listens`}
                          </small>
                        </div>
                        <span className="rank-value">
                          {sort === "time"
                            ? duration(item.listened_ms, true)
                            : number.format(item.plays)}
                        </span>
                      </li>
                    ))
                  ) : (
                    <li className="empty">Your favourites will appear here.</li>
                  )}
                </ol>
              </article>
            ))}
          </div>
          <article className="panel">
            <div className="panel-title padded">
              <h2>Recent listening</h2>
              <Link to="/history">View history</Link>
            </div>
            <ListenList items={data.recent} preferences={preferences} />
          </article>
        </>
      ) : (
        !error && <p role="status">Loading your archive…</p>
      )}
    </>
  );
}
export function HistoryPage({
  preferences,
  visible,
}: {
  preferences: Preferences;
  visible: boolean;
}) {
  const blank = { q: "", from: "", to: "", status: "all" };
  const [draft, setDraft] = useState(blank),
    [filters, setFilters] = useState(blank),
    [page, setPage] = useState(1);
  const params = new URLSearchParams({ ...filters, page: String(page) });
  const { data, error } = useLoad<History>(
    () => api.history(params),
    params.toString(),
    visible,
  );
  return (
    <>
      <Heading title="Listening history" eyebrow="EVERY LISTEN HAS A STORY">
        Search the archive, including short plays and original source records.
      </Heading>
      <form
        className="filters"
        onSubmit={(e) => {
          e.preventDefault();
          setFilters(draft);
          setPage(1);
        }}
      >
        <label>
          Search
          <input
            id="history-q"
            value={draft.q}
            placeholder="Artist, album or track"
            onChange={(e) => setDraft({ ...draft, q: e.target.value })}
          />
        </label>
        <label>
          From
          <input
            type="date"
            value={draft.from}
            onChange={(e) => setDraft({ ...draft, from: e.target.value })}
          />
        </label>
        <label>
          Through
          <input
            type="date"
            value={draft.to}
            onChange={(e) => setDraft({ ...draft, to: e.target.value })}
          />
        </label>
        <label>
          Include
          <select
            id="history-status"
            value={draft.status}
            onChange={(e) => setDraft({ ...draft, status: e.target.value })}
          >
            {[
              ["all", "All records"],
              ["counted", "Counted listens"],
              ["short", "Below threshold"],
              ["actual", "Actual time"],
              ["estimated", "Estimated / unknown time"],
              ["review", "Possible duplicates"],
            ].map(([v, text]) => (
              <option key={v} value={v}>
                {text}
              </option>
            ))}
          </select>
        </label>
        <button type="submit" className="primary">
          Search
        </button>
        <button
          type="button"
          className="secondary"
          onClick={() => {
            setDraft(blank);
            setFilters(blank);
            setPage(1);
          }}
        >
          Reset
        </button>
      </form>
      <ErrorText>{error}</ErrorText>
      {data && (
        <article className="panel">
          <div className="panel-title padded">
            <h2 id="history-total">{number.format(data.total)} records</h2>
          </div>
          <ListenList
            id="history-list"
            items={data.items}
            preferences={preferences}
          />
          <div className="pagination">
            <button
              className="secondary"
              disabled={page <= 1}
              onClick={() => setPage(page - 1)}
            >
              Previous
            </button>
            <span>
              Page {page} of{" "}
              {Math.max(1, Math.ceil(data.total / data.page_size))}
            </span>
            <button
              className="secondary"
              disabled={page * data.page_size >= data.total}
              onClick={() => setPage(page + 1)}
            >
              Next
            </button>
          </div>
        </article>
      )}
    </>
  );
}
export function ImportPage({ refresh }: { refresh: () => Promise<void> }) {
  const [files, setFiles] = useState<FileList | null>(null);
  const [busy, setBusy] = useState(false);
  const [jobId, setJobId] = useState("");
  const [error, setError] = useState("");
  const [version, setVersion] = useState(0);
  const limits = useLoad<ImportLimits>(api.importLimits, "limits");
  const report = useLoad<ActivityDetail>(
    () => api.activity(jobId),
    `${jobId}:${version}`,
    !!jobId,
    1000,
  );
  const toast = useToast();
  const job = report.data?.activity;
  const lastState = useRef("");
  useEffect(() => {
    if (!job) return;
    const state = `${job.id}:${job.state}:${job.outcome}`;
    if (lastState.current === state) return;
    lastState.current = state;
    if (["completed", "failed"].includes(job.state)) {
      toast(
        job.outcome === "complete"
          ? "Import complete. Report saved in Activities."
          : `Import ${job.outcome || job.state}. Open the saved report for details.`,
        job.outcome === "error",
      );
      void refresh();
    }
  }, [job?.id, job?.state, job?.outcome]);
  return (
    <>
      <Heading
        title="Import your listening history"
        eyebrow="BRING YOUR PAST WITH YOU"
      >
        Add your Spotify Extended Streaming History ZIP or JSON files.
      </Heading>
      <div className="two-column">
        <article className="panel padded">
          <form
            onSubmit={async (e) => {
              e.preventDefault();
              setBusy(true);
              setError("");
              setJobId("");
              try {
                const form = new FormData();
                if (files) for (const file of files) form.append("files", file);
                const result = await api.importSpotify(form);
                setJobId(result.job_id);
                setVersion((v) => v + 1);
                toast(
                  "Files received. Processing continues on the server; you can close this page.",
                );
                await refresh();
              } catch (e) {
                const message = `${e instanceof Error ? e.message : e} Check Activities: a lost connection does not prove processing failed. Sending the complete upload again is safe.`;
                setError(message);
                toast(message, true);
                await refresh();
              } finally {
                setBusy(false);
              }
            }}
          >
            <label>
              History ZIP or JSON files
              <input
                id="import-files"
                type="file"
                accept=".zip,.json,application/json,application/zip"
                multiple
                required
                onChange={(e) => setFiles(e.target.files)}
              />
            </label>
            {limits.data ? (
              <p>
                Upload limit: {number.format(limits.data.upload_bytes)} bytes
                per selection. Expanded limit:{" "}
                {number.format(limits.data.expanded_bytes)} bytes; per file:{" "}
                {number.format(limits.data.member_bytes)} bytes; per JSON row:{" "}
                {number.format(limits.data.row_bytes)} bytes; at most{" "}
                {number.format(limits.data.files)} files. Limits are configured
                on the server.
              </p>
            ) : (
              <p>Loading server limits...</p>
            )}
            <ErrorText>{limits.error}</ErrorText>
            <button
              className="primary"
              disabled={busy || !files?.length || !limits.data}
            >
              {busy
                ? "Receiving files... Keep this page open"
                : "Import listening history"}
            </button>
          </form>
          <ErrorText>{error || report.error}</ErrorText>
          {job && (
            <div id="import-result" role="status">
              <h3>
                {job.outcome === "complete"
                  ? "Import complete"
                  : job.outcome === "no_music"
                    ? "No music found"
                    : `Import ${job.outcome || job.state}`}
              </h3>
              <p>
                Phase: {job.phase}. {job.processed} source rows processed.
              </p>
              <p>
                {number.format(job.added)} added |{" "}
                {number.format(job.reconciled)} reconciled |{" "}
                {number.format(job.duplicates)} already present |{" "}
                {job.originals} new originals | {job.variants} new variants |{" "}
                {job.skipped} non-music skipped | {job.invalid} invalid |{" "}
                {job.below_threshold} below threshold
              </p>
              <p>
                Row and original counts describe source coverage; they are not a
                sum of new listens.
              </p>
              {job.ambiguous > 0 && (
                <p className="warning">
                  {job.ambiguous} ambiguous matches were kept separate.
                </p>
              )}
              <Link to={`/activities/${job.id}`}>
                Open full saved report and retry controls
              </Link>
              <ul>
                {report.data?.members.map((m) => (
                  <li key={m.member}>
                    <strong>{m.member}</strong>: {m.kind} | {m.state} |{" "}
                    {m.processed} processed / {m.expected_rows ?? "unknown"}{" "}
                    rows{" "}
                    {m.error_code && (
                      <p className="form-error">{m.error_code}</p>
                    )}
                  </li>
                ))}
              </ul>
              <Link to="/dashboard">Explore your archive</Link>
            </div>
          )}
        </article>
        <aside className="panel padded">
          <h2>Keep your originals</h2>
          <p>
            Upload the entire ZIP without extracting or splitting it. The report
            inventories every member, including nested folders and excluded
            content.
          </p>
          <p>
            Keep this page open until receipt is confirmed. After receipt,
            processing continues with the page closed and resumes after a server
            restart. Reports remain in Activities.
          </p>
          <p>
            Originals, variants and source positions are preserved. Reimporting
            renamed or reformatted files does not multiply listens. Podcast and
            audiobook rows are excluded. Music below 30 seconds remains in the
            archive and stays outside valid-listen statistics.
          </p>
          <Link to="/activities">View persistent activities</Link>
        </aside>
      </div>
    </>
  );
}
export { useLoad };
