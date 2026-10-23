import { PasswordFeedback } from "./account";
import { useRef, useState } from "react";
import { api } from "./api";
import type {
  Asset,
  ExportSelection,
  Manifest,
  Preferences,
} from "./generated";
import { useLoad } from "./library";
import { date, ErrorText, Modal, useToast } from "./ui";
import { Link } from "./navigation";

export function Recovery({ preferences }: { preferences: Preferences }) {
  const [revision, setRevision] = useState(0),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [restore, setRestore] = useState<Asset | null>(null),
    [password, setPassword] = useState(""),
    [repeat, setRepeat] = useState(""),
    [confirmed, setConfirmed] = useState(false),
    [restart, setRestart] = useState(false),
    [format, setFormat] = useState("json"),
    [selection, setSelection] = useState<ExportSelection>({
      from: null,
      to: null,
      q: null,
      status: "all",
    });
  const input = useRef<HTMLInputElement>(null);
  const loaded = useLoad(() => api.recovery(), String(revision), true, 2000);
  const toast = useToast();
  async function perform(work: () => Promise<unknown>, message: string) {
    setBusy(true);
    setError("");
    try {
      await work();
      setRevision((v) => v + 1);
      toast(message);
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setError(message);
      toast(message, true);
    } finally {
      setBusy(false);
    }
  }
  const restarting = restart || loaded.data?.restart_required;
  return (
    <section className="settings-form recovery-form">
      <ErrorText>{error || loaded.error}</ErrorText>
      {restarting && (
        <p className="warning" role="status">
          Restore is prepared and writes are paused. Restart the container or
          local application. Then sign in with the new password, review Settings
          & connections and resume below.
        </p>
      )}
      {loaded.data?.paused && (
        <div className="warning">
          <h3>Archive restored — collection and pending jobs are paused</h3>
          <p>
            Review the archive and server preferences. Configure Spotify
            credentials and connect again; recreate API keys when needed. The
            destination’s network and HTTPS configuration is kept separately.
          </p>
          <button
            className="primary"
            disabled={busy || restarting}
            onClick={() =>
              void perform(
                () => api.resumeRecovery(),
                "Collection and pending jobs can run again.",
              )
            }
          >
            Resume collection and pending jobs
          </button>
        </div>
      )}
      <h3>Export listening history</h3>
      <p>
        JSON includes normalized observations, fingerprints and provenance. Raw
        source JSON is not retained. CSV has one row per listen. These portable
        exports are for other tools; use a backup to restore this instance. By
        default, export includes all history and short listens.
      </p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void perform(
            () => api.exportArchive(format, selection),
            "Export queued. You can close this page; follow progress in Activities.",
          );
        }}
      >
        <div className="form-grid">
          <label>
            Export format
            <select value={format} onChange={(e) => setFormat(e.target.value)}>
              <option value="json">JSON — observations and provenance</option>
              <option value="csv">CSV — one row per listen</option>
            </select>
          </label>
          <label>
            From date
            <input
              type="date"
              value={selection.from || ""}
              onChange={(e) =>
                setSelection((s) => ({ ...s, from: e.target.value || null }))
              }
            />
          </label>
          <label>
            Through date
            <input
              type="date"
              value={selection.to || ""}
              onChange={(e) =>
                setSelection((s) => ({ ...s, to: e.target.value || null }))
              }
            />
          </label>
          <label>
            Search metadata
            <input
              maxLength={500}
              value={selection.q || ""}
              onChange={(e) =>
                setSelection((s) => ({ ...s, q: e.target.value || null }))
              }
            />
          </label>
          <label>
            Listen selection
            <select
              value={selection.status || "all"}
              onChange={(e) =>
                setSelection((s) => ({ ...s, status: e.target.value }))
              }
            >
              <option value="all">All listens, including short</option>
              <option value="counted">Counted listens</option>
              <option value="short">Below threshold</option>
              <option value="actual">Actual listening time</option>
              <option value="estimated">Estimated or unknown time</option>
              <option value="review">Needs review</option>
            </select>
          </label>
        </div>
        <p>
          Selection: {selection.from || "earliest date"} through{" "}
          {selection.to || "latest date"}, using each listen’s assigned
          calendar; {selection.status || "all"}
          {selection.q ? `; matching “${selection.q}”` : "; all metadata"}.
          Deleted listens are excluded.
        </p>
        <button className="primary" disabled={busy || restarting}>
          Create export
        </button>
      </form>
      <h3>Manual backup</h3>
      <p>
        Backups preserve normalized history, fingerprints, provenance, server
        preferences and reports. Source files are kept for unfinished or partial
        imports and released after successful completion. To import them again,
        upload them again. Passwords, sessions, Spotify credentials and API keys
        are excluded. Browser preferences and network, Compose, DNS and
        certificate configuration are separate.
      </p>
      <p>
        Copies contain personal listening history and are not encrypted. Keep a
        downloaded copy on another device or dataset. Copies are retained until
        you delete them.
      </p>
      <div className="connection-actions">
        <button
          className="primary"
          disabled={busy || restarting}
          onClick={() =>
            void perform(
              () => api.createBackup(),
              "Backup queued. It continues on the server if you close this page.",
            )
          }
        >
          Create backup
        </button>
        <button
          className="secondary"
          disabled={busy || restarting}
          onClick={() => input.current?.click()}
        >
          Upload backup
        </button>
        <input
          ref={input}
          type="file"
          accept=".zip,application/zip"
          aria-label="Backup ZIP"
          hidden
          onChange={(e) => {
            const file = e.target.files?.[0];
            if (file)
              void perform(
                () => api.uploadBackup(file),
                "Backup received. Compatibility and integrity checks continue in Activities.",
              );
            e.target.value = "";
          }}
        />
      </div>
      <p>
        Backup size limit: 2 GiB compressed, 8 GiB expanded. Allow free disk
        space for the backup, working copies and retained previous archive.
        Progress, errors and finished operations are saved in{" "}
        <Link to="/activities">Activities</Link>.
      </p>
      <h3>Saved files</h3>
      {loaded.data?.assets.length === 0 && <p>No saved exports or backups.</p>}
      {loaded.data?.assets.map((asset) => {
        const manifest: Partial<Manifest> & {
          selection?: ExportSelection;
          listens?: number;
        } = asset.manifest_json ? JSON.parse(asset.manifest_json) : {};
        return (
          <div className="job" key={asset.id}>
            <strong>
              {asset.kind === "export"
                ? `${asset.format.toUpperCase()} export`
                : "Archive backup"}{" "}
              ·{" "}
              {date(manifest.created_at_ms || asset.created_at_ms, preferences)}
            </strong>
            <small>
              {asset.id.slice(0, 12)} ·{" "}
              {asset.size_bytes === null
                ? "Size pending"
                : `${(asset.size_bytes / 1048576).toFixed(2)} MiB`}{" "}
              · {asset.state}
            </small>
            {manifest.app_version && (
              <p>
                App {manifest.app_version} · format {manifest.format_version} ·
                schema {manifest.schema_version} ·{" "}
                {manifest.counts?.listens ?? "?"} listens · revision{" "}
                {manifest.revision}
              </p>
            )}
            {asset.kind === "export" && manifest.selection && (
              <p>
                Selection: {manifest.selection.from || "earliest"} through{" "}
                {manifest.selection.to || "latest"},{" "}
                {manifest.selection.status || "all"}
                {manifest.selection.q
                  ? `; matching “${manifest.selection.q}”`
                  : ""}{" "}
                · {manifest.listens} listens.
              </p>
            )}
            {asset.format === "zip" && (
              <p>
                {asset.verified_at_ms
                  ? `Restore verified: ${date(asset.verified_at_ms, preferences)}. Isolated trial completed for this copy.`
                  : "Restore not yet verified. Creation and integrity checks are separate from Verify backup."}
              </p>
            )}
            <ErrorText>{asset.verification_error}</ErrorText>
            <ErrorText>{asset.error}</ErrorText>
            <div className="connection-actions">
              <Link to={`/activities/${asset.job_id}`}>Operation details</Link>
              {asset.state === "ready" && (
                <a
                  className="button secondary"
                  href={`/api/v1/recovery/${asset.id}`}
                  download
                >
                  Download
                </a>
              )}
              {asset.state === "ready" && asset.format === "zip" && (
                <>
                  <button
                    className="secondary"
                    disabled={busy || restarting}
                    onClick={() =>
                      void perform(
                        () => api.backupAction(asset.id, "verify"),
                        "Isolated restore trial queued. The active archive and Spotify are not used.",
                      )
                    }
                  >
                    Verify backup
                  </button>
                  <button
                    className="secondary"
                    disabled={busy || restarting}
                    onClick={() => {
                      setRestore(asset);
                      setPassword("");
                      setRepeat("");
                      setConfirmed(false);
                    }}
                  >
                    Restore…
                  </button>
                </>
              )}
              {asset.state !== "pending" && (
                <button
                  className="secondary danger"
                  disabled={busy || restarting}
                  onClick={() =>
                    void perform(
                      () => api.backupAction(asset.id, "delete"),
                      "Saved file deleted.",
                    )
                  }
                >
                  Delete saved file
                </button>
              )}
            </div>
          </div>
        );
      })}
      {restore && (
        <Modal
          title="Replace this archive"
          close={() => {
            if (!busy) setRestore(null);
          }}
        >
          <p>
            This replaces the current archive with the backup from{" "}
            {date(
              restore.manifest_json
                ? (JSON.parse(restore.manifest_json) as Manifest).created_at_ms
                : restore.created_at_ms,
              preferences,
            )}{" "}
            ({restore.id.slice(0, 12)}). Archives are not merged. The previous
            data is retained on the server for recovery.
          </p>
          <p>
            You must restart after preparation. All old sessions and API keys
            become invalid. Spotify must be configured and connected again.
            Collection and pending jobs stay paused until you review Settings
            and explicitly resume.
          </p>
          <p>
            After confirmation, preparation cannot be cancelled. The next
            restart activates the replacement.
          </p>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void perform(async () => {
                await api.restoreBackup(restore.id, password);
                setRestart(true);
                setRestore(null);
                setPassword("");
                setRepeat("");
              }, "Restore prepared. Restart the container, then use your new password.");
            }}
          >
            <label>
              New owner password
              <input
                autoComplete="new-password"
                type="password"
                minLength={15}
                maxLength={1024}
                required
                value={password}
                onChange={(e) => setPassword(e.target.value)}
              />
            </label>
            <PasswordFeedback password={password} />
            <label>
              Repeat new password
              <input
                autoComplete="new-password"
                type="password"
                required
                value={repeat}
                onChange={(e) => setRepeat(e.target.value)}
              />
            </label>
            <label className="check">
              <input
                type="checkbox"
                checked={confirmed}
                onChange={(e) => setConfirmed(e.target.checked)}
              />
              I confirm replacement of the current archive
            </label>
            <ErrorText>{error}</ErrorText>
            <button
              className="primary"
              disabled={
                busy ||
                !confirmed ||
                Array.from(password).length < 15 ||
                password !== repeat
              }
            >
              {busy
                ? "Preparing restore…"
                : "Confirm replacement and prepare restart"}
            </button>
          </form>
        </Modal>
      )}
    </section>
  );
}
