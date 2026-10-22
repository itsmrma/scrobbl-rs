# Archive foundations

Implementation handoff for [Implementare fondamenta dell’archivio, lavori persistenti e calendario assegnato](https://github.com/itsmrma/scrobbl-rs/issues/28).
Product decisions remain in the issue's linked contracts; this document describes the code and operational limits.

## Storage and startup

`storage::open` owns the OS file lock for a data directory. Keep the returned `Storage` alive;
drain requests and workers before `Storage::close`. A second process is rejected, including after
resolving a directory alias. SQLite uses a single writer, four read-only connections, foreign keys,
WAL and FULL synchronous durability. The writer page cache is capped at 128 MiB;
readers and private storage use 8 MiB per connection. The archive writer requests a
checkpoint after about 64 MiB of WAL (16,384 pages at the usual 4 KiB page size).
This threshold is not a hard WAL disk limit; active readers can postpone checkpointing.
The private database has
its own single connection. The pool sizes are ceilings, not eagerly opened connections.

Use local storage on the machine running the process. SMB/NFS storage is unsupported for the
active database ([SQLite WAL restrictions](https://www.sqlite.org/wal.html)). Deployment must
verify its mounts; startup cannot reliably classify every host/container filesystem.

| Path under `SCROBBL_DATA_DIR` | Purpose |
| --- | --- |
| `instance.lock` | OS-held process lock; the empty file may remain after shutdown |
| `archive.db` and SQLite sidecars | Music, original observations, preferences, jobs, reports, revisions |
| `private/auth.db` and sidecars | Owner password verifier, sessions, OAuth state, Spotify credentials/tokens, key digests |
| `spool/<sha256>` | Fully received immutable files, addressed by content |
| `spool/incoming/` | Unpublished uploads; interrupted sends are never queued |
| `snapshots/` | Expiring internal coherent copies; never serve these database files to clients |
| `cache/` | Reserved reconstructible assets |
| `backups/pre-upgrade-<id>/` | Protected technical safety copies, **including private authentication data** |

Unix directories use mode 0700 and live database files 0600. Windows inherits the directory ACL;
the deployment account must restrict access. A private database missing from an existing instance
blocks startup. Pending cross-database access changes finish idempotently before workers start:
private changes and intent commit together; clearing archive diagnostics and completing the intent
are separate recoverable steps. Refreshing a token never advances the musical checkpoint.

The old `scrobbl.db` is detected before opening databases. This build requires a new data directory
and reimport; it does not migrate or alter the personal prototype.

Release schemas use `release-migrations/` and `private-migrations/`. Do not edit applied migrations.
Before an upgrade, startup creates both database copies using `VACUUM INTO`, verifies their integrity
and checks referenced spool files before publishing `complete.json`. A copy failure prevents migration.
The manifest identifies schema/app versions and immutable spool references. These startup copies are
not portable user backups: they depend on the same preserved spool. The [user-facing backup
format](recovery.md) excludes private authentication storage and carries its own received files.
Recovery jobs and the React interface implement creation, isolated verification and replacement.

For an offline rollback, stop all processes, retain the entire failed data directory and its spool,
and work on a separate copy. Use only a pre-upgrade directory with `complete.json`; place its
`archive.db` and `auth.db` at the live archive/private paths in that separate copy, without stale
SQLite sidecars, and use the previous application version. Do not copy over an open WAL database.
An archive in maintenance deliberately refuses normal startup; the recovery module must complete
or reverse its durable operation before clearing that barrier. There is no automatic reset to setup.

SQLite is bundled through SQLx 0.9/libsqlite3-sys 0.37. Startup requires at least 3.51.3 and the
tests query `sqlite_version()` and `sqlite_source_id()` from the executable. This covers the
[WAL-reset fix](https://www.sqlite.org/wal.html#the_wal_reset_bug), rather than trusting a crate name.

## Module interfaces

`db::ingest` accepts source-adapter events inside a caller-owned transaction. `ingest_at` also
accepts an acquisition clock value for deterministic tests. Internal IDs are opaque; source links,
provisional identity scopes, observations, semantic payload variants and source occurrences are
separate. Provisional names are not global identity evidence. Structured Spotify metadata wins;
ties use a deterministic metadata fingerprint. Local corrections, groups and identity decisions
have separate storage prepared for subsequent library work.

An identical observation payload is already known; different payloads with the same source event
key become variants. Changed event times/durations are different source keys, as chosen by each
adapter. Only the approved provisional Spotify API/export reconciliation is automatic here.
Ambiguous candidates remain separate. Conflicting actual durations preserve every original and an
explicit conflict; the canonical duration becomes unknown. The API's timestamp takes precedence
when reconciled with an export. All visible changes and affected listen revisions commit together.
If a later batch exposes ambiguity in an earlier provisional pair, the pair is split with a
persistent decision record. The first observation keeps its internal listen ID; the detached
observation inherits acquisition/calendar state. Every affected listen receives a revision.
`db::set_trashed` publishes removal/restoration revisions; reimport preserves the trash state.

`Storage::receive` accepts an async byte stream and an explicit byte limit. EOF, file flush and
content-addressed publication precede its database record. `enqueue` references only received files.
Workers claim a supported kind and use `commit_batch` for a single file, or `commit_member_batch`
with independent member row numbers for ZIP members. Batches are capped at 1,000 rows. Parse and
network operations happen outside the writer transaction. Refresh the job after every successful
batch: checkpoint and attempt fence stale writes. `record_member` persists inventory, including
unreadable/excluded members and unknown row counts.

`ingest` executes each bounded batch through rusqlite while holding SQLx's exclusive
SQLite handle lock, inside the existing caller-owned transaction. It never opens a
second writer or commits independently. This removes a thread/channel crossing per
small statement. The borrowed connection and its bounded statement cache are dropped
before releasing the handle. Both wrappers share `libsqlite3-sys` 0.37; keep those
versions compatible. Tokio can service other tasks while the synchronous batch runs.

New archive entity/listen/observation/variant IDs retain their opaque 64-hex shape
and do not encode source metadata or listen time. A random process prefix and counter
give adjacent inserts locality in B-trees; a 128-bit random suffix preserves uniqueness
and unpredictability. Existing IDs remain valid and are never rewritten. Authentication
secrets still use independent 256-bit OS randomness.

Music, original/variant/known-row counters, provenance and checkpoints commit atomically. Exceptions
roll back the whole batch. Cancellation preserves confirmed work and prevents automatic restart;
retry is explicit. Running jobs become queued after process failure, waiting jobs retain their
retry time, and cancelled jobs remain cancelled. `activities` exposes persistent state for the later
global UI. Spotify ZIP/JSON receipt and workers now use this interface; the source-specific
contract and checks are in [Spotify import and timing](spotify-import.md). Other adapters remain
in their own import tickets.

`change_calendar` applies either to future acquisitions or queues a global job. Existing listens
retain their first acquisition and assigned calendar on reimport/reconciliation. Global preparation
is staged in bounded batches. Concurrent archive revisions force a rebase; preferences and all
derived dates publish in one transaction. Cancellation/failure publishes none of the staged dates.
Continuous writes can delay publication through repeated rebases. `calendar::filter` uses each
assigned calendar for date/week/month/year periods and UTC elapsed time for rolling windows.

`read_view` pins one revision for a bounded server operation. The history and dashboard handlers
share one view per response. `changes` validates instance/generation/retention, returns whole
revisions and a cursor for the returned interval. Selection-specific payloads and external cursor
encoding belong to the API implementation. `materialize` freezes a server-side SQLite copy with
bounded preparation time, count, total disk quota and expiry. Initial limits: four copies, 2 GiB
total, 15 minutes, 60 seconds preparation; callers can supply `SnapshotLimits`. Expired/incomplete
copies cannot be paged. `snapshot_page` uses stable UTC-time/ID ordering. Generation rotation is
allowed only behind the durable maintenance barrier and invalidates old cursors/copies.

## Verification

Run `cargo test --locked`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and
the existing `npm run test:e2e` Chromium check. Temporary-data tests cover real process exit while a
transaction is open, restart, fencing/cancellation, variants/provenance, writer concurrency, read
coherence, mixed calendars/rebase, snapshot quotas/expiry, secret separation, independent-process
locking, migration safety copies and refusal after backup failure.

Run the separate resource check with:

```sh
cargo test --test foundations foundation_load -- --ignored --nocapture
```

This uses 18 synthetic byte streams totalling 124,396,022 bytes for spool I/O, then 100,000 synthetic
observations across 5,000 tracks for acquisition/query/calendar/copy measurements. It is not a test
of ZIP parsing, the owner's JSON files or real Spotify reconciliation. Windows development-build
measurements are indicative; server/Linux, disk exhaustion, abrupt power loss and complete recovery
acceptance remain to be verified in their owning tickets.

Measured on Windows x86_64, Rust 1.99.1, unoptimized development build, SQLite 3.51.3:

| Measurement | Result |
| --- | --- |
| 18 received files, 124,396,022 bytes | 2.590 s |
| 100,000 observations, 5,000 tracks, batches of 500 | 191.558 s |
| Count and duration total over 100,000 listens | 45.309 ms |
| Whole-history assigned-calendar recalculation | 13.123 s |
| Coherent copy preparation | 3.655 s |
| Archive after WAL checkpoint | 715,800,576 bytes |
| Process peak working set observed by Windows monitoring | 41,005,056 bytes (about 39 MiB) |

The first 512 MiB copy quota rejected this workload. The 2 GiB total quota passed, leaving room
for two copies of this archive under the disk quota; the separate count ceiling remains four.
Original payload preservation and indexes have significant disk cost. These numbers size the
initial defaults, not a throughput guarantee for a server or an optimized build. Final checks:
32 normal Rust tests, the explicit load test, rustfmt, Clippy with warnings denied, and the existing
Chromium desktop/mobile flow passed.
