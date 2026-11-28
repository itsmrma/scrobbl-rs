import { useEffect, useState } from "react";
import { navigate, useSearch } from "./navigation";
import { stored } from "./preferences";
import {
  applySharedPeriod,
  rememberPeriod,
  setSharedPeriod,
  sharedPeriod,
} from "./shared-period";
export type Kind = "artists" | "albums" | "tracks";
export const kinds: Kind[] = ["artists", "albums", "tracks"];
const PAGE_KEYS = [
  "page",
  "history_page",
  "artists_page",
  "albums_page",
  "tracks_page",
] as const;
const isSize = (v: unknown): v is number =>
  typeof v === "number" && [10, 25, 50, 100].includes(v);
const isView = (v: unknown): v is string => v === "list" || v === "grid";
export function useListPreferences(defaultView = "list", scope = "") {
  const viewKey = scope ? `ranking-view-${scope}` : "ranking-view";
  const [size, setSize] = useState(() => stored("page-size", 50, isSize));
  const [view, setView] = useState(() => stored(viewKey, defaultView, isView));
  useEffect(() => {
    setView(stored(viewKey, defaultView, isView));
    const update = (event: Event) => {
      if (event instanceof CustomEvent) {
        if (event.detail.key === "page-size") setSize(event.detail.value);
        else if (event.detail.key === viewKey) setView(event.detail.value);
        return;
      }
      setSize(stored("page-size", 50, isSize));
      setView(stored(viewKey, defaultView, isView));
    };
    window.addEventListener("consultation-preference", update);
    window.addEventListener("storage", update);
    return () => {
      window.removeEventListener("consultation-preference", update);
      window.removeEventListener("storage", update);
    };
  }, [viewKey, defaultView]);
  function save(key: string, value: string | number) {
    try {
      localStorage.setItem(`scrobbl:${key}`, JSON.stringify(value));
    } catch {
      /* Works for this visit. */
    }
    if (key === "page-size") setSize(Number(value));
    else setView(String(value));
    window.dispatchEvent(
      new CustomEvent("consultation-preference", { detail: { key, value } }),
    );
  }
  return {
    size,
    view,
    setSize: (n: number) => save("page-size", n),
    setView: (v: string) => save(viewKey, v),
  };
}
export function useSelection(visible: boolean, initialPeriod = "all") {
  const search = useSearch();
  const [saved, setSaved] = useState(search);
  const [, setPeriodTick] = useState(0);
  useEffect(() => {
    const onPeriodChange = () => setPeriodTick((v) => v + 1);
    window.addEventListener("selected-period-changed", onPeriodChange);
    return () =>
      window.removeEventListener("selected-period-changed", onPeriodChange);
  }, []);
  useEffect(() => {
    if (visible) setSaved(search);
  }, [search, visible]);
  useEffect(() => {
    if (visible) rememberPeriod(new URLSearchParams(search));
  }, [search, visible]);
  const params = new URLSearchParams(visible ? search : saved);
  // A screen opened without a period shows the persisted shared period.
  if (!params.has("period")) {
    if (sharedPeriod()) applySharedPeriod(params, false);
    else params.set("period", initialPeriod);
  }
  const update = (patch: Record<string, string>, resetPage = true) => {
    const next = new URLSearchParams(visible ? search : saved);
    if (resetPage) for (const key of PAGE_KEYS) next.delete(key);
    const touchesPeriod = "period" in patch || "from" in patch || "to" in patch;
    if (touchesPeriod) {
      const nextPeriod =
        patch.period ??
        next.get("period") ??
        params.get("period") ??
        initialPeriod;
      const nextFrom =
        "from" in patch ? patch.from : next.get("from") || params.get("from") || "";
      const nextTo =
        "to" in patch ? patch.to : next.get("to") || params.get("to") || "";
      setSharedPeriod(nextPeriod, nextFrom, nextTo, true);
      for (const [key, value] of Object.entries(patch)) {
        if (key === "period" || key === "from" || key === "to") continue;
        if (!value || ((PAGE_KEYS as readonly string[]).includes(key) && value === "1")) {
          next.delete(key);
        } else {
          next.set(key, value);
        }
      }
      next.delete("period");
      next.delete("from");
      next.delete("to");
      if (!("bucket" in patch)) next.delete("bucket");
      if (!("bucket_start" in patch)) next.delete("bucket_start");
    } else {
      for (const [key, value] of Object.entries(patch)) {
        if (!value || ((PAGE_KEYS as readonly string[]).includes(key) && value === "1")) {
          next.delete(key);
        } else {
          next.set(key, value);
        }
      }
    }
    const qs = next.toString();
    navigate(qs ? `${location.pathname}?${qs}` : location.pathname, false);
  };
  return { params, update };
}
export function context(params: URLSearchParams) {
  const result = new URLSearchParams(params);
  for (const key of [
    "page",
    "history_page",
    "artists_page",
    "albums_page",
    "tracks_page",
    "page_size",
    "q",
    "status",
    "bucket",
    "bucket_start",
  ])
    result.delete(key);
  return result;
}
export function musicLink(
  kind: Kind,
  id: string,
  params = new URLSearchParams(location.search),
) {
  const next = context(params);
  for (const key of ["artist", "album", "track"]) next.delete(key);
  const qs = next.toString();
  return qs
    ? `/music/${kind}/${encodeURIComponent(id)}?${qs}`
    : `/music/${kind}/${encodeURIComponent(id)}`;
}
export function scoped(params: URLSearchParams, kind: Kind, id: string) {
  const next = new URLSearchParams(params);
  next.set(
    kind === "artists" ? "artist" : kind === "albums" ? "album" : "track",
    id,
  );
  return next;
}
export function hasIncompleteCustomPeriod(params: URLSearchParams) {
  return (
    params.get("period") === "custom" &&
    !params.get("from") &&
    !params.get("to")
  );
}
export function entityScopeKey(params: URLSearchParams): string {
  return `${params.get("artist") || ""}:${params.get("album") || ""}:${params.get("track") || ""}`;
}
