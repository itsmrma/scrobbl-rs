import { useEffect, useState } from "react";
import type { MouseEvent, ReactNode } from "react";

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
  history.pushState(null, "", destination(path));
  window.dispatchEvent(new PopStateEvent("popstate"));
}
export function Link({
  to,
  children,
  ...props
}: {
  to: string;
  children: ReactNode;
  className?: string;
  "aria-label"?: string;
  "aria-current"?: "page";
  onClick?: () => void;
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
        navigate(to);
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
        null,
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
