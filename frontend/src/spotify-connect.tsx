import { useEffect, useRef, useState } from "react";
import { api } from "./api";
import type { ConnectOutcome } from "./api";
import { Icon, Modal } from "./ui";
import { SkeletonCard } from "./skeleton";

type Flow = "opening" | "waiting" | "interrupted" | "connected" | "error";
const message = (error: unknown) =>
  error instanceof Error ? error.message : String(error);

function closeLocalTab(tab: Window | null) {
  // Safari cannot remotely close an opener-less tab while it is on Spotify.
  // After the callback returns to our origin, it is safe to close it.
  try {
    if (
      tab &&
      !tab.closed &&
      (tab.location.href === "about:blank" ||
        tab.location.origin === location.origin)
    )
      tab.close();
  } catch {
    /* The external tab is isolated; its user can close it. */
  }
}

function ConnectionResult({ state, detail }: { state: Flow; detail: string }) {
  const waiting = state === "opening" || state === "waiting";
  return (
    <div
      className={`spotify-connect-result ${state}`}
      role="status"
      aria-live="polite"
    >
      {waiting ? (
        <span className="connect-spinner" aria-hidden="true" />
      ) : (
        <Icon>{state === "connected" ? "check_circle" : "error_outline"}</Icon>
      )}
      <p>{detail}</p>
    </div>
  );
}

export function SpotifyConnectButton({
  connected,
  disabled,
  refresh,
}: {
  connected: boolean;
  disabled: boolean;
  refresh: () => Promise<void>;
}) {
  const [flow, setFlow] = useState<Flow | null>(null);
  const [detail, setDetail] = useState("");
  const [attempt, setAttempt] = useState<string | null>(null);
  const [cancelling, setCancelling] = useState(false);
  const tab = useRef<Window | null>(null);
  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;

  useEffect(() => {
    if (!attempt) return;
    let stopped = false,
      busy = false,
      finished = false;
    const started = Date.now();
    const poll = async () => {
      if (busy || finished) return;
      if (Date.now() - started > 11 * 60_000) {
        finished = true;
        setFlow("error");
        setDetail("This connection attempt timed out. Please try again.");
        return;
      }
      busy = true;
      try {
        const result = await api.connectStatus(attempt);
        if (stopped) return;
        if (result.outcome === "connected" || result.outcome === "error") {
          finished = true;
          setFlow(result.outcome);
          setDetail(
            result.outcome === "connected"
              ? "Connected. Your Spotify authorization is saved."
              : result.message ||
                  "Spotify could not connect. Please try again.",
          );
          closeLocalTab(tab.current);
          void refreshRef.current().catch(() => {});
        } else if (tab.current?.closed && result.outcome === "pending") {
          setFlow("interrupted");
          setDetail(
            "The Spotify tab was closed or is no longer reachable. You can try again or cancel this attempt.",
          );
        } else {
          setFlow("waiting");
          setDetail(
            result.outcome === "processing"
              ? "Completing the connection with Spotify…"
              : "Waiting for Spotify. Complete authorization in the new tab, then return here.",
          );
        }
      } catch (error) {
        if (!stopped) {
          setFlow("error");
          setDetail(`Unable to check the connection. ${message(error)}`);
        }
      } finally {
        busy = false;
      }
    };
    void poll();
    const timer = setInterval(() => void poll(), 1000);
    const wake = () => {
      if (!document.hidden) void poll();
    };
    document.addEventListener("visibilitychange", wake);
    window.addEventListener("focus", wake);
    return () => {
      stopped = true;
      clearInterval(timer);
      document.removeEventListener("visibilitychange", wake);
      window.removeEventListener("focus", wake);
    };
  }, [attempt]);

  async function cancel() {
    if (flow === "opening" || cancelling) return;
    setCancelling(true);
    try {
      if (attempt && flow !== "connected") await api.cancelConnect(attempt);
      closeLocalTab(tab.current);
      setAttempt(null);
      setFlow(null);
    } catch (error) {
      setFlow("error");
      setDetail(`Unable to cancel the attempt. ${message(error)}`);
    } finally {
      setCancelling(false);
    }
  }

  async function start() {
    // Open synchronously within the click, before fetching the authorization URL.
    const opened = window.open("about:blank", "_blank");
    if (opened) opened.opener = null;
    closeLocalTab(tab.current);
    tab.current = opened;
    setAttempt(null);
    setFlow("opening");
    setDetail("Opening Spotify in a new tab…");
    try {
      if (attempt) await api.cancelConnect(attempt);
      if (!opened)
        throw new Error(
          "Your browser blocked the Spotify tab. Allow new tabs for scrobbl.rs, then try again.",
        );
      const result = await api.connect();
      const url = new URL(result.url);
      if (url.origin !== "https://accounts.spotify.com")
        throw new Error("Unexpected Spotify authorization URL.");
      setAttempt(result.attempt);
      if (opened.closed) {
        await api.cancelConnect(result.attempt);
        setFlow("error");
        setDetail(
          "The Spotify tab was closed before authorization started. Please try again.",
        );
        return;
      }
      opened.location.replace(url.href);
      setFlow("waiting");
      setDetail(
        "Waiting for Spotify. Complete authorization in the new tab, then return here.",
      );
    } catch (error) {
      closeLocalTab(opened);
      setFlow("error");
      setDetail(message(error));
    }
  }

  return (
    <>
      <button
        className="primary"
        disabled={disabled || flow !== null}
        onClick={() => void start()}
      >
        {connected ? "Reconnect Spotify" : "Connect Spotify"}
        <Icon>open_in_new</Icon>
      </button>
      {flow && (
        <Modal
          title={
            flow === "connected" ? "Connected to Spotify" : "Connect Spotify"
          }
          close={() => void cancel()}
        >
          <ConnectionResult state={flow} detail={detail} />
          <div className="connection-actions">
            {(flow === "error" || flow === "interrupted") && (
              <button
                className="primary"
                disabled={cancelling}
                onClick={() => void start()}
              >
                Try again
              </button>
            )}
            <button
              className="secondary"
              disabled={flow === "opening" || cancelling}
              onClick={() => void cancel()}
            >
              {flow === "connected" ? "Done" : "Cancel"}
            </button>
          </div>
        </Modal>
      )}
    </>
  );
}

// The return tab reads the same session-bound result as the original tab.
// No tokens, callback parameters or postMessage claims are trusted by the UI.
export function SpotifyConnectReturn({ attempt }: { attempt: string }) {
  const [result, setResult] = useState<ConnectOutcome | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    let stopped = false;
    void api
      .connectStatus(attempt)
      .then((value) => {
        if (!stopped) setResult(value);
      })
      .catch((error) => {
        if (!stopped) setError(message(error));
      });
    return () => {
      stopped = true;
    };
  }, [attempt]);
  const connected = result?.outcome === "connected";
  return (
    <main className="connect-return panel padded">
      <h1>{connected ? "Connected to Spotify" : "Spotify connection"}</h1>
      {!result && !error ? (
        <SkeletonCard label="Checking the connection…" />
      ) : (
        <ConnectionResult
          state={
            error || result?.outcome === "error"
              ? "error"
              : connected
                ? "connected"
                : "waiting"
          }
          detail={
            error ||
            result?.message ||
            (connected
              ? "Connected. You can close this tab and return to scrobbl.rs."
              : "Checking the connection…")
          }
        />
      )}
      <a className="button secondary" href="/settings/connections">
        Back to Connections
      </a>
    </main>
  );
}
