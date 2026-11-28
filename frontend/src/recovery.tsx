import { Select } from "./select";
import { passwordChecks, PasswordFeedback } from "./account";
import { useRef, useState } from "react";
import { api } from "./api";
import type {
  Asset,
  ExportSelection,
  Manifest,
  Preferences,
} from "./generated";
import { useLoad } from "./load";
import { date, ErrorText, Icon, Modal, useToast } from "./ui";
import { Link } from "./navigation";
import { SkeletonList } from "./skeleton";

export function Recovery({
  preferences,
  active = true,
}: {
  preferences: Preferences;
  active?: boolean;
}) {
  const [revision, setRevision] = useState(0),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [restore, setRestore] = useState<Asset | null>(null),
    [password, setPassword] = useState(""),
    [repeat, setRepeat] = useState(""),
    [confirmed, setConfirmed] = useState(false),
    [restart, setRestart] = useState(false),
    [resetOpen, setResetOpen] = useState(false),
    [resetConfirmed, setResetConfirmed] = useState(false),
    [resetBusy, setResetBusy] = useState(false),
    [resetError, setResetError] = useState(""),
    [format, setFormat] = useState("json"),
    [selection, setSelection] = useState<ExportSelection>({
      from: null,
      to: null,
      q: null,
      status: "all",
    });
  const input = useRef<HTMLInputElement>(null);
  const loaded = useLoad(() => api.recovery(), String(revision), active, 2000);
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
  async function wipeArchive() {
    setResetBusy(true);
    setResetError("");
    try {
      await api.resetArchive();
      window.location.reload();
    } catch (e) {
      setResetError(e instanceof Error ? e.message : String(e));
    } finally {
      setResetBusy(false);
    }
  }
  const assets = loaded.data?.assets ?? [];
  const backupAssets = assets.filter((a) => a.kind !== "export");
  const exportAssets = assets.filter((a) => a.kind === "export");

  function renderAsset(asset: Asset) {
    const manifest: Partial<Manifest> & {
      selection?: ExportSelection;
      listens?: number;
    } = asset.manifest_json ? JSON.parse(asset.manifest_json) : {};
    const isBackup = asset.kind !== "export";
    const stateLabel =
      asset.state === "ready"
        ? "Ready"
        : asset.state === "pending"
          ? "In progress"
          : asset.state;
    return (
      <div className="recovery-asset-row" key={asset.id}>
        <div className="recovery-asset-head">
          <div className="recovery-asset-title">
            <strong>
              {isBackup
                ? "Archive backup"
                : `${asset.format.toUpperCase()} export`}{" "}
              · {date(manifest.created_at_ms || asset.created_at_ms, preferences)}
            </strong>
            <small className="tabular-nums">
              {asset.id.slice(0, 12)} ·{" "}
              {asset.size_bytes === null
                ? "Size pending"
                : `${(asset.size_bytes / 1048576).toFixed(2)} MiB`}
            </small>
          </div>
          <div className="recovery-asset-badges">
            <span
              className={`ai-badge ${asset.state === "ready" ? "ai-badge-active" : ""}`}
            >
              {stateLabel}
            </span>
            {isBackup && asset.verified_at_ms && (
              <span className="ai-badge ai-badge-active">Verified</span>
            )}
          </div>
        </div>
        {manifest.app_version && (
          <p className="recovery-asset-meta">
            App {manifest.app_version} · format {manifest.format_version} ·
            schema {manifest.schema_version} ·{" "}
            {manifest.counts?.listens ?? "?"} listens · revision{" "}
            {manifest.revision}
          </p>
        )}
        {!isBackup && manifest.selection && (
          <p className="recovery-asset-meta">
            Selection: {manifest.selection.from || "earliest"} through{" "}
            {manifest.selection.to || "latest"},{" "}
            {manifest.selection.status || "all"}
            {manifest.selection.q
              ? `; matching “${manifest.selection.q}”`
              : ""}{" "}
            · {manifest.listens} listens
          </p>
        )}
        {asset.format === "zip" && (
          <p className="field-hint recovery-asset-meta">
            {asset.verified_at_ms
              ? `Restore verified on ${date(asset.verified_at_ms, preferences)}.`
              : "Restore not yet verified."}
          </p>
        )}
        <ErrorText>{asset.verification_error}</ErrorText>
        <ErrorText>{asset.error}</ErrorText>
        <div className="connection-actions">
          {asset.state === "ready" && (
            <a
              className="button secondary"
              href={`/api/v1/recovery/${asset.id}`}
              download
            >
              <Icon>download</Icon>Download
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
                <Icon>verified</Icon>Verify backup
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
                <Icon>restore</Icon>Restore…
              </button>
            </>
          )}
          <Link className="button secondary" to={`/activities/${asset.job_id}`}>
            Operation details
          </Link>
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
              <Icon>delete_outline</Icon>Delete saved file
            </button>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="recovery-layout">
      <ErrorText>{error || loaded.error}</ErrorText>
      {restarting && (
        <p className="warning" role="status">
          Restore is prepared and writes are paused. Restart the container or
          local application. Then sign in with the new password, review Settings
          &amp; connections and resume below.
        </p>
      )}
      {loaded.data?.paused && (
        <article className="panel padded warning">
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
        </article>
      )}

      <article className="panel padded settings-form">
        <div className="panel-title">
          <h2>
            <Icon>backup</Icon>Archive backups &amp; restore
          </h2>
        </div>
        <p>
          Full server backups for migration or disaster recovery. Preserves
          normalized listens, fingerprints, provenance, and preferences; excludes
          passwords, sessions, Spotify credentials, and API keys.
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
            <Icon>add_circle_outline</Icon>Create backup
          </button>
          <button
            className="secondary"
            disabled={busy || restarting}
            onClick={() => input.current?.click()}
          >
            <Icon>upload_file</Icon>Upload backup
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
        <p className="field-hint">
          Backups are unencrypted ZIPs (max 2 GiB compressed, 8 GiB expanded).
          Keep a downloaded copy off-server. Progress is tracked in{" "}
          <Link to="/activities">Activities</Link>.
        </p>
        <div className="recovery-subsection">
          <h3>Saved backups</h3>
          {!loaded.data && !loaded.error && (
            <SkeletonList count={2} label="Loading backups…" />
          )}
          {loaded.data && backupAssets.length === 0 && (
            <p className="field-hint">No saved backups yet.</p>
          )}
          {backupAssets.length > 0 && (
            <div className="recovery-asset-list">
              {backupAssets.map(renderAsset)}
            </div>
          )}
        </div>
      </article>

      <article className="panel padded settings-form">
        <div className="panel-title">
          <h2>
            <Icon>download</Icon>Portable exports (JSON &amp; CSV)
          </h2>
        </div>
        <p>
          Export listening history for spreadsheets or external tools. Use a
          backup above to restore this instance.
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
              <Select
                aria-label="Export format"
                value={format}
                onChange={(value) => setFormat(value)}
              >
                <option value="json">JSON — observations and provenance</option>
                <option value="csv">CSV — one row per listen</option>
              </Select>
            </label>
            <label>
              Listen selection
              <Select
                aria-label="Listen selection"
                value={selection.status || "all"}
                onChange={(value) =>
                  setSelection((s) => ({ ...s, status: value }))
                }
              >
                <option value="all">All listens, including short</option>
                <option value="counted">Counted listens</option>
                <option value="short">Below threshold</option>
                <option value="actual">Actual listening time</option>
                <option value="estimated">Estimated or unknown time</option>
                <option value="review">Needs review</option>
              </Select>
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
          </div>
          <p className="field-hint">
            Selection: {selection.from || "earliest date"} through{" "}
            {selection.to || "latest date"}, {selection.status || "all"}
            {selection.q ? `; matching “${selection.q}”` : "; all metadata"}.
            Deleted listens are excluded.
          </p>
          <button className="primary" disabled={busy || restarting}>
            <Icon>file_download</Icon>Create export
          </button>
        </form>
        <div className="recovery-subsection">
          <h3>Saved exports</h3>
          {!loaded.data && !loaded.error && (
            <SkeletonList count={2} label="Loading exports…" />
          )}
          {loaded.data && exportAssets.length === 0 && (
            <p className="field-hint">No saved exports yet.</p>
          )}
          {exportAssets.length > 0 && (
            <div className="recovery-asset-list">
              {exportAssets.map(renderAsset)}
            </div>
          )}
        </div>
      </article>

      <article className="panel padded settings-form recovery-danger-card">
        <div className="panel-title">
          <h2>
            <Icon>warning</Icon>Danger zone: Delete archive
          </h2>
        </div>
        <p>
          Delete every listen, import, catalog entry and saved report in this
          archive. Your account, settings and calendars stay, and the files on
          disk are left alone.
        </p>
        <p className="warning">
          This cannot be undone. Create a backup first if you may want the data
          back.
        </p>
        <div className="connection-actions">
          <button
            className="danger"
            disabled={busy || restarting}
            onClick={() => {
              setResetConfirmed(false);
              setResetError("");
              setResetOpen(true);
            }}
          >
            <Icon>delete_forever</Icon>Delete all archive data
          </button>
        </div>
      </article>
      {resetOpen && (
        <Modal
          title="Delete all archive data"
          close={() => {
            if (!resetBusy) setResetOpen(false);
          }}
        >
          <p>
            Every listen, imported file record and catalog entry is deleted now.
            Your account, settings and calendars are kept. Make a backup first
            if you may want to restore this archive.
          </p>
          <p className="warning">
            This cannot be undone and imported files will need importing again.
          </p>
          <label className="check">
            <input
              type="checkbox"
              checked={resetConfirmed}
              onChange={(e) => setResetConfirmed(e.target.checked)}
            />
            I understand every archive row will be deleted
          </label>
          <ErrorText>{resetError}</ErrorText>
          <div className="form-actions">
            <button
              className="danger"
              disabled={!resetConfirmed || resetBusy}
              onClick={() => void wipeArchive()}
            >
              {resetBusy ? "Deleting…" : "Delete everything"}
            </button>
            <button
              className="secondary"
              disabled={resetBusy}
              onClick={() => setResetOpen(false)}
            >
              Cancel
            </button>
          </div>
        </Modal>
      )}
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
            Preparation can be cancelled until it is ready for restart. Once
            ready, the next restart activates the replacement.
          </p>
          {busy && (
            <button
              type="button"
              className="secondary"
              onClick={async () => {
                try {
                  const jobs = await api.activities();
                  const pending = jobs.find(
                    (job) =>
                      job.kind === "recovery_restore" &&
                      ["queued", "running", "waiting"].includes(job.state),
                  );
                  if (pending) {
                    await api.activityAction(pending.id, "cancel");
                    setRestore(null);
                    toast(
                      "Restore cancellation requested. The live archive remains unchanged.",
                    );
                  } else
                    toast(
                      "No running preparation found. Check Activities for its current state.",
                    );
                } catch (error) {
                  toast(
                    error instanceof Error ? error.message : String(error),
                    true,
                  );
                }
              }}
            >
              Cancel preparation
            </button>
          )}
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
                minLength={8}
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
                !passwordChecks(password).every((check) => check.met) ||
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
    </div>
  );
}
