# scrobbl.rs

A self-hosted, single-owner listening archive written in Rust, initially focused on Spotify.

Heavily inspired by [Maloja](https://github.com/krateng/maloja) by krateng.

Run locally or install the prebuilt linux/amd64 Docker image with a persistent data directory.
It includes owner authentication, Spotify authorization and background collection,
extended-history imports, a dashboard and searchable listening history. For generic Docker,
Compose and private HTTPS configuration, see [Docker installation](docs/docker.md).
Integration API keys and advanced library management are future work; see the [development map](https://github.com/itsmrma/scrobbl-rs/issues/1)
for requirements, confirmed choices, prototype status and open decisions.

## Run locally

Install a stable [Rust toolchain](https://rustup.rs/) and the native build tools for your operating
system (on macOS: Xcode Command Line Tools). Then, from this repository:

```sh
./scripts/run-local.sh
```

Or run `cargo run --locked` directly. Open **http://127.0.0.1:3210**, set your owner password
(at least 12 characters), and open **Settings** to configure Spotify. The service binds only to
`127.0.0.1`; use that exact address, including the port, rather than `localhost`.

The archive foundations require Rust 1.94 or later and a **new data directory** when moving from
the prototype. If `scrobbl.db` is present, startup stops without opening it. Set `SCROBBL_DATA_DIR`
to a new directory and reimport there; see [archive foundations](docs/archive-foundations.md).

The committed React/TypeScript frontend is embedded in the Rust executable. To change it,
install Node.js 24 and run `npm ci` then `npm run build` before compiling Rust; see
[frontend development and contracts](docs/frontend.md). Node.js is not required to run
the compiled application. The first Rust build downloads dependencies and compiles bundled SQLite.
Keep the terminal process running to collect listens; closing the browser does
not stop collection. Sleep, shutdown or stopping the process suspends collection. Press Ctrl+C
to stop cleanly. Restart with the same command and data directory to resume.

For the Spotify procedure, see [Spotify setup](docs/spotify-setup.md), or the instructions inside
**Settings**. An [Italian local setup guide](docs/local-setup.it.md) is also available.

## What works

- Owner password stored with Argon2id; expiring, revocable browser sessions and CSRF checks.
- Spotify authorization code flow, refresh tokens, background synchronization (every 120 seconds by
  default), bounded recent-history recovery and persistent diagnostics.
- Now playing, cached and fetched only while the overview is open (every 30 seconds by default).
- Spotify **Extended Streaming History** ZIP or multiple JSON uploads, including nested folders.
  Receipt precedes durable background processing, with restart recovery, inventory and saved reports.
  Configurable defaults: 256 MiB uploaded, 1 GiB expanded, 256 MiB per file and 2 MiB per row.
  See [Spotify import and timing](docs/spotify-import.md) for limits, retries and verification.
- Idempotent repeated imports and collection; conservative reconciliation between the two sources.
- Actual, estimated and unavailable durations are distinguished. Short imports remain searchable.
- Dashboard totals and top five artists, albums and tracks, ordered by listens or listening time.
- Calendar periods, rolling 7/30/365-day windows and custom date ranges on the dashboard.
- Searchable, paginated listening history with date, duration and count-status filters.
- React shell with direct page routes, five-second toast notifications, saved activity reports,
  and a collapsible, resizable sidebar that adapts to mobile.
- Settings for instance name, default dashboard period, 12/24-hour clock, IANA time zone and
  week start. Calendar changes explicitly apply to new acquisitions or the whole history.
- Browser-specific light/dark/system theme and palette/custom HEX accent; locally bundled
  Material Icons Outlined and image placeholders.
- Connections separates saved/verified authorization, collection status and now playing,
  including last attempt/success, zero-result syncs, action required and request limits.

## Data rules

Spotify export `ts` is the end timestamp. Imported start time is inferred as `ts - ms_played`;
pauses, seeking and offline records can make this approximate. API `played_at` is retained as
provided and treated as a start timestamp for reconciliation. All source timestamps and original
record payloads are preserved separately, so this interpretation can be revised after live testing.
Instants are stored as UTC milliseconds. Date filters use the calendar assigned at first acquisition
(initially Europe/Rome), including daylight saving. Settings → General can change the calendar
for future acquisitions (including old listens first imported later), or recalculate the whole archive.
Custom end dates include the whole local day. Rolling windows are elapsed 24-hour days.

Imported music plays count in statistics at **30,000 ms or longer**. Shorter records are kept in
history. API recent-history entries count as listens; their track length is an **estimate**, never
presented as measured listening time. If duration is unavailable, the listen still counts and the
duration is marked unknown. Now playing itself never creates a listen.

Exact reimports preserve source occurrences without multiplying listens or original variants.
For a known Spotify track, equal export time and listened duration identify one observation;
changed metadata becomes a preserved original variant. Uncertain identities remain separate and
reviewable. Cross-source reconciliation
requires the same Spotify track URI, start times within two seconds and exactly one eligible
counterpart. A listen has at most one observation per source. Actual duration replaces the API
estimate, including the count-status decision. Distinct plays from the same source are kept.
Multiple eligible counterparts, including competing events within an incoming batch, are kept
separate, reported and marked for the **Possible duplicates** history filter. Timestamp differences outside this
window, relinked IDs and local files are not merged by fuzzy metadata. They may require later
library-cleanup work. The matching rule is covered by synthetic tests; matching behavior still needs
validation with real API/export pairs.

API metadata credits every structured track artist once, while total listens and tracks count once.
Extended history usually supplies only the album artist string; it is preserved as one credit,
without guessing how to split names or featuring credits. A track seen through the API gains its
structured artists, album and available cover. Imported-only metadata uses provisional identities;
rankings may initially separate imported and API artist/album entries until identity resolution is
expanded. Original imported metadata remains in the source record.

## Configuration and persistence

Startup settings come from environment variables, then an optional `.env` in the repository root,
then defaults. Copy `.env.example` to `.env` only if you need overrides. Settings entered in the web
interface are stored in SQLite. When **both** Spotify credential environment variables are set, they
take precedence and the web credential form is disabled. Restart after changing environment values.

| Variable | Default | Purpose |
| --- | --- | --- |
| `SCROBBL_BIND` | `127.0.0.1` | Internal listening IP; Docker uses `0.0.0.0` |
| `SCROBBL_PORT` | `3210` | Internal HTTP port |
| `SCROBBL_EXTERNAL_ORIGIN` | local listener origin | Browser-facing origin for Host/Origin, Spotify callback and Secure cookies; required beyond loopback |
| `SCROBBL_DATA_DIR` | `./data` | Persistent SQLite directory, relative to the process working directory |
| `SCROBBL_SYNC_SECONDS` | `120` | Background recent-history collection interval, minimum 60 seconds |
| `SCROBBL_NOW_PLAYING_SECONDS` | `30` | Now-playing cache/poll interval, minimum 15 seconds |
| `SCROBBL_IMPORT_UPLOAD_BYTES` | `268435456` | Total received file bytes per selection |
| `SCROBBL_IMPORT_EXPANDED_BYTES` | `1073741824` | Cumulative expanded ZIP/JSON bytes |
| `SCROBBL_IMPORT_MEMBER_BYTES` | `268435456` | Expanded bytes per member/file |
| `SCROBBL_IMPORT_ROW_BYTES` | `2097152` | Encoded JSON row budget, including adjacent separators/whitespace |
| `SCROBBL_IMPORT_FILES` | `10000` | Input files and inventoried archive entries |
| `SPOTIFY_CLIENT_ID` | unset | Optional Spotify app ID, paired with the secret |
| `SPOTIFY_CLIENT_SECRET` | unset | Optional Spotify app secret, paired with the ID |

`data/archive.db` contains music, original source records, preferences and persistent job/revision
state. `data/private/auth.db` separately contains the owner verifier, sessions and Spotify secrets.
`data/`, local `.env` files and SQLite files are
excluded from Git. On Unix, newly created data directories and database files have private filesystem
permissions. Credentials and tokens are not returned by status/history endpoints or logged.

For a manual local copy, stop the process cleanly and copy the whole data directory; keep any `.env`
with the copy if you use one. Start with that directory to use the copy. Automated backups and a
restore interface are not implemented yet.

Release upgrades make verified technical safety copies before schema changes. These protected
copies include authentication storage and depend on the preserved spool; they are distinct from
the planned portable application backup. Details and offline rollback guidance are in
[archive foundations](docs/archive-foundations.md).

## Checks

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
```

Rust tests use temporary databases and a local fake Spotify server. They cover session security,
OAuth state replay, token renewal, HTTP 401/429, pagination, atomic checkpoints, restart persistence,
imports, short plays, reconciliation, artist attribution, search escaping and daylight saving.

Optional browser checks require Node.js:

```sh
npm ci
npm run build
npx playwright install chromium --only-shell
npm run test:e2e
```

Browser checks start an isolated instance on port 3211 with a temporary archive, exercise the real
UI and capture desktop/mobile screenshots in `test-results/`. They do not contact Spotify or use
personal credentials. `npm run format:web` formats frontend sources and browser checks.
Rust tests verify the generated browser DTO declarations; CI also verifies embedded assets.

## Remaining verification and scope

Live Spotify authorization, actual account access, timestamp reconciliation and recovery coverage
must be checked with an owner-created app. Spotify's API returns limited recent history; importing
extended history is the way to reconstruct the past. Request-limit backoff is shared between
collection and now playing. An interrupted recovery does not advance the durable sync checkpoint.

Generic Docker and private LAN/VPN/HTTPS configuration are implemented; live server/browser,
Spotify and certificate-renewal acceptance still require recorded evidence. Public hosting, integration API keys,
`spotify_analysis`, other import sources, entity pages, charts, editing/merging, custom images,
exports and scheduled backups are outside this implementation.

## License

Copyright (c) 2026 itsmrma and scrobbl.rs contributors. Original project code is licensed under
**GPL-3.0-or-later** (GNU GPL version 3 or any later version), without warranty; see [LICENSE](LICENSE).
Per-file declarations are recorded in [REUSE.toml](REUSE.toml). Dependency and asset notices,
matching source distributions and their build instructions are in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
No Maloja code has been copied.
