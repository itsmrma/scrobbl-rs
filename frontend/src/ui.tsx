import { queueArtwork, useArtworkRevision } from "./artwork-activity";
import { createContext, useContext, useEffect, useRef, useState } from "react";
import type { ButtonHTMLAttributes, CSSProperties, ReactNode } from "react";
import type { Preferences } from "./generated";

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
    <span className="wordmark">
      {name}
      {!name.endsWith(".") && <span className="accent">.</span>}
    </span>
  );
}

export type ButtonVariant =
  "primary" | "secondary" | "ghost" | "icon" | "danger";

export const buttonClass = (
  variant: ButtonVariant = "secondary",
  extra = "",
) => {
  const map: Record<ButtonVariant, string> = {
    primary: "btn btn-primary primary",
    secondary: "btn btn-secondary secondary",
    ghost: "btn btn-ghost",
    icon: "btn-icon icon-button",
    danger: "btn btn-danger danger",
  };
  return `${map[variant]} ${extra}`.trim();
};

export function Button({
  variant = "secondary",
  className = "",
  type = "button",
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: ButtonVariant }) {
  return (
    <button type={type} className={buttonClass(variant, className)} {...rest}>
      {children}
    </button>
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
        className="btn btn-secondary secondary"
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
  children,
}: {
  url?: string | null;
  large?: boolean;
  children?: ReactNode;
}) {
  const [failed, setFailed] = useState(false);
  const globalRevision = useArtworkRevision(url);
  const [localRevision, setLocalRevision] = useState(0);
  const revision = globalRevision + localRevision;
  const [ready, setReady] = useState<{ source: string; src: string } | null>(
    null,
  );
  const [loaded, setLoaded] = useState(false);
  const [cacheMiss, setCacheMiss] = useState("");
  const [zoomed, setZoomed] = useState(false);
  const marker = useRef<HTMLElement | null>(null);
  const lightboxRef = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    if (!zoomed) return;
    const dialog = lightboxRef.current;
    if (!dialog) return;
    dialog.showModal();
    return () => {
      if (dialog.open) dialog.close();
    };
  }, [zoomed]);

  const cleanUrl = url?.split("?")[0] ?? "";
  const local = cleanUrl.startsWith("/api/v1/artwork/");
  const source = local ? `${cleanUrl}?image_revision=${revision}` : "";

  useEffect(() => {
    setLoaded(false);
    const src = ready?.src;
    return () => {
      if (src?.startsWith("blob:")) URL.revokeObjectURL(src);
    };
  }, [ready?.src]);

  useEffect(() => {
    if (!local || !source || !marker.current) return;
    let pending = false;
    let current = true;
    const observer = new IntersectionObserver(
      (entries) => {
        if (!entries.some((entry) => entry.isIntersecting)) return;
        if (cacheMiss !== source) {
          observer.disconnect();
          setReady({ source, src: `${source}&cached=true` });
        } else if (!pending) {
          pending = true;
          observer.disconnect();
          void queueArtwork(source, !!marker.current?.closest(".music-heading"))
            .then((blob) => {
              if (current) setReady({ source, src: URL.createObjectURL(blob) });
            })
            .catch(() => {
              if (current) setFailed(true);
            });
        }
      },
      { rootMargin: cacheMiss === source ? "0px" : "240px" },
    );
    observer.observe(marker.current);
    return () => {
      current = false;
      observer.disconnect();
    };
  }, [local, source, cacheMiss, cleanUrl]);

  useEffect(() => setFailed(false), [cleanUrl, globalRevision]);

  useEffect(() => {
    if (!failed || !local) return;
    const due = Date.now() + 60_000;
    const retry = () => {
      if (!document.hidden && Date.now() >= due) {
        setFailed(false);
        setLocalRevision((v) => v + 1);
      }
    };
    const timer = setTimeout(retry, 60_000);
    document.addEventListener("visibilitychange", retry);
    window.addEventListener("online", retry);
    return () => {
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", retry);
      window.removeEventListener("online", retry);
    };
  }, [failed, local]);

  return local && !failed && ready?.source === source ? (
    <span className={`cover image-frame ${large ? "large" : ""}`}>
      <img
        ref={(node) => {
          marker.current = node;
          if (node?.complete && node.naturalWidth > 0 && !loaded) {
            setLoaded(true);
          }
        }}
        className={`cover-image ${large ? "zoomable" : ""} ${loaded ? "is-loaded" : ""}`}
        src={ready.src}
        alt=""
        loading="eager"
        referrerPolicy="no-referrer"
        tabIndex={large ? 0 : undefined}
        role={large ? "button" : undefined}
        aria-label={large ? "View enlarged artwork" : undefined}
        onClick={large ? () => setZoomed(true) : undefined}
        onKeyDown={
          large
            ? (e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  setZoomed(true);
                }
              }
            : undefined
        }
        onLoad={() => setLoaded(true)}
        onError={() => {
          if (ready.src.endsWith("&cached=true")) {
            setReady(null);
            setCacheMiss(source);
            return;
          }
          setFailed(true);
        }}
      />
      {children}
      {large && zoomed && (
        <dialog
          ref={lightboxRef}
          className="artwork-lightbox"
          aria-label="Enlarged artwork"
          onClick={(event) => {
            if (event.target === lightboxRef.current) setZoomed(false);
          }}
          onCancel={(e) => {
            e.preventDefault();
            setZoomed(false);
          }}
        >
          <div className="artwork-lightbox-container">
            <button
              type="button"
              className="btn-icon icon-button artwork-lightbox-close"
              aria-label="Close enlarged artwork"
              onClick={() => setZoomed(false)}
            >
              <Icon>close</Icon>
            </button>
            <img src={ready.src} alt="" className="artwork-lightbox-image" />
          </div>
        </dialog>
      )}
    </span>
  ) : (
    <span
      ref={(node) => {
        marker.current = node;
      }}
      className={`cover placeholder ${large ? "large" : ""}`}
    >
      <Icon>music_note</Icon>
      {children}
    </span>
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

const dateFormatters = new Map<string, Intl.DateTimeFormat>();
export function date(ms: number | null, preferences: Preferences) {
  if (ms === null) return "Never";
  const hourCycle = preferences.hour_format === "12" ? "h12" : "h23";
  const key = `${preferences.timezone}:${hourCycle}`;
  let formatter = dateFormatters.get(key);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat("en-GB", {
      timeZone: preferences.timezone,
      day: "2-digit",
      month: "short",
      year: "numeric",
      hour: "2-digit",
      minute: "2-digit",
      hourCycle,
    });
    dateFormatters.set(key, formatter);
  }
  return formatter.format(ms);
}

const SOURCE_LABELS: Record<string, string> = {
  spotify_api: "Spotify collection",
  spotify_import: "Spotify import",
  listenbrainz: "ListenBrainz",
};

export function formatSources(sources: string[]): string {
  return [...new Set(sources)]
    .map((source) => SOURCE_LABELS[source] || source)
    .join(" + ");
}

/**
 * Signature typographic header (`Name.`): every page and entity title ends
 * immediately with the warm/accent period `<span className="accent">.</span>`.
 */
export function Heading({
  title,
  children,
  actions,
}: {
  title: string;
  children?: ReactNode;
  actions?: ReactNode;
}) {
  const cleanTitle = title.endsWith(".") ? title.slice(0, -1) : title;
  return (
    <div className="page-heading">
      <div className="page-heading-copy">
        <h1>
          {cleanTitle}
          <span className="accent">.</span>
        </h1>
        {children && <p>{children}</p>}
      </div>
      {actions && <div className="page-heading-actions">{actions}</div>}
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

const ToastContext = createContext<(message: string, error?: boolean) => void>(
  () => {},
);
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
              type="button"
              className="btn-icon icon-button"
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
  className = "",
}: {
  title: string;
  children: ReactNode;
  close: () => void;
  className?: string;
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
      className={className || undefined}
      onClick={(e) => {
        if (e.target === ref.current) close();
      }}
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
      aria-labelledby="dialog-title"
    >
      <h2 id="dialog-title">
        {title}
        {!title.endsWith(".") && <span className="accent">.</span>}
      </h2>
      {children}
    </dialog>
  );
}

export interface SegmentedOption<T extends string = string> {
  value: T;
  label: ReactNode;
  icon?: string;
  className?: string;
  ariaLabel?: string;
}

export interface SegmentedControlProps<T extends string = string> extends Omit<
  React.HTMLAttributes<HTMLDivElement>,
  "onChange"
> {
  value: T;
  onChange: (value: T) => void;
  options: SegmentedOption<T>[];
  ariaLabel: string;
}

export function SegmentedControl<T extends string = string>({
  value,
  onChange,
  options,
  ariaLabel,
  className = "",
  ...rest
}: SegmentedControlProps<T>) {
  return (
    <div
      role="group"
      aria-label={ariaLabel}
      className={`segmented-control ${className}`.trim()}
      {...rest}
    >
      {options.map((option) => {
        const active = option.value === value;
        return (
          <button
            key={option.value}
            type="button"
            className={`segmented-option ${option.className || ""}`.trim()}
            aria-pressed={active}
            aria-label={option.ariaLabel}
            onClick={() => onChange(option.value)}
          >
            {option.icon && <Icon>{option.icon}</Icon>}
            {option.label}
          </button>
        );
      })}
    </div>
  );
}

export type Style = CSSProperties & Record<`--${string}`, string | number>;
