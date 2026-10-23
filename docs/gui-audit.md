# Release GUI audit

The source requirements and owner feedback live in [Controllare e rifinire tutta la GUI con Impeccable](https://github.com/itsmrma/scrobbl-rs/issues/26), including its comments and the [explicit scope extension](https://github.com/itsmrma/scrobbl-rs/issues/26#issuecomment-6349443163). This document records implementation evidence, not another product specification.

## Workflow and findings

Used the current [Impeccable](https://github.com/pbakaus/impeccable) workflow at commit `69c34b7c3d6267dfd783dacb9f58f04522656b2b`: context loading, technical audit, adaptation, animation and polish references. The existing application and issue contracts supplied the design context. The manual source detector (`impeccable detect --json frontend/src`, engine 4.7.0) returned `[]`, exit 0. A separate scan of the subsequently restored ListenBrainz component also returned `[]`, exit 0. This source scan does not certify accessibility or visual quality.

| Priority | Verified finding | Change and regression evidence |
| --- | --- | --- |
| P0 | The ListenBrainz feature-branch merge had not reached main; its GUI and endpoints were absent. | Restored the existing implementation from `6a25581`, integrated its source adapter with compact ingestion, added migration 10 without changing existing migration checksums, and restored parser/network/browser regression coverage. Source payload availability now follows compact retention. |
| P1 | The extended-history input had no drop handler. | Drop zone, explicit file picker, file inventory, clear selection, type/size errors and upload state. A browser drop imports a real synthetic history file without opening a file dialog. |
| P1 | Artist rankings returned an absent image unconditionally. | Local artwork endpoint, Spotify metadata enrichment, persistent image bytes and owner overrides. Identity evidence stays separate from library identity and listen reconciliation. |
| P1 | A new customization path must preserve original metadata and restore safety. | Local name/title overlays and personal ratings; CSRF/session/maintenance guards; reingest, frozen reads and portable backup/restore tests. |
| P2 | Connections put diagnostics and intervals before app setup. | Ordered private-address, app-setup and authorization steps. The Developer Dashboard Save action is explicit. Spotify remains expandable; collection diagnostics and interval controls use separate disclosure. |
| P2 | The time-help target was 28 px. The current help already opened with keyboard and click. | A 44 px target; keyboard focus, Escape and click tests. No claim that the owner's historical failure reproduced on current main. |
| P2 | Phone history compressed cover, title, timestamp and duration into four narrow columns. | Stacked history metadata; fluid forms, settings tabs, rankings, recovery controls, charts and dialogs. The chart is the intentionally scrollable region. |
| P2 | Green surfaces were fixed independently of the chosen accent. | Neutral dark/light surfaces mixed with the browser accent; shared tokens, readable foregrounds, semantic error/warning colors and palette contrast checks. |
| P2 | Wordmark spacing differed between login and sidebar. Active navigation used a side stripe. | One contiguous wordmark, compared with the prototype; active navigation uses a filled surface. Logo symbol retained. |
| P2 | Activities exposed a sparse list of names, raw states and counters. | Status badges, phase descriptions, progress, review summary, meaningful empty state, saved results and one report destination per row. Unknown totals remain indeterminate. |
| P2 | Now playing existed only on Overview. | A shared polling provider feeds Overview and a compact sidebar widget on every route. Navigation does not create another provider request. Existing server cadence and request-limit behavior remain authoritative. |
| P3 | State changes and navigation lacked continuity. | Short control feedback, page/tab transitions, sidebar/drawer, dialog and toast entrances. Reduced motion removes spatial effects while keeping state feedback and the five-second toast countdown. |

The system uses the existing React stack and Material Icons Outlined; no motion dependency or remote production script was added. The production JavaScript is approximately 102 KB compressed including ListenBrainz.

| Audit dimension | Score | Practical limit |
| --- | --- | --- |
| Accessibility | 3/4 | Keyboard, labels, target sizes, contrast and reduced motion checked; no full screen-reader certification. |
| Performance | 3/4 | Shared playback polling, lazy images, bounded provider cache and short transitions; real account cold-cache performance still needs release observation. |
| Theming | 3/4 | Shared tinted surfaces and palette contrast checks; no claim that every possible custom-color combination is certified. |
| Responsive design | 3/4 | Five viewport/orientation combinations and three engines; physical touch-device acceptance remains. |
| Implementation integrity | 3/4 | Source detector clean; behavior has regression evidence and source originals remain distinct from local corrections. |
| Total | **15/20 — Good** | Owner visual acceptance and final device tests remain. |

## Coverage

Automated browser checks use isolated processes, temporary archives and synthetic files. They cover owner setup/login; Overview and time help; history and combined filters; artist/album/track rankings and music pages; period charts; Spotify import receipt, invalid rows and retries; General, Appearance, Connections and HTTPS instructions; Data & recovery; API keys; global Activities, sidebar and toasts; image/name editing and half-star ratings.

The responsive sweep visits twelve global routes and six music/edit routes at **390×844, 844×390, 768×1024, 1024×768 and 1440×1000**. It asserts that the page does not overflow and captures Overview, import, Connections and Activities. Existing tests also cover loading and empty archives, rejected saves, interrupted/declined/successful Spotify authorization, restore confirmation and restart, indeterminate progress, collapsed/resized sidebar, keyboard drawer dismissal and theme changes. New checks cover contrast across both themes, all palette accents and black/white custom accents, plus a single shared playback sample.

## Personal library data

Artist and album edit pages accept **PNG, JPEG and WebP**, at most **5 MB** per image. File uploads and pasted public HTTPS image URLs store image bytes in SQLite; the browser subsequently requests them from the instance. URL fetching rejects private/special addresses, pins validated DNS results, follows no redirects, forwards no credentials and enforces time and byte limits. Custom images take precedence and can be reset to Spotify artwork. Name/title corrections can be removed independently; source values remain available. Personal ratings support **1, 1.5, …, 5** and can be cleared.

Spotify artwork is fetched on demand through background image requests, with one download slot, catalogue requests spaced by at least three seconds and coalescing per display group. Successful samples last seven days; unavailable samples are retried after one day. The provider cache is capped at 128 MB and expired rows are cleaned by the retention worker. Owner images do not expire. Portable backups contain personal images, corrections and ratings. History JSON/CSV exports remain listening-history exports.

The [Spotify artist](https://developer.spotify.com/documentation/web-api/reference/get-an-artist), [album](https://developer.spotify.com/documentation/web-api/reference/get-an-album) and [track](https://developer.spotify.com/documentation/web-api/reference/get-track) endpoints provide image metadata. The cache is temporary under [Spotify's storage terms](https://developer.spotify.com/terms). Missing IDs, ambiguous imported artist credits, unavailable endpoints and failed downloads retain a placeholder and the manual-image alternative. An optional artwork endpoint failure does not mark otherwise healthy collection authorization as broken.

## Remaining acceptance

Physical Firefox on PC/Mac and Safari on iPhone/iPad, real account artwork availability, live OAuth, server behavior and the owner's visual acceptance of Connections remain part of the single final release test. Browser-engine automation and emulated viewports are additional evidence; they do not attest those physical tests. The GUI ticket stays open pending the requested visual feedback.

## Local verification record

Screenshots from temporary archives and synthetic music:

- [Connections, desktop](screenshots/gui-connections-desktop.png)
- [Import, phone](screenshots/gui-import-mobile.png)
- [Activities, phone](screenshots/gui-activities-mobile.png)
- [Artist edit and rating](screenshots/gui-edit-artist.png)
- [Dark theme with a purple accent](screenshots/gui-overview-dark.png)
- [Login wordmark](screenshots/gui-login.png)

- `npm run typecheck` and `npm run build` pass; committed browser contracts and embedded assets are current.
- `cargo test --locked --all-targets` passes; additional restored ListenBrainz coverage includes restart, pagination, invalid rows, ambiguous identities and compact source retention. The three synthetic load cases and the external ListenBrainz sample case remain opt-in.
- `cargo clippy --locked --all-targets -- -D warnings`, `cargo fmt --check` and `git diff --check` pass.
- The final browser suite passes 16 scenarios per engine on Chromium, Firefox and WebKit, including ListenBrainz upload/reimport. Additional focused checks cover the isolated ListenBrainz process and expanded music/edit viewport sweep. Physical device acceptance remains in the final release ticket.

The reproduced pre-fix browser failures were an absent drop target and a 28 px time-help target. Post-fix tests import actual dropped files, open and dismiss time help, edit/reset names and ratings, persist and reset artwork, reject private image URLs, check all release routes at five sizes and verify playback request sharing and text contrast.

## Owner feedback after the local preview

The owner reported oversized time help, plain disclosure arrows, missing username/password guidance, unclear Spotify status/errors, missing images and rankings that repeated the top tracks under artist/album labels.

The ranking defect was reproduced on the imported archive and in an isolated two-track regression: provisional artist/album identities were scoped to each track. The owner explicitly chose **local grouping by source name with reversible separation**, avoiding catalogue requests for identity consolidation. This supersedes the earlier ID-only restriction for this consultation feature; it does not identify namesakes as the same source identity. Artists group by trimmed source name (ASCII case ignored); albums additionally require the same source artist names. Tracks and original observations remain unchanged. Group IDs are persistent, and edit pages support source-entry search, selection, separation into another group and rejoining the default name group. Choices survive reimport and portable backup. Existing raw entity URLs resolve to their current group. Personal corrections/images/ratings belong to the display group; pre-existing source overrides remain stored. API clients must take a new full copy after the migration changes the archive generation.

Spotify artwork shares the display group cache and uses one download slot, with optional catalogue requests spaced by at least three seconds. Cached bytes remain local; no catalogue scan is needed for grouping. A temporary request limit, network or download failure no longer becomes a 24-hour missing-image entry. Old negative entries are discarded once, and failed browser images can recover after the cooldown. Explicit catalogue denial/not-found responses retain a bounded negative cache. Real Spotify availability still depends on that app/account and its request limits.

Connections uses expandable cards, a green badge for authorization verified on the last request, and a separate notice for automatic provider cooldowns with a human-readable wait. Generic form errors now have an icon, heading and message. Time help draws an 18 px glyph inside its existing 44 px touch target.

Owner setup now requires a username, confirmed password and live checks. New passwords require at least 15 characters, reject a local set of common/repeating/account-derived values, and retain the 1024-byte upper bound. Existing passwords continue to work; existing owners choose a username after signing in. Restore keeps the destination username and validates its replacement password. Password rules follow the length and blocklist approach in [NIST SP 800-63B-4](https://pages.nist.gov/800-63-4/sp800-63b/authenticators/#passwordver); this local rule set is not a breach-corpus check.

The local preview and ticket remain open for the owner's visual feedback. Physical device and live-service acceptance remain in the final release validation.


The subsequent feedback also covered the activity chart and history filters: short period labels, an unclipped readout outside the scroll area, accent bars, a removable segment filter, and a responsive filter panel with separate actions now replace those layouts. Synthetic yearly samples reproduce the clipped-label scenario without publishing private history.

Verification after these changes: 114 Rust tests pass, four opt-in cases remain ignored; TypeScript/Vite, release build, Clippy with warnings denied, rustfmt, Prettier and diff checks pass. 20 scenarios pass in Chromium and 20 in WebKit, including reversible grouping and readable yearly activity. The final time-help sizing was checked again with the relevant consultation/help/chart scenarios. Firefox automation cannot launch its local profile (Could not find profile folder), so this run makes no Firefox pass claim. The owner monitors the pushed GitHub checks; they were not polled.


[Updated yearly activity chart](screenshots/gui-activity-refined.png). Separating a single source entry also restores any older source-specific correction or custom image, retaining the group display choices as a fallback. A regression covers migration of a personal correction, separation, and backup/restore of that decision.

## Second local feedback pass — PR #47

The owner supplied eleven further findings in [GUI #26](https://github.com/itsmrma/scrobbl-rs/issues/26). The earlier source scan and overflow checks had not detected these alignment and loading problems.

- The activity report now has a padded return action with a back arrow and a 44 px target.
- Connections uses an explicit grid for title, status and collection. Removed the old blanket styles that changed nested icon spacing and font sizes. Native selects now use the same 44 px control treatment in Chromium and WebKit.
- History filters start collapsed, show the active selection, and remember their disclosure state in the browser. Closing them preserves filters, results and the URL.
- Artist, album and track headers share larger artwork (180 px desktop, 160 px tablet, 112 px phone), a title column and aligned edit/rating/provider actions.
- Track history omits repeated music metadata. Album history retains track and artist, without repeating the album. Both expose original UTC playback timestamp, assigned day/week, statistics eligibility, acquisition date and duration evidence. The album header also includes its artists.
- ListenBrainz presents separate file-import and public-fetch views, preserving entered values between them. Detailed account, storage, date and matching explanations use disclosures.

### Loading diagnosis and regression evidence

`cargo test --lib cold_artwork_does_not_queue -- --nocapture` initially failed after 500 ms: a cold artwork request remained queued behind the occupied provider slot. Slow image responses can consume the browser's available HTTP connections and delay subsequent data requests. The handler now returns a retryable, non-cacheable response immediately when a download or an identity is busy; cached/personal images bypass those download gates. The test passes and also checks that a busy slot creates no missing-image cache entry and does not block saved artwork.

The browser also queues visible or nearby artwork before starting its HTTP requests, releases cancelled cards, and starts the next image as soon as the current one finishes. This preserves sequential image progress without forcing every queued picture into the server's retry delay. `visible artwork waits in the client while music data remains available` failed with two concurrent cold image requests before the client queue; it now verifies one pending request while the chart is already visible.

The browser test `music charts start independently and back navigation restores the visited page while refreshing` initially failed with the album header deliberately held: the activity chart had not started. It now loads independently. Consultation keeps at most 64 reads for 60 seconds in session memory, immediately displays a recently visited selection and refreshes it in the background. Writes and session changes invalidate saved reads; responses started before invalidation cannot repopulate the cache. A delayed album response verifies browser Back restores the visited header and chart within the test's 500 ms limit.

On an isolated copy of the local archive, concurrent HTTP reads for music headers, activity and history took 20–329 ms in one sample, without Spotify credentials or provider requests. Direct activity queries measured about 10–212 ms when idle and up to 1.4 seconds during concurrent builds/browser runs. These observations do not certify a production latency threshold or reproduce the complete reported twenty-second live-provider wait. The opt-in read-only measurement is:

```sh
SCROBBL_BROWSE_BENCH_DB=/path/to/archive.db cargo test --test consultation music_browse_latency -- --ignored --nocapture
```

The full automated suite passes 115 Rust tests with five opt-in cases skipped (including that measurement), plus the separately executed read-only measurement. Chromium and WebKit each pass 23 browser scenarios. Focused layout/navigation checks are repeated after the final native-select refinement. TypeScript/Vite, release build, Clippy with warnings denied, rustfmt, Prettier and diff checks pass. Firefox's previously recorded local profile limitation remains; no new Firefox pass is claimed.

Synthetic screenshot evidence: [track](screenshots/gui-music-track-refined.png), [album](screenshots/gui-music-album-refined.png), [artist](screenshots/gui-music-artist-refined.png), [Connections](screenshots/gui-connections-aligned.png), [activity return action](screenshots/gui-activity-return.png), [ListenBrainz files](screenshots/gui-listenbrainz-files.png), [ListenBrainz public fetch](screenshots/gui-listenbrainz-public.png).

The remaining live-provider cold/warm-cache timing and physical PC/Mac/iOS scenarios are consolidated in [final release validation #31](https://github.com/itsmrma/scrobbl-rs/issues/31). GUI #26 remains open for owner visual acceptance; the GitHub CI rerun is monitored by the owner after handoff.
