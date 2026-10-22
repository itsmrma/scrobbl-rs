import { useRef, useState } from "react";
import { api } from "./api";
import { useLoad } from "./library";
import { ErrorText, Icon, useToast } from "./ui";
import { Link } from "./navigation";

export function ListenBrainzImport({
  refresh,
}: {
  refresh: () => Promise<void>;
}) {
  const limits = useLoad(api.listenBrainzLimits, "listenbrainz-limits");
  const preferences = useLoad(api.preferences, "listenbrainz-calendar");
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
      <h2>ListenBrainz</h2>
      <p>
        Import an official ZIP, JSONL files, a JSON array or an API history
        response. Normalized observations and saved reports stay in the archive.
        Keep your original ZIP; compact storage releases successfully imported
        files.
      </p>
      <div className="two-column">
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void submit("upload");
          }}
        >
          <label>
            ListenBrainz username for files (optional)
            <input
              value={account}
              onChange={(e) => setAccount(e.target.value)}
              maxLength={64}
            />
          </label>
          <p>
            The official ZIP and API responses identify their account
            automatically. For standalone files, enter the same account to
            recognize reimports across formats. Without an account, uncertain
            matches stay separate and are flagged for review.
          </p>
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
            <p>
              Up to 50 files per upload;{" "}
              {Math.floor(limits.data.upload_bytes / 1048576)} MiB per file,{" "}
              {Math.floor(limits.data.expanded_bytes / 1048576)} MiB expanded,{" "}
              {limits.data.members} ZIP members and{" "}
              {Math.floor(limits.data.row_bytes / 1024)} KiB per row or JSON
              metadata section. Oversized parts are reported; originals are
              retained for unfinished or partial imports.
            </p>
          )}
          <ErrorText>{limits.error}</ErrorText>
          <button className="primary" disabled={!!busy || !files?.length}>
            {busy === "upload"
              ? "Uploading… Keep this page open until received"
              : "Upload ListenBrainz files"}
          </button>
        </form>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void submit("fetch");
          }}
        >
          <h3>Fetch a recent public interval</h3>
          <label>
            Public ListenBrainz username
            <input
              required
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              maxLength={64}
            />
          </label>
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
          <p>
            Dates include the whole selected day in{" "}
            {preferences.data?.timezone || "the instance time zone"}. Without a
            start date: the last 14 days of the interval. Without an end date:
            stop at the moment you start. The saved interval stays fixed during
            retries.
          </p>
          <p>
            No token required. For months, years or a complete history, download
            your ZIP from{" "}
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
      <p>
        Actual listening time, estimates and unknown durations remain separate.
        Plays below 30 seconds of actual listening are retained and excluded
        from valid statistics. Matches based only on names or nearby times stay
        separate.
      </p>
    </article>
  );
}

type Originals = {
  total: number;
  files: { file_id: string; name: string }[];
  rows: {
    member: string;
    row_number: number;
    outcome: string;
    reason: string | null;
    raw: unknown;
  }[];
};
export function ListenBrainzOriginals({ id }: { id: string }) {
  const [offset, setOffset] = useState(0);
  const [data, setData] = useState<Originals | null>(null),
    [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  async function load(next: number) {
    setBusy(true);
    setError("");
    try {
      const response = await fetch(
        `/api/v1/listenbrainz/jobs/${encodeURIComponent(id)}/originals?offset=${next}`,
      );
      const result = await response.json();
      if (!response.ok) throw new Error(result.error);
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
