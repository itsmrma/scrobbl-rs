# Export, backup and recovery

Open **Settings → Data & recovery** while signed in as the owner. Mutations require
the owner session, instance origin and CSRF token. Read-only API keys do not grant
administrative access. Product requirements remain in the [recovery decision](https://github.com/itsmrma/scrobbl-rs/issues/12#issuecomment-6300319358), amended by the owner’s [compact retention decision](https://github.com/itsmrma/scrobbl-rs/issues/12#issuecomment-6340572663).

## Export listening history

Choose JSON or CSV, optionally enter an inclusive date range, metadata search and
listen selection, review the summary, then choose **Create export**. The default
includes all non-deleted history and short listens. Date filters use each listen's
assigned calendar, just like History. Empty date boundaries are unbounded.
The operation continues if the browser closes; follow its phase/error in Activities
and download the completed file from Data & recovery.

Both formats use UTF-8, UTC milliseconds since the Unix epoch for timestamps and
milliseconds for durations. Source timestamps remain distinct from the canonical
listen timestamp. `duration_kind` is `actual`, `estimated` or `unknown`; unknown
listening time is JSON `null` or an empty CSV field, never zero. `counted` records
whether the listen contributes to statistical counts. `track_duration_ms` is the
track's length, separate from listening time. Calendar week starts are 0 = Monday
through 6 = Sunday.

JSON format `scrobbl-export`, version 2, contains snapshot identity/revision,
explicit selection and one `listens` array. Each listen preserves metadata,
assigned calendar/date/week, acquisition time, duration classification, normalized source
observations, variant fingerprints and file/member/row provenance.
`original_retention` is `compact`: source observations/variants report `raw: null`,
`raw_available: false` and `raw_fingerprint` (SHA-256 of canonical source JSON).
Occurrence fingerprints identify their original row; `source_retained` states whether
the snapshot still references source bytes. Raw fields not modeled by the app cannot
be reconstructed. This explicit change from version 1 avoids presenting missing
originals as exact source data. Multiple source
observations stay in one listen; repeated distinct plays remain distinct.

CSV has one header and one row per listen, CRLF line endings, double-quoted text
with doubled quotes and JSON in `artists_json`. Text beginning with spreadsheet
formula characters gets an apostrophe prefix for safe spreadsheet opening. Use
JSON for exact original text and provenance. Export is for other tools: it does
not reconstruct an instance or add a generic import of native exports.
For example, a title `+44` is written as `'+44` in the CSV text field; JSON
preserves `+44`. This does not alter stored metadata. Artist names inside
`artists_json` remain unchanged because the cell starts with `[`.

## Manual portable backups

**Create backup** queues an operation that continues after the page closes. A
coherent SQLite `VACUUM INTO` copy fixes one archive point. The single writer is
occupied during this copy, temporarily pausing archive commits; queries continue
and collection/import commits resume afterwards. The file inventory comes from
the frozen database and references the received files still retained at that point. Later uploads or
commits cannot enter this earlier snapshot. Source cleanup waits until the package
owns every retained file from that point; upload publication and job attachment
are protected from cleanup as one operation. See [SQLite's documented guarantees](https://www.sqlite.org/lang_vacuum.html#vacuum_with_an_into_clause).

Portable format `scrobbl-backup`, version 1, is a ZIP containing:

- `manifest.json`: format, app/schema versions, snapshot identity/revision/time,
  table counts and SHA-256/size inventory;
- `archive.db`: music, normalized observations, variant fingerprints, provenance, relationships, ordinary
  server preferences, assigned calendars, jobs/checkpoints and saved reports;
- `spool/<sha256>`: retained files referenced by that snapshot, including
  material needed to resume unfinished or partial imports. Metadata, hashes and
  reports for successfully completed imports remain in the database, even after
  their bytes have been released. A complete history backup may contain no spool files.

The package excludes private authentication storage, passwords/verifiers,
sessions, Spotify credentials/tokens and API key digests. It also excludes other
backups/exports, temporary work, snapshots and caches. Recovery records depending
on omitted outputs are excluded. Browser preferences, Compose/environment, bind
paths, DNS, proxy and certificates belong to destination infrastructure and must
be configured separately. Environment-provided credentials stay on the destination;
they are not copied from the source. Spotify authorization must still be renewed.

Copies are **not encrypted** and contain personal listening history. Download
them to another device/dataset. No automatic deletion of saved copies occurs.
The list shows date, identifier, size, versions, creation state and optional restore
verification result, with **Download** and **Delete saved file** actions. Technical
upgrade copies described below are deliberately absent from downloadable copies.

Creation checks integrity, relationships, inventory and checksums before marking
a package ready. It extracts the frozen database for SQLite checks and streams
every received file through SHA-256/ZIP checks without writing another spool copy.
Incomplete/failed output is not downloadable as complete.
Interrupted production runs again from the beginning after restart; previously
completed copies remain intact. After a reported failure, start a new operation.
Progress, phase and outcome remain in Activities.

**Verify backup** performs a new isolated restore: extract inventoried files,
check SHA-256, SQLite integrity, foreign keys, table counts, snapshot identity,
calendar/job/file relationships, run supported migrations on the copy, and recheck
invariants. It starts no source workers or network clients and does not modify
the live archive. Only explicit successful verification sets **Restore verified**,
with a date for that saved copy. A later failed trial clears that success flag.
Creation, verification and collection health are separate states.

Limits: 2 GiB compressed, 8 GiB expanded, 100,002 ZIP entries and a 16 MiB manifest.
Unexpected/duplicate entries, traversal paths, symlinks, incompatible versions,
checksum mismatches, broken relationships and migration failures are refused.
Allow space for the archive, output, temporary copies and retained old data.
Creation removes its frozen database after compression, before extracting the
database for the creation check. Extra package file space is approximately
`snapshot database + ZIP`; source files are read from the live immutable spool
and then checked from the ZIP without another extracted copy. Explicit verification
and restore preparation still need `expanded package + ZIP` for complete extraction.
At the accepted 2 GiB/8 GiB limits, those package files can approach 10 GiB;
leave additional headroom for SQLite journals/migrations, the live archive/WAL,
retained previous states, existing copies and other operations. The format limits
are not a reservation or a maximum for the whole data volume. Compression and
verification run serially with bounded file buffers, but consume CPU and disk I/O.
Configure reverse-proxy upload limits/timeouts for the entire upload; the server
accepts bytes before queuing persistent inspection.

## Replace or transfer an archive

1. On another server, install a compatible image with a new data directory,
   configure destination origin/HTTPS/storage and create its owner password.
2. Upload a ZIP or select a saved backup. Wait for inspection. Review snapshot
   date, versions, revision and count. Run **Verify backup** if desired.
3. Choose **Restore…**, review the summary, enter/repeat a **new owner password** of at least 15 characters
   and explicitly confirm replacement. The operation replaces; it does not merge.
   Confirmation commits to activation at the next restart; there is no cancellation
   after preparation. Close the confirmation dialog beforehand to leave it unchanged.
4. Preparation repeats isolated restoration/migrations, creates fresh private
   storage containing the new password verifier and the destination owner username, changes generation and
   publishes a recovery journal. New writes stop and the UI reports restart
   required. Restart the same container/image or local application.
5. Startup activates the prepared archive, access store and spool before pools or
   workers start. Sign in with the destination username and the new password. Old sessions, keys, OAuth state,
   tokens and snapshot cursors cannot be reused. API clients must reload a full copy.
6. Inspect counts, source observations/fingerprints, calendars, preferences and imports. Configure Spotify
   credentials and connect again; recreate API keys when available. Explicitly
   choose **Resume collection and pending jobs** when ready. Ordinary restarts
   continue to resume unfinished imports without this extra step.

The destination's saved-copy catalogue is retained. Previous live data, private
access store, spool and snapshots remain under `recovery-<id>/previous/`; they may
contain credentials and require private storage. No automatic deletion occurs.
Durable per-resource renames plus `recovery.pending` let startup finish interrupted
activation. Filesystem errors keep startup in controlled recovery, never public setup.

Before journal publication, failed preparation leaves the live archive usable.
The current failed attempt's staging is removed to avoid accumulating extracted
archives across retries; published preparations and retained previous data are kept.
After publication, retry that same restart rather than preparing a second restore.
If activation cannot complete, stop every app process and preserve the complete
directory. Inspect the marker and matching `new/`/`previous/` trees. Never combine
an old database with a new spool/access store or discard uncheckpointed SQLite
sidecars. Recover into a separate directory from a complete portable package or
retain a matching old resource set for administrator-assisted recovery. Do not
clear maintenance or reset owner setup just to bypass the error.

## Update failure: recovery without the UI

Record the current pinned image commit/digest before updating. Before pending
migrations, startup prepares both databases under `backups/pre-upgrade-<id>/`,
verifies integrity and spool contents, then publishes `complete.json`. A copy
failure prevents migration. The manifest records source application when known,
target version/build, schemas, database hashes and immutable spool inventory.
Older builds may lack the source build identity: keep the previous image reference.

Technical copies **include private authentication data**. New copies include a
`spool/` snapshot (immutable hard links, with a file-copy fallback); import cleanup
can release live files without invalidating offline recovery. Older technical
copies depend on the original live spool, so cleanup continues to pin their sources. They are not portable user backups. Preserve the whole failed installation.
An older image running against already migrated data is not a rollback.

Stop the application, then use the recovery-capable image to copy a completed
pre-upgrade state into a **new** directory, without opening the normal UI:

```sh
docker compose stop scrobbl
docker compose run --rm --no-deps scrobbl \
  --recover-upgrade pre-upgrade-FULL_ID --to /data/recovered-old-version
```

Replace the copy name with an actual directory. The command takes the instance
lock, refuses an existing destination, checks databases and file hashes, copies
the referenced spool, and writes `recovery.complete`. `recovery.incomplete` means
the destination cannot start. Correct the problem and retry into another new
directory; the failed installation is never overwritten.

Point the host's `SCROBBL_DATA_PATH` to the recovered directory and set
`SCROBBL_IMAGE` to the **previous pinned image**, then pull/start it. Keep the
infrastructure configuration separately. Inspect the recovered archive and access
before removing the failed copy. State returns to the pre-update point: later
listens may need recovery from their sources. No compilation on the server is needed.

## Validation and acceptance

Run `cargo test --locked --all-targets`, `cargo fmt --check`,
`cargo clippy --locked --all-targets -- -D warnings`, `npm run build` and the
Playwright suite. Temporary archives exercise portable content/secret separation,
export projections, persistent operations, corrupt-package refusal, protected
replacement, generation/access changes and restart behavior.

The owner accepted closing the implementation ticket after actual local recovery
trials and successful CI. Before publishing the release, [release acceptance](https://github.com/itsmrma/scrobbl-rs/issues/31)
still requires an actual restore on the agreed TrueNAS/Portainer/linux-amd64
installation, with Spotify/ListenBrainz data and API clients when integrated.
Check Firefox PC/Mac and Safari iPhone/iPad on the physical devices,
interruption, original data, received files, retained old state and explicit
resumption. Local or isolated verification does not establish server acceptance.
Physical disk exhaustion, abrupt machine power loss and device-specific acceptance
must be recorded separately from deterministic I/O failures and process restarts.

Local follow-up on 2026-10-22: all 65 Rust tests and seven Chromium scenarios passed.
The dedicated browser trial submits confirmation on an isolated instance, checks the
restart banner across reload, restarts the process, rejects the old session, signs in
with the new password and explicitly resumes pending work. Repeated failed preparation
after database opening releases staging; queued/running/waiting verification prevents
deletion. CSV testing includes legitimate `+44` text and exact JSON preservation.

The owner retained mandatory restart and accepted the loading/import timings;
the subsequent work below specifically addresses disk use.

## Disk use follow-up, 2026-10-22

Historical behavior before compact retention: schema 0007 stored each variant's original JSON once in `original_payloads`.
Observations and accepted source occurrences reference those same bytes. Reimport
still adds a separate occurrence/report, but does not repeat that payload. Changed
variants, the first observation's original, invalid/skipped input and legacy rows
whose payload differs from their linked variant are preserved exactly. That earlier follow-up retained original payloads and did not change export version 1.
Schema 0008 below deliberately supersedes those retention guarantees.

Existing release archives receive the verified pre-upgrade copy first. The migration
rebuilds provenance tables in a transaction; startup then compacts freed pages before
readers/workers start. A persistent marker retries compaction after interruption.
This update needs temporary headroom for the old database, new tables, SQLite WAL
and compaction, in addition to the retained pre-upgrade copy: a smaller final archive
does not mean an upgrade can run on an already full volume. A failed update retains
the offline recovery route described above. Earlier schema-6 portable backups remain
restorable and are migrated/compacted in isolation.

The writer retains at most 8 MiB of reusable WAL allocation after a successful
checkpoint/reset. This is a retention target, not a cap during writes or while old
read transactions prevent reset; see [SQLite journal_size_limit](https://www.sqlite.org/pragma.html#pragma_journal_size_limit).
Never manually delete an active WAL.

The same Windows release fixture (`cargo test --locked --release --test foundations
foundation_load -- --ignored --nocapture`) produced:

| Measurement | Before | After |
| --- | ---: | ---: |
| Live archive after calendar/snapshot workload and checkpoint | 662,130,688 B | 511,246,336 B (-22.8%) |
| Sampled peak under `backups/` | 840,289,182 B | 477,627,188 B (-43.2%) |
| Saved ZIP | 86,153,640 B | 83,374,900 B (-3.2%) |
| Backup creation including checks | 18.929 s | 14.276 s |

Both runs used 100,000 observations, 5,000 tracks and 124,396,022 source-file bytes.
Backup-directory sampling excludes the live archive, spool, WAL, consultation
snapshots and retained upgrade/recovery states; it is not a volume-wide maximum.
The database measurement includes the same later calendar/snapshot workload in both
runs. In a focused regression (256 observations imported three times, 768 occurrences,
8 KiB original padding per observation), compact database size fell from 12,091,392
to 3,424,256 bytes (-71.7%). Savings depend on payload size and reimport history.
These synthetic local results do not establish TrueNAS acceptance or personal-archive
sizes. Import/loading algorithms were not tuned further in this follow-up.

Saved backups/exports, pre-upgrade copies and `recovery-*/previous/` are deliberately
retained. Updating does not automatically reclaim them or the owner's existing test
directories. Download/verify saved backups before deleting them via the existing UI;
retain technical recovery states until the update/restore has been accepted. No
personal archive or server directory was changed for these measurements.

## Compact retention, 2026-10-22

The owner chose normalized history over keeping raw JSON. New ingestion writes
only payload fingerprints, normalized fields and provenance; accepted, invalid and
excluded rows keep their outcome/reason and position. All variant fingerprints
remain distinct and semantic reimport still avoids duplicate listens. The internal
`raw_json` sentinel is JSON `null`, explicitly exposed as unavailable in export v2.
No optional full-originals mode is added in this change.

After a successful complete/no-music import, source bytes are released when all
jobs sharing that file have completed successfully. Pending, interrupted, failed,
partial and cancelled imports retain their files for retry; unattached uploads
are retained too. Reimport a completed selection by uploading its source again.
Publication, backup and cleanup share a file gate. Metadata is marked released
before unlinking; interrupted or failed cleanup retries without relabeling a
committed import as failed. Saved backups, previous restore states and technical
upgrade copies are never automatically deleted.

Schema 0008 hashes existing originals in bounded, restartable transactions before
readers or workers open, then compacts the database using the persistent VACUUM
marker. The verified pre-upgrade copy preserves the previous schema and raw data.
Old portable backups remain accepted: their normalized history and provenance
are migrated in isolation under the current retention policy; the input ZIP stays
unchanged. New technical copies independently retain required source bytes.

Same owner-authorized ZIP, same macOS arm64 release build environment, one initial
import in each new isolated instance (decimal bytes, after stopping/checkpointing):

| Measurement | Previous retention | Compact retention |
| --- | ---: | ---: |
| Original ZIP (unchanged) | 8,436,462 B | 8,436,462 B |
| Expanded ZIP content | 126,014,161 B | 126,014,161 B |
| Active database | 685,248,512 B | 583,581,696 B |
| Live source spool after completion | 8,436,462 B | 0 B |
| Database + live source spool | 693,684,974 B | 583,581,696 B (-15.9%) |
| Import | 16.759 s | 15.156 s |
| First Overview request | 67.106 ms | 67.294 ms |

Both retained 158,576 listens/observations, 158,975 variants, 159,560 occurrences
and zero invalid rows. These are local measurements, not a server throughput
guarantee or a total including retained backup/rollback directories. A ZIP compresses
repeated JSON very well; SQLite additionally holds normalized records, relationships
and indexes. Compact retention removes raw JSON writes but does not make an indexed
database as small as the compressed source.

Reproduce with `python3 scripts/verify-spotify-archive.py PATH_TO_ZIP --binary
target/release/scrobbl-rs --benchmark --max-import-seconds 180 --max-dashboard-ms 3000`.
The budgets are explicit trial limits, not product performance promises. Use
`--recovery` instead of `--benchmark` to interrupt/resume, reimport, create and
verify a backup, download/upload it into another isolated instance, confirm
replacement, restart, compare normalized tables/preferences, reject the old
session and check explicit resumption. It retains private trial data outside Git
and verifies that the supplied ZIP’s hash, size and mtime remain unchanged.

Local recovery follow-up: process termination after 2,000 committed rows resumed
to completion on attempt 2 (15.392 s); reupload/reimport took 5.793 s and added no
listens, observations or variants. Occurrences increased from 159,560 to 319,120
for the two reports. The actual 148,391,474-byte portable backup was created with
checks in 20.526 s, explicitly verified, downloaded and uploaded to another new
local instance. Confirmation/restart/login took 13.512 s; 19 normalized tables,
fingerprints/provenance and server preferences matched, old access was refused,
previous destination data remained available and explicit resumption succeeded.
The two-import source database was 714,780,672 bytes; its restored compact copy
was 653,008,896 bytes. Those larger figures include the second report’s provenance
and must not be compared to the single-import benchmark as if they were the same load.

A separate offline trial cloned the stopped schema-7 personal test archive,
upgraded it (14.057 s), compared 13 normalized tables, released its completed
source and recovered the independently retained technical copy using the CLI.
The previous schema-7 binary started successfully against the recovered directory;
owner storage and all 105,362,275 bytes of previous raw JSON were preserved.
Offline recovery plus the previous binary’s startup took 7.333 s. The source ZIP
and the owner’s application/server directories were not modified.
