import type {
  Activity,
  ActivityDetail,
  Dashboard,
  RankingPage,
  MusicPage,
  ActivityChart,
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
  Asset,
  ExportSelection,
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
  rankings: (kind: string, params: URLSearchParams) =>
    request<RankingPage>(`/api/v1/consultation/rankings/${kind}?${params}`),
  music: (kind: string, id: string, params: URLSearchParams) =>
    request<MusicPage>(
      `/api/v1/consultation/music/${kind}/${encodeURIComponent(id)}?${params}`,
    ),
  chart: (params: URLSearchParams) =>
    request<ActivityChart>(`/api/v1/consultation/activity?${params}`),
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
  recovery: () =>
    request<{ assets: Asset[]; paused: boolean; restart_required: boolean }>(
      "/api/v1/recovery",
    ),
  createBackup: () => post("/api/v1/recovery", { kind: "backup" }),
  exportArchive: (format: string, selection: ExportSelection) =>
    post("/api/v1/recovery", { kind: "export", format, selection }),
  backupAction: (id: string, action: string) =>
    post(`/api/v1/recovery/${encodeURIComponent(id)}`, { action }),
  uploadBackup: (file: File) => {
    const form = new FormData();
    form.append("backup", file);
    return post("/api/v1/recovery/upload", form);
  },
  restoreBackup: (asset_id: string, password: string) =>
    post("/api/v1/recovery/restore", {
      asset_id,
      password,
      confirm_replace: true,
    }),
  resumeRecovery: () => post("/api/v1/recovery/resume"),
};
