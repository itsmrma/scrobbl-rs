import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type {
  ActivityDetail,
  ImportLimits,
  NowPlaying,
  SpotifyStatus,
} from "./generated";
import { api } from "./api";
import {
  Cover,
  duration,
  ErrorText,
  Heading,
  number,
  SpotifyLink,
  useToast,
} from "./ui";
import { useLoad } from "./load";
import { Link } from "./navigation";

export function Now({
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
    </>
  );
}
export { useLoad } from "./load";
