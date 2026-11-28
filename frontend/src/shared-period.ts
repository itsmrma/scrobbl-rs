import { api } from "./api";

// The selected period follows the owner across every consultation screen and
// across reloads/devices via server preferences.
const KEY = "scrobbl:selected-period";
const PERIOD_KEYS = ["period", "from", "to"] as const;
const PAGE_KEYS = [
  "page",
  "history_page",
  "artists_page",
  "albums_page",
  "tracks_page",
  "bucket",
  "bucket_start",
];

type SharedPeriod = { period: string; from: string; to: string };

let memory: SharedPeriod | null = null;
let lastPersisted = "";

const isPeriod = (v: unknown): v is string =>
  typeof v === "string" && /^[a-z0-9]{1,16}$/.test(v);
const isDate = (v: unknown): v is string =>
  typeof v === "string" && (v === "" || /^\d{4}-\d{2}-\d{2}$/.test(v));

function writeStored(next: SharedPeriod) {
  memory = next;
  try {
    localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    /* Works for this page. */
  }
}

export function sharedPeriod(): SharedPeriod | null {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(KEY) || "null");
    if (
      value &&
      typeof value === "object" &&
      "period" in value &&
      "from" in value &&
      "to" in value &&
      isPeriod(value.period) &&
      isDate(value.from) &&
      isDate(value.to)
    )
      return { period: value.period, from: value.from, to: value.to };
  } catch {
    /* Storage-blocked browsers keep the period in memory for this page. */
  }
  return memory;
}

/** Seed the shared period from server preferences on boot or refresh. */
export function hydrateSharedPeriod(
  period: string,
  from?: string | null,
  to?: string | null,
) {
  if (!isPeriod(period)) return;
  const next: SharedPeriod = {
    period,
    from: period === "custom" ? from || "" : "",
    to: period === "custom" ? to || "" : "",
  };
  if (!isDate(next.from) || !isDate(next.to)) return;
  lastPersisted = `${next.period}:${next.from}:${next.to}`;
  const current = sharedPeriod();
  if (
    current &&
    current.period === next.period &&
    current.from === next.from &&
    current.to === next.to
  )
    return;
  writeStored(next);
  window.dispatchEvent(new CustomEvent("selected-period-changed"));
}

/** Update the shared period and optionally persist it on the server. */
export function setSharedPeriod(
  period: string,
  from = "",
  to = "",
  persist = true,
) {
  if (!isPeriod(period)) return;
  const next: SharedPeriod = {
    period,
    from: period === "custom" ? from : "",
    to: period === "custom" ? to : "",
  };
  if (!isDate(next.from) || !isDate(next.to)) return;
  const current = sharedPeriod();
  const changed =
    !current ||
    current.period !== next.period ||
    current.from !== next.from ||
    current.to !== next.to;
  if (changed) {
    writeStored(next);
    window.dispatchEvent(new CustomEvent("selected-period-changed"));
  }
  const complete = next.period !== "custom" || Boolean(next.from || next.to);
  const sig = `${next.period}:${next.from}:${next.to}`;
  if (persist && complete && sig !== lastPersisted) {
    lastPersisted = sig;
    void api
      .saveSelectedPeriod(next.period, next.from, next.to)
      .then((prefs) => {
        window.dispatchEvent(
          new CustomEvent("preferences-updated", { detail: prefs }),
        );
      })
      .catch(() => {});
  }
}

/** Remember the period carried by explicit URL consultation parameters, if any. */
export function rememberPeriod(params: URLSearchParams) {
  const period = params.get("period");
  if (!isPeriod(period)) return;
  const next = {
    period,
    from: period === "custom" ? params.get("from") || "" : "",
    to: period === "custom" ? params.get("to") || "" : "",
  };
  if (!isDate(next.from) || !isDate(next.to)) return;
  setSharedPeriod(next.period, next.from, next.to, false);
}

/**
 * Apply the shared period to `params`. Returns true when the period changed,
 * in which case paging and chart-bucket selections of the old period are
 * dropped because they no longer belong to the visible range.
 */
export function applySharedPeriod(
  params: URLSearchParams,
  resetPaging = true,
): boolean {
  const shared = sharedPeriod();
  if (!shared) return false;
  const changed =
    params.get("period") !== shared.period ||
    (params.get("from") || "") !== shared.from ||
    (params.get("to") || "") !== shared.to;
  if (!changed) return false;
  for (const key of PERIOD_KEYS) {
    const value = shared[key];
    if (value) params.set(key, value);
    else params.delete(key);
  }
  if (resetPaging) for (const key of PAGE_KEYS) params.delete(key);
  return true;
}
