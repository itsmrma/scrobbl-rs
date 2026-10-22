import { useEffect, useRef, useState } from "react";
export function useLoad<T>(
  load: () => Promise<T>,
  key: string,
  enabled = true,
  interval?: number,
) {
  const [data, setData] = useState<T | null>(null),
    [error, setError] = useState("");
  const dataKey = useRef<string | null>(null);
  useEffect(() => {
    if (!enabled) return;
    if (dataKey.current !== key) {
      setData(null);
      dataKey.current = key;
    }
    setError("");
    let active = true,
      busy = false;
    const refresh = async () => {
      if (busy) return;
      busy = true;
      try {
        const result = await load();
        if (active) {
          setData(result);
          setError("");
        }
      } catch (e) {
        if (active) setError(String(e instanceof Error ? e.message : e));
      } finally {
        busy = false;
      }
    };
    void refresh();
    const wake = () => {
      if (!document.hidden) void refresh();
    };
    document.addEventListener("visibilitychange", wake);
    const timer = interval
      ? setInterval(() => {
          if (!document.hidden) void refresh();
        }, interval)
      : undefined;
    return () => {
      active = false;
      clearInterval(timer);
      document.removeEventListener("visibilitychange", wake);
    };
    // The serialized key defines the request. Callers avoid request-function identity as a dependency.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, enabled, interval]);
  return { data, error };
}
