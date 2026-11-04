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
  title: string;
  detail: string;
}[] = [
  {
    key: "server",
    title: "Local HTTP listener",
    detail: "Bound and accepting connections",
  },
  {
    key: "database",
    title: "SQLite WAL archive",
    detail: "Schema and snapshots verified",
  },
  {
    key: "session",
    title: "Owner session",
    detail: "Authentication and CSRF checked",
  },
  {
    key: "preferences",
    title: "Library preferences",
    detail: "Timezone calendar and theme loaded",
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
    ? "Archive startup needs attention."
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
            <img src="/favicon.svg" alt="" width="30" height="30" />
            <Wordmark name={instanceName} />
          </div>
          <span
            className={`startup-badge ${error ? "is-error" : stage === "ready" ? "is-ready" : ""}`}
          >
            <span>
              {error
                ? "Startup error"
                : stage === "ready"
                  ? "Ready"
                  : "Starting"}
            </span>
          </span>
        </div>

        <div className="startup-hero">
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

        <ol className="startup-stage-list">
          {STAGES.map((item, idx) => {
            const done = !error && currentIdx > idx;
            const active = !error && currentIdx === idx;
            return (
              <li
                key={item.key}
                className={`startup-stage ${done ? "done" : ""} ${active ? "active" : ""}`}
              >
                <span className="startup-stage-icon" aria-hidden="true">
                  <Icon>
                    {done
                      ? "check_circle"
                      : active
                        ? "schedule"
                        : "radio_button_unchecked"}
                  </Icon>
                </span>
                <div className="startup-stage-copy">
                  <strong>{item.title}</strong>
                  <small>{item.detail}</small>
                </div>
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
              ? "Startup sequence preview · SQLite WAL"
              : "Self-hosted listening archive"}
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
