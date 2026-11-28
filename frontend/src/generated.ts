// Generated from Rust DTOs. Run UPDATE_BROWSER_TYPES=1 cargo test browser_contract_is_current.
export type Preferences = { name: string, initial_period: string, selected_period: string, selected_from: string | null, selected_to: string | null, hour_format: string, timezone: string, week_start: number, version: number, };
export type SavePreferences = { preferences: Preferences, calendar_scope: CalendarScope | null, };
export type SaveSelectedPeriod = { period: string, from: string | null, to: string | null, };
export type CalendarScope = "future" | "all";
export type SavedPreferences = { preferences: Preferences, job_id: string | null, };
export type Artist = { id: string, name: string, };
export type DurationKind = "actual" | "estimated" | "unknown";
export type IngestReport = { originals: number, variants: number, added: number, reconciled: number, duplicates: number, skipped: number, invalid: number, ambiguous: number, errors: Array<string>, };
export type Listen = { id: string, track_id: string, title: string, album_id: string | null, timezone: string, local_date: string, week_date: string, album: string | null, cover_url: string | null, spotify_url: string | null, played_at_ms: number, first_acquired_at_ms: number, listened_ms: number | null, duration_kind: DurationKind, counted: boolean, needs_review: boolean, artists: Array<Artist>, sources: Array<string>, };
export type History = { items: Array<Listen>, total: number, page: number, page_size: number, timezone: string, };
export type Totals = { records: number, listens: number, tracks: number, actual_ms: number, estimated_ms: number, unknown_durations: number, };
export type Ranking = { id: string, name: string, subtitle: string | null, cover_url: string | null, plays: number, listened_ms: number, position: number, actual_ms: number, estimated_ms: number, unknown_durations: number, };
export type Rankings = { artists: Array<Ranking>, albums: Array<Ranking>, tracks: Array<Ranking>, };
export type Dashboard = { totals?: Totals, rankings?: Rankings, recent?: Array<Listen>, timezone: string, };
export type EntityChoice = { id: string, name: string, subtitle: string, };
export type SearchEdition = { id: string, subtitle: string, cover_url: string | null, plays: number, };
export type SearchItem = { id: string, name: string, subtitle: string, cover_url: string | null, plays: number, editions: Array<SearchEdition>, };
export type RankingPage = { items: Array<Ranking>, total: number, page: number, page_size: number, };
export type MusicPage = { id: string, name: string, kind: string, cover_url: string | null, album_id: string | null, album: string | null, artists: Array<Artist>, totals: Totals, 
/**
 * Position by plays in the matching ranking for the same period, with
 * shared positions for equal play counts. Absent when the selection has
 * no counted plays for the entity.
 */
rank: number | null, };
export type ActivityBucket = { start: string, through: string, listens: number, actual_ms: number, estimated_ms: number, unknown_durations: number, partial: boolean, };
export type ActivityChart = { grain: string, buckets: Array<ActivityBucket>, mixed_week_starts: boolean, partial: boolean, };
export type DiscoveryRatio = { discoveries: number, repeats: number, total: number, discovery_percent: number, };
export type CalendarDay = { date: string, count: number, };
export type ListeningCalendar = { days: Array<CalendarDay>, min_date: string | null, max_date: string | null, total_listens: number, years: Array<number>, };
export type ListenMilestone = { played_at_ms: number, track_title: string, };
export type MilestoneThreshold = { count: number, reached_at_ms: number, };
export type MilestoneStreak = { days: number, start_date: string | null, end_date: string | null, };
export type ListeningMilestones = { total_listens: number, first_listen: ListenMilestone | null, last_listen: ListenMilestone | null, thresholds: Array<MilestoneThreshold>, longest_streak: MilestoneStreak, current_streak: MilestoneStreak, };
export type NowPlaying = { track_id: string | null, title: string, artists: Array<Artist>, album: string | null, cover_url: string | null, spotify_url: string, duration_ms: number | null, progress_ms: number | null, observed_at_ms: number, source_timestamp_ms: number | null, requested_at_ms: number, source_requested_at_ms: number, response_received_at_ms: number, served_at_ms: number, };
export type NowPlayingTiming = { source_timestamp_ms: number | null, requested_at_ms: number, source_requested_at_ms: number, response_received_at_ms: number, };
export type SpotifyIntervals = { sync_seconds: number, now_playing_seconds: number, };
export type SpotifyStatus = { configured: boolean, connected: boolean, credentials_from_env: boolean, redirect_uri: string, sync_seconds: number, now_playing_seconds: number, connection: string, collection: string, last_attempt_ms: number | null, last_sync_ms: number | null, last_added: number | null, verified_at_ms: number | null, last_error: string | null, now_playing_error: string | null, now_playing_timing: NowPlayingTiming | null, coverage_warning: string | null, retry_at_ms: number | null, rate_limit_reason: string | null, review_count: number, };
export type Activity = { id: string, created_at_ms: number, kind: string, state: string, filename: string | null, attempt: number, processed: number, added: number, reconciled: number, duplicates: number, invalid: number, skipped: number, ambiguous: number, retry_at_ms: number | null, error_code: string | null, expected_rows: number | null, phase: string, outcome: string | null, inventory_complete: boolean, originals: number, variants: number, below_threshold: number, target_timezone: string | null, target_week_start: number | null, };
export type ActivityMember = { originals: number, variants: number, duplicates: number, added: number, reconciled: number, invalid: number, skipped: number, below_threshold: number, member: string, kind: string, state: string, expected_rows: number | null, processed: number, error_code: string | null, };
export type ActivityDetail = { source_parameters: unknown, issues: Array<RowIssue>, activity: Activity, members: Array<ActivityMember>, };
export type RowIssue = { member: string, row_number: number, reason: string, };
export type ImportFile = { file_id: string, filename: string, job_id: string, created_at_ms: number, state: string, outcome: string | null, error_code: string | null, size_bytes: number, received_at_ms: number, listens: number, added: number, duplicates: number, invalid: number, skipped: number, };
export type Limits = { upload_bytes: number, expanded_bytes: number, members: number, row_bytes: number, };
export type GroupMember = { id: string, name: string, sample: string, tracks: number, listens: number, };
export type GroupPage = { members: Array<GroupMember>, total: number, page: number, can_rejoin: boolean, };
export type Details = { name: string, original_name: string, rating: number | null, custom_image: boolean, spotify_url: string | null, image_credit: ImageCredit | null, };
export type ImageProvider = "deezer" | "itunes" | "cover_art_archive" | "wikimedia";
export type ProviderPreference = { provider: ImageProvider, enabled: boolean, };
export type ArtworkSettings = { providers: Array<ProviderPreference>, version: number, };
export type ImageCredit = { provider: string, source_url: string, image_url: string, author: string | null, license: string | null, license_url: string | null, credit: string | null, };
export type Edit = { name: string | null, rating: number | null, };
export type ImportLimits = { upload_bytes: number, expanded_bytes: number, member_bytes: number, row_bytes: number, files: number, };
export type ImportResponse = { job_id: string, };
export type ExportSelection = { from: string | null, to: string | null, q: string | null, status: string | null, };
export type InventoryFile = { name: string, sha256: string, size_bytes: number, };
export type Manifest = { format: string, format_version: number, app_version: string, schema_version: number, created_at_ms: number, instance_id: string, generation: string, revision: number, counts: { [key in string]: number }, files: Array<InventoryFile>, };
export type Asset = { id: string, job_id: string, kind: string, format: string, state: string, created_at_ms: number, size_bytes: number | null, manifest_json: string | null, verified_at_ms: number | null, verification_error: string | null, error: string | null, };
export type ApiKey = { id: string, name: string, application: string, permissions: string, created_at_ms: number, last_used_at_ms: number | null, expires_at_ms: number | null, revoked_at_ms: number | null, };
export type ApiKeySettings = { base_url: string, cors_origins: Array<string>, keys: Array<ApiKey>, };
export type CreateApiKey = { name: string, application: string, expires_at_ms: number | null, };
export type CreatedApiKey = { id: string, secret: string, };
export type ApiCorsSettings = { origins: Array<string>, };
export type ApiSelection = { from_ms: number | null, until_ms: number | null, counted_only: boolean, };
export type ApiListen = { id: string, played_at_ms: number, track_id: string, title: string, album_id: string | null, album: string | null, artists: Array<Artist>, track_duration_ms: number | null, listened_ms: number | null, duration_kind: DurationKind, counted: boolean, sources: Array<string>, };
export type ApiRemoval = { id: string, replacement_id: string | null, };
export type ApiPage = { api_version: number, instance_id: string, generation: string, from_revision: number | null, revision: number, retained_from: number, snapshot_id: string, expires_at_ms: number, selection: ApiSelection, items: Array<ApiListen>, removed: Array<ApiRemoval>, next_cursor: string | null, sync_cursor: string | null, complete: boolean, };
export type WidgetConfig = { id: string, type: string, colSpan: 2 | 3 | 4 | 6, periodOverride?: string | null, };
export type HeaderSize = "small" | "medium" | "large" | "xlarge";
export type DashboardLayout = { version: number, widgets: Array<WidgetConfig>, 
/**
 * Music templates only; absent means the default `Medium` header.
 */
headerSize?: HeaderSize | null, };
export type AskStatus = "ok" | "unsupported" | "model_unavailable" | "model_missing" | "failed";
export type SqlAnswerKind = "value" | "table" | "series" | "ranking" | "unsupported";
export type ColumnFormat = "count" | "duration_ms" | "date" | "datetime" | "percent" | "text" | "artist" | "album" | "track";
export type SandboxRule = "empty_sql" | "sql_too_long" | "nul_byte" | "not_select" | "multiple_statements" | "bound_parameters" | "non_read_only" | "unauthorized_action" | "unauthorized_table" | "unauthorized_column" | "timeout" | "instruction_limit" | "cancelled" | "blob_not_allowed" | "output_too_large" | "sql_error" | "projection_unavailable";
export type SqlValue = null | number | string;
export type SqlColumn = { name: string, format: ColumnFormat, };
export type SqlAnswer = { title: string, computed: string, answer_kind: SqlAnswerKind, sql: string, columns: Array<SqlColumn>, rows: Array<Array<SqlValue>>, truncated: boolean, rule_code: SandboxRule | null, as_of_ms?: number, cell_links: Array<Array<string | null>>, history_href: string | null, };
export type AskResponse = { status: AskStatus, question: string, message: string | null, runtime_available: boolean, sql?: SqlAnswer, };
export type SqlPrevious = { title: string, computed: string, sql: string, };
export type AskPrevious = { question: string, sql?: SqlPrevious, };
export type AskRequest = { question: string, 
/**
 * The answer this question follows up on, when the owner continues from it.
 */
previous: AskPrevious | null, request_id?: string, };
export type AskCancelRequest = { request_id?: string, };
export type AskCancelResponse = { cancelled: boolean, };
export type AskSqlRequest = { sql: string, title: string | null, computed: string | null, answer_kind: SqlAnswerKind | null, columns: Array<SqlColumn>, };
export type AiSettings = { enabled: boolean, provider: string, active_model: string | null, custom_model_path: string | null, threads: number, backend: string, unload_minutes: number, model_dir: string | null, effective_model_dir: string, openai_base_url: string | null, openai_model: string | null, openai_api_key_set: boolean, runtime_available: boolean, model_installed: boolean, model_ready: boolean, message: string | null, };
export type SaveAiSettings = { enabled: boolean, provider: string, active_model: string | null, custom_model_path: string | null, threads: number, unload_minutes: number, model_dir: string | null, openai_base_url: string | null, openai_model: string | null, openai_api_key: string | null, clear_openai_api_key: boolean, };
export type AiAskStatus = { enabled: boolean, provider: string, active_model: string | null, custom_model_path: string | null, openai_model: string | null, runtime_available: boolean, model_ready: boolean, model_loading: boolean, };
export type AiLicense = { name: string, url: string, };
export type AiDownload = { repo: string, revision: string, file: string, sha256: string, size_bytes: number, };
export type AiRequirements = { disk_bytes: number, ram_min_bytes: number, ram_recommended_bytes: number, cpu_cores: number, gpu_optional: boolean, context_tokens: number, };
export type AiSpeed = { p50_ms: number, p95_ms: number, ttft_ms: number, decode_tokens_per_second: number, prefill_tokens_per_second: number, measured_on: string, };
export type AiDownloadJob = { id: string, state: string, processed_bytes: number, expected_bytes: number, bytes_per_second: number | null, eta_seconds: number | null, cancel_requested: boolean, error_code: string | null, };
export type AiCatalogModel = { id: string, name: string, tier: string, status: string, description: string, license: AiLicense, download: AiDownload, requirements: AiRequirements, speed: AiSpeed | null, backend: string, selectable: boolean, installed: boolean, installed_size_bytes: number | null, checksum_verified: boolean, error: string | null, download_job: AiDownloadJob | null, };
export type AiInstalledModel = { id: string, name: string | null, file: string | null, size_bytes: number, installed_at_ms: number | null, checksum_verified: boolean, custom: boolean, error: string | null, };
export type AiCatalog = { runtime_available: boolean, backend: string, model_dir: string, models: Array<AiCatalogModel>, installed: Array<AiInstalledModel>, total_installed_bytes: number, disk_used_bytes: number, };
export type AiDownloadStarted = { job_id: string, };
export type AiDeleteRequest = { confirm: boolean, force: boolean, };
export type AiDeleteResult = { id: string, freed_bytes: number, };
export type AiSelection = { active_model: string, };
export type AiTestResult = { runtime_available: boolean, backend: string, model_loaded: boolean, model_ready: boolean, message: string, };
export type UpdateStatus = { current_version: string, latest_version: string | null, update_available: boolean, release_url: string | null, checked_at_ms: number | null, error: string | null, };
