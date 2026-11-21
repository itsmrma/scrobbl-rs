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
  <a href="https://scrobblrs.benassimatteo.dev"><strong>Website</strong></a>
</p>

<p align="center">
  <a href="brag-output-2026-11-20/brag.mp4"><img src="brag-output-2026-11-20/brag.gif" alt="A short tour of scrobbl.rs: Overview, import, music pages with rank and rating, listening patterns, Ask with local SQL, and self-hosting" /></a>
</p>

**scrobbl.rs** is a self-hosted music listening archive for one owner. It collects your Spotify listens and imports your full Spotify and ListenBrainz history. The result is a private archive you can browse, rank, chart, curate and question in plain language. It runs as one Rust binary with an embedded React interface and SQLite storage, with no cloud service in between.

Inspired by [Maloja](https://github.com/krateng/maloja). This is a personal passion project under active development.

> The animation uses fictional demo data. It does not show a real listening history or account.

## Highlights

### Collect and import

- **Spotify collection** runs in the background, with a live **Now playing** card.
- **Spotify Extended Streaming History** imports from ZIP or JSON. **ListenBrainz** imports from JSON, JSONL or ZIP. Imports are idempotent, resume after an interruption, and keep a persistent report.
- Export details are kept in normalized form: device/platform, country, shuffle, skipped, offline, and why playback started and ended.

### Explore

- An **Overview** dashboard built from widgets you can rearrange and resize: totals, top artists/albums/tracks, activity charts, listening patterns, milestones and now playing.
- A **period selector** shared by every screen: All time, Today, Yesterday, This week/month/year, the last 7–365 days, or custom dates. Choose "Today" once and the Overview, Listening history, Rankings and music pages all show today. An "i" next to *Period* explains when the current period is still running.
- **Listening history** with search, artist/album/source filters, newest- or oldest-first order, and an optional switch that includes plays under 30 seconds.
- **Rankings** by listens or listening time, in descending or ascending order, as a list, grid or shareable **collage**.
- **Artist, album and track pages** that open with large artwork, the entity's position in the period ranking (**#1**, **#2**, …) and your star rating. Each page has its own customizable layout.
- **Listening patterns** per period or entity, by hour, weekday, device, platform, country and start/end reason.
- A **library search** that keeps the most played match first, with grouped editions.

### Curate

- Personal artwork from file or URL, plus configurable [image providers](docs/image-providers.md) (personal images always win).
- Local corrections of names and titles, **0.5–5 star ratings**, and reversible grouping of namesake artists and albums. Original source data is always preserved.
- **Duplicate review** with a recoverable trash and undo, plus conservative automatic resolution with preview.

### Ask your archive

Ask answers questions such as *"Who did I listen to most on my phone last month?"* or *"Quanto ho saltato Ado nel 2025?"*. A **local Qwen3.5 2B** model translates the question into one read-only SQL query. scrobbl.rs runs that query in a strict [SQL sandbox](docs/ask-projection.md) over a separate projection of your archive. Nothing leaves the server. The model never sees your listens, only the schema. Every answer shows its SQL, which you can edit and run again.

AI is off by default. Enable it in **Settings → AI**, download the pinned Qwen3.5 2B SQL model (verified by SHA-256), or point to any local `.gguf`, with optional GPU layer offload (`gpu_layers`) and automatic CPU fallback. The project includes a sandbox-validated fine-tuning dataset and recipe that specialize the 2B model on scrobbl.rs SQL: see [Training the Ask SQL model](docs/ask-training.md).

### Own your data

- JSON/CSV export, portable backups without credentials, a verified restore, and automatic safety copies before upgrades.
- Read-only [API keys](docs/read-api.md) and incremental archive copies for other tools.
- Light, dark or automatic theme with a custom accent color. The layout works on desktop and mobile.

## Quick start

### Docker (server)

Use an image published by the **Docker** workflow and follow the [Docker installation guide](docs/docker.md):

```sh
SCROBBL_IMAGE=ghcr.io/itsmrma/scrobbl-rs:sha-FULL_COMMIT docker compose up -d
```

Then follow the [Spotify setup guide](docs/spotify-setup.md). It covers the private HTTPS origin Spotify requires.

### From source (local)

Install a stable [Rust toolchain](https://rustup.rs/) (1.94 or later) and your system's native build tools (Xcode Command Line Tools on macOS). Then run:

```sh
./scripts/run-local.sh      # or: cargo run --locked
```

On **macOS Apple Silicon**, Docker Desktop runs containers inside a Linux VM without access to Apple Metal. For native Metal GPU acceleration in Ask, either build from source with `--features metal` (`cargo run --locked --features metal`) or download the prebuilt `scrobbl-rs-macos-arm64-metal.tar.gz` archive from GitHub Releases.

Open **http://127.0.0.1:3210**, create the owner account, and connect Spotify or import your history from **Import history**. The service binds only to `127.0.0.1`; use that exact address instead of `localhost`. An [Italian local setup guide](docs/local-setup.it.md) is also available.

The process must keep running to collect listens. Closing the browser does not stop collection. Restart with the same data directory to resume.

## How listens are counted

- A play counts in statistics at **30 seconds or more** of actual playback. Shorter plays stay archived and searchable.
- Listening time is **actual** when the source measured it and **estimated** when only the track length is known, for example from Spotify recent history. A missing duration stays **unknown** and is never treated as zero.
- Spotify's export timestamp marks the end of playback, so the start is inferred. Instants are stored in UTC. Dates, weeks and hours follow the assigned calendar (initially Europe/Rome), including daylight saving.
- Repeated imports and collection never duplicate listens. Uncertain cross-source matches stay separate for review.

Read [Spotify import and timing](docs/spotify-import.md), the [consultation guide](docs/consultation.md) and the [glossary](GLOSSARY.md) for the full rules.

## Data and privacy

The archive is meant for one owner on a private server, reachable through LAN, VPN or your own HTTPS. Music and listening records live in `data/archive.db`. The owner verifier, sessions and Spotify secrets live separately in `data/private/auth.db`. Credentials are never returned by the API or written to logs.

Compact retention ([ADR 0001](docs/adr/0001-compact-retention.md)) keeps normalized listens, fingerprints, variants and provenance rather than raw payloads. Keep your original Spotify ZIP if you may need to reimport it. See [backup and recovery](docs/recovery.md).

Ask reads only `ask_projection.db`, a separate read-only copy, through a sandbox that allows nothing but `SELECT`. It has time, row and size limits. The model receives no archive values. See [Local AI settings](docs/ai.md) and the [security guardrails](docs/agents/security.md).

## Configuration

Startup settings come from environment variables, then an optional `.env` in the repository root, then defaults. Settings changed in the web interface are stored in SQLite. Spotify credentials set in the environment take precedence over the web form.

| Variable | Default | Purpose |
| --- | --- | --- |
| `SCROBBL_BIND` | `127.0.0.1` | Listening IP; Docker uses `0.0.0.0` |
| `SCROBBL_PORT` | `3210` | HTTP port |
| `SCROBBL_EXTERNAL_ORIGIN` | Local listener origin | Browser origin for Host/Origin checks, the Spotify callback and Secure cookies; required beyond loopback |
| `SCROBBL_DATA_DIR` | `./data` | Persistent data directory |
| `SCROBBL_LOGIN_ATTEMPTS_PER_MINUTE` | `8` | Sign-in and owner-setup attempts per minute |
| `SCROBBL_ASK_SQL_TIMEOUT_MS` | `3000` | Time limit for one Ask SQL query |
| `SCROBBL_IMPORT_UPLOAD_BYTES` | `268435456` | Total received bytes per file selection |
| `SCROBBL_IMPORT_EXPANDED_BYTES` | `1073741824` | Cumulative expanded ZIP/JSON bytes |
| `SCROBBL_IMPORT_MEMBER_BYTES` | `268435456` | Expanded bytes per member/file |
| `SCROBBL_IMPORT_ROW_BYTES` | `2097152` | Encoded JSON row budget |
| `SCROBBL_IMPORT_FILES` | `10000` | Input files and inventoried archive entries |
| `SPOTIFY_CLIENT_ID` | Unset | Optional Spotify app ID, paired with the secret |
| `SPOTIFY_CLIENT_SECRET` | Unset | Optional Spotify app secret, paired with the ID |

Collection and now-playing intervals can be changed in **Settings → Connections → Spotify**, without restarting.

## Development

The backend is **Rust** (axum, sqlx/SQLite, llama.cpp bindings for Ask). The frontend is **React 19 + TypeScript** built with Vite and embedded in the binary from `static/dist/`. See [frontend development and contracts](docs/frontend.md), [CI checks](docs/ci.md) and the [build cache guide](docs/build-cache.md).

```sh
npm ci && npm run build          # lint, typecheck and bundle the frontend
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
npx playwright install chromium --only-shell && npm run test:e2e   # optional browser checks
```

Building with `--no-default-features` skips the llama.cpp runtime for faster builds. Ask then reports that no runtime is available. Browser checks start an isolated instance with a temporary archive and never contact Spotify.

Before changing routes, SQLite access, Ask, outbound HTTP or frontend code, read the [security and anti-slop guardrails](docs/agents/security.md). The [development map](https://github.com/itsmrma/scrobbl-rs/issues/1) records release scope and future work.

## Scope

Live Spotify collection needs an owner-created Spotify app and a private server. Spotify's API exposes only recent history, so the extended-history import reconstructs the past. Maloja/Last.fm imports, scheduled backups and public hosting are future work. Podcasts, audiobooks and multiple owners per installation are outside the current design.

## License

Original project code is licensed under **GPL-3.0-or-later**. See [LICENSE](LICENSE) and [REUSE.toml](REUSE.toml). Dependency and asset notices, matching source distributions and build instructions are in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). No Maloja code has been copied. The curated Ask model is an Apache-2.0 work by the Qwen team, downloaded only on explicit owner action.

---

Music in the video: “Happy Beats & Business Moves Vol. 10” by Sascha Ende, [ende.app](https://ende.app/en), licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
