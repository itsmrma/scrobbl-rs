import { createContext, useContext, useEffect, useRef, useState } from "react";
import type { CSSProperties, ReactNode } from "react";
import type { Preferences } from "./generated";

export function Icon({ children }: { children: string }) {
  return (
    <span className="material-icons-outlined" aria-hidden="true">
      {children}
    </span>
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
  useEffect(() => setFailed(false), [url]);
  return url?.startsWith("https://i.scdn.co/") && !failed ? (
    <img
      className={`cover ${large ? "large" : ""}`}
      src={url}
      alt=""
      loading="lazy"
      referrerPolicy="no-referrer"
      onError={() => setFailed(true)}
    />
  ) : (
    <span className={`cover placeholder ${large ? "large" : ""}`}>
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
  eyebrow,
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
        {eyebrow && <p className="eyebrow">{eyebrow}</p>}
        <h1>
          {title}
          <span className="accent">.</span>
        </h1>
        {children && <p>{children}</p>}
      </div>
    </div>
  );
}
export function ErrorText({ children }: { children?: ReactNode }) {
  return children ? (
    <p className="form-error" role="alert">
      {children}
    </p>
  ) : null;
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
