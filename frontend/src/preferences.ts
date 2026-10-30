import { useEffect, useState } from "react";
export const defaultAccent = "#c9ee8b";
export const WEEKDAYS = [
  "Monday",
  "Tuesday",
  "Wednesday",
  "Thursday",
  "Friday",
  "Saturday",
  "Sunday",
] as const;
export const periods = [
  ["all", "All time"],
  ["today", "Today"],
  ["week", "This week"],
  ["month", "This month"],
  ["year", "This year"],
  ["7d", "Last 7 days"],
  ["30d", "Last 30 days"],
  ["365d", "Last 365 days"],
];
export function periodBadgeLabel(period: string): string {
  const found = periods.find(([p]) => p === period);
  return found ? found[1] : period;
}
export function stored<T>(
  key: string,
  fallback: T,
  validate: (value: unknown) => value is T,
): T {
  try {
    if (key === "theme" && localStorage.getItem("scrobbl:theme") === null) {
      const legacy: unknown = localStorage.getItem("scrobbl-theme");
      if (validate(legacy)) return legacy;
    }
    const value: unknown = JSON.parse(
      localStorage.getItem(`scrobbl:${key}`) || "null",
    );
    return validate(value) ? value : fallback;
  } catch {
    return fallback;
  }
}
export function useBrowserPreference<T>(
  key: string,
  fallback: T,
  validate: (value: unknown) => value is T,
) {
  const [value, setValue] = useState(() => stored(key, fallback, validate));
  useEffect(() => {
    try {
      localStorage.setItem(`scrobbl:${key}`, JSON.stringify(value));
    } catch {
      /* A private/storage-blocked browser still works for this visit. */
    }
  }, [key, value]);
  return [value, setValue] as const;
}
export const isTheme = (v: unknown): v is string =>
  ["auto", "light", "dark"].includes(String(v));
export const isHex = (v: unknown): v is string =>
  typeof v === "string" && /^#[0-9a-f]{6}$/i.test(v);
export const isBool = (v: unknown): v is boolean => typeof v === "boolean";
export const isWidth = (v: unknown): v is number =>
  typeof v === "number" && v >= 216 && v <= 360;
export function ink(hex: string) {
  const values = [1, 3, 5].map((i) => {
    const s = parseInt(hex.slice(i, i + 2), 16) / 255;
    return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  });
  return values[0] * 0.2126 + values[1] * 0.7152 + values[2] * 0.0722 > 0.179
    ? "#000000"
    : "#ffffff";
}
export function useAppearance() {
  const [theme, setTheme] = useBrowserPreference("theme", "auto", isTheme);
  const [accent, setAccent] = useBrowserPreference(
    "accent",
    defaultAccent,
    isHex,
  );
  useEffect(() => {
    const media = matchMedia("(prefers-color-scheme: light)");
    const update = () =>
      document.documentElement.classList.toggle(
        "theme-light",
        theme === "light" || (theme === "auto" && media.matches),
      );
    update();
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, [theme]);
  useEffect(() => {
    document.documentElement.style.setProperty("--accent", accent);
    document.documentElement.style.setProperty("--accent-ink", ink(accent));
  }, [accent]);
  return { theme, setTheme, accent, setAccent };
}
