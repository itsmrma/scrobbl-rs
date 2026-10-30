import {
  createContext,
  useContext,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import type { ReactNode } from "react";
import type {
  ActivityDetail,
  ImportLimits,
  NowPlaying,
  Preferences,
  SpotifyStatus,
} from "./generated";
import { api } from "./api";
import {
  Cover,
  duration,
  ErrorText,
  Heading,
  Icon,
  number,
  useToast,
} from "./ui";
import { useLoad } from "./load";
import { Link } from "./navigation";
import { musicLink } from "./selection";
import { ListenBrainzImport } from "./listenbrainz";
import { phaseLabel, Progress } from "./activities";
import { isBool, useBrowserPreference } from "./preferences";
import { MusicPageSkeleton, SkeletonList, SkeletonPlayback } from "./skeleton";

const NowContext = createContext<{
  data: NowPlaying | null;
  error: string;
  loading: boolean;
}>({
  data: null,
  error: "",
  loading: true,
});
export function NowPlayingProvider({
  status,
  statusError,
  children,
}: {
  status: SpotifyStatus | null;
  statusError: string;
  children: ReactNode;
}) {
  const enabled =
    !!status?.connected && (status?.now_playing_seconds ?? 30) > 0;
  const { data, error, loading } = useLoad<NowPlaying | null>(
    api.nowPlaying,
    `now:${enabled}`,
    enabled,
    (status?.now_playing_seconds || 30) * 1000,
  );
  return (
    <NowContext
      value={{
        data: status?.connected ? data : null,
        error: statusError || (status?.connected ? error : ""),
        loading: !status ? !statusError : status.connected && loading && !data,
      }}
    >
      {children}
    </NowContext>
  );
}
export function NowPlayingPage() {
  const { data, error, loading } = useContext(NowContext);
  return (
    <>
      <Link className="back-link" to="/dashboard">
        <Icon>arrow_back</Icon>Overview
      </Link>
      <ErrorText>{error}</ErrorText>
      {loading ? (
        <MusicPageSkeleton label="Loading now playing…" />
      ) : data ? (
        <>
          <div className="music-heading">
            <Cover url={data.cover_url} large />
            <Heading title={data.title}>
              Track by {data.artists.map((artist) => artist.name).join(", ")}
            </Heading>
            <div className="music-personal">
              <a
                className="spotify-attribution"
                href={data.spotify_url}
                target="_blank"
                rel="noreferrer"
              >
                View on Spotify<Icon>open_in_new</Icon>
              </a>
            </div>
          </div>
          <article className="panel padded">
            <h2>Playing now</h2>
            <p>{data.album || "Album not reported"}</p>
            {data.track_id ? (
              <Link
                className="button secondary"
                to={musicLink("tracks", data.track_id)}
              >
                Open track history<Icon>arrow_forward</Icon>
              </Link>
            ) : (
              <p>
                This track has not been collected in your archive yet. Its
                listening history will appear after collection.
              </p>
            )}
          </article>
        </>
      ) : (
        !error && (
          <Heading title="Nothing playing">
            Start a track on Spotify to see it here.
          </Heading>
        )
      )}
    </>
  );
}
export function Now({
  status,
  paused = false,
}: {
  status: SpotifyStatus | null;
  paused?: boolean;
}) {
  const { data, error, loading } = useContext(NowContext);
  const [expanded, setExpanded] = useState(false);
  const [mobileMinimized, setMobileMinimized] = useBrowserPreference(
    "now-playing-minimized",
    false,
    isBool,
  );
  const [clock, setClock] = useState(Date.now());
  const titleText = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    const mobile = matchMedia("(max-width: 600px)");
    const resetDesktopState = () => {
      if (!mobile.matches) setMobileMinimized(false);
    };
    mobile.addEventListener("change", resetDesktopState);
    return () => mobile.removeEventListener("change", resetDesktopState);
  }, []);
  useLayoutEffect(() => {
    const text = titleText.current;
    const container = text?.parentElement;
    if (!text || !container) return;
    const reduced = matchMedia("(prefers-reduced-motion: reduce)");
    let animation: Animation | undefined;
    const update = () => {
      animation?.cancel();
      const distance = text.scrollWidth - container.clientWidth;
      if (reduced.matches || !container.clientWidth || distance < 1) return;
      const end = `translateX(-${distance}px)`;
      animation = text.animate(
        [
          { transform: "translateX(0)", offset: 0 },
          { transform: "translateX(0)", offset: 0.1 },
          { transform: end, offset: 0.45 },
          { transform: end, offset: 0.55 },
          { transform: "translateX(0)", offset: 0.9 },
          { transform: "translateX(0)", offset: 1 },
        ],
        {
          duration: Math.max(5000, Math.min(30000, 2000 + distance * 80)),
          iterations: Infinity,
          easing: "linear",
        },
      );
    };
    const observer = new ResizeObserver(update);
    observer.observe(text);
    observer.observe(container);
    reduced.addEventListener("change", update);
    update();
    return () => {
      observer.disconnect();
      reduced.removeEventListener("change", update);
      animation?.cancel();
    };
  }, [data?.title]);
  useEffect(() => {
    if (!data || paused) return;
    setClock(Date.now());
    const timer = setInterval(() => {
      if (!document.hidden) setClock(Date.now());
    }, 1000);
    return () => clearInterval(timer);
  }, [data, paused]);
  const progress =
    data?.progress_ms == null
      ? null
      : Math.min(
          data.duration_ms ?? Infinity,
          data.progress_ms + Math.max(0, clock - data.response_received_at_ms),
        );
  if (loading) return <SkeletonPlayback />;
  return (
    <div
      className={`sidebar-now ${data ? "is-playing" : "is-idle"} ${expanded && data?.cover_url ? "artwork-expanded" : ""} ${mobileMinimized ? "mobile-minimized" : ""}`}
      aria-label="Now playing"
    >
      <button
        type="button"
        className="now-section-toggle"
        aria-label={
          mobileMinimized ? "Expand now playing" : "Collapse now playing"
        }
        aria-expanded={!mobileMinimized}
        onClick={() => setMobileMinimized(!mobileMinimized)}
      >
        <Icon>{mobileMinimized ? "expand_more" : "expand_less"}</Icon>
      </button>
      <div
        className={`now-details${mobileMinimized ? " mobile-minimized" : ""}`}
      >
        <Cover url={data?.cover_url} />
        {data?.cover_url && (
          <button
            className="icon-button now-expand-toggle nav-copy"
            aria-label={
              expanded
                ? "Collapse now playing artwork"
                : "Expand now playing artwork"
            }
            aria-expanded={expanded}
            onClick={() => setExpanded(!expanded)}
          >
            <Icon>{expanded ? "expand_more" : "expand_less"}</Icon>
          </button>
        )}
        <div
          className="now-artwork-expansion nav-copy"
          aria-hidden={!expanded || !data?.cover_url}
          inert={!expanded || !data?.cover_url}
        >
          <div>{data?.cover_url && <Cover url={data.cover_url} large />}</div>
        </div>
        <div className="nav-copy playback-copy">
          <small>
            {error
              ? "Playback unavailable"
              : data
                ? "Listening on Spotify"
                : "Now playing"}
          </small>
          {data ? (
            <>
              <Link
                className="playback-title"
                to={
                  data.track_id
                    ? musicLink("tracks", data.track_id)
                    : "/music/now-playing"
                }
              >
                <span
                  className="playback-title-text"
                  ref={titleText}
                  title={data.title}
                >
                  {data.title}
                </span>
              </Link>
              <span>{data.artists.map((a) => a.name).join(", ")}</span>
            </>
          ) : (
            <span>
              {status?.connected
                ? "Nothing playing"
                : "Connect Spotify in Settings"}
            </span>
          )}
        </div>
        {data && (
          <div className="sidebar-playback nav-copy">
            {progress !== null && data.duration_ms && (
              <progress
                aria-label="Playback progress"
                value={progress}
                max={data.duration_ms}
              />
            )}
            <div className="playback-time">
              <span>{duration(progress)}</span>
              <Icon>graphic_eq</Icon>
              <span>{duration(data.duration_ms)}</span>
            </div>
          </div>
        )}
      </div>
      <span className="collapsed-label">
        {data
          ? `${data.title} · ${data.artists.map((a) => a.name).join(", ")}`
          : "Nothing playing"}
      </span>
    </div>
  );
}
export function ImportPage({
  refresh,
  visible,
  preferences,
}: {
  refresh: () => Promise<void>;
  visible: boolean;
  preferences: Preferences;
}) {
  const [files, setFiles] = useState<File[]>([]);
  const [dragging, setDragging] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState(false);
  const [jobId, setJobId] = useState("");
  const [error, setError] = useState("");
  const [version, setVersion] = useState(0);
  const limits = useLoad<ImportLimits>(api.importLimits, "limits", visible);
  const report = useLoad<ActivityDetail>(
    () => api.activity(jobId),
    `${jobId}:${version}`,
    !!jobId,
    1000,
  );
  const toast = useToast();
  const job = report.data?.activity;
  const lastState = useRef("");
  function selectFiles(selection: File[]) {
    if (busy) return;
    if (selection.some((file) => !/\.(zip|json)$/i.test(file.name))) {
      setError(
        "Choose a Spotify history ZIP or JSON files. Other file types cannot be imported.",
      );
      return;
    }
    if (
      limits.data &&
      selection.reduce((size, file) => size + file.size, 0) >
        limits.data.upload_bytes
    ) {
      setError(
        "This selection exceeds the server upload limit. Choose fewer files or a smaller ZIP.",
      );
      return;
    }
    setFiles(selection);
    setError("");
  }
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
      <Heading title="Import your listening history">
        Add your Spotify Extended Streaming History or ListenBrainz archive.
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
            <div
              id="spotify-dropzone"
              className={`upload-dropzone ${dragging ? "dragging" : ""}`}
              onDragOver={(e) => {
                e.preventDefault();
                if (!busy) {
                  e.dataTransfer.dropEffect = "copy";
                  setDragging(true);
                }
              }}
              onDragLeave={(e) => {
                if (!e.currentTarget.contains(e.relatedTarget as Node | null))
                  setDragging(false);
              }}
              onDrop={(e) => {
                e.preventDefault();
                setDragging(false);
                selectFiles(Array.from(e.dataTransfer.files));
              }}
              aria-busy={busy}
            >
              <Icon>upload_file</Icon>
              <h2>Drop your Spotify history here</h2>
              <p>
                One ZIP, or several JSON files. Your files stay on your server.
              </p>
              <button
                type="button"
                className="secondary"
                disabled={busy}
                onClick={() => input.current?.click()}
              >
                <Icon>folder_open</Icon>Choose files
              </button>
              <input
                ref={input}
                id="import-files"
                className="visually-hidden"
                tabIndex={-1}
                aria-label="History ZIP or JSON files"
                type="file"
                accept=".zip,.json,application/json,application/zip"
                multiple
                disabled={busy}
                onChange={(e) => selectFiles(Array.from(e.target.files || []))}
              />
            </div>
            {files.length > 0 && (
              <div className="upload-selection" role="status">
                <strong>
                  {files.length} {files.length === 1 ? "file" : "files"} ready ·{" "}
                  {number.format(
                    Math.ceil(
                      files.reduce((size, file) => size + file.size, 0) / 1024,
                    ),
                  )}{" "}
                  KB
                </strong>
                <ul>
                  {files.map((file, i) => (
                    <li key={`${file.name}:${i}`}>
                      <Icon>description</Icon>
                      <span>{file.name}</span>
                      <small>
                        {number.format(Math.ceil(file.size / 1024))} KB
                      </small>
                    </li>
                  ))}
                </ul>
                <button
                  type="button"
                  className="secondary"
                  disabled={busy}
                  onClick={() => {
                    setFiles([]);
                    if (input.current) input.current.value = "";
                  }}
                >
                  Clear selection
                </button>
              </div>
            )}
            {limits.data ? (
              <details className="upload-limits">
                <summary>Which files can I upload?</summary>
                <p>
                  Choose the ZIP from Spotify, or the JSON history files inside
                  it. You can upload up to {number.format(limits.data.files)}{" "}
                  files together, totalling{" "}
                  {number.format(
                    Math.floor(limits.data.upload_bytes / 1024 / 1024),
                  )}{" "}
                  MB. Leave ZIP files zipped — we open them for you.
                </p>
                <p>
                  For large archives, the files inside the ZIP can total up to{" "}
                  {number.format(
                    Math.floor(limits.data.expanded_bytes / 1024 / 1024),
                  )}{" "}
                  MB, with no single file larger than{" "}
                  {number.format(
                    Math.floor(limits.data.member_bytes / 1024 / 1024),
                  )}{" "}
                  MB. If your selection is too large, upload fewer files at a
                  time.
                </p>
              </details>
            ) : (
              !limits.error && (
                <SkeletonList count={1} label="Checking upload availability…" />
              )
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
          {jobId && !report.data && !report.error && (
            <SkeletonList count={2} label="Loading import report…" />
          )}
          {job && (
            <div id="import-result" role="status">
              <h3>
                {job.outcome === "complete"
                  ? "Import complete"
                  : job.outcome === "no_music"
                    ? "No music found"
                    : `Import ${job.outcome || job.state}`}
              </h3>
              <p>{phaseLabel(job)}</p>
              <Progress job={job} />
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
            Normalized observations, variant fingerprints and source positions
            are preserved. Raw JSON is discarded; source files are released
            after successful import. Keep your own ZIP for future reimports.
            Reimporting renamed or reformatted files does not multiply listens.
            Podcast and audiobook rows are excluded. Music below 30 seconds
            remains in the archive and stays outside valid-listen statistics.
          </p>
          <Link to="/activities">View persistent activities</Link>
        </aside>
      </div>
      <ListenBrainzImport
        refresh={refresh}
        visible={visible}
        preferences={preferences}
      />
    </>
  );
}
