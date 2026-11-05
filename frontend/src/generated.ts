// Generated from Rust DTOs. Run UPDATE_BROWSER_TYPES=1 cargo test browser_contract_is_current.
export type Preferences = { name: string, initial_period: string, hour_format: string, timezone: string, week_start: number, version: number, };
export type SavePreferences = { preferences: Preferences, calendar_scope: CalendarScope | null, };
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
export type MusicPage = { id: string, name: string, kind: string, cover_url: string | null, album_id: string | null, album: string | null, artists: Array<Artist>, totals: Totals, };
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
export type Limits = { upload_bytes: number, expanded_bytes: number, members: number, row_bytes: number, };
export type GroupMember = { id: string, name: string, sample: string, tracks: number, listens: number, };
export type GroupPage = { members: Array<GroupMember>, total: number, page: number, can_rejoin: boolean, };
export type Details = { name: string, original_name: string, rating: number | null, custom_image: boolean, spotify_url: string | null, image_credit: ImageCredit | null, };
export type ImageProvider = "cover_art_archive" | "musicbrainz_image" | "wikidata" | "itunes";
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
export type DashboardLayout = { version: number, widgets: Array<WidgetConfig>, };
export type Query = { metric: Metric, entity: Entity | null, period: Period, 
/**
 * Chip/API only; never emitted by the grammar.
 */
year: number | null, 
/**
 * 1..=12
 */
month: number | null, weekday: Weekday | null, 
/**
 * 0..=23, in the listen's assigned calendar timezone.
 */
hour: number | null, source: string | null, group_by: GroupBy | null, sort_by: SortBy, order: SortOrder, limit: Limit | null, };
export type Metric = "plays" | "time_listened" | "distinct_tracks" | "distinct_artists" | "distinct_albums" | "first_play" | "last_play" | "streak" | "milestones";
export type EntityKind = "artist" | "album" | "track";
export type Entity = { kind: EntityKind, 
/**
 * Literal name from the question; resolved against the library by Rust.
 */
name: string, 
/**
 * Set after resolution or by an explicit candidate/context choice.
 */
id: string | null, };
export type Period = "all_time" | "today" | "yesterday" | "this_week" | "last_week" | "this_month" | "last_month" | "this_year" | "last_year";
export type Weekday = "monday" | "tuesday" | "wednesday" | "thursday" | "friday" | "saturday" | "sunday";
export type GroupBy = "day" | "week" | "month" | "year" | "hour" | "weekday" | "artist" | "album" | "track";
export type SortBy = "metric" | "date";
export type SortOrder = "asc" | "desc";
export type Limit = "one" | "five" | "ten" | "twenty_five" | "fifty" | "hundred";
export type Chip = { key: string, label: string, value: string, };
export type AskStatus = "ok" | "unsupported" | "ambiguous" | "model_unavailable" | "model_missing" | "failed";
export type AskResponse = { status: AskStatus, question: string, query: Query | null, chips: Array<Chip>, result: AskResult | null, message: string | null, candidates: Array<EntityCandidate>, runtime_available: boolean, };
export type AskResult = { metric: Metric, 
/**
 * Count, or timestamp in milliseconds for first/last.
 */
scalar: number | null, time: TimeCoverage | null, rows: Array<ResultRow>, first: ListenPoint | null, last: ListenPoint | null, streak: StreakResult | null, milestones: Array<MilestonePoint>, 
/**
 * Row/result cardinality used by the deterministic card.
 */
total: number, links: Array<ResultLink>, };
export type TimeCoverage = { known_ms: number, actual_ms: number, estimated_ms: number, unknown_listens: number, unknown_percent: number, };
export type ResultRow = { label: string, key: string, plays: number, time: TimeCoverage | null, href: string | null, };
export type ListenPoint = { played_at_ms: number, title: string, artist: string, album: string | null, track_id: string, };
export type StreakResult = { current: StreakSpan, longest: StreakSpan, };
export type StreakSpan = { days: number, start: string | null, end: string | null, };
export type MilestonePoint = { count: number, reached_at_ms: number, };
export type ResultLink = { label: string, href: string, };
export type EntityCandidate = { id: string, name: string, subtitle: string, kind: EntityKind, };
export type AskRequest = { question: string, context: AskContext | null, };
export type AskContext = { kind: EntityKind, id: string, };
export type AskQueryRequest = { query: Query, };
export type AskStatusResponse = { enabled: boolean, runtime_available: boolean, model_ready: boolean, model_loading: boolean, };
