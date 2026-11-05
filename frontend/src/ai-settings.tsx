import { useEffect, useState } from "react";
import type {
  AiCatalog,
  AiCatalogModel,
  AiInstalledModel,
  AiSettings,
} from "./generated";
import { api } from "./api";
import { Select } from "./select";
import { Button, ErrorText, Icon, Modal, number, useToast } from "./ui";
import { SkeletonCard } from "./skeleton";

const GIB = 1024 * 1024 * 1024;

const tierLabel: Record<string, string> = {
  light: "Light",
  default: "Default",
  quality: "Quality",
  gpu: "GPU",
};

const statusLabel: Record<string, string> = {
  available: "Available",
  gpu_future: "GPU · later release",
  unverified: "Unverified",
};

const downloadError: Record<string, string> = {
  model_disk_full:
    "Not enough free disk space. Free space on the server and retry the download.",
  model_checksum_mismatch:
    "The downloaded file failed SHA-256 verification and was removed. Retry the download.",
  model_source_rejected:
    "The model source refused the download. Check the server connection and retry.",
  model_download_failed: "The model download failed. Check the server log and retry.",
  model_download_retry:
    "The download was interrupted. It resumes automatically with the bytes already saved.",
  model_cancelled: "The download was cancelled.",
  model_unavailable: "This model is no longer in the catalog.",
};

function bytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "0 B";
  if (value >= GIB) return `${(value / GIB).toFixed(1)} GB`;
  if (value >= 1024 * 1024) return `${(value / (1024 * 1024)).toFixed(0)} MB`;
  if (value >= 1024) return `${(value / 1024).toFixed(0)} KB`;
  return `${number.format(value)} B`;
}

function percent(done: number, total: number): number {
  if (!Number.isFinite(total) || total <= 0) return 0;
  return Math.max(0, Math.min(100, (done / total) * 100));
}

function evalLabel(model: AiCatalogModel): string {
  const scores = model.languages
    .filter((language) => language.exact_match !== null)
    .map(
      (language) =>
        `${language.language.toUpperCase()} ${Math.round(
          (language.exact_match ?? 0) * 100,
        )}%`,
    );
  return scores.length ? scores.join(" · ") : "Not measured";
}

function isRunning(job: AiCatalogModel["download_job"]): boolean {
  return (
    job !== null &&
    ["queued", "running", "waiting"].includes(job.state) &&
    !job.error_code?.startsWith("model_cancelled")
  );
}

function DownloadProgress({
  model,
  onCancel,
  busy,
}: {
  model: AiCatalogModel;
  onCancel: () => void;
  busy: boolean;
}) {
  const job = model.download_job;
  if (!job) return null;
  const running = isRunning(job);
  const failed = job.state === "failed";
  const cancelled = job.state === "cancelled";
  if (!running && !failed && !cancelled) return null;
  return (
    <div className="ai-progress">
      <div className="ai-progress-head">
        <span>{running ? "Downloading" : failed ? "Download failed" : "Cancelled"}</span>
        <span className="tabular-nums">
          {bytes(job.processed_bytes)} of {bytes(job.expected_bytes)}
        </span>
      </div>
      <progress
        value={job.processed_bytes}
        max={job.expected_bytes || 1}
        aria-label={`Download progress for ${model.name}`}
      />
      {job.error_code && downloadError[job.error_code] && (
        <p
          className={failed ? "form-error" : "field-hint"}
          role={failed ? "alert" : undefined}
        >
          {downloadError[job.error_code]}
        </p>
      )}
      {running && (
        <Button variant="secondary" onClick={onCancel} disabled={busy}>
          <Icon>close</Icon>
          Cancel download
        </Button>
      )}
    </div>
  );
}

function ModelCard({
  model,
  enabled,
  activeModel,
  busy,
  onDownload,
  onCancel,
  onSelect,
  onDelete,
}: {
  model: AiCatalogModel;
  enabled: boolean;
  activeModel: string | null;
  busy: string;
  onDownload: () => void;
  onCancel: () => void;
  onSelect: () => void;
  onDelete: () => void;
}) {
  const job = model.download_job;
  const running = isRunning(job);
  const canDownload =
    enabled && model.selectable && !model.installed && !running;
  const progress = job ? percent(job.processed_bytes, job.expected_bytes) : 0;
  return (
    <article className="ai-card" aria-labelledby={`ai-model-${model.id}`}>
      <div className="ai-card-head">
        <div>
          <h3 id={`ai-model-${model.id}`}>{model.name}</h3>
          <p className="field-hint">{model.description}</p>
        </div>
        <div className="ai-badges">
          <span className={`ai-badge ai-badge-${model.tier}`}>
            {tierLabel[model.tier] ?? model.tier}
          </span>
          <span
            className={`ai-badge ${
              model.status === "available" ? "" : "ai-badge-muted"
            }`}
          >
            {statusLabel[model.status] ?? model.status}
          </span>
          {activeModel === model.id && (
            <span className="ai-badge ai-badge-active">Active</span>
          )}
        </div>
      </div>
      <dl className="ai-facts">
        <div>
          <dt>Download</dt>
          <dd className="tabular-nums">{bytes(model.download.size_bytes)}</dd>
        </div>
        <div>
          <dt>Minimum RAM</dt>
          <dd className="tabular-nums">{bytes(model.requirements.ram_min_bytes)}</dd>
        </div>
        <div>
          <dt>Recommended RAM</dt>
          <dd className="tabular-nums">
            {bytes(model.requirements.ram_recommended_bytes)}
          </dd>
        </div>
        <div>
          <dt>Speed (desktop p50)</dt>
          <dd className="tabular-nums">
            {model.speed ? `${number.format(model.speed.p50_ms)} ms` : "Not measured"}
          </dd>
        </div>
        <div>
          <dt>Accuracy (spike)</dt>
          <dd>{evalLabel(model)}</dd>
        </div>
        <div>
          <dt>License</dt>
          <dd>
            <a href={model.license.url} target="_blank" rel="noreferrer">
              {model.license.name}
            </a>
          </dd>
        </div>
      </dl>
      {model.parser.exact_match === null && (
        <p className="field-hint">
          Parser evaluation on the real question set is still pending.
        </p>
      )}
      {model.installed && (
        <p className="ai-installed-note">
          <Icon>check_circle</Icon>
          Installed{model.checksum_verified ? ", checksum verified" : ""} ·{" "}
          {bytes(model.installed_size_bytes ?? model.download.size_bytes)}
        </p>
      )}
      {model.error && <p className="form-error">{model.error}</p>}
      {running && (
        <div className="ai-inline-progress" aria-hidden="true">
          <span style={{ width: `${progress}%` }} />
        </div>
      )}
      <DownloadProgress
        model={model}
        onCancel={onCancel}
        busy={busy === model.id}
      />
      {model.selectable && !model.installed && !running && !enabled && (
        <p className="field-hint">Enable AI and save to start a download.</p>
      )}
      <div className="ai-actions">
        {model.selectable && !model.installed && (
          <Button
            variant="primary"
            onClick={onDownload}
            disabled={!canDownload || busy === model.id}
          >
            <Icon>download</Icon>
            Download
          </Button>
        )}
        {model.installed && model.selectable && activeModel !== model.id && (
          <Button
            variant="secondary"
            onClick={onSelect}
            disabled={busy === model.id}
          >
            <Icon>radio_button_checked</Icon>
            Use this model
          </Button>
        )}
        {model.installed && (
          <Button
            variant="danger"
            onClick={onDelete}
            disabled={busy === model.id}
          >
            <Icon>delete_outline</Icon>
            Delete
          </Button>
        )}
      </div>
    </article>
  );
}

export function AiSettingsSection() {
  const [settings, setSettings] = useState<AiSettings | null>(null);
  const [catalog, setCatalog] = useState<AiCatalog | null>(null);
  const [enabled, setEnabled] = useState(false);
  const [active, setActive] = useState("");
  const [threads, setThreads] = useState("4");
  const [unload, setUnload] = useState("15");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState("");
  const [deleteTarget, setDeleteTarget] = useState<AiInstalledModel | null>(null);
  const toast = useToast();

  function adopt(next: AiSettings) {
    setSettings(next);
    setEnabled(next.enabled);
    setActive(next.active_model ?? "");
    setThreads(String(next.threads));
    setUnload(String(next.unload_minutes));
  }

  async function load() {
    setError("");
    const [settingsResult, catalogResult] = await Promise.all([
      api.aiSettings(),
      api.aiCatalog(),
    ]);
    adopt(settingsResult);
    setCatalog(catalogResult);
  }

  useEffect(() => {
    load().catch((e) =>
      setError(e instanceof Error ? e.message : String(e)),
    );
  }, []);

  const polling =
    catalog?.models.some((model) => isRunning(model.download_job)) ?? false;
  useEffect(() => {
    if (!polling) return;
    const timer = setInterval(() => {
      api
        .aiCatalog()
        .then(setCatalog)
        .catch((e) => setError(e instanceof Error ? e.message : String(e)));
    }, 2000);
    return () => clearInterval(timer);
  }, [polling]);

  async function save() {
    setBusy("settings");
    setError("");
    setNotice("");
    try {
      const next = await api.saveAiSettings({
        enabled,
        active_model: active || null,
        threads: Number(threads),
        unload_minutes: Number(unload),
        model_dir: settings?.model_dir ?? null,
      });
      adopt(next);
      setCatalog(await api.aiCatalog());
      toast("AI settings saved.");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy("");
    }
  }

  async function action(
    key: string,
    run: () => Promise<void>,
    message?: string,
  ) {
    setBusy(key);
    setError("");
    setNotice("");
    try {
      await run();
      setCatalog(await api.aiCatalog());
      const next = await api.aiSettings();
      adopt(next);
      if (message) toast(message);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy("");
    }
  }

  async function test() {
    setBusy("test");
    setError("");
    setNotice("");
    try {
      const result = await api.testAiModel();
      setNotice(result.message);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy("");
    }
  }

  if (!settings || !catalog) {
    return (
      <div className="ai-settings">
        {error ? <ErrorText>{error}</ErrorText> : <SkeletonCard label="Loading AI settings and model catalog." />}
      </div>
    );
  }

  const installedUsable = catalog.installed.filter(
    (model) => model.checksum_verified && !model.error,
  );
  const activeModel = settings.active_model;

  return (
    <div className="ai-settings">
      <article className="panel padded">
        <div className="panel-title">
          <h2>AI</h2>
          <span className="ai-badge">
            {settings.runtime_available ? "Runtime available" : "Runtime not in this build"}
          </span>
        </div>
        <p>
          Ask questions in natural language about your listening archive. The
          model runs locally: your listens never leave this instance; the model
          only turns your question into a query and never sees your data. Every
          number is calculated by scrobbl.rs.
        </p>
        <p className="field-hint">
          AI is off by default. Nothing is downloaded or loaded until you enable
          it and choose a model. The only network request is the model download
          you start.
        </p>
        {!settings.runtime_available && (
          <div className="notice notice-warning" role="status">
            <Icon>info</Icon>
            <div className="notice-body">
              <strong>This build does not include the inference runtime.</strong>
              <span>
                You can still manage settings and download models. Asking
                questions stays disabled until the runtime is available.
              </span>
            </div>
          </div>
        )}
        {error && <ErrorText>{error}</ErrorText>}
        {notice && (
          <div className="notice notice-info" role="status">
            <Icon>info</Icon>
            <div className="notice-body">{notice}</div>
          </div>
        )}
        {settings.message && (
          <div className="notice notice-info" role="status">
            <Icon>info</Icon>
            <div className="notice-body">{settings.message}</div>
          </div>
        )}
        <form
          className="ai-settings-form"
          onSubmit={(event) => {
            event.preventDefault();
            save();
          }}
        >
          <label className="ai-toggle">
            <input
              type="checkbox"
              checked={enabled}
              onChange={(event) => setEnabled(event.target.checked)}
            />
            <span>
              <strong>Enable local AI</strong>
              <small>
                Saving is an explicit consent to download and run the selected
                model on this instance.
              </small>
            </span>
          </label>
          <label>
            Active model
            <Select
              aria-label="Active model"
              value={active}
              onChange={setActive}
            >
              <option value="">No model selected</option>
              {installedUsable.map((model) => (
                <option key={model.id} value={model.id}>
                  {model.name ?? model.id}
                </option>
              ))}
              {activeModel &&
                !installedUsable.some((model) => model.id === activeModel) && (
                  <option value={activeModel} disabled>
                    {activeModel} (not installed)
                  </option>
                )}
            </Select>
          </label>
          <div className="ai-actions">
            <Button
              type="submit"
              variant="primary"
              disabled={busy === "settings"}
            >
              <Icon>save</Icon>
              {busy === "settings" ? "Saving…" : "Save AI settings"}
            </Button>
          </div>
        </form>
      </article>

      <article className="panel padded">
        <div className="panel-title">
          <h2>Models</h2>
          <span className="field-hint tabular-nums">
            {bytes(catalog.total_installed_bytes)} installed ·{" "}
            {bytes(catalog.disk_used_bytes)} on disk
          </span>
        </div>
        <p className="field-hint">
          Only this curated catalog can be downloaded. Each file is pinned to a
          repository revision and verified with its SHA-256. GPU entries are
          listed for planning and cannot be selected in this release.
        </p>
        <div className="ai-grid">
          {catalog.models.map((model) => (
            <ModelCard
              key={model.id}
              model={model}
              enabled={settings.enabled}
              activeModel={activeModel}
              busy={busy}
              onDownload={() =>
                action(
                  model.id,
                  async () => {
                    await api.downloadAiModel(model.id);
                  },
                  "Model download queued. Progress appears here and in Activities.",
                )
              }
              onCancel={() =>
                action(model.id, async () => {
                  await api.cancelAiModelDownload(model.id);
                })
              }
              onSelect={() =>
                action(
                  model.id,
                  async () => {
                    await api.selectAiModel(model.id);
                  },
                  "Active model updated.",
                )
              }
              onDelete={() => setDeleteTarget({ id: model.id, name: model.name, file: model.download.file, size_bytes: model.download.size_bytes, installed_at_ms: null, checksum_verified: model.checksum_verified, error: null })}
            />
          ))}
        </div>
      </article>

      <article className="panel padded">
        <div className="panel-title">
          <h2>Installed models</h2>
          <span className="field-hint tabular-nums">
            {bytes(catalog.disk_used_bytes)} used in {catalog.model_dir}
          </span>
        </div>
        {catalog.installed.length === 0 ? (
          <div className="empty">
            <Icon>memory</Icon>
            <p>No models are installed yet.</p>
          </div>
        ) : (
          <ul className="ai-installed-list">
            {catalog.installed.map((model) => (
              <li key={model.id}>
                <div>
                  <strong>{model.name ?? model.id}</strong>
                  <span className="field-hint">
                    {model.file ?? "No readable file"} · {bytes(model.size_bytes)}
                    {model.id === activeModel ? " · Active" : ""}
                  </span>
                  {model.error && <span className="form-error">{model.error}</span>}
                </div>
                <Button
                  variant="danger"
                  onClick={() => setDeleteTarget(model)}
                  disabled={busy === model.id}
                >
                  <Icon>delete_outline</Icon>
                  Delete
                </Button>
              </li>
            ))}
          </ul>
        )}
      </article>

      <article className="panel padded">
        <div className="panel-title">
          <h2>Test and advanced options</h2>
        </div>
        <p className="field-hint">
          The model loads lazily on the first question. Testing only reports
          whether this build and the selected file can run; it never starts an
          inference or downloads anything.
        </p>
        <div className="ai-actions">
          <Button
            variant="secondary"
            onClick={test}
            disabled={busy === "test" || !settings.runtime_available}
            title={
              settings.runtime_available
                ? undefined
                : "This build does not include the inference runtime."
            }
          >
            <Icon>play_circle_outline</Icon>
            Test active model
          </Button>
        </div>
        <form
          className="ai-advanced"
          onSubmit={(event) => {
            event.preventDefault();
            save();
          }}
        >
          <label>
            Threads
            <input
              type="number"
              min={1}
              max={64}
              value={threads}
              onChange={(event) => setThreads(event.target.value)}
              inputMode="numeric"
            />
          </label>
          <label>
            Unload after inactivity (minutes)
            <input
              type="number"
              min={1}
              max={1440}
              value={unload}
              onChange={(event) => setUnload(event.target.value)}
              inputMode="numeric"
            />
          </label>
          <label>
            Model folder
            <code className="ai-path">{catalog.model_dir}</code>
          </label>
          <Button type="submit" variant="secondary" disabled={busy === "settings"}>
            <Icon>save</Icon>
            Save options
          </Button>
        </form>
      </article>

      {deleteTarget && (
        <Modal
          title="Delete model"
          close={() => setDeleteTarget(null)}
        >
          <p>
            Delete <strong>{deleteTarget.name ?? deleteTarget.id}</strong> and
            free {bytes(deleteTarget.size_bytes)}?
          </p>
          {deleteTarget.id === activeModel && (
            <p className="notice notice-warning">
              This is the active model. Deleting it also clears the active
              selection.
            </p>
          )}
          <div className="ai-actions">
            <Button
              variant="danger"
              disabled={busy === deleteTarget.id}
              onClick={() => {
                const target = deleteTarget;
                action(
                  target.id,
                  async () => {
                    const result = await api.deleteAiModel(
                      target.id,
                      true,
                      target.id === activeModel,
                    );
                    setDeleteTarget(null);
                    toast(
                      `Model deleted. ${bytes(result.freed_bytes)} freed.`,
                    );
                  },
                );
              }}
            >
              <Icon>delete_outline</Icon>
              Delete model
            </Button>
            <Button variant="secondary" onClick={() => setDeleteTarget(null)}>
              Cancel
            </Button>
          </div>
        </Modal>
      )}
    </div>
  );
}
