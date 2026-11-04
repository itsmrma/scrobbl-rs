import { Icon, Wordmark } from "./ui";

export type BootStageId =
  "server" | "database" | "session" | "preferences" | "ready";

export type BootStage = BootStageId;

export interface StartupScreenProps {
  stage: BootStageId;
  detail?: string;
  statusText?: string;
  instanceName?: string;
  preview?: boolean;
  error?: string;
  onReload?: () => void;
  onRetry?: () => void;
  onDismiss?: () => void;
  onDismissPreview?: () => void;
}

const STAGES: {
  key: BootStageId;
  index: string;
  title: string;
  detail: string;
}[] = [
  {
    key: "server",
    index: "01",
    title: "HTTP Listener",
    detail: "Local endpoint bound",
  },
  {
    key: "database",
    index: "02",
    title: "SQLite WAL Archive",
    detail: "Schema & snapshots verified",
  },
  {
    key: "session",
    index: "03",
    title: "Access & CSRF",
    detail: "Owner session checked",
  },
  {
    key: "preferences",
    index: "04",
    title: "Atelier Workspace",
    detail: "Calendar & theme loaded",
  },
];

const STAGE_ORDER: Record<BootStageId, number> = {
  server: 0,
  database: 1,
  session: 2,
  preferences: 3,
  ready: 4,
};

export function StartupScreen({
  stage,
  detail,
  statusText,
  instanceName,
  preview = false,
  error,
  onReload,
  onRetry,
  onDismiss,
  onDismissPreview,
}: StartupScreenProps) {
  const currentIdx = STAGE_ORDER[stage] ?? 0;
  const dismiss = onDismiss || onDismissPreview;
  const retry = onReload || onRetry;
  const headline = error
    ? "Archive startup needs attention"
    : detail ||
      statusText ||
      (stage === "server"
        ? "Binding local server listener…"
        : stage === "database"
          ? "Opening and verifying SQLite WAL archive…"
          : stage === "session"
            ? "Verifying owner session and security state…"
            : stage === "preferences"
              ? "Loading calendar, preferences and layout…"
              : "Your local listening archive is ready.");

  const progressPct = Math.min(100, Math.round(((currentIdx + 1) / 4) * 100));

  return (
    <div
      className="startup-screen"
      data-testid="startup-screen"
      role="status"
      aria-live="polite"
      aria-label="Application startup"
    >
      <div className="startup-card">
        <div className="startup-header">
          <div className="startup-brand">
            <img src="/favicon.svg" alt="" width="34" height="34" />
            <Wordmark />
          </div>
          <span className={`startup-badge ${error ? "is-error" : ""}`}>
            <span className="startup-dot" aria-hidden="true" />
            <span>
              {error
                ? "Startup error"
                : stage === "ready"
                  ? "Ready"
                  : "Booting"}
            </span>
          </span>
        </div>

        <div className="startup-hero">
          <p className="eyebrow">
            {instanceName || "PRECISION LISTENING ARCHIVE"}
          </p>
          <h1>
            {stage === "ready" ? "Archive ready" : "Opening your archive"}
            <span className="accent">.</span>
          </h1>
          <p className="startup-subtitle">{headline}</p>
        </div>

        <div className="startup-progress-track" aria-hidden="true">
          <div
            className="startup-progress-fill"
            style={{ width: `${error ? 100 : progressPct}%` }}
          />
        </div>

        <ol className="startup-stage-grid">
          {STAGES.map((item, idx) => {
            const done = !error && currentIdx > idx;
            const active = !error && currentIdx === idx;
            return (
              <li
                key={item.key}
                className={`startup-stage ${done ? "done" : ""} ${active ? "active" : ""}`}
              >
                <span className="startup-stage-index">{item.index}</span>
                <div className="startup-stage-copy">
                  <strong>{item.title}</strong>
                  <small>{item.detail}</small>
                </div>
                <span className="startup-stage-icon" aria-hidden="true">
                  <Icon>
                    {done
                      ? "check_circle"
                      : active
                        ? "pending"
                        : "radio_button_unchecked"}
                  </Icon>
                </span>
              </li>
            );
          })}
        </ol>

        {error && (
          <div className="notice notice-danger" role="alert">
            <Icon>error_outline</Icon>
            <div className="notice-body">
              <strong>Could not finish startup</strong>
              <p>{error}</p>
              <div>
                <button
                  type="button"
                  className="btn btn-primary primary"
                  onClick={() => (retry ? retry() : window.location.reload())}
                >
                  <Icon>refresh</Icon>Reload archive
                </button>
              </div>
            </div>
          </div>
        )}

        <div className="startup-footer">
          <span>
            {preview
              ? "Startup sequence preview · Self-hosted SQLite WAL"
              : "Self-hosted listening archive · Local-first"}
          </span>
          {dismiss && (
            <button
              type="button"
              className="btn btn-primary primary"
              onClick={dismiss}
            >
              Continue to archive
              <Icon>arrow_forward</Icon>
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
