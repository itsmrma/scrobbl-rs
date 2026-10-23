import { queueArtwork } from "./artwork-activity";
import { downloadArtwork } from "./api";
import { createContext, useContext, useEffect, useRef, useState } from "react";
import type { CSSProperties, ReactNode } from "react";
import type { Preferences } from "./generated";

export const ArtworkContext = createContext<{
  connected: boolean;
  retryAt: number | null;
}>({ connected: false, retryAt: null });

export function Icon({ children }: { children: string }) {
  return (
    <span className="material-icons-outlined" aria-hidden="true">
      {children}
    </span>
  );
}
export function Wordmark({ name = "scrobbl.rs" }: { name?: string }) {
  return name === "scrobbl.rs" ? (
    <span className="wordmark">
      scrobbl<span className="accent">.</span>
      <span className="wordmark-suffix">rs</span>
    </span>
  ) : (
    <span className="wordmark">{name}</span>
  );
}
export function CopyButton({
  value,
  children,
  disabled = false,
}: {
  value: string;
  children: ReactNode;
  disabled?: boolean;
}) {
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    setCopied(false);
    setError("");
  }, [value]);
  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 3000);
    return () => clearTimeout(timer);
  }, [copied]);
  return (
    <div className="copy-control">
      <button
        type="button"
        className="secondary"
        disabled={disabled}
        onClick={async () => {
          setError("");
          try {
            await navigator.clipboard.writeText(value);
            setCopied(true);
          } catch {
            setCopied(false);
            setError("Could not copy. Select and copy the text manually.");
          }
        }}
      >
        <Icon>content_copy</Icon>
        {children}
      </button>
      <span className="copy-feedback" role="status">
        {copied && (
          <>
            <Icon>check</Icon>
            <span className="visually-hidden">Copied</span>
          </>
        )}
      </span>
      {error && (
        <span className="copy-error" role="alert">
          {error}
        </span>
      )}
    </div>
  );
}
export function Cover({
  url,
  large = false,
}: {
  url?: string | null;
  large?: boolean;
}) {
  const [failed, setFailed] = useState(false);
  const [revision, setRevision] = useState(0);
  const [readySource, setReadySource] = useState("");
  const [cacheMiss, setCacheMiss] = useState("");
  const marker = useRef<HTMLElement | null>(null);
  const release = useRef<{
    source: string;
    done: (loaded: boolean | null) => void;
  } | null>(null);
  const local = !!url?.startsWith("/api/v1/artwork/");
  const source = local
    ? `${url}${url!.includes("?") ? "&" : "?"}image_revision=${revision}`
    : url;
  useEffect(() => {
    if (!local || !source || !marker.current) return;
    let cancel = () => {};
    let pending = false;
    let current = true;
    const observer = new IntersectionObserver(
      (entries) => {
        if (!entries.some((entry) => entry.isIntersecting)) return;
        if (cacheMiss !== source) {
          observer.disconnect();
          setReadySource(`${source}&cached=true`);
        } else if (!pending) {
          pending = true;
          cancel = queueArtwork((done) => {
            const rect = marker.current?.getBoundingClientRect();
            // Scrolling away must not leave invisible cold images ahead of
            // the page the owner is actually reading. Observe them again.
            if (
              !rect?.width ||
              rect.bottom <= 0 ||
              rect.top >= window.innerHeight
            ) {
              pending = false;
              done(null);
              return;
            }
            observer.disconnect();
            release.current = { source, done };
            void downloadArtwork(source)
              .then(() => {
                done(true);
                if (current)
                  setReadySource(source + "&cached=true&downloaded=true");
              })
              .catch(() => {
                done(false);
                if (current) setFailed(true);
              });
          }, !!marker.current?.closest(".music-heading"));
        }
      },
      { rootMargin: cacheMiss === source ? "0px" : "240px" },
    );
    observer.observe(marker.current);
    return () => {
      current = false;
      observer.disconnect();
      cancel();
    };
  }, [local, source, cacheMiss]);
  const finish = (loaded: boolean) => {
    const current = release.current;
    if (current && current.source === source) current.done(loaded);
  };
  const artwork = useContext(ArtworkContext);
  useEffect(() => setFailed(false), [url]);
  useEffect(() => {
    if (!failed || !artwork.connected || !url?.startsWith("/api/v1/artwork/"))
      return;
    // Retry once the shared cooldown expires. Failed images also recover after
    // a temporary network failure without requiring the owner to reload.
    const due =
      Date.now() + Math.max(60_000, (artwork.retryAt || 0) - Date.now() + 1000);
    const retry = () => {
      if (!document.hidden && Date.now() >= due) {
        setFailed(false);
        setRevision((v) => v + 1);
      }
    };
    const timer = setTimeout(retry, due - Date.now());
    document.addEventListener("visibilitychange", retry);
    window.addEventListener("online", retry);
    return () => {
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", retry);
      window.removeEventListener("online", retry);
    };
  }, [failed, artwork.connected, artwork.retryAt, url]);
  useEffect(() => {
    if (!failed || !local) return;
    const changed = () => {
      setFailed(false);
      setRevision((v) => v + 1);
    };
    window.addEventListener("artwork-cache-changed", changed);
    return () => window.removeEventListener("artwork-cache-changed", changed);
  }, [failed, local]);
  useEffect(() => {
    const changed = (event: Event) => {
      const { kind, id } = (event as CustomEvent<{ kind: string; id: string }>)
        .detail;
      if (url?.split("?")[0] === `/api/v1/artwork/${kind}/${id}`) {
        setFailed(false);
        setRevision((v) => v + 1);
      }
    };
    window.addEventListener("artwork-updated", changed);
    return () => window.removeEventListener("artwork-updated", changed);
  }, [url]);
  return (url?.startsWith("https://i.scdn.co/") || local) &&
    !failed &&
    (!local ||
      readySource === source ||
      readySource === `${source}&cached=true` ||
      readySource === source + "&cached=true&downloaded=true") ? (
    <img
      ref={(node) => {
        marker.current = node;
      }}
      className={`cover ${large ? "large" : ""}`}
      src={(local ? readySource : source) || undefined}
      alt=""
      loading={local ? "eager" : "lazy"}
      referrerPolicy="no-referrer"
      onLoad={() => finish(true)}
      onError={() => {
        if (local && readySource.endsWith("&cached=true")) {
          setReadySource("");
          setCacheMiss(source || "");
          return;
        }
        finish(false);
        setFailed(true);
      }}
    />
  ) : (
    <span
      ref={(node) => {
        marker.current = node;
      }}
      className={`cover placeholder ${large ? "large" : ""}`}
    >
      <Icon>music_note</Icon>
    </span>
  );
}
export function SpotifyLink({
  url,
  children,
}: {
  url?: string | null;
  children: ReactNode;
}) {
  return url?.startsWith("https://open.spotify.com/track/") ? (
    <a href={url} target="_blank" rel="noreferrer">
      {children}
    </a>
  ) : (
    <strong>{children}</strong>
  );
}
export const number = new Intl.NumberFormat("en-GB");
export function duration(ms: number | null, compact = false): string {
  if (ms === null) return "Unknown";
  const seconds = Math.floor(ms / 1000),
    minutes = Math.floor(seconds / 60),
    hours = Math.floor(minutes / 60);
  return compact
    ? hours
      ? `${number.format(hours)}h ${minutes % 60}m`
      : `${number.format(minutes)}m`
    : `${minutes}:${String(seconds % 60).padStart(2, "0")}`;
}
export function date(ms: number | null, preferences: Preferences) {
  return ms === null
    ? "Never"
    : new Intl.DateTimeFormat("en-GB", {
        timeZone: preferences.timezone,
        day: "2-digit",
        month: "short",
        year: "numeric",
        hour: "2-digit",
        minute: "2-digit",
        hourCycle: preferences.hour_format === "12" ? "h12" : "h23",
      }).format(ms);
}
export function Heading({
  title,
  children,
}: {
  eyebrow?: string;
  title: string;
  children?: ReactNode;
}) {
  return (
    <div className="page-heading">
      <div>
        <h1>
          {title}
          <span className="accent">.</span>
        </h1>
        {children && <p>{children}</p>}
      </div>
    </div>
  );
}
export function Notice({
  children,
  title,
  tone = "error",
}: {
  children?: ReactNode;
  title: string;
  tone?: "error" | "warning" | "success" | "info";
}) {
  return children ? (
    <div
      className={`notice notice-${tone}`}
      role={tone === "error" ? "alert" : "status"}
    >
      <Icon>
        {
          {
            error: "error_outline",
            warning: "schedule",
            success: "check_circle",
            info: "info",
          }[tone]
        }
      </Icon>
      <div>
        <strong>{title}</strong>
        <div className="notice-message">{children}</div>
      </div>
    </div>
  ) : null;
}
export function ErrorText({ children }: { children?: ReactNode }) {
  return <Notice title="Something needs attention">{children}</Notice>;
}
export const ToastContext = createContext<
  (message: string, error?: boolean) => void
>(() => {});
export const useToast = () => useContext(ToastContext);
export function Toasts({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<
    { id: number; message: string; error: boolean }[]
  >([]);
  const next = useRef(0);
  const timers = useRef(new Set<ReturnType<typeof setTimeout>>());
  useEffect(
    () => () => {
      timers.current.forEach(clearTimeout);
    },
    [],
  );
  const dismiss = (id: number) =>
    setToasts((items) => items.filter((item) => item.id !== id));
  function notify(message: string, error = false) {
    const id = next.current++;
    setToasts((items) => [...items, { id, message, error }]);
    const timer = setTimeout(() => {
      dismiss(id);
      timers.current.delete(timer);
    }, 5000);
    timers.current.add(timer);
  }
  return (
    <ToastContext value={notify}>
      {children}
      <div className="toasts" aria-live="polite" aria-relevant="additions">
        {toasts.map((item) => (
          <div
            className={`toast ${item.error ? "error" : ""}`}
            key={item.id}
            role="status"
          >
            <div>{item.message}</div>
            <button
              className="icon-button"
              aria-label="Dismiss notification"
              onClick={() => dismiss(item.id)}
            >
              <Icon>close</Icon>
            </button>
            <span className="toast-time" aria-hidden="true" />
          </div>
        ))}
      </div>
    </ToastContext>
  );
}
export function Modal({
  title,
  children,
  close,
}: {
  title: string;
  children: ReactNode;
  close: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const focus = document.activeElement;
    ref.current?.showModal();
    return () => {
      if (focus instanceof HTMLElement) focus.focus();
    };
  }, []);
  return (
    <dialog
      ref={ref}
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
      aria-labelledby="dialog-title"
    >
      <h2 id="dialog-title">{title}</h2>
      {children}
    </dialog>
  );
}
export type Style = CSSProperties & Record<`--${string}`, string | number>;
