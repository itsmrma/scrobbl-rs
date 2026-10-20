import { useEffect, useState } from "react";
import type { MouseEvent, ReactNode } from "react";

export function navigate(path: string) {
  history.pushState(null, "", path);
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
      href={to}
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
