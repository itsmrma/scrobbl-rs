# Ask projection and SQL sandbox

Ask lets a local GGUF model translate a question into one read-only SQL query.
The application never runs that SQL against the archive. It runs it against a
separate SQLite file, the **Ask projection**, opened read-only inside the
**SQL sandbox**: an allow-list authorizer, hard resource limits, a timeout and
an output cap. Every piece of **model-written SQL** goes through
`src/ask/sandbox.rs::execute`; there is no other path for it. The one other
projection reader is the history-link resolver below: it runs its own fixed
queries with bound parameters, never model text as SQL, and opens the same file
through `crate::ask::projection::open_read_only`.

Model configuration and custom `.gguf` paths are described in
[Local AI settings](ai.md). The evaluation that scores SQL answers
by result is in the [Ask SQL evaluation report](reports/ask-eval-sql.md).

## What the model receives

Every Ask question sends only:

- the projection **schema** and the meaning of every table and column, generated
  from `src/ask/projection.rs::TABLES` by the same `schema_description()` the
  prompt embeds;
- the counting rules and a few worked examples;
- today's date in the assigned calendar;
- the **question text**;
- when a query fails, the **sanitized error**: the SQLite error class and only
  identifiers that appear in the model's own SQL.

It never receives archive rows, values, names, counts, results, prior results,
the archive file, or any credential. Follow-ups send the previous question, the
previous title/computed sentence and the previous SQL, never its result.
Rejection messages and local logs name the rule code and never echo a value read
from the projection.

## Where it lives and how it is rebuilt

- Path: `<data_dir>/ask_projection.db`, next to `archive.db` but a different
  file. It is never the archive and never derived from a client-supplied path.
- Source: the canonical consultation selection, so counting rules, trash, name
  groups, local corrections, album/track grouping and the assigned calendar are
  applied exactly as in the rest of the app.
- Content: only what the owner can already see in the GUI. No settings, no
  Spotify client secret, no sessions, no API keys, no raw source payloads.
- Rebuild: a background worker notices archive revision changes (debounced) and
  rebuilds the whole file. A first build happens when AI is enabled; nothing is
  built while AI is off, and disabling AI deletes the file. A calendar change
  (timezone or week start) also invalidates the projection because `meta`
  records the calendar id.
- Atomic swap: the new file is written to
  `ask_projection.tmp-<random>` in the same directory and renamed over the
  target while readers hold the swap lock. A reader either sees the previous
  complete projection or the new one, never a partial file. Ask serves the
  previous complete projection while a rebuild runs and reports
  `meta.built_at_ms` as the as-of time of the answer.
- Backups, exports and recovery: the projection is excluded. A backup contains
  `archive.db` and immutable received files only; derived data is rebuilt after
  a restore or an upgrade, never shipped.
- Storage: its size appears in storage settings as "Ask projection on disk" and
  is `0` after AI is disabled.

## Counting rules and identity

- **Trash**: listens moved to the trash are already excluded from `listens`.
  They neither appear in answers nor keep an artist, album or track row alive.
- **Possible duplicates**: listens still awaiting review stay in `listens` and
  are counted like the rest of the app. `needs_review = 1` marks them so a
  question can ask about them on their own.
- **Counted plays**: `counted = 1` for statistics. A short actual play (under
  30 seconds) has `counted = 0`; it is present but excluded from counts unless
  the question asks about short or all plays.
- **Duration**: `listened_ms` is the played time, `NULL` when
  `duration_kind = 'unknown'`. Unknown is never zero; report known time with
  `SUM(listened_ms)` and count unknown rows separately when relevant.
  `duration_kind` is `actual`, `estimated` or `unknown`.
- **Identity**: `artists`, `albums` and `tracks` carry display names after local
  corrections and canonical grouped ids. `track_artists` holds every credited
  artist in credit order (`ordinal = 0` is the primary artist); `album_artists`
  holds the album artists (first-credited on at least one track, or credited on
  every track of a multi-track album, so a single-track guest is excluded).
- **Editions**: `tracks.id` identifies one edition. `tracks.edition_key` is
  shared by editions of the same song (folded title plus credited artists), so
  "how many songs" can count editions together while "which edition" stays
  possible.
- **Calendar**: `listens.local_date` is the assigned local date and
  `week_date` its week start; `local_year`, `local_month`, `weekday` (1 =
  Monday) and `local_hour` are precomputed. `local` means the calendar assigned
  when the listen was acquired, not the current one. The `calendar` table has one
  row per day from the archive's first listen through today, with the current
  week start, so gaps and streaks need no recursive CTE.
- **Sources**: `listens.sources` is a JSON array of observation sources
  (`spotify_api`, `spotify_import`, `listenbrainz`); query it with
  `json_each(sources)`.
- **Spotify export context**: `device`, `country`, `shuffle`, `skipped`,
  `offline`, `reason_start` and `reason_end` are projected only when the import
  observations agree without conflict; otherwise they are `NULL`. They are
  never invented from a single ambiguous observation.
- **Meta**: `meta` has one row with `revision`, `built_at_ms`, `timezone` and
  `week_start` in force for this build.
- **Internal**: `_projection_meta` records the generation, revision, build time
  and calendar id used to decide whether the file is current. It is not in
  `TABLES` and the authorizer denies it.

## Projection schema

<!-- BEGIN GENERATED: ask projection schema -->
The tables are generated from `src/ask/projection.rs::TABLES`, the same source as the
model prompt and the sandbox allow-list. Do not edit this section by hand; regenerate it
with `UPDATE_ASK_DOC` when `TABLES` changes.

### `artists`

canonical grouped artists with local corrections applied

| column | type | constraints | meaning |
| --- | --- | --- | --- |
| `id` | TEXT | PRIMARY KEY | canonical grouped artist ID and `/music/artists/<id>` link key |
| `name` | TEXT | NOT NULL | display name after local corrections |

### `albums`

canonical grouped albums with local corrections applied

| column | type | constraints | meaning |
| --- | --- | --- | --- |
| `id` | TEXT | PRIMARY KEY | canonical grouped album ID and `/music/albums/<id>` link key |
| `name` | TEXT | NOT NULL | display name after local corrections |

### `tracks`

tracks with local corrections applied; album_id may be NULL

| column | type | constraints | meaning |
| --- | --- | --- | --- |
| `id` | TEXT | PRIMARY KEY | track ID and `/music/tracks/<id>` link key |
| `title` | TEXT | NOT NULL | display title after local corrections |
| `album_id` | TEXT | — | canonical grouped `albums.id`, or NULL |
| `duration_ms` | INTEGER | — | catalog track duration in milliseconds, or NULL |
| `edition_key` | TEXT | NOT NULL | shared key across editions of the same song (folded title + credited artists) |

### `track_artists`

artist credits per track in credit order

| column | type | constraints | meaning |
| --- | --- | --- | --- |
| `track_id` | TEXT | NOT NULL | referenced `tracks.id` |
| `artist_id` | TEXT | NOT NULL | canonical grouped `artists.id` |
| `ordinal` | INTEGER | NOT NULL | zero-based credit order (`0` is the primary artist) |

### `album_artists`

primary album artists (first-credited on at least one track, or credited on every track of a multi-track album; single-track guests excluded)

| column | type | constraints | meaning |
| --- | --- | --- | --- |
| `album_id` | TEXT | NOT NULL | referenced `albums.id` |
| `artist_id` | TEXT | NOT NULL | canonical grouped `artists.id` |
| `ordinal` | INTEGER | NOT NULL | zero-based album artist order |

### `listens`

non-trashed listens (trashed listens are already excluded)

| column | type | constraints | meaning |
| --- | --- | --- | --- |
| `id` | TEXT | PRIMARY KEY | listen ID |
| `track_id` | TEXT | NOT NULL | referenced `tracks.id` |
| `played_at_ms` | INTEGER | NOT NULL | UTC epoch milliseconds when the listen started |
| `played_at_local` | TEXT | NOT NULL | local datetime (`YYYY-MM-DD HH:MM:SS`) in the listen's assigned calendar timezone |
| `local_date` | TEXT | NOT NULL | `YYYY-MM-DD` in the listen's assigned calendar |
| `week_date` | TEXT | NOT NULL | `YYYY-MM-DD` start of the listen's assigned calendar week |
| `local_year` | INTEGER | NOT NULL | integer year (e.g. `2024`) |
| `local_month` | INTEGER | NOT NULL | integer month `1..12` |
| `weekday` | INTEGER | NOT NULL | integer `1` (Monday) .. `7` (Sunday) |
| `week_start` | INTEGER | NOT NULL | assigned calendar week start (`0` = Monday .. `6` = Sunday) |
| `local_hour` | INTEGER | NOT NULL | integer `0..23` in the listen's assigned calendar timezone |
| `listened_ms` | INTEGER | — | milliseconds played, or `NULL` when `duration_kind = 'unknown'` |
| `duration_kind` | TEXT | NOT NULL | `'actual'`, `'estimated'`, or `'unknown'` |
| `counted` | INTEGER | NOT NULL | `1` for counted plays, `0` for short actual plays (<30s) |
| `needs_review` | INTEGER | NOT NULL | `1` if awaiting duplicate review, `0` otherwise |
| `sources` | TEXT | NOT NULL | JSON array of observation sources (e.g. `["spotify_api","spotify_import"]`); query with `json_each(sources)` |
| `device` | TEXT | — | Spotify export device/platform when known without conflict, else `NULL` |
| `country` | TEXT | — | Spotify export connection country code when known without conflict, else `NULL` |
| `shuffle` | INTEGER | — | Spotify export shuffle (`0`/`1`) when known without conflict, else `NULL` |
| `skipped` | INTEGER | — | Spotify export skipped (`0`/`1`) when known without conflict, else `NULL` |
| `offline` | INTEGER | — | Spotify export offline (`0`/`1`) when known without conflict, else `NULL` |
| `reason_start` | TEXT | — | Spotify export reason playback started when known without conflict, else `NULL` |
| `reason_end` | TEXT | — | Spotify export reason playback ended when known without conflict, else `NULL` |

### `calendar`

one row per day across the archive's date range for gap/streak queries without recursive CTEs

| column | type | constraints | meaning |
| --- | --- | --- | --- |
| `local_date` | TEXT | PRIMARY KEY | date (`YYYY-MM-DD`) |
| `week_date` | TEXT | NOT NULL | week start date (`YYYY-MM-DD`) under the current configured week start |
| `year` | INTEGER | NOT NULL | calendar year |
| `month` | INTEGER | NOT NULL | calendar month `1..12` |
| `day` | INTEGER | NOT NULL | day of month `1..31` |
| `weekday` | INTEGER | NOT NULL | day of week (`1` = Monday .. `7` = Sunday) |

### `meta`

projection metadata (as-of revision, build time, timezone, week start)

| column | type | constraints | meaning |
| --- | --- | --- | --- |
| `revision` | INTEGER | NOT NULL | archive revision reflected by this projection |
| `built_at_ms` | INTEGER | NOT NULL | UTC epoch milliseconds when this projection was built |
| `timezone` | TEXT | NOT NULL | current configured IANA timezone |
| `week_start` | INTEGER | NOT NULL | current configured week start (`0` = Monday .. `6` = Sunday) |

### `_projection_meta` (internal)

One row per build, holding the archive generation, revision, build time and calendar id
used to decide whether the file is current. It is deliberately absent from `TABLES` and
from every allow-list: no query from the model may read it.

### Indexes

The projection carries `listens(track_id, played_at_ms)`, `listens(local_date)`,
`listens(counted)`, `track_artists(artist_id, track_id)`,
`album_artists(artist_id, album_id)`, `tracks(album_id)` and `tracks(edition_key)`, so
ordinary rankings, streaks and gap scans do not become full scans.

### Exact DDL

```sql
CREATE TABLE artists (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL
);
CREATE TABLE albums (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL
);
CREATE TABLE tracks (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    album_id TEXT,
    duration_ms INTEGER,
    edition_key TEXT NOT NULL
);
CREATE TABLE track_artists (
    track_id TEXT NOT NULL,
    artist_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    PRIMARY KEY (track_id, artist_id)
);
CREATE TABLE album_artists (
    album_id TEXT NOT NULL,
    artist_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    PRIMARY KEY (album_id, artist_id)
);
CREATE TABLE listens (
    id TEXT PRIMARY KEY,
    track_id TEXT NOT NULL,
    played_at_ms INTEGER NOT NULL,
    played_at_local TEXT NOT NULL,
    local_date TEXT NOT NULL,
    week_date TEXT NOT NULL,
    local_year INTEGER NOT NULL,
    local_month INTEGER NOT NULL,
    weekday INTEGER NOT NULL,
    week_start INTEGER NOT NULL,
    local_hour INTEGER NOT NULL,
    listened_ms INTEGER,
    duration_kind TEXT NOT NULL,
    counted INTEGER NOT NULL,
    needs_review INTEGER NOT NULL,
    sources TEXT NOT NULL,
    device TEXT,
    country TEXT,
    shuffle INTEGER,
    skipped INTEGER,
    offline INTEGER,
    reason_start TEXT,
    reason_end TEXT
);
CREATE TABLE calendar (
    local_date TEXT PRIMARY KEY,
    week_date TEXT NOT NULL,
    year INTEGER NOT NULL,
    month INTEGER NOT NULL,
    day INTEGER NOT NULL,
    weekday INTEGER NOT NULL
);
CREATE TABLE meta (
    revision INTEGER NOT NULL,
    built_at_ms INTEGER NOT NULL,
    timezone TEXT NOT NULL,
    week_start INTEGER NOT NULL
);
CREATE INDEX idx_listens_track ON listens(track_id, played_at_ms);
CREATE INDEX idx_listens_date ON listens(local_date);
CREATE INDEX idx_listens_counted ON listens(counted);
CREATE INDEX idx_track_artists_artist ON track_artists(artist_id, track_id);
CREATE INDEX idx_album_artists_artist ON album_artists(artist_id, album_id);
CREATE INDEX idx_tracks_album ON tracks(album_id);
CREATE INDEX idx_tracks_edition ON tracks(edition_key);
CREATE TABLE _projection_meta (
singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
generation TEXT NOT NULL,
revision INTEGER NOT NULL,
built_at_ms INTEGER NOT NULL,
calendar_id TEXT NOT NULL
);
```
<!-- END GENERATED: ask projection schema -->

### Regenerating this section

The section between the markers is generated from `TABLES`. After changing a
table, column or description, regenerate it and commit the result:

```powershell
$env:UPDATE_ASK_DOC='1'
cargo test --locked --lib the_checked_in_projection_document_matches_the_generated_schema
Remove-Item Env:UPDATE_ASK_DOC
```

The same test fails in CI when the checked-in document and `TABLES` disagree, so
the prompt, the SQLite schema and this document cannot drift apart.

## SQL sandbox rules

Every layer is independent. A query must pass all of them.

### 1. Connection

The projection file is opened with `SQLITE_OPEN_READ_ONLY` and
`SQLITE_OPEN_NO_MUTEX`, so even a bypass cannot write. The connection then sets
`query_only = ON`, `trusted_schema = OFF`, `temp_store = MEMORY`,
`temp.max_page_count = 1024`, `cache_size = -2000` and `cell_size_check = ON`,
and disables defensive-incompatible features: `DEFENSIVE`, no triggers, no
views, no double-quoted string literals in DML or DDL, and extension loading
turned off at the SQLite `db_config` level. `SQLITE_LIMIT_ATTACHED` is `0`, so
`ATTACH` is impossible; temporary storage stays in memory.

### 2. Static checks before prepare

UTF-8 text only, at most 8 KB, no NUL byte, and a non-empty statement that
starts with `SELECT` or `WITH` after leading whitespace and comments. The scan
ignores string literals, quoted identifiers and comments, rejects any second
statement after `;` and rejects bound-parameter placeholders (`?`, `:x`, `@x`,
`$x`). `EXPLAIN`, `PRAGMA`, `ATTACH`, `VACUUM`, `INSERT`, `UPDATE`, `DELETE`,
`CREATE`, `DROP` and `ALTER` never reach SQLite.

### 3. Authorizer allow-list

The authorizer permits only `SELECT` and `RECURSIVE`, reads of the documented
projection tables and columns, `json_each`/`json_tree` for the `sources` array,
named CTEs declared by the query itself, and a fixed function list: core
aggregates, window functions, string and math functions, date/time functions
and the JSON read helpers. Everything else is denied: writes, `ATTACH`,
`PRAGMA` in table-valued form, `load_extension`, `sqlite_*` and
`pragma_*` tables, `_projection_meta`, unknown functions, and any column that
does not exist in the projection. `randomblob`/`zeroblob` are not allowed at all;
`printf` widths are bounded by the length and output limits.

### 4. SQLite limits

`SQLITE_LIMIT_VARIABLE_NUMBER = 0` (no bound parameters),
`SQLITE_LIMIT_ATTACHED = 0`, `SQLITE_LIMIT_SQL_LENGTH = 8 KB`,
`SQLITE_LIMIT_EXPR_DEPTH = 64`, `SQLITE_LIMIT_COMPOUND_SELECT = 16`,
`SQLITE_LIMIT_FUNCTION_ARG = 16`, `SQLITE_LIMIT_LIKE_PATTERN_LENGTH = 256`,
`SQLITE_LIMIT_COLUMN = 64`, `SQLITE_LIMIT_LENGTH = 1 MB`,
`SQLITE_LIMIT_WORKER_THREADS = 0` and `SQLITE_LIMIT_VDBE_OP = 25_000`.

### 5. Time and cancellation

A progress handler stops a query after the wall-clock budget or a
5,000,000-instruction budget, whichever comes first. The wall-clock budget is
3 seconds by default and is configurable with `SCROBBL_ASK_SQL_TIMEOUT_MS`
(positive whole milliseconds, read once when Ask is installed; see
[Local AI settings](ai.md#ask-parser-runtime-178)). The owner's **Cancel**
action flips a cancellation flag and interrupts the live SQLite connection; a
cancelled query never returns a partial answer. The archive writer and other
readers are unaffected because the projection is a separate file.

### 6. Output

At most 1,000 rows and 1 MB are fetched. A larger result is truncated and
flagged, never silently cut. Values are typed as integer, real, text or null;
BLOBs are rejected rather than rendered.

### 7. Isolation

Each query runs on a dedicated blocking worker (`tokio::task::spawn_blocking`)
with its own connection. The sandbox is serialized process-wide: one semaphore
permit is taken before the blocking task starts and released after it finishes
or is cancelled, so two projects or two questions on one instance never run
sandbox SQL concurrently. Ask never blocks the archive writer or other readers,
and a slow query cannot hold a tokio worker. Rejection is local and the server
keeps serving other requests.

### 8. History links

After an answer, `src/ask/history_link.rs` may add `/history?...` links to a
result, or `/music/...` links to artist, album and track cells. It opens the
projection read-only as well, with `query_only = ON`, and runs only its own
fixed lookup queries with bound parameters; model text is parsed for filters,
never executed as SQL. That is why it is not routed through the sandbox above:
it is not model-written SQL.

### Stable rejection rule codes

| rule code | layer | meaning |
| --- | --- | --- |
| `empty_sql` | static | the query is empty |
| `sql_too_long` | static | the query exceeds 8 KB |
| `nul_byte` | static | the query contains a NUL byte |
| `not_select` | static | the statement is not `SELECT` or `WITH` |
| `multiple_statements` | static | more than one statement |
| `bound_parameters` | static | a `?`/`:x`/`@x`/`$x` placeholder |
| `non_read_only` | prepare | the prepared statement is not read-only |
| `unauthorized_action` | authorizer | an action, function or statement outside the allow-list |
| `unauthorized_table` | authorizer | a table outside the projection (including `sqlite_*`, `pragma_*`, `_projection_meta` and the archive) |
| `unauthorized_column` | authorizer | a column outside the projection |
| `timeout` | runtime | the wall-clock budget expired |
| `instruction_limit` | runtime | the instruction budget expired |
| `cancelled` | runtime | the owner cancelled the question |
| `blob_not_allowed` | output | a result value was a BLOB |
| `output_too_large` | output | a value or result exceeded the size cap |
| `sql_error` | prepare/runtime | any other SQLite error, reported by class only |
| `projection_unavailable` | connection | the projection is missing or could not be opened |

Owner-facing messages name the rule. The error sent to the model for a repair
round is sanitized to the class and identifiers from the model's own SQL; a
local log line records the rule and detail, never values.
