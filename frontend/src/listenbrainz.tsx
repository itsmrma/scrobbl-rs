import { useRef, useState } from "react";
import { api } from "./api";
import type { ListenBrainzOriginalsPage } from "./api";
import type { Preferences } from "./generated";
import { useLoad } from "./load";
import { ErrorText, Icon, useToast } from "./ui";
import { Link } from "./navigation";
import { SkeletonList } from "./skeleton";

export function ListenBrainzImport({
  refresh,
  visible,
  preferences,
}: {
  refresh: () => Promise<void>;
  visible: boolean;
  preferences: Preferences;
}) {
  const limits = useLoad(
    api.listenBrainzLimits,
    "listenbrainz-limits",
    visible,
  );
  const [method, setMethod] = useState("upload");
  const [files, setFiles] = useState<File[]>([]);
  const input = useRef<HTMLInputElement>(null);
  const [dragging, setDragging] = useState(false);
  const [account, setAccount] = useState("");
  const [username, setUsername] = useState("");
  const [from, setFrom] = useState(""),
    [to, setTo] = useState("");
  const [busy, setBusy] = useState(""),
    [error, setError] = useState("");
  const [jobs, setJobs] = useState<string[]>([]);
  const toast = useToast();
  async function submit(kind: string) {
    setBusy(kind);
    setError("");
    try {
      let result: { jobs: string[] };
      if (kind === "upload") {
        const form = new FormData();
        form.append("account", account);
        if (files) for (const file of files) form.append("files", file);
        result = await api.importListenBrainz(form);
      } else {
        result = await api.fetchListenBrainz({ username, from, to });
      }
      setJobs(result.jobs);
      toast(
        "ListenBrainz activity saved. Processing continues on the server; follow it in Activities.",
      );
      await refresh();
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setError(message);
      toast(message, true);
    } finally {
      setBusy("");
    }
  }
  return (
    <article className="panel padded listenbrainz-import">
      <div className="panel-title">
        <h2>ListenBrainz</h2>
        <span className="pill">History import</span>
      </div>
      <p>
        Bring your listening history into your archive. Use an export for your
        full history, or fetch a short public interval.
      </p>
      <div
        className="import-methods"
        role="group"
        aria-label="ListenBrainz import method"
      >
        <button
          type="button"
          className="secondary"
          aria-pressed={method === "upload"}
          onClick={() => setMethod("upload")}
        >
          <Icon>upload_file</Icon>Import files
        </button>
        <button
          type="button"
          className="secondary"
          aria-pressed={method === "fetch"}
          onClick={() => setMethod("fetch")}
        >
          <Icon>cloud_download</Icon>Fetch public listens
        </button>
      </div>
      <div className="listenbrainz-method">
        <form
          hidden={method !== "upload"}
          onSubmit={(e) => {
            e.preventDefault();
            void submit("upload");
          }}
        >
          <h3>Import your export</h3>
          <p>
            Upload the official ZIP, JSON or JSONL files. For a full history,{" "}
            <a
              href="https://listenbrainz.org/settings/export/"
              target="_blank"
              rel="noreferrer"
            >
              download your ListenBrainz export
            </a>
            .
          </p>
          <label>
            ListenBrainz username for files (optional)
            <input
              value={account}
              onChange={(e) => setAccount(e.target.value)}
              maxLength={64}
            />
          </label>
          <details className="import-help">
            <summary>When do I need a username?</summary>
            <p>
              The official ZIP and API responses identify their account
              automatically. For standalone files, enter the same account to
              recognize reimports across formats. Without an account, uncertain
              matches stay separate and are flagged for review.
            </p>
          </details>
          <div
            className={`upload-dropzone ${dragging ? "dragging" : ""}`}
            onDragOver={(e) => {
              e.preventDefault();
              setDragging(true);
            }}
            onDragLeave={() => setDragging(false)}
            onDrop={(e) => {
              e.preventDefault();
              setDragging(false);
              if (!busy) setFiles(Array.from(e.dataTransfer.files));
            }}
          >
            <Icon>upload_file</Icon>
            <h3>Drop ListenBrainz files here</h3>
            <p>JSON, JSONL or ZIP</p>
            <button
              type="button"
              className="secondary"
              disabled={!!busy}
              onClick={() => input.current?.click()}
            >
              Choose ListenBrainz files
            </button>
            <input
              ref={input}
              className="visually-hidden"
              type="file"
              aria-label="ListenBrainz history files"
              accept=".json,.jsonl,.zip"
              multiple
              onChange={(e) => setFiles(Array.from(e.target.files || []))}
            />
            {files.length > 0 && (
              <ul>
                {files.map((file, i) => (
                  <li key={i}>{file.name}</li>
                ))}
              </ul>
            )}
          </div>
          {limits.data && (
            <details className="import-help">
              <summary>File limits &amp; storage</summary>
              <p>
                Up to 50 files per upload;{" "}
                {Math.floor(limits.data.upload_bytes / 1048576)} MiB per file,{" "}
                {Math.floor(limits.data.expanded_bytes / 1048576)} MiB expanded,{" "}
                {limits.data.members} ZIP members and{" "}
                {Math.floor(limits.data.row_bytes / 1024)} KiB per row or JSON
                metadata section. Oversized parts are reported; originals are
                retained for unfinished or partial imports. Keep your original
                export: successfully imported files are released from server
                storage.
              </p>
            </details>
          )}
          <ErrorText>{limits.error}</ErrorText>
          {!limits.data && !limits.error && (
            <SkeletonList
              count={1}
              label="Loading ListenBrainz upload limits…"
            />
          )}
          <button className="primary" disabled={!!busy || !files?.length}>
            {busy === "upload"
              ? "Uploading… Keep this page open until received"
              : "Upload ListenBrainz files"}
          </button>
        </form>
        <form
          hidden={method !== "fetch"}
          onSubmit={(e) => {
            e.preventDefault();
            void submit("fetch");
          }}
        >
          <h3>Fetch recent public listens</h3>
          <p>
            No token needed. Up to 31 days and ten pages per fetch. Use a file
            export for a complete history.
          </p>
          <label>
            Public ListenBrainz username
            <input
              required
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              maxLength={64}
            />
          </label>
          <div className="listenbrainz-dates">
            <label>
              ListenBrainz start date
              <input
                type="date"
                value={from}
                onChange={(e) => setFrom(e.target.value)}
              />
            </label>
            <label>
              ListenBrainz end date
              <input
                type="date"
                value={to}
                onChange={(e) => setTo(e.target.value)}
              />
            </label>
          </div>
          <details className="import-help">
            <summary>Dates, retries &amp; coverage</summary>
            <p>
              Dates include the whole selected day in {preferences.timezone}.
              Without a start date: the last 14 days of the interval. Without an
              end date: stop at the moment you start. The saved interval stays
              fixed during retries.
            </p>
            <p>
              No token required. For months, years or a complete history,
              download your ZIP from{" "}
              <a
                href="https://listenbrainz.org/settings/export/"
                target="_blank"
                rel="noreferrer"
              >
                ListenBrainz Settings → Export
              </a>{" "}
              and upload it here. Public fetches are limited to 31 days and ten
              pages; uncertain coverage is reported with the ZIP action.
            </p>
          </details>
          <button className="secondary" disabled={!!busy || !username.trim()}>
            {busy === "fetch"
              ? "Saving activity…"
              : "Fetch public ListenBrainz listens"}
          </button>
        </form>
      </div>
      <ErrorText>{error}</ErrorText>
      {!!jobs.length && (
        <div role="status">
          <p>
            Saved on the server. You can close this page. Follow progress, saved
            rows and any partial results:
          </p>
          {jobs.map((id) => (
            <p key={id}>
              <Link to={`/activities/${id}`}>View ListenBrainz activity</Link>
            </p>
          ))}
        </div>
      )}
      <details className="import-help">
        <summary>How imported listens are counted and matched</summary>
        <p>
          Actual listening time, estimates and unknown durations remain
          separate. Plays below 30 seconds of actual listening are retained and
          excluded from valid statistics. Matches based only on names or nearby
          times stay separate.
        </p>
      </details>
    </article>
  );
}

export function ListenBrainzOriginals({ id }: { id: string }) {
  const [offset, setOffset] = useState(0);
  const [data, setData] = useState<ListenBrainzOriginalsPage | null>(null),
    [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  async function load(next: number) {
    setBusy(true);
    setError("");
    try {
      const result = await api.listenBrainzOriginals(id, next);
      setData(result);
      setOffset(next);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section>
      <h3>Source observations &amp; provenance</h3>
      <p>
        Compact storage keeps source positions, fingerprints and reasons, but no
        raw row payloads. Original files are available only while needed for
        incomplete or partial imports.
      </p>
      <button
        className="secondary"
        disabled={busy}
        onClick={() => void load(0)}
      >
        Read saved source rows
      </button>
      <ErrorText>{error}</ErrorText>
      {busy && !data && (
        <SkeletonList count={3} label="Loading source observations…" />
      )}
      {data && (
        <>
          <p>
            {data.total} saved source rows · showing{" "}
            {data.total ? offset + 1 : 0}–{Math.min(offset + 10, data.total)}
          </p>
          {data.files.map((file) => (
            <p key={`${file.file_id}:${file.name}`}>
              <a
                href={`/api/v1/listenbrainz/jobs/${encodeURIComponent(id)}/files/${encodeURIComponent(file.file_id)}`}
                download
              >
                Download original: {file.name}
              </a>
            </p>
          ))}
          {data.rows.map((row) => (
            <details key={`${id}:${row.member}:${row.row_number}`}>
              <summary>
                {row.member} · row {row.row_number} · {row.outcome}
              </summary>
              {row.reason && <p>{row.reason}</p>}
              <p>Raw row payload unavailable under compact storage.</p>
            </details>
          ))}
          <div className="connection-actions">
            <button
              className="secondary"
              disabled={busy || offset === 0}
              onClick={() => void load(Math.max(0, offset - 10))}
            >
              Previous original rows
            </button>
            <button
              className="secondary"
              disabled={busy || offset + 10 >= data.total}
              onClick={() => void load(offset + 10)}
            >
              Next original rows
            </button>
          </div>
        </>
      )}
    </section>
  );
}
