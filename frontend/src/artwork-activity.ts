import { useSyncExternalStore } from "react";
import type { Activity } from "./generated";
import { api, ArtworkRetry, downloadArtwork } from "./api";

export type ServerArtworkSync = {
  id: string;
  processed: number;
  expected: number | null;
  state: string;
  paused: boolean;
};

export type ArtworkActivity = {
  total: number;
  loaded: number;
  failed: number;
  pending: number;
  deferred: number;
  serverSync: ServerArtworkSync | null;
};

let snapshot: ArtworkActivity = {
  total: 0,
  loaded: 0,
  failed: 0,
  pending: 0,
  deferred: 0,
  serverSync: null,
};

const listeners = new Set<() => void>();
const priorityQueue: (() => Promise<void>)[] = [];
const browseQueue: (() => Promise<void>)[] = [];
const requests = new Map<string, Promise<Blob>>();
let busy = false;

function publish(next: ArtworkActivity) {
  snapshot = next;
  listeners.forEach((listener) => listener());
}

function isBrowseSuppressed(): boolean {
  return !!snapshot.serverSync && !snapshot.serverSync.paused;
}

/**
 * Coordinates client on-the-fly browse artwork downloads (#155) with active
 * server-side full-library artwork jobs. While an unpaused server `artwork`
 * job is running/queued, non-priority browse fetches defer so they do not
 * compete for the single provider rate-limit slot.
 */
export function syncServerArtworkJobs(jobs: Activity[]) {
  const activeJob = jobs.find(
    (j) =>
      j.kind === "artwork" &&
      ["queued", "running", "waiting"].includes(j.state),
  );
  const nextSync: ServerArtworkSync | null = activeJob
    ? {
        id: activeJob.id,
        processed: activeJob.processed,
        expected: activeJob.expected_rows,
        state: activeJob.state,
        paused: activeJob.error_code === "artwork_paused",
      }
    : null;

  const prevSuppressed = isBrowseSuppressed();
  const changed =
    JSON.stringify(snapshot.serverSync) !== JSON.stringify(nextSync);
  if (changed) {
    publish({
      ...snapshot,
      serverSync: nextSync,
      deferred: nextSync && !nextSync.paused ? browseQueue.length : 0,
    });
  }
  if (prevSuppressed && !isBrowseSuppressed() && browseQueue.length > 0) {
    void pump();
  }
}

async function pump() {
  if (busy) return;
  busy = true;
  try {
    while (
      priorityQueue.length > 0 ||
      (!isBrowseSuppressed() && browseQueue.length > 0)
    ) {
      const nextTask =
        priorityQueue.shift() ??
        (!isBrowseSuppressed() ? browseQueue.shift() : undefined);
      if (!nextTask) break;
      if (
        snapshot.deferred !== (isBrowseSuppressed() ? browseQueue.length : 0)
      ) {
        publish({
          ...snapshot,
          deferred: isBrowseSuppressed() ? browseQueue.length : 0,
        });
      }
      await nextTask();
    }
  } finally {
    busy = false;
  }
}

const MAX_RETRIES = 60;

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

  let attempts = 0;
  const run = async () => {
    let loaded = false;
    let retry = false;
    try {
      const blob = await downloadArtwork(url);
      loaded = true;
      window.dispatchEvent(
        new CustomEvent("artwork-downloaded", { detail: { url: key } }),
      );
      resolve(blob);
    } catch (error) {
      if (error instanceof ArtworkRetry && attempts++ < MAX_RETRIES) {
        retry = true;
        const delay =
          typeof error.delayMs === "number" && error.delayMs > 0
            ? error.delayMs
            : 1000;
        setTimeout(() => {
          if (priority) priorityQueue.push(run);
          else browseQueue.push(run);
          void pump();
        }, delay);
      } else {
        reject(error);
      }
    } finally {
      if (!retry) {
        requests.delete(key);
        publish({
          ...snapshot,
          pending: Math.max(0, snapshot.pending - 1),
          deferred: isBrowseSuppressed() ? browseQueue.length : 0,
          loaded: snapshot.loaded + (loaded ? 1 : 0),
          failed: snapshot.failed + (loaded ? 0 : 1),
        });
      }
    }
  };

  if (priority) priorityQueue.unshift(run);
  else browseQueue.push(run);

  publish(
    snapshot.pending
      ? {
          ...snapshot,
          total: snapshot.total + 1,
          pending: snapshot.pending + 1,
          deferred: isBrowseSuppressed() ? browseQueue.length : 0,
        }
      : {
          ...snapshot,
          total: 1,
          loaded: 0,
          failed: 0,
          pending: 1,
          deferred: isBrowseSuppressed() ? browseQueue.length : 0,
        },
  );

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

export async function triggerArtworkBackfill(
  toast?: (message: string, error?: boolean) => void,
  onRefresh?: () => Promise<void> | void,
) {
  const res = await api.backfillArtwork();
  if (onRefresh) await onRefresh();
  window.dispatchEvent(new Event("artwork-cache-changed"));
  if (toast) {
    toast(
      res.queued > 0
        ? `Queued ${res.queued} items (${res.queued_albums} album covers, ${res.queued_artists} artist portraits) for artwork retrieval.`
        : "No missing artwork found to retrieve.",
    );
  }
  return res;
}
