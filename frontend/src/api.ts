import type {
  Activity,
  ActivityDetail,
  Dashboard,
  RankingPage,
  EntityChoice,
  SearchItem,
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
  ApiKeySettings,
  CreateApiKey,
  CreatedApiKey,
  Limits,
  Details,
  ArtworkSettings,
  ImageCredit,
  Edit,
  GroupPage,
  Listen,
  DashboardLayout,
  DiscoveryRatio,
  ListeningCalendar,
  ListeningMilestones,
} from "./generated";

class SessionExpired extends Error {}
let csrf: string | null = null;
export function setCsrf(value: string | null) {
  csrf = value;
  if (value === null) window.dispatchEvent(new Event("read-cache-cleared"));
}
// Explicit cancellation before document teardown avoids WebKit reporting pending
// same-origin reads as access-control errors when the old document disappears.
const pendingRequests = new Set<AbortController>();
let leavingDocument = false;
function leaveDocument() {
  leavingDocument = true;
  for (const controller of pendingRequests) controller.abort();
}
window.addEventListener("beforeunload", leaveDocument);
window.addEventListener("pagehide", leaveDocument);
window.addEventListener("pageshow", () => {
  leavingDocument = false;
});
// Artwork downloads survive SPA navigation but must be cancelled before the
// document disappears, just like the JSON reads below (not left to WebKit).
export class ArtworkRetry extends Error {
  constructor(public readonly delayMs: number) {
    super("Artwork temporarily unavailable");
  }
}
export async function downloadArtwork(path: string): Promise<Blob> {
  if (leavingDocument)
    throw new DOMException("The page is being left.", "AbortError");
  const controller = new AbortController();
  pendingRequests.add(controller);
  const timeout = setTimeout(() => controller.abort(), 45_000);
  try {
    const response = await fetch(path, {
      signal: controller.signal,
      credentials: "same-origin",
    });
    if (response.status === 503) {
      const seconds = Number(response.headers.get("Retry-After"));
      throw new ArtworkRetry(
        Number.isFinite(seconds) && seconds > 0
          ? Math.min(seconds * 1000, 2_147_483_647)
          : 60_000,
      );
    }
    if (!response.ok) throw new Error("Artwork download failed");
    return await response.blob();
  } finally {
    clearTimeout(timeout);
    pendingRequests.delete(controller);
  }
}
async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
  if (leavingDocument)
    throw new DOMException("The page is being left.", "AbortError");
  const controller = new AbortController();
  const cancel = () => controller.abort(options.signal?.reason);
  if (options.signal?.aborted) cancel();
  else options.signal?.addEventListener("abort", cancel, { once: true });
  pendingRequests.add(controller);
  try {
    const headers = new Headers(options.headers);
    if (
      options.body &&
      !(options.body instanceof FormData) &&
      !(options.body instanceof Blob)
    )
      headers.set("Content-Type", "application/json");
    if (options.method === "POST" && csrf) headers.set("X-CSRF-Token", csrf);
    const response = await fetch(path, {
      ...options,
      signal: controller.signal,
      headers,
      credentials: "same-origin",
    });
    const data = await response.json().catch((error) => {
      if (controller.signal.aborted) throw error;
      throw new Error("The server returned an unreadable response.");
    });
    if (response.status === 401 && path !== "/api/login") {
      setCsrf(null);
      window.dispatchEvent(new Event("session-expired"));
      throw new SessionExpired("Your session expired. Sign in again.");
    }
    if (!response.ok)
      throw new Error(data.error || `Request failed (${response.status}).`);
    if (options.method === "POST")
      window.dispatchEvent(new Event("read-data-changed"));
    return data;
  } finally {
    pendingRequests.delete(controller);
    options.signal?.removeEventListener("abort", cancel);
  }
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
  username_required?: boolean;
  username_setup_required?: boolean;
  username?: string;
};
export type AccountProfile = {
  username: string | null;
  name: string;
  avatar_url: string | null;
};
export type ConnectOutcome = {
  outcome: "pending" | "processing" | "connected" | "error";
  message: string | null;
};
export type ReviewPage = {
  total: number;
  page: number;
  decisions_total: number;
  groups: { id: string; items: (Listen & { revision: number })[] }[];
  recent: { id: string; at_ms: number; count: number; removed: number }[];
};
export type ListenBrainzOriginalsPage = {
  total: number;
  files: { file_id: string; name: string }[];
  rows: {
    member: string;
    row_number: number;
    outcome: string;
    reason: string | null;
    raw: unknown;
  }[];
};
export const api = {
  account: () => request<AccountProfile>("/api/v1/account"),
  saveAccount: (name: string) =>
    post<AccountProfile>("/api/v1/account", { name }),
  uploadAvatar: (image: Blob) =>
    request<AccountProfile>("/api/v1/account/avatar", {
      method: "POST",
      body: image,
    }),
  removeAvatar: () => post<AccountProfile>("/api/v1/account/avatar/remove"),
  changePassword: (current_password: string, password: string) =>
    post("/api/v1/account/password", { current_password, password }),
  application: () => request<{ boot_id: string }>("/api/v1/application"),
  controlApplication: (action: "restart" | "stop") =>
    post("/api/v1/application", { action, confirmed: true }),
  listeningPatterns: (params: URLSearchParams) =>
    request<{
      total: number;
      series: Record<string, { label: string; count: number }[]>;
      coverage: Record<
        string,
        { known: number; unknown: number; conflicting: number }
      >;
    }>(`/api/v1/consultation/patterns?${params}`),
  reviewTrash: (page = 0) =>
    request<{
      total: number;
      items: (Listen & { decision_id: string | null; trashed_at_ms: number })[];
    }>(`/api/v1/review/trash?page=${page}`),
  automaticReview: () =>
    request<{
      token: string;
      groups: number;
      records: number;
      remaining: number;
    }>("/api/v1/review/automatic"),
  executeAutomaticReview: (token: string) =>
    post<{ removed: number }>("/api/v1/review/automatic", { token }),
  review: (page = 0, decisionsPage = 0) =>
    request<ReviewPage>(
      `/api/v1/review?page=${page}&decisions_page=${decisionsPage}`,
    ),
  decideReview: (ids: string[], revisions: number[], keep_id: string | null) =>
    post<{ decision_id: string; removed: number }>("/api/v1/review", {
      ids,
      revisions,
      keep_id,
    }),
  undoReview: (id: string) =>
    post(`/api/v1/review/${encodeURIComponent(id)}/undo`),
  nameGroup: (kind: string, id: string, q: string, page: number) =>
    request<GroupPage>(
      `/api/v1/library/${kind}/${id}/group?${new URLSearchParams({ q, page: String(page) })}`,
    ),
  changeNameGroup: (
    kind: string,
    id: string,
    action: string,
    member_ids: string[],
  ) =>
    post<{ group_id: string }>(`/api/v1/library/${kind}/${id}/group`, {
      action,
      member_ids,
    }),
  listenBrainzLimits: () => request<Limits>("/api/v1/listenbrainz/limits"),
  listenBrainzOriginals: (id: string, offset: number) =>
    request<ListenBrainzOriginalsPage>(
      `/api/v1/listenbrainz/jobs/${encodeURIComponent(id)}/originals?offset=${offset}`,
    ),
  importListenBrainz: (form: FormData) =>
    post<{ jobs: string[] }>("/api/v1/listenbrainz/upload", form),
  fetchListenBrainz: (form: { username: string; from: string; to: string }) =>
    post<{ jobs: string[] }>("/api/v1/listenbrainz/fetch", form),
  libraryDetails: (kind: string, id: string) =>
    request<Details>(`/api/v1/library/${kind}/${encodeURIComponent(id)}`),
  editMusic: (kind: string, id: string, form: Edit) =>
    post(`/api/v1/library/${kind}/${encodeURIComponent(id)}`, form),
  uploadArtwork: (kind: string, id: string, file: File) =>
    request(`/api/v1/artwork/${kind}/${encodeURIComponent(id)}/upload`, {
      method: "POST",
      body: file,
      headers: { "Content-Type": file.type || "application/octet-stream" },
    }),
  artworkSettings: () => request<ArtworkSettings>("/api/v1/artwork-providers"),
  artworkCredit: (url: string) =>
    request<ImageCredit | null>(url.split("?")[0] + "/credit"),
  saveArtworkSettings: (settings: ArtworkSettings) =>
    post<ArtworkSettings>("/api/v1/artwork-providers", settings),
  artworkUrl: (kind: string, id: string, url: string | null) =>
    post(`/api/v1/artwork/${kind}/${encodeURIComponent(id)}`, { url }),
  apiKeys: () => request<ApiKeySettings>("/api/v1/api-keys"),
  createApiKey: (form: CreateApiKey) =>
    post<CreatedApiKey>("/api/v1/api-keys", form),
  revokeApiKey: (id: string) =>
    post(`/api/v1/api-keys/${encodeURIComponent(id)}/revoke`),
  saveApiOrigins: (origins: string[]) =>
    post("/api/v1/api-keys/cors", { origins }),
  session: () => request<Session>("/api/session"),
  login: (password: string, setup: boolean, username?: string) =>
    post<{ csrf: string }>(setup ? "/api/setup" : "/api/login", {
      password,
      username,
    }),
  setUsername: (username: string) => post("/api/owner/username", { username }),
  logout: () => post("/api/logout"),
  preferences: () => request<Preferences>("/api/v1/preferences"),
  savePreferences: (form: SavePreferences) =>
    post<SavedPreferences>("/api/v1/preferences", form),
  dashboardLayout: (template?: string) =>
    request<DashboardLayout>(
      template
        ? `/api/v1/dashboard/layout?template=${encodeURIComponent(template)}`
        : "/api/v1/dashboard/layout",
    ),
  saveDashboardLayout: (layout: DashboardLayout, template?: string) =>
    post<DashboardLayout>(
      template
        ? `/api/v1/dashboard/layout?template=${encodeURIComponent(template)}`
        : "/api/v1/dashboard/layout",
      layout,
    ),
  dashboard: (params: URLSearchParams) =>
    request<Dashboard>(`/api/dashboard?${params}`),
  history: (params: URLSearchParams) =>
    request<History>(`/api/history?${params}`),
  choices: (kind: string, q: string) =>
    request<EntityChoice[]>(
      `/api/v1/consultation/choices/${kind}?${new URLSearchParams({ q })}`,
    ),
  search: (kind: string, q: string) =>
    request<SearchItem[]>(
      `/api/v1/consultation/search/${kind}?${new URLSearchParams({ q })}`,
    ),
  rankings: (kind: string, params: URLSearchParams) =>
    request<RankingPage>(`/api/v1/consultation/rankings/${kind}?${params}`),
  music: (kind: string, id: string, params: URLSearchParams) =>
    request<MusicPage>(
      `/api/v1/consultation/music/${kind}/${encodeURIComponent(id)}?${params}`,
    ),
  chart: (params: URLSearchParams) =>
    request<ActivityChart>(`/api/v1/consultation/activity?${params}`),
  discoveryRatio: (params: URLSearchParams) =>
    request<DiscoveryRatio>(`/api/v1/consultation/discovery-ratio?${params}`),
  calendar: (params: URLSearchParams) =>
    request<ListeningCalendar>(`/api/v1/consultation/calendar?${params}`),
  milestones: (params: URLSearchParams) =>
    request<ListeningMilestones>(`/api/v1/consultation/milestones?${params}`),
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
    action === "cancel"
      ? post(`/api/v1/activities/${encodeURIComponent(id)}/cancel`)
      : post(`/api/v1/activities/${encodeURIComponent(id)}`, { action }),
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
