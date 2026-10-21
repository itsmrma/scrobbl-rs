# Spotify import and now-playing timing

This documents the implementation of [Implementare import Spotify affidabile e verificare
raccolta e now playing](https://github.com/itsmrma/scrobbl-rs/issues/29). Product requirements
remain in that ticket's linked decisions.

## Receipt and processing

`POST /api/import/spotify` accepts an owner/CSRF-authorized multipart selection of ZIP and
JSON files. It streams each file into the immutable spool and returns `{ "job_id": "…" }`
only after the entire selection is received. A receiving activity records fully received
inputs; interrupted receipt fails visibly and never becomes processable. Closing the browser
after confirmation does not stop the worker. A lost response does not prove failure: inspect
Activities before sending the selection again.

The worker inventories central-directory entries before processing. Entry keys include input
ordinal, member ordinal and original path, so duplicate ZIP names retain separate provenance.
Members are read in place, never extracted to their supplied paths. Unsafe paths, symlinks,
encryption, unsupported compression, bad CRC/syntax and resource limits have explicit outcomes.
An unreadable ZIP directory or entry-count limit reports incomplete inventory.

Content identifies extended history, including JSON arrays stored under unexpected names.
The parser validates each array before acquisition, then streams source rows through a
bounded channel. Batches stop at 250 rows or about 1 MiB of encoded data, with one larger row
permitted up to the row limit. Parsing, decompression and network I/O stay outside archive
transactions. Original bytes remain in the spool; accepted, invalid and excluded musical-source
rows retain their source positions and reasons.

Each batch commits observations, variants, occurrences, counters and checkpoint together.
Startup requeues interrupted work; replay scans the immutable source and skips confirmed rows.
Cancellation retains committed data and needs explicit retry. Retry uses current server limits
and retains existing row counters; reupload creates a separate report without multiplying
musical identities. A corrected source file needs a new upload.

## Limits and reports

`GET /api/v1/import/limits` publishes effective limits to the authenticated browser before upload.
Environment overrides are in `.env.example`: 256 MiB received per selection, 1 GiB expanded,
256 MiB per member, 2 MiB per encoded row and 10,000 input files/inventory entries. The HTTP
envelope has a separate bounded allowance for multipart headers. Row budgets also include
adjacent separators and whitespace. There is no 200,000-row ceiling and no splitting requirement.

Expanded-size checks include excluded members. Actual member reads are bounded too. Write
failures preserve confirmed transactions; free server storage before retrying a storage failure.
Spool and database disk usage grow with retained originals and source occurrences, including
reimport provenance. This block does not add automatic retention or garbage collection.

Activities distinguishes receiving/inventory/processing/finished and complete/partial/error/
no-music outcomes. It exposes per-file classification and counters, unknown row counts as null,
and paginated invalid-row reasons. Row/original/variant counts are not sums of new listens.
Raw music/account payloads and job parameters are not returned in these endpoints.
Activity history supports filtering and pages of 100, so collection reports cannot permanently
hide older import reports. Read access requires the owner session; actions require CSRF too.

For a known Spotify track, equal export timestamp and listened duration preserve one observation
with all original variants. Changed times/durations stay distinct. Uncertain identities use
semantic-payload evidence instead of merging changed metadata by name. Missing track identity
and conflicting durations at exactly the same export timestamp remain reviewable in history.
Reimport preserves assigned calendar and trash state. API/export reconciliation still uses the
approved provisional policy; the separate real overlapping-export check remains required.

## Collection and timing

Spotify collection now has persistent activities, attempts, waits and source occurrences.
The whole bounded recent-history fetch succeeds before the musical commit: source rows,
job checkpoint, cursor and successful-collection diagnostics commit in one transaction.
An incomplete fetch cannot advance the cursor. Restart refetches an interrupted attempt;
provider retry times remain persistent and shared with now playing. Authorization storage
and token refresh remain separate from the archive transaction.

Collection has its own gate. A provider-request gate protects token access and lets now-playing
requests run between collection pages. Credential changes/disconnection still coordinate with
collection. The cache uses polling windows, avoiding an extra full interval when the browser
polls just before a relative cache TTL would expire. Returning to a visible page refreshes it.

The sample records browser-handler arrival, actual provider-request start, parsed response,
source timestamp and response-serving time. Overview displays queue/request/cache timing and
browser request/response plus render-commit time. Connections retains the last server sample
timing even without a current track. Neither the sample nor a successful authorization check
creates a listen or proves ongoing collection health.

Spotify documents `timestamp` as the last playback-state change, including play, pause, skip
and scrub. It is not a sample-production timestamp or a measurement of propagation delay.
[Spotify's currently-playing reference](https://developer.spotify.com/documentation/web-api/reference/get-the-users-currently-playing-track)
supports this distinction. End-to-end source delay needs comparison with actual playback;
server and browser clocks are not assumed synchronized. No latency guarantee is inferred.

## Verification

Normal checks: `cargo test --locked --all-targets`, `cargo fmt --check`,
`cargo clippy --locked --all-targets -- -D warnings`, `npm run build` and browser checks.
Run each browser against a fresh temporary server:

```sh
npm run test:e2e -- --browser=chromium
npm run test:e2e -- --browser=firefox
npm run test:e2e -- --browser=webkit
cargo test --locked --test spotify_import full_size_spotify_zip -- --ignored --nocapture
```

`SCROBBL_E2E_PORT` can override port 3211 when another checkout is running tests.
The explicit load test uses 18 synthetic nested JSON members totalling exactly 124,396,022
bytes and 18,000 synthetic rows. It is distinct from the owner's original archive.
Tests cover receipt failures, CSRF, inventory/exclusions, row errors and pagination, variants,
source positions, threshold boundaries, semantic reimports, limits, restart, cancellation,
calendars/trash and fake-Spotify paging/refresh/rate limits/timing.

The owner-approved acceptance helper exercises a complete ZIP through HTTP, kills only its
isolated server during processing, restarts it and reimports. It opens the source read-only,
checks its hash/size/mtime afterward and retains its temporary archive and private reports:

```sh
cargo build --locked
python scripts/verify-spotify-archive.py /path/to/owner-approved-history.zip
```

Do not pass personal exports without owner agreement. Do not commit the temporary directory
or its detailed reports. The helper never connects Spotify and never opens the prototype.
Live Spotify, device/browser acceptance and overlapping-export reconciliation remain separate
from synthetic tests and the original-ZIP acceptance run.
