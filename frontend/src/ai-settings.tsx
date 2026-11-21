import { useEffect, useState } from "react";
import type {
  AiCatalog,
  AiCatalogModel,
  AiInstalledModel,
  AiSettings,
} from "./generated";
import { api } from "./api";
import { Select } from "./select";
import {
  Button,
  ErrorText,
  Icon,
  Modal,
  lookupRecord,
  number,
  safeExternalUrl,
  useToast,
} from "./ui";

const GIB = 1024 * 1024 * 1024;

const TIER_LABELS: Record<string, string> = {
  light: "Light",
  default: "Default",
  quality: "Quality",
  gpu: "GPU",
};

const STATUS_LABELS: Record<string, string> = {
  available: "Available",
  gpu_future: "GPU · later release",
  unverified: "Unverified",
};

const BACKEND_LABELS: Record<string, string> = {
  cpu: "Backend: CPU",
  metal: "Backend: Metal",
  vulkan: "Backend: Vulkan",
  unavailable: "Backend: unavailable",
};

const DOWNLOAD_ERRORS: Record<string, string> = {
  model_disk_full:
    "Not enough free disk space. Free space on the server and retry the download.",
  model_checksum_mismatch:
    "The downloaded file failed SHA-256 verification and was removed. Retry the download.",
  model_source_rejected:
    "The model source refused the download. Check the server connection and retry.",
  model_download_failed:
    "The model download failed. Check the server log and retry.",
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

function speed(bps: number): string {
  if (!Number.isFinite(bps) || bps <= 0) return "";
  if (bps >= 1024 * 1024) return `${(bps / (1024 * 1024)).toFixed(1)} MB/s`;
  if (bps >= 1024) return `${(bps / 1024).toFixed(0)} KB/s`;
  return `${number.format(bps)} B/s`;
}

function eta(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "";
  if (seconds < 60) return `${seconds}s left`;
  const mins = Math.floor(seconds / 60);
  const secs = seconds % 60;
  if (mins < 60) return secs > 0 ? `${mins}m ${secs}s left` : `${mins}m left`;
  const hours = Math.floor(mins / 60);
  const remMins = mins % 60;
  return remMins > 0 ? `${hours}h ${remMins}m left` : `${hours}h left`;
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
  const speedText =
    running && job.bytes_per_second ? speed(job.bytes_per_second) : "";
  const etaText =
    running && job.eta_seconds !== null && job.eta_seconds !== undefined
      ? eta(job.eta_seconds)
      : "";
  const metrics = [
    `${bytes(job.processed_bytes)} of ${bytes(job.expected_bytes)}`,
    speedText,
    etaText,
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <div className="ai-progress">
      <div className="ai-progress-head">
        <span className="ai-progress-status">
          {running && (
            <span className="spinner-dot spinner-sm" aria-hidden="true" />
          )}
          {running
            ? job.state === "queued"
              ? "Starting download…"
              : job.state === "waiting"
                ? "Retrying download…"
                : "Downloading"
            : failed
              ? "Download failed"
              : "Cancelled"}
        </span>
        <span className="tabular-nums">{metrics}</span>
      </div>
      <progress
        value={job.processed_bytes}
        max={job.expected_bytes || 1}
        aria-label={`Download progress for ${model.name}`}
      />
      {(() => {
        const message = lookupRecord(DOWNLOAD_ERRORS, job.error_code);
        return (
          message && (
            <p
              className={failed ? "form-error" : "field-hint"}
              role={failed ? "alert" : undefined}
            >
              {message}
            </p>
          )
        );
      })()}
      {running && (
        <Button variant="secondary" onClick={onCancel} disabled={busy}>
          {busy ? (
            <span className="spinner-dot spinner-sm" aria-hidden="true" />
          ) : (
            <Icon>close</Icon>
          )}
          {busy ? "Cancelling…" : "Cancel download"}
        </Button>
      )}
    </div>
  );
}

function ModelCard({
  model,
  activeModel,
  busy,
  onDownload,
  onCancel,
  onSelect,
  onDelete,
}: {
  model: AiCatalogModel;
  activeModel: string | null;
  busy: string;
  onDownload: () => void;
  onCancel: () => void;
  onSelect: () => void;
  onDelete: () => void;
}) {
  const job = model.download_job;
  const running = isRunning(job);
  const canDownload = model.selectable && !model.installed && !running;
  return (
    <article className="ai-card" aria-labelledby={`ai-model-${model.id}`}>
      <div className="ai-card-head">
        <div>
          <h3 id={`ai-model-${model.id}`}>{model.name}</h3>
          <p className="field-hint">{model.description}</p>
        </div>
        <div className="ai-badges">
          <span className={`ai-badge ai-badge-${model.tier}`}>
            {lookupRecord(TIER_LABELS, model.tier) ?? model.tier}
          </span>
          <span
            className={`ai-badge ${
              model.status === "available" ? "" : "ai-badge-muted"
            }`}
          >
            {lookupRecord(STATUS_LABELS, model.status) ?? model.status}
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
          <dd className="tabular-nums">
            {bytes(model.requirements.ram_min_bytes)}
          </dd>
        </div>
        <div>
          <dt>Recommended RAM</dt>
          <dd className="tabular-nums">
            {bytes(model.requirements.ram_recommended_bytes)}
          </dd>
        </div>
        <div>
          <dt>Context window</dt>
          <dd className="tabular-nums">
            {number.format(model.requirements.context_tokens)} tokens
          </dd>
        </div>
        <div>
          <dt>Speed (desktop p50)</dt>
          <dd className="tabular-nums">
            {model.speed
              ? `${number.format(model.speed.p50_ms)} ms`
              : "Not measured"}
          </dd>
        </div>
        <div>
          <dt>Backend</dt>
          <dd>{model.backend}</dd>
        </div>
        <div>
          <dt>GPU offload</dt>
          <dd>
            {model.requirements.gpu_optional
              ? "Supported when available"
              : "CPU only"}
          </dd>
        </div>
        <div>
          <dt>License</dt>
          <dd>
            {(() => {
              const licenseUrl = safeExternalUrl(model.license.url);
              return licenseUrl ? (
                <a href={licenseUrl} target="_blank" rel="noreferrer">
                  {model.license.name}
                </a>
              ) : (
                <span>{model.license.name}</span>
              );
            })()}
          </dd>
        </div>
      </dl>
      {model.installed && (
        <p className="ai-installed-note">
          <Icon>check_circle</Icon>
          Installed{model.checksum_verified ? ", checksum verified" : ""} ·{" "}
          {bytes(model.installed_size_bytes ?? model.download.size_bytes)}
        </p>
      )}
      {model.error && <p className="form-error">{model.error}</p>}
      <DownloadProgress
        model={model}
        onCancel={onCancel}
        busy={busy === model.id}
      />
      <div className="ai-actions">
        {model.selectable && !model.installed && !running && (
          <Button
            variant="primary"
            onClick={onDownload}
            disabled={!canDownload || busy === model.id}
          >
            {busy === model.id ? (
              <span className="spinner-dot spinner-sm" aria-hidden="true" />
            ) : (
              <Icon>download</Icon>
            )}
            {busy === model.id ? "Starting…" : "Download"}
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
  const [customModelPath, setCustomModelPath] = useState("");
  const [threads, setThreads] = useState("4");
  const [gpuMode, setGpuMode] = useState<"auto" | "cpu" | "custom">("auto");
  const [customGpuLayers, setCustomGpuLayers] = useState("16");
  const [unload, setUnload] = useState("15");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState("");
  const [deleteTarget, setDeleteTarget] = useState<AiInstalledModel | null>(
    null,
  );
  const toast = useToast();

  function adopt(next: AiSettings) {
    setSettings(next);
    setEnabled(next.enabled);
    setActive(next.active_model ?? "");
    setCustomModelPath(next.custom_model_path ?? "");
    setThreads(String(next.threads));
    if (next.gpu_layers === -1) {
      setGpuMode("auto");
    } else if (next.gpu_layers === 0) {
      setGpuMode("cpu");
    } else {
      setGpuMode("custom");
      setCustomGpuLayers(String(next.gpu_layers));
    }
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
    load().catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, []);

  const polling =
    catalog?.models.some((model) => isRunning(model.download_job)) ?? false;
  useEffect(() => {
    if (!polling) return;
    const timer = setInterval(() => {
      Promise.all([api.aiCatalog(), api.aiSettings()])
        .then(([cat, set]) => {
          setCatalog(cat);
          adopt(set);
          if (!set.active_model) {
            const newlyInstalled = cat.models.find(
              (m) => m.installed && m.selectable && !isRunning(m.download_job),
            );
            if (newlyInstalled) {
              void api
                .selectAiModel(newlyInstalled.id)
                .then(() => Promise.all([api.aiCatalog(), api.aiSettings()]))
                .then(([c, s]) => {
                  setCatalog(c);
                  adopt(s);
                })
                .catch(() => {});
            }
          }
        })
        .catch((e) => setError(e instanceof Error ? e.message : String(e)));
    }, 2000);
    return () => clearInterval(timer);
  }, [polling]);

  async function save() {
    setBusy("settings");
    setError("");
    setNotice("");
    try {
      const gpuLayers =
        gpuMode === "auto"
          ? -1
          : gpuMode === "cpu"
            ? 0
            : Number(customGpuLayers);
      const next = await api.saveAiSettings({
        enabled,
        active_model: active || null,
        custom_model_path: customModelPath.trim() || null,
        threads: Number(threads),
        gpu_layers: gpuLayers,
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
        {error ? (
          <ErrorText>{error}</ErrorText>
        ) : (
          <div className="panel padded ai-loading-panel" role="status">
            <span className="spinner-dot spinner-sm" aria-hidden="true" />
            <span>Loading AI settings and model catalog…</span>
          </div>
        )}
      </div>
    );
  }

  const installedUsable = catalog.installed.filter(
    (model) => (model.checksum_verified || model.custom) && !model.error,
  );
  const activeModel = settings.active_model;

  return (
    <div className="ai-settings">
      <article className="panel padded">
        <div className="panel-title">
          <h2>AI</h2>
          <div className="ai-badges">
            <span className="ai-badge">
              {lookupRecord(BACKEND_LABELS, settings.backend) ??
                settings.backend}
            </span>
            <span className="ai-badge">
              {settings.runtime_available
                ? "Runtime available"
                : "Runtime not in this build"}
            </span>
          </div>
        </div>
        <p>
          Ask questions in natural language about your listening archive. A
          local GGUF model writes read-only SQL against your listening
          projection, and every number is computed locally inside a sandbox.
        </p>
        <p className="field-hint">
          AI is off by default. You can download a curated Qwen3.5 model, drop a
          custom GGUF folder inside the model folder, or point to any local GGUF
          file on the server. The model only loads when AI is turned on.
        </p>
        {!settings.runtime_available && (
          <div className="notice notice-warning" role="status">
            <Icon>info</Icon>
            <div className="notice-body">
              <strong>
                This build does not include the inference runtime.
              </strong>
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
              <strong>Enable AI</strong>
              <small>
                Saving is an explicit consent to run local SQL generation on
                this instance.
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
          <label>
            Custom local model path (.gguf)
            <input
              value={customModelPath}
              onChange={(event) => setCustomModelPath(event.target.value)}
              placeholder="e.g. /models/my-qwen3.5.gguf or my-model/model.gguf"
              autoComplete="off"
              spellCheck={false}
            />
            <small className="field-hint">
              Optional. Point to any local GGUF file on the server (absolute
              path or relative to <code>{catalog.model_dir}</code>). When set,
              Ask uses this model instead of the active catalog model above.
            </small>
          </label>
          {customModelPath.trim() !== "" && (
            <div className="ai-actions">
              <Button
                type="button"
                variant="secondary"
                onClick={() => setCustomModelPath("")}
              >
                <Icon>close</Icon>
                Clear custom path
              </Button>
            </div>
          )}
          <div className="ai-actions">
            <Button
              type="submit"
              variant="primary"
              disabled={busy === "settings"}
            >
              {busy === "settings" ? (
                <span className="spinner-dot spinner-sm" aria-hidden="true" />
              ) : (
                <Icon>save</Icon>
              )}
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
          Default local SQL models (Qwen3.5) can be downloaded from this curated
          catalog with pinned revisions and SHA-256 verification. You can also
          import any local <code>.gguf</code> model via a subfolder in{" "}
          <code>{catalog.model_dir}</code> or by setting the custom path above.
        </p>
        <div className="ai-grid">
          {catalog.models.map((model) => (
            <ModelCard
              key={model.id}
              model={model}
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
              onDelete={() =>
                setDeleteTarget({
                  id: model.id,
                  name: model.name,
                  file: model.download.file,
                  size_bytes: model.download.size_bytes,
                  installed_at_ms: null,
                  checksum_verified: model.checksum_verified,
                  custom: false,
                  error: null,
                })
              }
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
                    {model.file ?? "No readable file"} ·{" "}
                    {bytes(model.size_bytes)}
                    {model.custom ? " · Custom GGUF" : ""}
                    {model.id === activeModel ? " · Active" : ""}
                  </span>
                  {model.error && (
                    <span className="form-error">{model.error}</span>
                  )}
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
          whether this build and the selected model file can run; it never
          starts an inference or downloads anything.
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
            {busy === "test" ? (
              <span className="spinner-dot spinner-sm" aria-hidden="true" />
            ) : (
              <Icon>play_circle_outline</Icon>
            )}
            {busy === "test" ? "Testing model…" : "Test active model"}
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
            GPU offload
            <Select
              aria-label="GPU offload"
              value={gpuMode}
              onChange={(value) => {
                if (value === "auto" || value === "cpu" || value === "custom") {
                  setGpuMode(value);
                }
              }}
            >
              <option value="auto">Automatic (all layers on GPU)</option>
              <option value="cpu">CPU only (disable GPU offload)</option>
              <option value="custom">Custom layer count (1–256)</option>
            </Select>
          </label>
          {gpuMode === "custom" && (
            <label>
              Custom GPU layers
              <input
                type="number"
                min={1}
                max={256}
                value={customGpuLayers}
                onChange={(event) => setCustomGpuLayers(event.target.value)}
                inputMode="numeric"
              />
            </label>
          )}
          {settings.backend === "cpu" ? (
            <p className="field-hint">
              This instance was built for CPU-only execution. GPU offload
              settings are saved, but inference currently runs on CPU; GPU
              offload requires the Vulkan container image or the native macOS
              Metal build.
            </p>
          ) : (
            (settings.backend === "metal" || settings.backend === "vulkan") && (
              <p className="field-hint">
                If GPU loading fails at runtime (e.g. insufficient VRAM or
                missing driver), Ask automatically falls back to CPU-only
                execution.
              </p>
            )
          )}
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
          <Button
            type="submit"
            variant="secondary"
            disabled={busy === "settings"}
          >
            {busy === "settings" ? (
              <span className="spinner-dot spinner-sm" aria-hidden="true" />
            ) : (
              <Icon>save</Icon>
            )}
            {busy === "settings" ? "Saving…" : "Save options"}
          </Button>
        </form>
      </article>

      {deleteTarget && (
        <Modal title="Delete model" close={() => setDeleteTarget(null)}>
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
                action(target.id, async () => {
                  const result = await api.deleteAiModel(
                    target.id,
                    true,
                    target.id === activeModel,
                  );
                  setDeleteTarget(null);
                  toast(`Model deleted. ${bytes(result.freed_bytes)} freed.`);
                });
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
