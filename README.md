<div align="center">
  <img src="docs/assets/scrobbl-logo.svg" width="88" height="88" alt="scrobbl.rs logo: a lime tile with a listening waveform" />

  <h1>scrobbl.rs</h1>

  <p><strong>Your listening history, kept on your server.</strong></p>

  <p>
    <a href="https://github.com/itsmrma/scrobbl-rs/actions"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/itsmrma/scrobbl-rs/checks.yml?style=flat-square&label=checks" /></a>
    <img alt="Rust" src="https://img.shields.io/badge/Rust-backend-000000?style=flat-square&logo=rust&logoColor=white" />
    <img alt="TypeScript and React" src="https://img.shields.io/badge/TypeScript%20%2B%20React-frontend-3178C6?style=flat-square&logo=typescript&logoColor=white" />
    <img alt="GPL 3.0 or later" src="https://img.shields.io/badge/license-GPL--3.0--or--later-c9ee8b?style=flat-square" />
  </p>
</div>

<p align="center">
  <a href="brag-output-2026-10-24-164300/brag.mp4"><img src="brag-output-2026-10-24-164300/brag.gif" alt="A short tour of the scrobbl.rs listening archive: dashboard, import, listening history, and self-hosted setup" /></a>
</p>

**scrobbl.rs** is a self-hosted, single-owner archive for music listening. It collects Spotify listens, imports extended listening history, and turns the archive into searchable history, rankings, artist and album pages, and activity charts.

Inspired by [Maloja](https://github.com/krateng/maloja). This is a personal passion project under active development.

> The animation uses fictional demo data. It does not show a real listening history or account.

## What you can do

- Collect Spotify listens in the background and see now playing.
- Import Spotify Extended Streaming History and ListenBrainz JSON, JSONL, and ZIP files. Imports can resume after interruption and can be safely repeated.
- Explore searchable listening history, artist/album/track pages, complete rankings, and day/week/month/year activity charts.
- Personalize artist and album images, correct names and titles, rate music from half to five stars, and reversibly group namesakes.
- Export JSON or CSV, make portable backups, verify a restore, and recover safely from upgrades.
- Use read-only API keys and incremental archive copies with other tools.

See the [GUI audit and personal library guide](docs/gui-audit.md), [consultation guide](docs/consultation.md), [ListenBrainz import guide](docs/listenbrainz.md), and [read API guide](docs/read-api.md) for details. The [development map](https://github.com/itsmrma/scrobbl-rs/issues/1) records release scope and future work.

## Quick start

Install a stable [Rust toolchain](https://rustup.rs/) and your operating system's native build tools (Xcode Command Line Tools on macOS). Then run:

```sh
./scripts/run-local.sh
```

Or use `cargo run --locked`. Open **http://127.0.0.1:3210**, create the owner account, then open **Settings** to connect Spotify. The service binds only to `127.0.0.1`; use that exact address and port instead of `localhost`.

The terminal process must stay running to collect listens. Closing the browser does not stop collection; sleep, shutdown, or stopping the process suspends it. Restart with the same command and data directory to resume.

For server setup, use the [Docker installation guide](docs/docker.md) and [Spotify setup guide](docs/spotify-setup.md). An [Italian local setup guide](docs/local-setup.it.md) is also available.

## Installation notes

- Moving from the prototype requires Rust 1.94 or later and a **new data directory**. If `scrobbl.db` is present, startup stops without opening it. Set `SCROBBL_DATA_DIR` to a new directory and reimport; see [archive foundations](docs/archive-foundations.md).
- The React/TypeScript frontend is embedded in the Rust executable. To change it, install Node.js 24, then run `npm ci` and `npm run build` before compiling Rust. See [frontend development and contracts](docs/frontend.md). Node.js is not needed to run the compiled application.
- A first Rust build downloads dependencies and compiles bundled SQLite.

## Data and privacy

The archive is designed for one owner on a private server. Music and normalized listening records live in `data/archive.db`; the owner verifier, browser sessions, and Spotify secrets live separately in `data/private/auth.db`. The `data/` directory, `.env` files, and SQLite files are excluded from Git. Credentials and tokens are not returned by status/history endpoints or logged.

Spotify's extended-history timestamp is the end time. The imported start is inferred from `ts - ms_played`, so pauses, seeking, and offline records can make it approximate. API `played_at` is kept as supplied and treated as a start time for reconciliation. Instants are stored as UTC milliseconds; date filters use the assigned calendar (initially Europe/Rome), including daylight saving.

Compact retention keeps normalized listen data, fingerprints, variants, and provenance rather than raw JSON payloads. Successfully imported source files are released; keep your original ZIP if you may need to reimport it. Unfinished, partial, failed, and cancelled imports retain source material for retry.

Imported music plays count in statistics at **30,000 ms or longer**. Shorter records remain searchable in history. API recent-history duration is an estimate, never presented as measured listening time. A missing duration stays unknown, not zero. Now playing does not create a listen.

Repeated imports and collection are idempotent. Cross-source reconciliation is conservative: uncertain matches remain separate and reviewable. Read [Spotify import and timing](docs/spotify-import.md) and [backup and recovery](docs/recovery.md) for the full rules and recovery steps.

## Configuration

Startup settings come from environment variables, then an optional `.env` in the repository root, then defaults. Copy `.env.example` only if you need overrides. Settings changed in the web interface are stored in SQLite. If both Spotify credential variables are set, they take precedence and the web credential form is disabled; restart after changing environment values.

Collection and now-playing intervals can be changed in **Settings → Connections → Spotify → Refresh intervals**. They persist without restarting and override the initial environment defaults.

| Variable | Default | Purpose |
| --- | --- | --- |
| `SCROBBL_BIND` | `127.0.0.1` | Listening IP; Docker uses `0.0.0.0` |
| `SCROBBL_PORT` | `3210` | HTTP port |
| `SCROBBL_EXTERNAL_ORIGIN` | Local listener origin | Browser origin for Host/Origin checks, Spotify callback, and Secure cookies; required beyond loopback |
| `SCROBBL_DATA_DIR` | `./data` | Persistent SQLite directory |
| `SCROBBL_SYNC_SECONDS` | `120` | Initial collection interval; minimum 60 seconds |
| `SCROBBL_NOW_PLAYING_SECONDS` | `30` | Initial now-playing interval; minimum 15 seconds |
| `SCROBBL_IMPORT_UPLOAD_BYTES` | `268435456` | Total received bytes per file selection |
| `SCROBBL_IMPORT_EXPANDED_BYTES` | `1073741824` | Cumulative expanded ZIP/JSON bytes |
| `SCROBBL_IMPORT_MEMBER_BYTES` | `268435456` | Expanded bytes per member/file |
| `SCROBBL_IMPORT_ROW_BYTES` | `2097152` | Encoded JSON row budget |
| `SCROBBL_IMPORT_FILES` | `10000` | Input files and inventoried archive entries |
| `SPOTIFY_CLIENT_ID` | Unset | Optional Spotify app ID, paired with the secret |
| `SPOTIFY_CLIENT_SECRET` | Unset | Optional Spotify app secret, paired with the ID |

For a manual copy, stop the process cleanly and copy the whole data directory; include `.env` if you use one. Portable backups omit credentials. Release upgrades create verified technical safety copies before schema changes. See [archive foundations](docs/archive-foundations.md) and [backup and recovery](docs/recovery.md).

## Development

The backend is **Rust**; the frontend is **TypeScript and React**, with **CSS** and **HTML**. **SQL** defines the SQLite schema and queries. **Python** and **shell** support maintenance and packaging scripts. Node.js is used for the frontend build and browser checks.

Pull requests use Linux/Windows backend, browser, and Docker checks. See [CI checks and publication](docs/ci.md) for required checks and local commands.

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
```

Optional browser checks:

```sh
npm ci
npm run build
npx playwright install chromium --only-shell
npm run test:e2e
```

Browser checks start an isolated instance with a temporary archive and do not contact Spotify or use personal credentials.

## Scope and verification

Live Spotify authorization, actual account access, timestamp reconciliation, and recovery coverage require an owner-created app and a private server. Spotify's API provides limited recent history; extended-history import reconstructs the past. Generic Docker and private LAN/VPN/HTTPS configuration are implemented; see the [development map](https://github.com/itsmrma/scrobbl-rs/issues/1) for release status and remaining verification.

Public hosting, Maloja/Last.fm imports, identity merging, edition grouping, and scheduled backups are future work. Podcast/audiobook support and multiple owners per installation are outside the current design.

## License

Original project code is licensed under **GPL-3.0-or-later**. See [LICENSE](LICENSE) and [REUSE.toml](REUSE.toml). Dependency and asset notices, matching source distributions, and build instructions are in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). No Maloja code has been copied.

---

Music: “Happy Beats & Business Moves Vol. 10” by Sascha Ende, [ende.app](https://ende.app/en), licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/). Used as background music.
