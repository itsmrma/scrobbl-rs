import { useEffect, useState } from "react";
import { navigate, useSearch } from "./navigation";
import { stored } from "./preferences";
export type Kind = "artists" | "albums" | "tracks";
export const kinds: Kind[] = ["artists", "albums", "tracks"];
const isSize = (v: unknown): v is number =>
  typeof v === "number" && [10, 25, 50, 100].includes(v);
const isView = (v: unknown): v is string => v === "list" || v === "grid";
export function useListPreferences() {
  const [size, setSize] = useState(() => stored("page-size", 50, isSize));
  const [view, setView] = useState(() =>
    stored("ranking-view", "list", isView),
  );
  useEffect(() => {
    const update = (event: Event) => {
      if (event instanceof CustomEvent) {
        if (event.detail.key === "page-size") setSize(event.detail.value);
        else setView(event.detail.value);
        return;
      }
      setSize(stored("page-size", 50, isSize));
      setView(stored("ranking-view", "list", isView));
    };
    window.addEventListener("consultation-preference", update);
    window.addEventListener("storage", update);
    return () => {
      window.removeEventListener("consultation-preference", update);
      window.removeEventListener("storage", update);
    };
  }, []);
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
    setView: (v: string) => save("ranking-view", v),
  };
}
export function useSelection(visible: boolean, initialPeriod = "all") {
  const search = useSearch();
  const [saved, setSaved] = useState(search);
  useEffect(() => {
    if (visible) setSaved(search);
  }, [search, visible]);
  const params = new URLSearchParams(visible ? search : saved);
  if (!params.has("period")) params.set("period", initialPeriod);
  const update = (patch: Record<string, string>, resetPage = true) => {
    const next = new URLSearchParams(params);
    if (resetPage)
      for (const key of [
        "page",
        "history_page",
        "artists_page",
        "albums_page",
        "tracks_page",
      ])
        next.set(key, "1");
    for (const [key, value] of Object.entries(patch)) {
      if (value) next.set(key, value);
      else next.delete(key);
    }
    navigate(`${location.pathname}?${next}`);
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
  return `/music/${kind}/${encodeURIComponent(id)}?${next}`;
}
export function scoped(params: URLSearchParams, kind: Kind, id: string) {
  const next = new URLSearchParams(params);
  next.set(
    kind === "artists" ? "artist" : kind === "albums" ? "album" : "track",
    id,
  );
  return next;
}
