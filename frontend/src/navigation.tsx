import { useEffect, useState } from "react";
import type { CSSProperties, MouseEvent, ReactNode } from "react";
import { Icon } from "./ui";

type NavigationState = {
  index: number;
  back?: { index: number; url: string };
};
function navigationState(): NavigationState {
  return history.state?.scrobblNavigation || { index: 0 };
}
// Keep the entry index through reloads so page-back can skip filter changes.
if (!history.state?.scrobblNavigation)
  history.replaceState(
    { ...history.state, scrobblNavigation: { index: 0 } },
    "",
  );

const selections = new Map<string, string>();
const isConsultation = (path: string) =>
  path === "/" ||
  path === "/dashboard" ||
  path === "/history" ||
  path.startsWith("/rankings/") ||
  path.startsWith("/music/");
function rememberSelection() {
  if (isConsultation(location.pathname))
    selections.set(
      location.pathname === "/" ? "/dashboard" : location.pathname,
      (location.pathname === "/" ? "/dashboard" : location.pathname) +
        location.search,
    );
}
function destination(path: string) {
  return !path.includes("?") && isConsultation(path)
    ? selections.get(path) || path
    : path;
}
rememberSelection();
window.addEventListener("popstate", rememberSelection);
export function navigate(path: string) {
  rememberSelection();
  const target = destination(path);
  const current = navigationState();
  const back =
    new URL(target, location.href).pathname === location.pathname
      ? current.back
      : { index: current.index, url: location.pathname + location.search };
  history.pushState(
    { scrobblNavigation: { index: current.index + 1, back } },
    "",
    target,
  );
  window.dispatchEvent(new PopStateEvent("popstate"));
}
export function returnTo(path: string) {
  const current = navigationState();
  if (
    current.back &&
    new URL(current.back.url, location.href).pathname ===
      new URL(path, location.href).pathname
  ) {
    history.go(current.back.index - current.index);
  } else {
    history.replaceState({ scrobblNavigation: current }, "", destination(path));
    window.dispatchEvent(new PopStateEvent("popstate"));
  }
}
export function BackButton({ fallback }: { fallback: string }) {
  const current = navigationState();
  return (
    <button
      className="button secondary music-back"
      onClick={() => {
        if (current.back && current.back.index < current.index)
          history.go(current.back.index - current.index);
        else navigate(fallback);
      }}
    >
      <Icon>arrow_back</Icon>Back
    </button>
  );
}
export function Link({
  to,
  children,
  back = false,
  ...props
}: {
  to: string;
  back?: boolean;
  children: ReactNode;
  className?: string;
  style?: CSSProperties;
  "aria-label"?: string;
  "aria-describedby"?: string;
  "aria-current"?: "page";
  onClick?: () => void;
  onMouseEnter?: () => void;
  onFocus?: () => void;
}) {
  return (
    <a
      href={destination(to)}
      {...props}
      onClick={(event: MouseEvent<HTMLAnchorElement>) => {
        props.onClick?.();
        if (
          event.button ||
          event.ctrlKey ||
          event.metaKey ||
          event.shiftKey ||
          event.altKey
        )
          return;
        event.preventDefault();
        if (back) returnTo(to);
        else navigate(to);
      }}
    >
      {children}
    </a>
  );
}
export function useLocation() {
  const [path, setPath] = useState(location.pathname);
  useEffect(() => {
    const legacy = location.hash.slice(1);
    if (["dashboard", "history", "import", "settings"].includes(legacy)) {
      history.replaceState(
        history.state,
        "",
        legacy === "settings" ? "/settings/connections" : `/${legacy}`,
      );
      setPath(location.pathname);
    }
    const update = () => setPath(location.pathname);
    window.addEventListener("popstate", update);
    return () => window.removeEventListener("popstate", update);
  }, []);
  return path;
}

export function useSearch() {
  const [search, setSearch] = useState(location.search);
  useEffect(() => {
    const update = () => setSearch(location.search);
    window.addEventListener("popstate", update);
    return () => window.removeEventListener("popstate", update);
  }, []);
  return search;
}
