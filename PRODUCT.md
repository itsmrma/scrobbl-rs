# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Primary user: A music lover and archivist running a private, single-tenant instance to preserve, inspect, and curate their personal listening history across streaming services (Spotify, ListenBrainz) and historical file imports (Spotify Extended Streaming History JSON/ZIP).

## Product Purpose

scrobbl.rs is a self-hosted personal music archive and library management system. It provides durable preservation of listening events ("ascolti"), clean separation between raw source evidence and user corrections, deduplication/reconciliation, personal ratings, and fast responsive consultation (rankings, listening history, activity charts, and temporal patterns).

## Positioning

Unlike cloud-dependent scrobble aggregators (Last.fm) or raw SQLite viewers, scrobbl.rs provides a single, zero-dependency binary (Rust backend with embedded React SPA) executing compact retention (ADR 0001): raw import records remain accessible during ingestion, while normalized listening fingerprints, provenance, and user curation form a permanent, private, single-tenant archive with offline local resilience.

## Operating Context

- Desktop and mobile browsers (Chromium, Firefox, Safari on iOS/macOS).
- Single-user web interface with session authentication and CSRF protection.
- Background worker synchronization with Spotify Web API and optional ListenBrainz import.
- Local SQLite database holding listens, local metadata overrides, cached artwork, and user preferences.

## Capabilities and Constraints

- **Single binary distribution**: Rust web server embeds committed frontend assets from `static/dist/`.
- **Local asset independence**: Zero third-party CDN fonts, scripts, or analytics at runtime. Material Icons Outlined is self-hosted and bundled locally.
- **Compact retention model**: Normalized listens, fingerprints, and provenance are retained permanently; raw row payloads are ephemeral (ADR 0001).
- **Domain vocabulary**: Follows `GLOSSARY.md` strictly (Ascolto, Tempo ascoltato effettivo/stimato, Osservazione originale, Correzione locale, Voto personale, etc.).
- **Dynamic appearance**: User-configurable theme (`dark`, `light`, `auto`) and custom accent color tinting all background surfaces via OKLab color-mix.

## Brand Commitments

- **Name**: `scrobbl.rs`.
- **Wordmark**: `scrobbl.` in primary text with an accent-colored dot, followed by `rs` (`scrobbl<span class="accent">.</span><span class="wordmark-suffix">rs</span>`).
- **Signature Accent**: Lime / chartreuse (`#c9ee8b`), customizable by the user across 6 curated presets or arbitrary hex.
- **Iconography**: Material Icons Outlined (22px default, ligatures).

## Evidence on Hand

- Rust backend and SQLite database schema (`migrations/`).
- Full React 19 + TypeScript SPA (`frontend/src/`).
- Domain glossary (`GLOSSARY.md`) and Architectural Decision Record (`docs/adr/0001-compact-retention.md`).
- Audit notes and release checklists (`docs/gui-audit.md`, `docs/frontend.md`).

## Product Principles

1. **Observations are sacred, corrections are local**: Raw source metadata is retained for provenance, but user edits (names, grouping, ratings) always take precedence in consultation.
2. **Compact and resilient by design**: Fast, indexed SQLite queries, bounded caches, and zero runtime dependencies allow the archive to run smoothly for decades.
3. **Restraint and clarity**: Data density with high readability; music artwork and listening metrics take center stage without visual clutter.
4. **Universal responsiveness**: Full functionality on both high-resolution desktop monitors and mobile touch devices.
