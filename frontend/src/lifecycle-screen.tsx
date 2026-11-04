import { useEffect, useState } from "react";
import { Icon, Wordmark } from "./ui";

export interface LifecycleScreenProps {
  action: "restart" | "stop";
  onDismiss?: () => void;
  onReconnect?: () => void;
}

interface BootStatusResponse {
  ready?: boolean;
  stage?: string;
  error?: string | null;
}

async function fetchBootStatus(
  signal?: AbortSignal,
): Promise<BootStatusResponse | null> {
  try {
    const response = await fetch("/api/boot-status", {
      signal,
      cache: "no-store",
    });
    if (response.ok) {
      return (await response.json()) as BootStatusResponse;
    }
  } catch {
    // Expected during restart/shutdown
  }
  return null;
}

export function LifecycleScreen({
  action,
  onDismiss,
  onReconnect,
}: LifecycleScreenProps) {
  const [phase, setPhase] = useState<"polling" | "ready" | "stopped">(
    action === "restart" ? "polling" : "stopped",
  );
  const [statusMessage, setStatusMessage] = useState(
    action === "restart"
      ? "Restarting scrobbl.rs…"
      : "Application stopped. The archive has been closed safely. Restart the container from Docker when you are ready.",
  );
  const [details, setDetails] = useState("");
  const [checking, setChecking] = useState(false);
  const [checkFeedback, setCheckFeedback] = useState("");

  useEffect(() => {
    if (action !== "restart") return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;

    const poll = async () => {
      const data = await fetchBootStatus(controller.signal);
      if (data?.ready) {
        setPhase("ready");
        setStatusMessage("Restart complete. scrobbl.rs has restarted.");
        window.dispatchEvent(new Event("read-data-changed"));
        timer = setTimeout(() => {
          if (onReconnect) {
            onReconnect();
          } else {
            window.location.reload();
          }
        }, 1200);
        return;
      }
      if (data?.stage) {
        setDetails(`Startup stage: ${data.stage}`);
      }
      if (!controller.signal.aborted) {
        timer = setTimeout(poll, 1000);
      }
    };

    timer = setTimeout(poll, 1000);
    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [action, onReconnect]);

  const checkStatus = async () => {
    setChecking(true);
    setCheckFeedback("");
    try {
      const data = await fetchBootStatus();
      if (data?.ready) {
        setCheckFeedback("Server is ready! Reconnecting…");
        setTimeout(() => {
          if (onReconnect) onReconnect();
          else window.location.reload();
        }, 800);
      } else if (data?.stage) {
        setCheckFeedback(
          `Server is starting (${data.stage}). Try again shortly.`,
        );
      } else {
        setCheckFeedback(
          "Server is not yet reachable. Start the container from Docker when you are ready.",
        );
      }
    } finally {
      setChecking(false);
    }
  };

  return (
    <div
      className="lifecycle-screen"
      role="dialog"
      aria-modal="true"
      aria-label={
        action === "restart" ? "Restart in progress" : "Application stopped"
      }
    >
      <div className="startup-card lifecycle-card">
        <div className="startup-header">
          <div className="lifecycle-brand startup-brand">
            <img src="/favicon.svg" alt="" width="30" height="30" />
            <Wordmark />
          </div>
          {action === "restart" && phase !== "ready" && (
            <div className="lifecycle-loader startup-badge" aria-hidden="true">
              <Icon>sync</Icon>
              <span>Restarting</span>
            </div>
          )}
          {action === "restart" && phase === "ready" && (
            <div
              className="lifecycle-icon-success startup-badge is-ready"
              aria-hidden="true"
            >
              <Icon>check_circle</Icon>
              <span>Ready</span>
            </div>
          )}
          {action === "stop" && (
            <div
              className="lifecycle-icon-stopped startup-badge"
              aria-hidden="true"
            >
              <Icon>power_settings_new</Icon>
              <span>Offline</span>
            </div>
          )}
        </div>

        {action === "restart" && (
          <div className="startup-progress-track" aria-hidden="true">
            <div
              className="startup-progress-fill"
              style={{ width: phase === "ready" ? "100%" : "60%" }}
            />
          </div>
        )}

        <p className="lifecycle-status" role="status">
          {statusMessage}
        </p>
        {details && <p className="lifecycle-details">{details}</p>}

        {action === "stop" && (
          <>
            <small className="lifecycle-subtext">
              Stop accepted. Start scrobbl.rs from Docker or your process
              manager when you are ready.
            </small>
            <div className="lifecycle-actions">
              <button
                type="button"
                className="btn btn-primary primary"
                disabled={checking}
                onClick={() => void checkStatus()}
              >
                <Icon>refresh</Icon>
                {checking ? "Checking…" : "Check availability / Reconnect"}
              </button>
            </div>
            {checkFeedback && (
              <p className="lifecycle-feedback" role="alert">
                {checkFeedback}
              </p>
            )}
          </>
        )}
        {phase === "ready" && onDismiss && (
          <div className="lifecycle-actions">
            <button
              type="button"
              className="btn btn-secondary secondary"
              onClick={onDismiss}
            >
              Close
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
