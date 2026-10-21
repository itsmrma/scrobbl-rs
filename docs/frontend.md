# Frontend and Settings

React/TypeScript sources live in `frontend/`. Vite produces the committed `static/dist/`
assets, which Rust embeds in its executable. Node is a development/build dependency;
the running server needs only the Rust executable and its data directory.

```sh
npm ci
npm run build
cargo run --locked
```

Rebuild the frontend before rebuilding Rust after a UI change. `npm run typecheck`
checks TypeScript; `npm run test:e2e` exercises the built UI against an isolated server.
For hot reload, run the Rust backend on its default port and `npm run dev`, then open
`http://127.0.0.1:5173`. Its loopback-only proxy adjusts the development Origin/Host to
the backend's expected origin; production checks are unchanged. Use the backend origin
for live Spotify authorization, whose callback returns to port 3210.
CI checks that rebuilding does not change committed assets. Production script and font
requests stay on the instance's own origin; Material Icons Outlined is bundled locally.

Rust owns the DTOs in `src/dto.rs`. A Rust test verifies `frontend/src/generated.ts`
against those declarations, including the import reports and musical types used by the
client. After changing a contract, regenerate before type checking:

```sh
UPDATE_BROWSER_TYPES=1 cargo test browser_contract_is_current
```

In PowerShell, set `$env:UPDATE_BROWSER_TYPES='1'` for that command, then remove it
with `Remove-Item Env:UPDATE_BROWSER_TYPES`. Do not format the generated file separately.
JSON uses numbers for millisecond timestamps/counters, matching the Rust JSON encoding.

## Persistent state

- Instance name, initial period, clock format and calendar live in `archive.db`.
  General saves validate the entire request and use an optimistic version to reject
  stale forms without losing their input. Every change runs through the archive writer.
- Calendar changes require `future` or `all`. Future means first acquisition after the
  change, including old listens imported later. Reimport/reconciliation retains an
  existing listen's assignment. Whole-history changes enqueue the durable calendar
  worker; the calendar and derived dates activate together. Until then the form shows
  the effective calendar and Activities shows the requested target. Cancellation keeps
  the previous calendar. Retrying an old job cannot overlap another calendar job.
- Theme, accent, sidebar width and collapsed state belong to each browser. Components
  for later rankings can use the common browser-preference hook for list/grid/page size.
- `/api/v1/activities` and its detail endpoint require the owner session. Reports expose
  counters and member states without raw source payloads, job parameters or paths.
  Unknown progress remains indeterminate. Calendar controls require CSRF authorization.
- Toasts live in the shared shell for five seconds per notification, with an independently
  timed remaining-time bar. Reports and inline errors remain after notifications expire.
- `/settings/general`, `/settings/appearance`, `/settings/connections`, `/settings/data`,
  `/activities`, `/history`, `/import` and `/dashboard` support direct loads. Legacy hashes
  still resolve. Navigation retains current page selections within a browser visit.

## Spotify diagnostics and remaining integration

Connections distinguishes saved authorization, last successful verification, action
required and collection activity/error/backoff. Verification is a snapshot of the last
successful API request, not a continuous health guarantee. `Verify & sync now` uses the
existing collection path and request limits. Last attempt, last success and new-listen
count are stored; zero new listens after a successful request is normal.

Now-playing errors are separate from collection errors. Provider rate limits and explicit
authorization failures remain shared because they affect both paths. No token or upstream
error payload is returned. The HTTPS guide covers DuckDNS or an existing domain and generic
private reverse-proxy inputs. External-origin/trusted-proxy deployment options and real
server validation belong to the deployment work; the current backend remains loopback-only.

The Spotify ZIP/JSON workflow receives files before returning a durable activity ID. Activities
exposes inventory, per-file counters, paginated invalid-row reasons, cancellation and retry;
reports can be filtered by source and paged beyond the latest 100 activities. Its receipt and
resource contract is in [Spotify import and timing](spotify-import.md). Full consultation/list-grid
pages, read API/key management and backup/restore operations remain in their own release tickets. Data & recovery and Integrations → API keys reserve their
agreed locations without advertising unavailable actions.

## Browser verification

Automated tests use temporary archives and synthetic data, never the personal prototype.
Desktop/mobile checks on Chromium, Firefox and WebKit cover routes, import/search, preferences, calendar scope,
save failures, independent browser state, sidebar keyboard controls, status presentation,
activity persistence across UI reloads, live system-theme changes, legacy browser theme preferences, and notification timing. Rust tests cover actual
server restart, calendar/reimport invariants and fake-Spotify diagnostics.

Firefox on PC/Mac and Safari on physical iPhone/iPad, live Spotify, HTTPS and server renewal
remain release acceptance checks. An automated WebKit/Firefox run is additional coverage,
not evidence of tests on those physical devices.

## Third-party assets

The bundled Material Icons Outlined font is distributed under Apache License 2.0.
Its license is included in `docs/licenses/material-icons-LICENSE.txt`. React (MIT), Vite
(MIT), TypeScript (Apache-2.0), and ts-rs (MIT) are locked by the respective package files;
their package licenses accompany installed dependencies. The font is the only separately
copied third-party visual asset in the embedded frontend.
