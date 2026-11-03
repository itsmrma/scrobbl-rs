import { useEffect, useState } from "react";
import { Icon, Wordmark } from "./ui";

export interface LifecycleScreenProps {
  action: "restart" | "stop";
  onDismiss?: () => void;
  onReconnect?: () => void;
}

export function LifecycleScreen({
  action,
  onDismiss,
  onReconnect,
}: LifecycleScreenProps) {
  const [phase, setPhase] = useState<
    "starting" | "polling" | "ready" | "stopped"
  >(action === "restart" ? "polling" : "stopped");
  const [statusMessage, setStatusMessage] = useState(
    action === "restart"
      ? "Riavvio di scrobbl.rs in corso..."
      : "Applicazione arrestata. L'archivio è stato chiuso in sicurezza. Riavvia il container da Docker quando desideri.",
  );
  const [details, setDetails] = useState("");
  const [checking, setChecking] = useState(false);
  const [checkFeedback, setCheckFeedback] = useState("");

  useEffect(() => {
    if (action !== "restart") return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;

    const poll = async () => {
      try {
        const response = await fetch("/api/boot-status", {
          signal: controller.signal,
          cache: "no-store",
        });
        if (response.ok) {
          const data = (await response.json()) as {
            ready?: boolean;
            stage?: string;
            error?: string | null;
          };
          if (data.ready) {
            setPhase("ready");
            setStatusMessage("Riavvio completato. scrobbl.rs has restarted.");
            window.dispatchEvent(new Event("read-data-changed"));
            timer = setTimeout(() => {
              if (onReconnect) {
                onReconnect();
              } else {
                window.location.reload();
              }
            }, 1200);
            return;
          } else if (data.stage) {
            setDetails(`Stato di avvio: ${data.stage}`);
          }
        }
      } catch {
        // Expected network error while the server restarts
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
      const response = await fetch("/api/boot-status", { cache: "no-store" });
      if (response.ok) {
        const data = (await response.json()) as {
          ready?: boolean;
          stage?: string;
        };
        if (data.ready) {
          setCheckFeedback("Server pronto! Riconnessione in corso...");
          setTimeout(() => {
            if (onReconnect) onReconnect();
            else window.location.reload();
          }, 800);
          return;
        } else {
          setCheckFeedback(
            `Server in fase di avvio (${data.stage || "inizializzazione"}). Riprova a breve.`,
          );
        }
      } else {
        setCheckFeedback("Server non ancora raggiungibile. Riprova tra poco.");
      }
    } catch {
      setCheckFeedback(
        "Server non ancora raggiungibile. Avvia il container da Docker quando desideri.",
      );
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
        action === "restart" ? "Riavvio in corso" : "Applicazione arrestata"
      }
    >
      <div className="lifecycle-brand">
        <Wordmark />
      </div>
      {action === "restart" && phase !== "ready" && (
        <div className="lifecycle-loader" aria-hidden="true">
          <div className="lifecycle-dot" />
        </div>
      )}
      {action === "restart" && phase === "ready" && (
        <div className="lifecycle-icon-success" aria-hidden="true">
          <Icon>check_circle</Icon>
        </div>
      )}
      {action === "stop" && (
        <div className="lifecycle-icon-stopped" aria-hidden="true">
          <Icon>power_settings_new</Icon>
        </div>
      )}
      <p className="lifecycle-status" role="status">
        {statusMessage}
      </p>
      {details && <p className="lifecycle-details">{details}</p>}
      {action === "stop" && (
        <>
          <small className="lifecycle-subtext">
            Stop accepted. Start scrobbl.rs from Docker or your process manager
            when you are ready.
          </small>
          <div className="lifecycle-actions">
            <button
              type="button"
              className="primary"
              disabled={checking}
              onClick={() => void checkStatus()}
            >
              <Icon>refresh</Icon>
              {checking
                ? "Verifica in corso..."
                : "Verifica disponibilità / Riconnetti"}
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
          <button type="button" className="secondary" onClick={onDismiss}>
            Close
          </button>
        </div>
      )}
    </div>
  );
}
