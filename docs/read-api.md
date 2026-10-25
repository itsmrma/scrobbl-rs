# Read API and application keys

Implementation guide for [Implementare API di lettura, sincronizzazione incrementale e gestione delle chiavi](https://github.com/itsmrma/scrobbl-rs/issues/20).
The product contract lives in its linked resolution. The spotify_analysis adapter and real
GitHub Pages → private HTTPS browser acceptance belong to the following client ticket;
the owner's PC/Mac/iOS acceptance runs once on the stabilized release.

## Connect privately

1. Configure `SCROBBL_EXTERNAL_ORIGIN` with the browser-facing HTTPS origin, including
   any nonstandard port. Follow [private HTTPS setup](docker.md); the browser must reach
   this address on LAN/VPN and trust its certificate. The API URL uses this configured
   origin, never a request's Host, forwarded header or container hostname. HTTP loopback
   supports local development, not a connection from an HTTPS GitHub Pages site.
2. Sign in as owner and open **Settings → API keys**. The previous
   `/integrations/api-keys` link redirects there, preserving the later Settings decision.
3. Save the application's exact HTTPS browser origin, for example
   `https://itsmrma.github.io`, under **Allowed browser origins**. Use one per line,
   without paths or wildcards. All repositories hosted at that origin share CORS
   permission; CORS does not authenticate requests. HTTP origins are allowed only for
   literal loopback development. No cross-origin sites are enabled by default.
4. Create a named key with an application label and optional expiry. The form shows
   the actual UTC expiry corresponding to the device's local date/time. Without an
   expiry it remains valid until revoked. Copy the secret now: it cannot be retrieved
   later. Leaving the section or reloading clears it from the UI.
5. Copy the displayed API URL and secret into an application supporting this protocol.
   Treat the key as private. Revocation blocks new requests, including continuation
   pages, and does not erase data already downloaded by the client.

Keys authorize **only** `GET /api/v1/info`, `/api/v1/listens` and `/api/v1/changes`.
Use `Authorization: Bearer <secret>` on every request. Never put keys in a URL or log.
An owner cookie alone does not authorize these routes. Authorization headers on owner
routes are rejected even alongside a valid owner cookie; owner mutations retain their
Host, Origin and CSRF checks. The private database stores SHA-256 digests of independently
generated 256-bit secrets. It also stores key metadata and cursor handles, separate from
the music archive, exports and portable backups. Restore creates new private storage;
old keys, sessions and cursors cannot authorize the recovered archive.

The owner-only management routes are `GET/POST /api/v1/api-keys`,
`POST /api/v1/api-keys/{id}/revoke` and `POST /api/v1/api-keys/cors`.
Browser DTO declarations in `frontend/src/generated.ts` are generated and checked from Rust.

## Version and initial copy

`GET /api/v1/info` returns `api_version: 1`, `instance_id`, `generation`, `revision`,
`retained_from`, `timestamp_unit: "unix_ms_utc"`, `duration_unit: "ms"`,
`max_page_size: 1000`, `snapshot_ttl_seconds` and `change_retention_revisions: 10000`.
Clients must reject unsupported API versions before applying data.

Start with:

```http
GET /api/v1/listens?from_ms=1743292800000&until_ms=1743379200000&counted_only=false&limit=500
Authorization: Bearer <read-key>
```

`from_ms` is inclusive and `until_ms` exclusive; both are optional signed UTC Unix
milliseconds. They do not use the instance calendar or assigned time zone. Calendar
date clients convert an inclusive UTC end date into the next UTC midnight.
`counted_only` defaults to false; true excludes listens the archive marks uncounted.
`limit` defaults to 500, accepts 1–1000, and never silently clamps invalid input.
Unknown query parameters are rejected. Millisecond bounds are restricted to the
JavaScript Date range, ±8,640,000,000,000,000; an empty or reversed interval is rejected.

Example page (IDs and cursors below are descriptive placeholders):

```json
{
  "api_version": 1,
  "instance_id": "opaque-instance",
  "generation": "opaque-generation",
  "from_revision": null,
  "revision": 18,
  "retained_from": 0,
  "snapshot_id": "opaque-copy-id",
  "expires_at_ms": 1792692900000,
  "selection": {"from_ms": null, "until_ms": null, "counted_only": false},
  "items": [{
    "id": "opaque-listen-id",
    "played_at_ms": 1743292800000,
    "track_id": "opaque-track-id",
    "title": "Example song",
    "album_id": null,
    "album": null,
    "artists": [{"id": "opaque-artist-id", "name": "Example artist"}],
    "track_duration_ms": null,
    "listened_ms": null,
    "duration_kind": "unknown",
    "counted": true,
    "sources": ["listenbrainz"]
  }],
  "removed": [],
  "next_cursor": "opaque-page-cursor",
  "sync_cursor": null,
  "complete": false
}
```

Each item is one canonical listen, with ordered artist credits and distinct source
names. Repeated plays have different IDs; source observations of one play do not
multiply items. `duration_kind` is `actual`, `estimated` or `unknown`. Unknown
`listened_ms` is null, including for a counted listen: never substitute zero or a
three-minute estimate. `track_duration_ms` is separate. Names and album data are the
effective archive metadata; absent album information is explicitly null. Source
payloads, source account details and administrative data are not exposed.

Follow `next_cursor` at `/api/v1/listens?cursor=<cursor>&limit=500`. Omit selection
parameters when continuing: the opaque handle binds the original selection, key,
instance, generation and frozen revision. Changing a selection or instance requires
a new initial copy. The full copy is ordered by `(played_at_ms, id)` ascending.
Concurrent ingestion, reconciliation or metadata changes cannot alter later pages.

Only the last page has `complete: true`, `next_cursor: null` and a non-null
`sync_cursor`. Accumulate pages separately and replace the client's previous data
only after that page succeeds. This also applies to an empty archive.

## Incremental updates

Request `/api/v1/changes?cursor=<sync_cursor>&limit=500`. The server isolates one
target revision, and returns the same page envelope with a non-null `from_revision`.
Follow its `next_cursor` on `/api/v1/changes`, never on `/api/v1/listens`.

Each changed ID appears once in the update, ordered by ID. `items` contains its
latest selected state at the target revision. `removed` contains objects with `id`
and nullable `replacement_id` for trashed/deleted/replaced IDs or those now outside
the selection. A removal can name an ID the client never held: deletion must be
idempotent. This avoids requiring the client's prior selection membership on the
server. `replacement_id` carries the archive's replacement link when present;
current reconciliation also publishes all affected IDs as ordinary revisions.

New backdated listens, duration/provenance/metadata changes, countability changes,
selection entries/exits and restorations all participate. Changes arriving during
pagination belong to the next incremental, not the current target. Empty updates
still complete and return a new `sync_cursor`. Apply all upserts/removals to a staged
client copy; swap data and cursor together only on complete success.

```text
receive all pages into staging
if any request, parse, version or cancellation fails:
    keep previous data and sync cursor
else if complete:
    atomically publish staging and the new sync cursor
```

There is no authorization in a cursor. Revocation and expiry are checked again on
every page, and after preparation. Reset/restore changes generation. Too-old,
expired, wrong-route and missing-copy cursors, or cursors bound to another key, return `cursor_invalid`;
the client must load a new full copy while keeping its last successful data.

## Limits, errors and recovery

Copies use isolated SQLite files on the server. No transaction on the live database
stays open while a browser pages. The existing snapshot pool is shared with recovery
and export work. Defaults, configured at startup:

| Environment variable | Default | Meaning |
| --- | --- | --- |
| `SCROBBL_API_MAX_COPIES` | 4 | Concurrent live coherent copies |
| `SCROBBL_API_COPY_BYTES` | 2147483648 | Total bytes of materialized copies |
| `SCROBBL_API_COPY_TTL_SECONDS` | 900 | Copy lifetime, capped at 86400 seconds |
| `SCROBBL_API_COPY_TIMEOUT_SECONDS` | 60 | Preparation timeout, capped at 3600 seconds |

A final page releases its file; paging cursors for that completed copy can then no
longer be replayed. If a final response is lost, retry from the previous checkpoint
or create a new full copy. Incomplete copies expire and are cleaned during subsequent
materializations. Server-side cursor handles expire with their copy; synchronization
checkpoints expire after 24 hours, also across a process restart. At most 10,000
unexpired handles are retained across keys; expired entries are cleaned on creation.
The revision log retains the last **10,000 revisions**, pruned transactionally by
both archive publication paths. `retained_from` is the inclusive usable lower bound;
older checkpoints require a full copy. A revision can change many listens, so this
is a revision bound rather than a fixed byte bound.

Authenticated reads are limited to 120 requests per minute per key, persisted across
restarts. Preflight requests contain no key and do not count. No credentials are
allowed via CORS. Allowed origins receive CORS headers on success and API errors;
preflight accepts read requests with the Authorization header and returns no data.
Responses include `Access-Control-Allow-Private-Network: true` to support Private
Network Access from public web applications.

Errors use `{"error":{"code":"…","message":"…"}}`:

| HTTP | Code | Client action |
| --- | --- | --- |
| 400 | `invalid_request` | Correct parameters; do not retry unchanged |
| 401 | `invalid_key` | Stop automatic reads; request a new valid key |
| 403 | `origin_not_allowed`, `forbidden` | Correct origin/permissions |
| 405 | `read_only` | Use a read method |
| 409 | `cursor_invalid`, `version_incompatible` | Prepare a new full copy |
| 429 | `rate_limited`, `cursor_quota` | Wait; respect Retry-After |
| 503 | `snapshot_unavailable` | Copy quota, preparation failure or maintenance; retry later |
| 500 | `server_error` | Preserve data and retry with backoff |

Temporary errors include `Retry-After: 60` where applicable. Error messages do not
contain a key, SQL, filesystem path or source payload. Network failures may combine
DNS, TLS, CORS or local-network permissions into one browser-visible error; diagnose
without asserting an unobservable cause. The client should use `credentials: "omit"`,
keep keys/data/cursors only in page memory, and never fall back to owner login.

## Verification

`cargo test --locked --test read_api` covers coherent paging with concurrent writes,
UTC bounds and ties, retroactive arrivals, selection exits, revisions, removals and
restoration, real ingestion reconciliation/provenance, unknown time, key isolation,
revocation/expiry, cursor binding/generation/retention, invalid parameters,
CSRF, CORS/preflight/errors and rate/copy limits. The existing recovery suite covers
portable backups, access invalidation and pre-upgrade copies with the new schema.
The API keys Playwright scenario covers creation, one-time display, navigation,
reload, saved origins, revocation, errors and a narrow viewport.

Explicit synthetic load check:

```sh
cargo test --locked --test read_api read_api_load -- --ignored --nocapture
```

On this Windows development build: **100,000 listens / 5,000 tracks**, coherent copy
plus first 1,000 rows **1.60 s**; all 100 pages **8.37 s**. These are local synthetic
measurements, not the owner's archive or server/browser acceptance. The real adapter,
LAN/VPN/HTTPS checks and PC/Mac/iOS acceptance remain in the following tickets.
