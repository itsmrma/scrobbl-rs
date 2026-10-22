# Integration note

This feature was implemented in `6a25581` but its feature-branch merge did not reach main. The GUI audit restores it on the current archive format. The compact-retention decision supersedes the earlier feature branch's raw-payload retention: normalized observations, source fingerprints, source positions and reports persist; completed source files may be released. The integration is additive and does not change applied main-branch migration checksums.

# ListenBrainz implementation and verification

Implementation of [Implementare import e recupero manuale ListenBrainz](https://github.com/itsmrma/scrobbl-rs/issues/23).
The product contract remains in the [import decision](https://github.com/itsmrma/scrobbl-rs/issues/10#issuecomment-6299360191)
and [architecture decision](https://github.com/itsmrma/scrobbl-rs/issues/14#issuecomment-6300948589).

## Using the importer

Open **Import history → ListenBrainz**. Upload an official ZIP, one or more JSONL files,
a JSON array, or an API response containing `payload.listens`. Submission payloads and
playing-now responses are rejected. JSONL preserves physical line numbers, including invalid
lines. UTF-8 BOMs are accepted. JSON arrays/responses are validated in a streaming pass before
acquisition; a malformed JSON file fails independently of other uploaded files.

The official ZIP's `user.json`, API response's `payload.user_id`, and row-level `user_name` supply the account name.
For standalone JSON/JSONL without an account, the optional username field supplies explicit
provenance. Use the same account when importing equivalent files in different formats.
An account conflicting with one in the file is rejected. An absent account remains unknown;
the importer does not guess that an unknown file belongs to an account already in the archive.

Files become jobs only after successful EOF, flush and immutable spool publication. Keep the
page open during upload; after the activity link appears, processing continues on the server.
Each fully received file is queued independently: if a later upload fails, consult **Activities**
for earlier complete files and upload the incomplete files again. File names are display data.

Activities retain inventory, counts, partial outcomes and the first 100 row issues/warnings.
Read source provenance and reasons in pages of ten rows. Under compact retention raw row payloads are unavailable. Original files/API pages are downloadable only while retained for pending, failed or partial work.
Both require the owner's session. Original files are served as attachments, never executable HTML.
Counts distinguish new listens, original observations, variants, already-known observations,
reconciliation, invalid rows, excluded contents and uncertain overlaps. ZIP account, feedback,
pinned recordings and unknown members appear in the inventory as excluded from music;
their bytes remain in the original ZIP while that file is retained. Keep your own source ZIP. Unknown row totals remain unknown.

Cancel preserves confirmed rows and prevents automatic restart. Explicit retry resumes confirmed
file/member positions. After a process interruption, the existing job is reclaimed automatically.
Parsing rescans immutable input to reach its checkpoint rather than retaining a giant parser state.
Counters, identity slots, rows and checkpoints commit together in batches of up to 250 rows or
approximately 4 MiB of serialized row data. The parser channel holds at most eight rows.

## Public fetch

The server fetches a manually selected public account without a token. Dates include the whole
selected local day in the instance calendar, including 23/25-hour DST days. Without a start date,
the default is the last 14 elapsed days ending at the selected end. Without an end date, the
upper bound is fixed at job creation. The account, calendar, effective UTC interval and cursor
persist; changing preferences or retrying does not move this interval.

Public requests are limited to 31 elapsed days and ten pages of up to 1,000 rows. For longer
periods or a full history, download the official ZIP from ListenBrainz Settings → Export.
This application does not request authenticated exports or store ListenBrainz tokens.

A persistent gate serializes all ListenBrainz requests at no more than one per second, including
across jobs and restart. Requests use an identifying User-Agent, 30-second timeout and no redirects.
Provider rate headers and Retry-After control waits; network errors and 429/5xx preserve the cursor
and wait before retrying. Username not found, empty intervals, malformed/nonadvancing pages,
rate waits, cancellation and partial coverage have distinct saved outcomes.

Pages use only `max_ts`, descending order and exclusive bounds. For a full page, defer the oldest
second and request again with `max_ts=oldest+1`: that group's partial prefix has not been committed,
so a normal equal-second boundary neither drops nor duplicates rows. A page containing 1,000 listens
all at the same second cannot prove completeness. Save those rows, stop and explicitly recommend
the ZIP. The ten-page ceiling also reports uncertain coverage, never a successful complete fetch.
Rows, the next cursor and a terminal outcome commit atomically. Fully received malformed pages
remain downloadable, and attempts have separate provenance. An HTTP transport interrupted before
EOF is retried, rather than published as a complete source file.

## Identity, durations and overlap

ListenBrainz currently supplies no verified playback identifier used by this adapter.
`recording_msid` identifies a recording and is not treated as a playback key. Identity policy v1
uses the submitted semantic payload, account when known, and an occurrence ordinal. It ignores
transport `inserted_at`, recording MSID placement and response `mbid_mapping` enrichment for this
key, while retaining source fingerprints and positions in variants and occurrences. JSON field order,
whitespace, file names and integral numeric representation do not change the semantic key.
Ordinals preserve the maximum observable multiplicity of identical rows within one acquisition;
they persist in the same transaction as checkpoint progress. Equivalent ZIP/JSONL/JSON/API inputs
with the same account reuse those slots. Unknown and named accounts remain distinct and plausible
same-second overlaps are flagged. Changed submitted payloads without a verified event key remain
distinct. Deletions and local corrections remain in their existing archive layers.

`listened_at` is retained as UTC start seconds, converted explicitly to milliseconds. Integral
floating representations emitted by the official export writer are accepted; fractional seconds
and future/out-of-range times are rejected. Album, identifiers and duration may be absent.
Missing/invalid timestamp, artist or title records a source fingerprint with an explicit rejection.

`duration_played` in seconds is actual listening time, including a valid zero. Otherwise
`duration_ms`, then `duration`, supply an estimate. Without either, duration remains null/unknown
and the listen counts. Actual values below 30 seconds stay in the archive but do not count;
estimates alone do not prove a below-threshold play. Invalid/conflicting optional duration fields
produce row warnings. Shared history/statistics retain actual, estimated and unknown categories.

Source-submitted Spotify track URLs/URIs can link track identity. Similarity, ISRC and inferred
MBID mappings do not link playbacks. This adapter conservatively flags plausible overlaps and
does not automatically reconcile ListenBrainz with Spotify while Spotify timestamp semantics
remain unverified. The required real API/export overlap verification stays in
[Verificare riconciliazione e timestamp con il prossimo export esteso Spotify](https://github.com/itsmrma/scrobbl-rs/issues/18).
The existing provisional Spotify API/export policy is preserved.

## Limits and storage

Environment overrides are validated at startup and displayed in the import form:

| Variable | Default | Meaning |
| --- | ---: | --- |
| `SCROBBL_LB_UPLOAD_BYTES` | 536870912 | Bytes per uploaded file (512 MiB) |
| `SCROBBL_LB_EXPANDED_BYTES` | 2147483648 | Total declared expanded ZIP bytes, or standalone file bytes (2 GiB) |
| `SCROBBL_LB_MEMBERS` | 10000 | Maximum ZIP members |
| `SCROBBL_LB_ROW_BYTES` | 1048576 | JSONL line or JSON row/metadata section budget (1 MiB) |

At most 50 files per request; API responses are limited to 8 MiB each. ZIP entries are read directly,
never extracted. Unsafe paths, duplicate names, symlinks, unsupported/encrypted compression, size
overflows, CRC/read errors and exceeded expansion/row limits produce explicit outcomes. ZIPs use
stored/deflate support through zip 9.0.5. The spool, originals, reports, page references and identity
counters belong to the archive data directory and must travel with recovery copies. The ListenBrainz
integration migration is numbered 10, after artwork migration 9. The original feature branch migration 5 was not installed on main; the release recovery migration 5 stays unchanged. Startup detects
any unapplied migration, including a lower-numbered gap, before deciding whether a verified
pre-upgrade safety copy is necessary. Applied migrations are unchanged.

## Evidence and remaining release checks

Examined official documentation for [JSON](https://listenbrainz.readthedocs.io/en/latest/users/json.html),
[exports](https://listenbrainz.readthedocs.io/en/latest/users/api/export.html),
[public pagination](https://listenbrainz.readthedocs.io/en/latest/users/api/core.html), and
[request limits](https://listenbrainz.readthedocs.io/en/latest/users/api/index.html).
Compared with the actual [export writer](https://github.com/metabrainz/listenbrainz-server/blob/61ef77e4a7b2fd962943c56ba765d34fa8483ce8/listenbrainz/background/export.py).

The checked-in two-row JSONL fixture is verbatim upstream test data at revision
`61ef77e4a7b2fd962943c56ba765d34fa8483ce8`; its source is recorded beside it. ZIP tests package those
rows using the official member layout. This is not a claim that an authenticated production ZIP
download was performed. On 2026-10-21 a two-listen response from the public `rob` account was
fetched with an identifying User-Agent and imported successfully into a temporary archive.
The response/account data are outside the repository; no personal archive or credentials were used.

Automated checks cover format equivalence, multiplicity, reformatting, changed enrichment,
unknown accounts, invalid rows, partial files, durations, DST, equal-second boundaries, stalled
pages, HTTP 404/429/503, persisted waits, cancellation/retry/restart, originals, CSRF/session guards,
statistics categories and browser flows. Run:

```sh
npm ci
npm run build
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
npm run test:e2e
```

An optional test imports an already-downloaded real response in an isolated archive:
set `SCROBBL_LB_SAMPLE_PATH` to that JSON file, then run
`cargo test --locked --test listenbrainz actual_public -- --ignored --nocapture`.
The ordinary suite deliberately skips that external-sample check and the unrelated foundation
load benchmark and full-size Spotify workload. Browser screenshots are written under ignored `test-results/`.

Real overlapping Spotify/export reconciliation, the owner's complete archive, private server
deployment, disk exhaustion/power loss, and the required Firefox PC/Mac and physical Safari
iPhone/iPad acceptance remain release checks in their owning tickets. Synthetic fixtures and
Chromium verification do not certify those environments.
