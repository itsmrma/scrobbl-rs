import { useSyncExternalStore } from "react";
import { downloadArtwork } from "./api";

export type ArtworkActivity = {
  total: number;
  loaded: number;
  failed: number;
  pending: number;
};
let snapshot: ArtworkActivity = { total: 0, loaded: 0, failed: 0, pending: 0 };
const listeners = new Set<() => void>();
const queue: (() => Promise<void>)[] = [];
const requests = new Map<string, Promise<Blob>>();
let busy = false;
function publish(next: ArtworkActivity) {
  snapshot = next;
  listeners.forEach((listener) => listener());
}
async function pump() {
  if (busy) return;
  busy = true;
  try {
    while (queue.length) await queue.shift()!();
  } finally {
    busy = false;
  }
}
// A visible image hands the download to the session. Unmounting a card,
// scrolling away or opening Activities must not discard queued work.
export function queueArtwork(url: string, priority = false): Promise<Blob> {
  const key = url.split("?")[0];
  const existing = requests.get(key);
  if (existing) return existing;
  let resolve!: (blob: Blob) => void;
  let reject!: (error: unknown) => void;
  const result = new Promise<Blob>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  requests.set(key, result);
  publish(
    snapshot.pending
      ? {
          ...snapshot,
          total: snapshot.total + 1,
          pending: snapshot.pending + 1,
        }
      : { total: 1, loaded: 0, failed: 0, pending: 1 },
  );
  const run = async () => {
    let loaded = false;
    try {
      const blob = await downloadArtwork(url);
      loaded = true;
      window.dispatchEvent(
        new CustomEvent("artwork-downloaded", { detail: { url: key } }),
      );
      resolve(blob);
    } catch (error) {
      reject(error);
    } finally {
      requests.delete(key);
      publish({
        ...snapshot,
        pending: snapshot.pending - 1,
        loaded: snapshot.loaded + (loaded ? 1 : 0),
        failed: snapshot.failed + (loaded ? 0 : 1),
      });
    }
  };
  if (priority) queue.unshift(run);
  else queue.push(run);
  void pump();
  return result;
}
export function useArtworkActivity() {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => snapshot,
  );
}
