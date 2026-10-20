# scrobbl.rs

A self-hosted, single-owner listening archive written in Rust, initially focused on Spotify.

Heavily inspired by [Maloja](https://github.com/krateng/maloja) by krateng.

This first implementation runs directly on your computer. It includes owner authentication,
Spotify authorization and background collection, extended-history imports, a dashboard and a
searchable listening history. Docker, integration API keys and advanced library management are
future work; see [PROJECT.md](PROJECT.md).

## Run locally

Install a stable [Rust toolchain](https://rustup.rs/) and the native build tools for your operating
system (on macOS: Xcode Command Line Tools). Then, from this repository:

```sh
./scripts/run-local.sh
```

Or run `cargo run --locked` directly. Open **http://127.0.0.1:3210**, set your owner password
(at least 12 characters), and open **Settings** to configure Spotify. The service binds only to
`127.0.0.1`; use that exact address, including the port, rather than `localhost`.

The first build downloads dependencies and compiles bundled SQLite. Node.js is not required to run
the application. Keep the terminal process running to collect listens; closing the browser does
not stop collection. Sleep, shutdown or stopping the process suspends collection. Press Ctrl+C
to stop cleanly. Restart with the same command and data directory to resume.

For the Spotify procedure, see [Spotify setup](docs/spotify-setup.md), or the instructions inside
**Settings**. An [Italian local setup guide](docs/local-setup.it.md) is also available.

## What works

- Owner password stored with Argon2id; expiring, revocable browser sessions and CSRF checks.
- Spotify authorization code flow, refresh tokens, background synchronization (every 120 seconds by
  default), bounded recent-history recovery and persistent diagnostics.
- Now playing, cached and fetched only while the overview is open (every 30 seconds by default).
- Multiple Spotify **Extended Streaming History** JSON files per upload; unzip the downloaded archive
  first. Maximum upload: 64 MiB; maximum records per file: 200,000. Files commit individually.
- Idempotent repeated imports and collection; conservative reconciliation between the two sources.
- Actual, estimated and unavailable durations are distinguished. Short imports remain searchable.
- Dashboard totals and top five artists, albums and tracks, ordered by listens or listening time.
- Calendar periods, rolling 7/30/365-day windows and custom date ranges on the dashboard.
- Searchable, paginated listening history with date, duration and count-status filters.
- English interface, responsive layout, light/dark/system appearance, Europe/Rome dates,
  24-hour clock and Monday as the start of the week.

## Data rules

Spotify export `ts` is the end timestamp. Imported start time is inferred as `ts - ms_played`;
pauses, seeking and offline records can make this approximate. API `played_at` is retained as
provided and treated as a start timestamp for reconciliation. All source timestamps and original
record payloads are preserved separately, so this interpretation can be revised after live testing.
Dates are stored as UTC milliseconds and filtered/displayed in Europe/Rome, including daylight saving.
Custom end dates include the whole local day. Rolling windows are elapsed 24-hour days.

Imported music plays count in statistics at **30,000 ms or longer**. Shorter records are kept in
history. API recent-history entries count as listens; their track length is an **estimate**, never
presented as measured listening time. If duration is unavailable, the listen still counts and the
duration is marked unknown. Now playing itself never creates a listen.

Exact reimports are skipped using a source-specific event identity. Cross-source reconciliation
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
| `SCROBBL_PORT` | `3210` | Local HTTP port; also determines the Spotify callback URI |
| `SCROBBL_DATA_DIR` | `./data` | Persistent SQLite directory, relative to the process working directory |
| `SCROBBL_SYNC_SECONDS` | `120` | Background recent-history collection interval, minimum 60 seconds |
| `SCROBBL_NOW_PLAYING_SECONDS` | `30` | Now-playing cache/poll interval, minimum 15 seconds |
| `SPOTIFY_CLIENT_ID` | unset | Optional Spotify app ID, paired with the secret |
| `SPOTIFY_CLIENT_SECRET` | unset | Optional Spotify app secret, paired with the ID |

`data/scrobbl.db` contains the listening archive, original source records, owner password hash,
Spotify app credentials and authorization tokens. `data/`, local `.env` files and SQLite files are
excluded from Git. On Unix, newly created data directories and database files have private filesystem
permissions. Credentials and tokens are not returned by status/history endpoints or logged.

For a manual local copy, stop the process cleanly and copy the whole data directory; keep any `.env`
with the copy if you use one. Start with that directory to use the copy. Automated backups and a
restore interface are not implemented yet.

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
npx playwright install chromium --only-shell
npm run test:e2e
```

Browser checks start an isolated instance on port 3211 with a temporary archive, exercise the real
UI and capture desktop/mobile screenshots in `test-results/`. They do not contact Spotify or use
personal credentials. `npm run format:web` formats the frontend and browser checks.

## Remaining verification and scope

Live Spotify authorization, actual account access, timestamp reconciliation and recovery coverage
must be checked with an owner-created app. Spotify's API returns limited recent history; importing
extended history is the way to reconstruct the past. Request-limit backoff is shared between
collection and now playing. An interrupted recovery does not advance the durable sync checkpoint.

This build is for local use. LAN/VPN/HTTPS access, Docker, public hosting, integration API keys,
`spotify_analysis`, other import sources, entity pages, charts, editing/merging, custom images,
exports and scheduled backups are outside this implementation. The project license remains an open
product decision; no license selection is implied by this scaffold. No Maloja code has been copied.
