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
bounded channel. Batches stop at 1,000 rows or about 1 MiB of encoded data, with one larger row
permitted up to the row limit. Parsing, decompression and network I/O stay outside archive
transactions. Original bytes remain in the spool; accepted, invalid and excluded musical-source
rows retain their source positions and reasons.

Each batch commits observations, variants, occurrences, counters and checkpoint together.
Startup requeues interrupted work; replay scans the immutable source and skips confirmed rows.
Maintenance fences the current lease and requeues its confirmed checkpoint until the barrier
clears. Cancellation retains committed data and needs explicit retry. Retry uses current server limits
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
source timestamp and response-serving time. The queue/preparation interval includes authorization renewal and any earlier rejected attempt;
provider-request timing describes the final HTTP attempt. Overview displays queue/request/cache timing and
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

## Acceptance evidence, 2026-10-21

Windows development build, temporary release archives, bundled SQLite. The synthetic
18-member/124,396,022-byte ZIP completed in 227.141 seconds with 18,000 source rows.
The owner's separately authorized original ZIP was uploaded through HTTP without extraction
or splitting. Its 20 members included 18 JSON files totalling 124,396,022 bytes; total expanded
size including the two excluded ancillary members was 126,014,161 bytes.

| Original archive result | Observed |
| --- | ---: |
| Source rows processed | 159,560 |
| Musical source rows | 159,269 |
| New observations / retained listens | 158,576 |
| Additional original variants | 399 |
| Already-known musical rows | 294 |
| Excluded non-musical rows | 291 |
| Invalid rows | 0 |
| Below-threshold musical source rows | 30,562 |
| Counted listens | 128,149 |

An independent read of the source reproduced the observation, variant, threshold and row
counts. The isolated server was killed after 500 confirmed rows and restarted;
the same job completed on attempt 2, with exactly one occurrence per source row.
The full reimport processed 159,560 rows, adding zero listens, observations or
variants. Occurrences increased from 159,560 to
319,120, as two separate upload reports require.

Elapsed HTTP-upload/processing time including the forced restart was 560.630 seconds;
full reimport took 392.086 seconds. These are local development-build observations,
not server throughput promises. The original ZIP's hash, size and modification time remained
unchanged. The prototype archive was never opened. Detailed personal reports and databases
remain in the private temporary acceptance directory, outside Git.

By the owner's instruction, real Spotify authorization, background collection and playback
latency will be tested after installation on the server, in the deployment/final-acceptance
work. The tests above do not claim those results or real API/export reconciliation.

Final automated checks: 47 Rust tests passed; the separate ZIP load test passed explicitly.
The unrelated foundation load benchmark remains ignored. Four browser scenarios passed on each
of Chromium, Firefox and WebKit on Windows (12 total); device acceptance remains separate.

## Performance and recovery evidence, 2026-10-22

The owner requested a local PC trial of the same original ZIP, then accepted the
approximately 25-second result and explicitly stopped further tuning. The original
10–20-second aspiration was not reached. These are Windows release-build observations,
not server or hardware-independent throughput guarantees.

Uninterrupted import fell from 216.798 seconds in the initial trial to 24.577 seconds;
a final confirmation alongside local checks took 26.762 seconds. Both retained the
same 159,560 source rows, 158,576 listens/observations, 158,975 variants and zero invalid
rows. Source bytes/hash/mtime were unchanged. Overview fell from 2.2–16.4 seconds in
the initial trial to 135–158 ms cold and about 1–23 ms for subsequent absolute-selection
requests. Returning to the same selection keeps the visible result while refreshing.

The final process sample recorded 40 CPU seconds, a 232,017,920-byte peak working set
(about 221 MiB), and 2,135,176,534 bytes at peak in the isolated data directory (about
1.99 GiB, including SQLite WAL; executable excluded). The WAL checkpoint threshold
is not a hard file-size bound. The harness kills its isolated process, so these files
can retain uncheckpointed WAL; keep database and sidecars together.
These measurements precede the [disk-use follow-up](recovery.md#disk-use-follow-up-2026-10-22),
which shares original payloads and reduces reusable WAL allocation. That follow-up
uses synthetic fixtures and does not replace the personal-ZIP timing evidence above.

A separate final run killed the process after 2,000 confirmed rows. It completed on
attempt 2 in 28.384 seconds. Reimport completed in 16.588 seconds, added no musical
data, and correctly retained 319,120 occurrences for the two upload reports. Source
content was unchanged. Detailed personal reports stay in private temporary directories.

To repeat an owner-approved timing trial without the interruption/reimport steps:

```sh
cargo build --release --locked
python scripts/verify-spotify-archive.py /path/to/owner-approved-history.zip \
  --binary target/release/scrobbl-rs --benchmark \
  --max-import-seconds 30 --max-dashboard-ms 300
```

On Windows the binary has an `.exe` suffix. Timing budgets are explicit CLI inputs;
the summary records actual durations even when a budget assertion fails. Omit
`--benchmark` to test forced interruption, restart and idempotent reimport instead.
Synthetic report screenshots: [desktop](screenshots/spotify-import-desktop.png),
[mobile](screenshots/spotify-import-mobile.png).
