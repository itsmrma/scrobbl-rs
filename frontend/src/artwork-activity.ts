import { useSyncExternalStore } from "react";

export type ArtworkActivity = {
  total: number;
  loaded: number;
  failed: number;
  pending: number;
};
let snapshot: ArtworkActivity = { total: 0, loaded: 0, failed: 0, pending: 0 };
const listeners = new Set<() => void>();
const queue: (() => void)[] = [];
let busy = false;
function publish(next: ArtworkActivity) {
  snapshot = next;
  listeners.forEach((listener) => listener());
}
function pump() {
  if (!busy) queue.shift()?.();
}
export function queueArtwork(
  start: (done: (loaded: boolean | null) => void) => void,
  priority = false,
) {
  let started = false;
  let finished = false;
  publish(
    snapshot.pending
      ? {
          ...snapshot,
          total: snapshot.total + 1,
          pending: snapshot.pending + 1,
        }
      : { total: 1, loaded: 0, failed: 0, pending: 1 },
  );
  const done = (loaded: boolean | null) => {
    if (finished) return;
    finished = true;
    publish({
      total: snapshot.total - (loaded === null ? 1 : 0),
      pending: snapshot.pending - 1,
      loaded: snapshot.loaded + (loaded === true ? 1 : 0),
      failed: snapshot.failed + (loaded === false ? 1 : 0),
    });
    if (started) {
      busy = false;
      pump();
    }
  };
  const run = () => {
    if (finished) {
      pump();
      return;
    }
    started = true;
    busy = true;
    start(done);
  };
  if (priority) queue.unshift(run);
  else queue.push(run);
  pump();
  // A started download saves to the server even after its card unmounts.
  return () => {
    if (!started) done(null);
  };
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
