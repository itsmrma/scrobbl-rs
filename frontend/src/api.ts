import type {
  Activity,
  ActivityDetail,
  Dashboard,
  History,
  ImportResponse,
  ImportLimits,
  IngestReport,
  NowPlaying,
  Preferences,
  SavedPreferences,
  SavePreferences,
  SpotifyStatus,
  SpotifyIntervals,
} from "./generated";

export class SessionExpired extends Error {}
let csrf: string | null = null;
export function setCsrf(value: string | null) {
  csrf = value;
}
async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
  const headers = new Headers(options.headers);
  if (options.body && !(options.body instanceof FormData))
    headers.set("Content-Type", "application/json");
  if (options.method === "POST" && csrf) headers.set("X-CSRF-Token", csrf);
  const response = await fetch(path, {
    ...options,
    headers,
    credentials: "same-origin",
  });
  const data = await response
    .json()
    .catch(() => ({ error: "The server returned an unreadable response." }));
  if (response.status === 401 && path !== "/api/login") {
    setCsrf(null);
    window.dispatchEvent(new Event("session-expired"));
    throw new SessionExpired("Your session expired. Sign in again.");
  }
  if (!response.ok)
    throw new Error(data.error || `Request failed (${response.status}).`);
  return data;
}
const post = <T>(path: string, body: unknown = {}, signal?: AbortSignal) =>
  request<T>(path, {
    method: "POST",
    signal,
    body: body instanceof FormData ? body : JSON.stringify(body),
  });
export type Session = {
  authenticated: boolean;
  setup_required: boolean;
  csrf: string | null;
};
export type ConnectOutcome = {
  outcome: "pending" | "processing" | "connected" | "error";
  message: string | null;
};
export const api = {
  session: () => request<Session>("/api/session"),
  login: (password: string, setup: boolean) =>
    post<{ csrf: string }>(setup ? "/api/setup" : "/api/login", { password }),
  logout: () => post("/api/logout"),
  preferences: () => request<Preferences>("/api/v1/preferences"),
  savePreferences: (form: SavePreferences) =>
    post<SavedPreferences>("/api/v1/preferences", form),
  dashboard: (params: URLSearchParams) =>
    request<Dashboard>(`/api/dashboard?${params}`),
  history: (params: URLSearchParams) =>
    request<History>(`/api/history?${params}`),
  spotify: () => request<SpotifyStatus>("/api/spotify/status"),
  saveSpotifyIntervals: (intervals: SpotifyIntervals) =>
    post<SpotifyIntervals>("/api/spotify/intervals", intervals),
  nowPlaying: () => request<NowPlaying | null>("/api/spotify/now-playing"),
  credentials: (client_id: string, client_secret: string) =>
    post("/api/spotify/credentials", { client_id, client_secret }),
  connect: () =>
    post<{ url: string; attempt: string }>(
      "/api/spotify/connect",
      {},
      AbortSignal.timeout(15000),
    ),
  connectStatus: (attempt: string) =>
    request<ConnectOutcome>(
      `/api/spotify/connect/${encodeURIComponent(attempt)}`,
      { signal: AbortSignal.timeout(15000) },
    ),
  cancelConnect: (attempt: string) =>
    post(
      `/api/spotify/connect/${encodeURIComponent(attempt)}`,
      {},
      AbortSignal.timeout(15000),
    ),
  sync: () => post<IngestReport>("/api/spotify/sync"),
  disconnect: () => post("/api/spotify/disconnect"),
  importLimits: () => request<ImportLimits>("/api/v1/import/limits"),
  importSpotify: (form: FormData) =>
    post<ImportResponse>("/api/import/spotify", form),
  activities: (page = 0, kind = "") =>
    request<Activity[]>(
      `/api/v1/activities${page || kind ? `?page=${page}&kind=${encodeURIComponent(kind)}` : ""}`,
    ),
  activity: (id: string, issuesPage = 0) =>
    request<ActivityDetail>(
      `/api/v1/activities/${encodeURIComponent(id)}${issuesPage ? `?issues_page=${issuesPage}` : ""}`,
    ),
  activityAction: (id: string, action: string) =>
    post(`/api/v1/activities/${encodeURIComponent(id)}`, { action }),
};
