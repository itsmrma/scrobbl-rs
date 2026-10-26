import { useEffect, useRef, useState } from "react";
// Short-lived, bounded memory for revisiting consultation pages in this session.
// Every mount still refreshes; writes and session changes discard saved reads.
const remembered = new Map<string, { data: unknown; at: number }>();
let cacheGeneration = 0;
function clearRemembered() {
  cacheGeneration++;
  remembered.clear();
}
window.addEventListener("read-data-changed", clearRemembered);
window.addEventListener("session-expired", clearRemembered);
window.addEventListener("read-cache-cleared", clearRemembered);
function previous<T>(key: string): T | null {
  const entry = remembered.get(key);
  if (!entry || Date.now() - entry.at > 60_000) return null;
  return entry.data as T;
}
export function useLoad<T>(
  load: () => Promise<T>,
  key: string,
  enabled = true,
  interval?: number,
  remember = false,
) {
  const [data, setData] = useState<T | null>(() =>
      remember ? previous<T>(key) : null,
    ),
    [error, setError] = useState("");
  const dataKey = useRef<string | null>(key);
  const [loading, setLoading] = useState(true);
  useEffect(() => {
    if (!enabled) return;
    setLoading(true);
    if (dataKey.current !== key) {
      setData(remember ? previous<T>(key) : null);
      dataKey.current = key;
    }
    setError("");
    let active = true,
      busy = false,
      refreshAgain = false,
      revision = 0;
    const refresh = async () => {
      if (busy) return;
      busy = true;
      const startedAtRevision = revision;
      const startedAtGeneration = cacheGeneration;
      try {
        const result = await load();
        if (active && startedAtRevision === revision) {
          if (remember && startedAtGeneration === cacheGeneration) {
            remembered.delete(key);
            remembered.set(key, { data: result, at: Date.now() });
            if (remembered.size > 64)
              remembered.delete(remembered.keys().next().value!);
          }
          setData(result);
          setError("");
        }
      } catch (e) {
        if (active) setError(String(e instanceof Error ? e.message : e));
      } finally {
        busy = false;
        if (active) setLoading(false);
        if (active && refreshAgain) {
          refreshAgain = false;
          void refresh();
        }
      }
    };
    void refresh();
    const wake = () => {
      if (!document.hidden) void refresh();
    };
    const invalidate = () => {
      revision++;
      if (busy) refreshAgain = true;
      else void refresh();
    };
    document.addEventListener("visibilitychange", wake);
    if (remember) window.addEventListener("read-data-changed", invalidate);
    const timer = interval
      ? setInterval(() => {
          if (!document.hidden) void refresh();
        }, interval)
      : undefined;
    return () => {
      active = false;
      clearInterval(timer);
      document.removeEventListener("visibilitychange", wake);
      window.removeEventListener("read-data-changed", invalidate);
    };
    // The serialized key defines the request. Callers avoid request-function identity as a dependency.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, enabled, interval, remember]);
  return {
    data: dataKey.current === key ? data : remember ? previous<T>(key) : null,
    error,
    loading: enabled && (dataKey.current !== key || loading),
  };
}
