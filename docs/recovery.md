# Export, backup and recovery

Open **Settings → Data & recovery** while signed in as the owner. Mutations require
the owner session, instance origin and CSRF token. Read-only API keys do not grant
administrative access. Product requirements remain in the [recovery decision](https://github.com/itsmrma/scrobbl-rs/issues/12#issuecomment-6300319358).

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

JSON format `scrobbl-export`, version 1, contains snapshot identity/revision,
explicit selection and one `listens` array. Each listen preserves metadata,
assigned calendar/date/week, acquisition time, duration classification, original
observations, payload variants and file/member/row provenance. Multiple source
observations stay in one listen; repeated distinct plays remain distinct.

CSV has one header and one row per listen, CRLF line endings, double-quoted text
with doubled quotes and JSON in `artists_json`. Text beginning with spreadsheet
formula characters gets an apostrophe prefix for safe spreadsheet opening. Use
JSON for exact original text and provenance. Export is for other tools: it does
not reconstruct an instance or add a generic import of native exports.

## Manual portable backups

**Create backup** queues an operation that continues after the page closes. A
coherent SQLite `VACUUM INTO` copy fixes one archive point. The single writer is
occupied during this copy, temporarily pausing archive commits; queries continue
and collection/import commits resume afterwards. The file inventory comes from
the frozen database and references immutable received files. Later uploads or
commits cannot enter this earlier snapshot. See [SQLite's documented guarantees](https://www.sqlite.org/lang_vacuum.html#vacuum_with_an_into_clause).

Portable format `scrobbl-backup`, version 1, is a ZIP containing:

- `manifest.json`: format, app/schema versions, snapshot identity/revision/time,
  table counts and SHA-256/size inventory;
- `archive.db`: music, originals, variants, provenance, relationships, ordinary
  server preferences, assigned calendars, jobs/checkpoints and saved reports;
- `spool/<sha256>`: fully received files referenced by that snapshot, including
  material needed to resume unfinished imports.

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
a package ready. Incomplete/failed output is not downloadable as complete.
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
Configure reverse-proxy upload limits/timeouts for the entire upload; the server
accepts bytes before queuing persistent inspection.

## Replace or transfer an archive

1. On another server, install a compatible image with a new data directory,
   configure destination origin/HTTPS/storage and create its owner password.
2. Upload a ZIP or select a saved backup. Wait for inspection. Review snapshot
   date, versions, revision and count. Run **Verify backup** if desired.
3. Choose **Restore…**, review the summary, enter/repeat a **new owner password**
   and explicitly confirm replacement. The operation replaces; it does not merge.
4. Preparation repeats isolated restoration/migrations, creates fresh private
   storage containing only the new password verifier, changes generation and
   publishes a recovery journal. New writes stop and the UI reports restart
   required. Restart the same container/image or local application.
5. Startup activates the prepared archive, access store and spool before pools or
   workers start. Sign in with the new password. Old sessions, keys, OAuth state,
   tokens and snapshot cursors cannot be reused. API clients must reload a full copy.
6. Inspect counts, originals, calendars, preferences and imports. Configure Spotify
   credentials and connect again; recreate API keys when available. Explicitly
   choose **Resume collection and pending jobs** when ready. Ordinary restarts
   continue to resume unfinished imports without this extra step.

The destination's saved-copy catalogue is retained. Previous live data, private
access store, spool and snapshots remain under `recovery-<id>/previous/`; they may
contain credentials and require private storage. No automatic deletion occurs.
Durable per-resource renames plus `recovery.pending` let startup finish interrupted
activation. Filesystem errors keep startup in controlled recovery, never public setup.

Before journal publication, failed preparation leaves the live archive usable.
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

Technical copies **include private authentication data** and depend on the original
spool. They are not portable user backups. Preserve the whole failed installation.
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

Before closing the implementation ticket, perform an actual restore on the
agreed TrueNAS/Portainer/linux-amd64 installation, with Spotify/ListenBrainz data
and API clients when integrated. Check Firefox PC/Mac and Safari iPhone/iPad,
interruption, original data, received files, retained old state and explicit
resumption. Local or isolated verification does not establish server acceptance.
Physical disk exhaustion, abrupt machine power loss and device-specific acceptance
must be recorded separately from deterministic I/O failures and process restarts.
