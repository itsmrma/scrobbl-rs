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

Artist and album edit pages accept **PNG, JPEG and WebP**, at most **5 MB** per image. File uploads and pasted public HTTPS image URLs store image bytes in SQLite; the browser subsequently requests them from the instance. URL fetching rejects private/special addresses, pins validated DNS results, follows no redirects, forwards no credentials and enforces time and byte limits. Custom images take precedence and can be reset to Spotify artwork. Name/title corrections can be removed independently; source values remain available. Personal ratings support **0.5, 1, 1.5, …, 5** and can be cleared.

Spotify artwork is fetched on demand and stored permanently in SQLite after a successful download, per the owner's explicit fifth-pass instruction. Images do not expire or get evicted by a cache budget. Missing images are retried after one day; transient failures are not saved as missing. One download slot and the shared provider cooldown remain; optional catalogue requests are now spaced by one second, instead of the previous three-second local wait. Custom images keep precedence. Portable backups include saved provider and personal images, corrections and ratings. History JSON/CSV exports remain listening-history exports.

The [Spotify artist](https://developer.spotify.com/documentation/web-api/reference/get-an-artist), [album](https://developer.spotify.com/documentation/web-api/reference/get-an-album) and [track](https://developer.spotify.com/documentation/web-api/reference/get-track) endpoints provide image metadata. Missing IDs, ambiguous imported artist credits, unavailable endpoints and failed downloads retain a placeholder and the manual-image alternative. An optional artwork failure does not mark otherwise healthy collection authorization as broken.

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

Owner setup requires a username, confirmed password and live checks. The fourth local feedback pass supersedes the earlier 15-character rule: new passwords require at least 8 characters, a letter, a number and a special character; three-character ascending/descending alphabet, numeric and keyboard sequences and repeated patterns are rejected, as are normalized account-derived values. The 1024-byte upper bound remains. Existing sign-in passwords continue to work. Restore applies the same new-password policy and retains the destination username.

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

## Third local feedback pass — compact controls and fluid pages

The owner's twelve additional findings are addressed in PR #47; the branch also incorporates `main` through `91e9b1e` (#49).

- Results per page now belongs to the footer of every history/ranking list, including embedded lists on music pages. It stays available with filters collapsed and keeps the existing shared size preference. Singular record/result labels are corrected.
- Artist and album suggestions match their own names, rather than associated track titles. Suggestions wait for a short typing pause, show a separate subtitle, fit the field width, and close on focus leaving the control or Escape. A selected identity occupies the input itself with an adjacent clear action. Keyboard arrows navigate suggestions. Safari keeps input focus through a pointer selection so blur cannot remove the option before its click.
- History filters animate open/closed over 160 ms; reduced-motion preferences disable that duration. Buttons receive a short hover/pressed response without changing their layout.
- Ratings sit between the music name and edit action. Stars preview whole and half ratings on hover, support half-step changes from the keyboard and retain a clear action; the redundant select is removed from music and edit pages.
- Track history uses compact date/source, playback time and details columns. Expanded details retain all evidence fields. Music edits use the available page width; track details arrange name/source and rating alongside each other on desktop. Artwork preview and controls share a compact layout, with save/reset actions together. Back links no longer underline their arrows.
- Source-entry checkboxes have explicit 18 px dimensions and sit beside their text in a responsive multi-column list. The inherited column label and full-width input styles caused the large empty rows in the reported screenshot.
- Removed the fixed main/edit/settings width caps. Wide layout checks cover 1440 and 2400 CSS pixels. Album and track rankings now both offer independent, persisted List/Grid views, including on the artist page.

### Reproduction and verification

The initial command `npx playwright test tests/gui-audit.spec.cjs -g 'artist autocomplete|source-entry checkboxes|page size lives|stars preview' --reporter=list` failed all four cases: a track-title query returned an unrelated artist, the selection occupied a second row, the size selector was hidden with filters, and the fourth star stayed unfilled on a half-star hover. These cases pass after the fixes. Additional checks cover 18 px source checkboxes, compact track rows, un-underlined back links, wide edit pages and independent album/track views.

WebKit additionally reproduced lost pointer selections: the selected field remained `Seed` rather than `Seed artist`, and selecting an album failed to narrow 135 records to two. Preserving input focus on option pointer down fixes both; the original two scenarios pass on replay.

The complete browser suite has 28 scenarios per engine: Chromium's other 27 passed before updating an old `1 records` assertion, and the corrected local scenario passes; WebKit's other 26 passed before the focus fix, and the two affected scenarios pass on replay. The consultation and GUI scenarios are also rerun in Chromium after that focus fix. Rust's full suite, Clippy with warnings denied, rustfmt, TypeScript/Vite, release build, Prettier and diff checks pass. Firefox's previously recorded local profile limitation remains; this pass makes no new Firefox claim. CI is handed to the owner after pushing, without polling.

Synthetic screenshot evidence: [compact artist edit](screenshots/gui-edit-artist-compact.png), [wide track edit](screenshots/gui-edit-track-wide.png), [artist album/track views](screenshots/gui-artist-album-views.png), [compact track history](screenshots/gui-track-history-compact.png).

### Reported imported artist name

The original Spotify ZIP was inspected read-only. Row 12034 of `Streaming_History_Audio_2024.json` already contains `(((())))` as the artist of `Living Life In The Night - 8D Audio`. The stored value matches the source exactly; the owner confirmed that the punctuation name had been mistaken for corruption. No import repair or name replacement is needed.

The local preview is rebuilt and restarted using the existing archive, with its listen count unchanged. Physical PC/Mac/iOS checks remain consolidated in #31: zoom at 80%, wide/narrow layouts, pointer/touch selection, half-star preview/selection, filter motion and reduced motion, and List/Grid persistence. GUI #26 stays open for the owner's local visual acceptance.


## Fourth local feedback pass — PR #47

All twelve findings are tracked in [GUI #26](https://github.com/itsmrma/scrobbl-rs/issues/26). This pass changes shared layouts as well as individual controls:

- The password visibility button stays 44 px wide. Setup and restore use the requested 8-character/composition/sequence/username policy on the client and server. Confirmation uses the same icon, type size and colour as the checklist.
- Copy redirect URI, Copy API URL and Copy key share an adjacent live check, cleared after three seconds. Clipboard failures appear beside the action and create no success toast. Copying no longer triggers unrelated settings/status reloads.
- Connections aligns its title, status icon/text and disclosure control. Settings and API keys separate labels, inputs, actions and explanatory text; the key-creation fields share a desktop row and stack on narrow screens.
- Musical page covers grow to 200–280 px on desktop, 160 px on tablet and 112 px on narrow phones. Ratings retain an accessible legend without the visible “Your rating” line. Edit and Spotify actions share sizing, spacing and border treatment; Spotify icons are not underlined.
- Metrics use compact shared cards and become label/value rows on phones. Album grids cap card width at 220 px; artist albums/tracks use adjacent independent panels on wide screens. The whole musical card or ranking row opens its detail page through the existing accessible link, preserving keyboard and modified-click behaviour.
- The sidebar playback card includes artwork, title/artist, a progress bar, sample-based elapsed time and duration. The whole playing card links to Spotify; the narrow sidebar keeps the artwork and existing accessible tooltip. Empty and unavailable states retain readable labels.

### Artwork diagnosis

`npx playwright test tests/gui-audit.spec.cjs -g 'password field fills'` failed before the fix with **26 px** input width versus **292.5 px** minimum. It now passes: the form's full-width button rule applies only to the submit action.

`npx playwright test tests/gui-audit.spec.cjs -g 'cached artwork bypasses'` failed before the fix because no cached image progressed within the two-second window while the first cold request was held. The minimized browser scenario now passes. Visible artwork first probes `?cached=true`: authenticated SQLite-only reads bypass the client download queue. Only cache misses enter the single cold-download queue; data reads stay available and provider cooldowns remain enforced.

Artist and album artwork can reuse public track metadata for five minutes (maximum 64 entries), avoiding another provider request/cooldown for the same imported track. No errors or credentials enter that cache. The regression repeats the real catalogue lookup within 500 ms with only one provider response supplied.

Album parsing previously selected the last Spotify image, usually the smallest thumbnail. Both parser and artwork retrieval now choose the largest reported width, independently of response order, and artwork retrieval no longer trusts the old stored thumbnail URL. Migration 13 removes only disposable Spotify album cache entries so old low-resolution bytes are replaced; custom images, original metadata and listens are preserved. Cache-probe and migration tests verify that custom images remain readable without contacting Spotify.

The first uncached image still depends on Spotify and its request limit; these isolated tests establish removal of local serialization and duplicate metadata work, not a promise about live provider latency.

Synthetic screenshot evidence: [desktop artist](screenshots/gui-artist-balanced.png), [phone artist](screenshots/gui-artist-balanced-mobile.png), [password setup](screenshots/gui-password-policy.png), [copy and API spacing](screenshots/gui-api-spacing.png), [sidebar playback](screenshots/gui-sidebar-playback.png). The temporary API secret is masked. No private listening history is included.

Verification for this pass: the full Rust suite passes (four opt-in workloads remain ignored); Clippy with warnings denied, rustfmt, TypeScript/Vite, release build, Prettier for changed files and diff checks pass. All **34 Chromium** and **34 WebKit** scenarios pass. The interrupted WebKit run was restarted to obtain a complete result. No new Firefox claim is made; its previously documented local profile limit remains. The preview at `http://127.0.0.1:3210` uses the existing archive, with 54 listens and 54 observations before and after restart, and schema 13. GUI #26 remains open for local visual feedback; final physical-device/live-provider checks stay in #31. The owner follows CI after push; it was not awaited or monitored.


## Fifth local feedback pass — PR #47

The fourteen findings and the subsequent instruction to save Spotify images once are tracked in [GUI #26](https://github.com/itsmrma/scrobbl-rs/issues/26).

- Personal ratings now include **0.5/5**, by pointer and keyboard, and persist through the server. The first-star browser regression initially returned `star` instead of `star_half`; it now passes, including reload. The glossary records the owner's updated range; missing ratings remain distinct from zero.
- Inspected the three-size browser preference in the owner's [music library](https://github.com/itsmrma/uni-musiclib-project/blob/main/library/templates/library/album_list.html). Ranking grids default to small 140 px cards and provide minus/plus controls for 140/180/220 px sizes, persisted independently for artists/albums/tracks. Card metadata and search/view controls are compact.
- Music page Period/Rank by controls sit beside the title/actions. Headers show “Album by” and “Track by”. Covers, metrics and history rows use tighter proportions; redundant timezone/threshold explanation stays in expandable details/filters. Album rows remain under 100 px collapsed in the regression. The last ranking row keeps bottom padding and inset focus treatment; paired album/track panels share a bottom baseline and scroll long results independently.
- Album and artist ranking panels offer weighted collages for 9/16/25 items from the current page and selection, sized by listens or listening time. Each tile opens its local page. PNG export uses the artwork currently displayed; a browser test verifies tile coverage/no overlap and the downloaded 1200×1320 PNG. No external publication is performed.
- Sidebar playback labels share a left baseline, the collapse button has no hover outline/shadow, and the wordmark's dot uses the accent. Icons reserve one em before their font loads: a responsive regression exposed a fallback icon word extending the phone page to 418 px; it now fits 390 px without waiting for font loading.
- Now Playing opens the existing local track through its Spotify identity. Cached samples resolve the current archive identity again, so a track collected after sampling becomes linked without another Spotify request. Tracks awaiting collection open an internal live page; playback never creates a listen. The backend regression preserves a zero-listen count while resolving the local metadata identity.
- Connections uses a provider identity block, collection subtitle and separate verification badge. Their vertical centers are checked within one pixel. The old literal-text assertion was updated for the new two-line badge and the full state/toast/sidebar scenario was rerun.

### Image persistence and cold-loading evidence

The previous local three-second catalogue delay was reproduced by `cargo test --locked --lib cold_catalogue_images`: its second cold request exceeded 1800 ms. With a one-second local spacing the test passes; Spotify's shared 429/retry state remains enforced. Cold work now starts only for visible images, checks visibility again when its queue slot arrives and prioritizes the music-page cover. Cache reads remain independent of that queue.

Migration 14 keeps existing successful Spotify images by setting their expiry to NULL. New successful downloads are also permanent, with the existing per-image limit; the old seven-day expiry and 128 MB eviction budget no longer apply. Confirmed absences still expire and errors remain retryable. A regression starts with an old expired provider image, migrates it, runs cleanup, restarts the instance without Spotify and verifies identical bytes through the artwork endpoint and a portable backup/restore.

Synthetic screenshots: [adjustable cards and compact history](screenshots/gui-music-density.png), [album collage](screenshots/gui-album-collage.png), [Connections](screenshots/gui-connections-aligned.png), [sidebar](screenshots/gui-sidebar-aligned.png), [album/history layout](screenshots/gui-album-history-compact.png).

Verification for this pass: the Rust suite passes, with the four opt-in workloads still ignored; the final library-unit rerun also covers a cached Now Playing sample acquiring its local identity. Clippy with warnings denied, rustfmt, TypeScript/Vite, release build, Prettier on changed files and diff checks pass. All **37 Chromium scenarios** are covered with focused reruns after updating the old one-line Connected text expectation and the final playback link handling; the complete **37-scenario WebKit** suite passes. No new physical-device or Firefox result is claimed. The existing preview archive migrated to schema 14 with all 22 saved Spotify images retained and no positive-image expiry. Its 65 pre-existing listens were preserved; background collection added one new listen after restart. GUI #26 remains open for visual acceptance; CI after push stays with the owner and physical/live-service checks remain in #31.


## Sixth local feedback pass — Overview, history and activities

The owner's further visual feedback remains tracked in [GUI #26](https://github.com/itsmrma/scrobbl-rs/issues/26), in the existing PR #47.

- Music pages place the title, rating, actions and Period/Rank by controls in one column. Desktop artwork stretches from the top of that column through the bottom of its controls. Phone layouts stack the artwork and introduction.
- Listening history restores a disclosure arrow. Its 44 px target remains anchored to the row while the evidence expands beneath it; clicking the same coordinates closes it again. Collapsed rows remain compact. Wide contextual history uses its middle column for the already-stored archive date; narrow layouts retain that date in the full disclosure.
- Sidebar collapse hover uses a small accent-tinted surface and border. The listening-time information glyph is 14 px inside its 44 px target, with a restrained circle hover instead of the shared button shadow.
- The playback artwork and text share a centered grid, consistent line heights and left edges. Playback times use opposite edges of the same progress bar; collapsed navigation hides the text.
- Overview removes its duplicate playback card, keeps the existing totals, rankings, activity chart and recent history, and adds proportional mini-bars to its top lists. The bars follow the selected ranking metric (listens or listening time). Filters are compact; top-list titles and their first hover targets have separate space.
- Import guidance says which Spotify ZIP/JSON files to choose, uses MB for useful size limits and explains that ZIPs can stay zipped. Internal byte/JSON-row terminology is removed from this help.
- Activities has a compact running/review summary, separate history cards and pagination below the results. Sidebar running cards reuse the same progress component. Details lead with the report name, state, progress and four result counts, then context, actions and files; detailed counts remain available in a disclosure. Activities uses the same selected navigation treatment as the other pages, including detail routes.

### Reproduction and verification

The initial command `npx playwright test tests/gui-audit.spec.cjs -g 'music header and history disclosure'` reproduced **882 px of horizontal movement** when the disclosure opened. Its minimized regression now checks unchanged x/y coordinates on desktop and phone, closes at the original pointer coordinates, and measures the artwork against the title/control column. The root cause was the open details element moving into a different grid column; its summary now stays anchored to the containing row.

All **39 Chromium** and **39 WebKit** scenarios pass in complete runs. The Rust suite, Clippy with warnings denied, rustfmt, TypeScript/Vite, release build, Prettier for changed files and diff checks pass. Earlier failed browser expectations for the old import-help text and duplicate progress text were updated for the requested UI; the final Chromium run passes completely.

Synthetic evidence comes from isolated fixtures, including a five-item ranking and a running import, with no private listening history: [Overview](screenshots/gui-overview-refined.png), [music header](screenshots/gui-music-header-refined.png), [track history](screenshots/gui-track-history-stationary.png), [Activities](screenshots/gui-activities-refined.png), [phone Activities](screenshots/gui-activities-mobile-refined.png), [report](screenshots/gui-activity-report-refined.png), [phone report](screenshots/gui-activity-report-mobile.png), [import help](screenshots/gui-import-help.png), [sidebar](screenshots/gui-sidebar-refined.png).

The release preview at http://127.0.0.1:3210 was restarted only after confirming no activity was running. Read-only fingerprints confirm all **158,645 pre-existing listen identities** and **23 identical saved image payloads** remain present. No new schema change is introduced in this pass. GUI #26 stays open for visual feedback; physical devices/live services remain in #31, and the owner follows CI after push.


## Seventh local feedback pass — square artwork and visible downloads

The owner's next refinements remain in GUI #26 and PR #47.

- Current-period notices use a separate position below the filters. Changing All time to This month no longer shifts either Overview select. The initial regression measured about 230 px of horizontal movement.
- Music artwork uses a 1:1 frame with contain sizing. On desktop the square follows the natural introduction height, capped to preserve room for the text; narrow layouts stack it. Uploaded previews and collage frames also retain square proportions and show the whole source image.
- Collages pack full-width bands of squares, including dominant squares alongside smaller grids. A bounded deterministic layout search approximates the selected listening shares while keeping the overall mosaic compact. Each image stays square, with no gaps or overlap; the PNG height follows the mosaic and its title/footer. The active Collage control has a persistent outline.
- Collaborating artist links flow inline. Album history omits the repeated artist and increases the track title. Filters and their summary share one left edge. Expanded facts reuse the closed row's column sizes and continuous full-width separators.
- Native disclosures animate for 160 ms; controlled history filters keep their existing animation. The summary stays anchored, rapid reversals retain the original overflow style, and reduced-motion preferences use the immediate native action.
- Side-by-side edit panels stretch to the same height. Back to album/artist/track uses the shared secondary button interactions.
- The real cold-artwork queue publishes progress to Activities and its sidebar count/card. A started fetch continues when its image card unmounts; the resulting server cache read remains independent of the cold queue. Cache hits do not start a download activity. This is live work from the current browser session, rather than an additional durable server job; image bytes retain their existing permanent database storage.

Verification: the full 43-scenario Chromium run passes, followed by five focused passes covering the new weighted-collage case and the four new regressions (44 distinct scenarios in total). WebKit's other 43 scenarios pass; consultation initially reproduced a ResizeObserver delivery loop when the square resized wrapped text. Applying changed sizes on the next animation frame fixes that loop. Consultation and six affected geometry/download/collage cases pass on both WebKit and Chromium after the fix, covering all 44 distinct scenarios in each engine. The Rust suite, Clippy with warnings denied, rustfmt, TypeScript/Vite and release build pass. Geometry checks verify square frames, stable filter coordinates, inline collaborator baselines, equal edit-panel heights, aligned evidence, an intermediate animation height and reduced-motion behaviour. The download scenario holds a cold response, opens Activities, then verifies completion and a cache-only return. Collages are checked at 9/16/25 tiles, with differing sizes, no overlap and a valid PNG export.

Synthetic screenshots: [period notice](screenshots/gui-period-notice-stable.png), [square header](screenshots/gui-artwork-square.png), [inline collaborators](screenshots/gui-history-collaborators-inline.png), [album edit](screenshots/gui-album-edit-equal-panels.png), [artwork activity](screenshots/gui-artwork-activity.png), [aligned facts](screenshots/gui-history-aligned-evidence.png), [weighted square collage](screenshots/gui-collage-square-weighted.png).

The preview was restarted with no active server job. Read-only fingerprints confirm all 158,648 pre-existing listen identities and all 77 image payloads remain intact after the final restart. No new migration is introduced. GUI #26 remains open for visual feedback; physical devices/live-provider validation remains in #31 and CI is followed by the owner after push.

## Eighth owner pass: actionable review and compact timelines — 23 October 2026

The latest request is recorded in [GUI #26](https://github.com/itsmrma/scrobbl-rs/issues/26). This pass keeps the existing archive identity and makes identification and exceptions lead; metadata supports them and repetitive successful work collapses into a secondary layer.

| Finding | Final behavior and evidence |
| --- | --- |
| History artwork stayed 38 px even when metadata made rows much taller. | A shared observer measures only the collapsed date/source block. Cover and title grow or shrink within bounds; opening evidence does not enlarge them. The initial browser test failed at 38 px and now verifies both growth and shrinkage. Squares retain the full image. |
| Connections pointed at flagged history with no resolution actions. | `/review`, linked from history and Connections, compares nearby flagged records. No choice is preselected. Keep all separately, or select one and move the other displayed records to trash. Original observations/variants remain; persistent paginated decisions expose Undo. Owner session, CSRF, maintenance fencing, current revisions and transactional publication protect writes. Undo preserves later annotations/calendar changes, but refuses new source evidence or intervening review/trash changes. No real owner record was resolved by the agent. |
| Monthly timelines needed horizontal scrolling on desktop. | Omit zero months and combine adjacent low-activity months to fit the measured width, preserving peaks while possible. Every displayed total is exact; the label and drilldown use the complete range. An 84-month fixture fits 14 desktop bars and preserves its 600-listen peak. Mobile keeps scrolling individual non-empty months. |
| Successful background collection repeated dozens of large cards. | One initially collapsed collection group contains dated result rows linked to reports. Active, failed and partial jobs remain prominent. Reports fill the available content width; paragraphs retain readable measure. |
| View controls and Settings differed in spacing and grouping. | List/Grid/Collage share one control group; grid sizing follows it. Settings share panel width, heading spacing, field/action gaps and mobile insets. Private archive was removed from the header. |
| CI Docker setup failed with HTTP 400. | Disposable setup and restored-owner passwords in the smoke script now meet the requested number/special-character policy. No security requirement was weakened. Docker is verified by the Linux CI job; no local Docker runtime is installed. |
| CI WebKit reported access-control errors for pending artwork on document teardown. | Artwork downloads now participate in the same explicit page-exit cancellation as JSON requests, while surviving SPA navigation. The failing consultation path passes locally in WebKit; no console-error filter was added. |

### Impeccable follow-through

Used the same pinned Impeccable 4.7.0 source as the original audit. `context` ran once. Applied the `audit`, `layout`, `adapt`, `clarify`, `harden`, `optimize` and `polish` references to the requested surfaces. The layout reference's two isolated assessments separated visual/source hierarchy from the mechanical scan. The final visual assessment found no layout blocker in scope; five fresh desktop Settings captures and mobile General supplement the chart, comparison and Activities captures.

The layout-scoped detector initially returned `[]`. The full source detector then flagged the pre-existing thick left accent on running activity cards. Replaced it with a subtle whole-surface/border state and removed a similar new selection stripe. The subsequent `detect --json frontend/src` returned `[]`, exit 0. This is source detector evidence, not a claim of complete WCAG certification. New product/branding, `bolder`, `overdrive` and live variant generation are not part of this refinement request.

| Dimension | Score | Verified scope and limit |
| --- | --- | --- |
| Accessibility | 3/4 | Native radio/disclosure controls, labels, focus, error/status feedback, readable palette tests and reduced motion; full physical screen-reader acceptance remains in #31. |
| Performance | 3/4 | Bounded review pages, indexed candidate lookup, memoized monthly grouping, batched observer writes and no new runtime dependency. Fifty candidate searches across the 613 real flagged records took about 30 ms in a read-only check; this is not a full network/page benchmark. |
| Theming | 4/4 | New surfaces use existing semantic tokens and radio accent; light/dark synthetic captures retain the established palette. |
| Responsive design | 3/4 | Compact desktop chart and report, mobile comparison and Settings with no horizontal page overflow; physical devices remain in the final test. |
| Implementation integrity | 4/4 | Clean final source detector; decisions retain originals and publish canonical archive changes. No invented counts, progress or automatic duplicate deletions. |
| Total | **17/20 — Good** | No known blocker in the requested layout scope. The score is evidence-bounded, not a promise of flawless design. |

Synthetic captures: [duplicate comparison](screenshots/gui-duplicate-review.png), [mobile comparison](screenshots/gui-duplicate-review-mobile.png), [monthly timeline](screenshots/gui-compact-monthly-chart.png), [grouped collection](screenshots/gui-grouped-collection-runs.png), [full-width report](screenshots/gui-activity-full-width.png), [General](screenshots/gui-settings-general-uniform.png), [Appearance](screenshots/gui-settings-appearance-uniform.png), [Connections](screenshots/gui-settings-connections-uniform.png), [Data](screenshots/gui-settings-data-uniform.png), [API keys](screenshots/gui-settings-api-keys-uniform.png), [mobile General](screenshots/gui-settings-general-mobile-uniform.png).

Local preview restarted with all **160,818 pre-existing listen IDs** and identical bytes for **101 saved images** confirmed. Embedded JavaScript/CSS hashes match the built files. No new schema migration in this pass. The branch incorporates the five current main updates. GUI #26 remains open for owner acceptance; this request explicitly includes checking the final CI/merge state before handoff.

The local full browser runs covered 48 scenarios each: all 48 Chromium passed; WebKit passed 47 and exposed a separate export-test race. The trace showed its POST cancelled by the test's immediate hard navigation before server receipt, leaving no export. The test now waits for the durable request acknowledgement before checking that generation survives navigation; no fixed sleep or longer assertion timeout was added. The CI WebKit consultation failure's artwork teardown path passes locally after explicit cancellation.

The duplicate-review regression additionally verifies atomic rollback when a later selected row is stale, preservation of a later personal title/rating during Undo, and rejection of Undo after new original source evidence. Final Rust/Clippy/format/build checks and the final CI result are recorded in #26 and the PR handoff.

CI run 40011388535 passed Linux/Windows backend, Docker smoke, all 48 Chromium and all 48 WebKit scenarios. Firefox passed 47/48 and exposed one geometry assertion reporting 43.9998779296875 for a 44 CSS-pixel select. The regression allows 0.01 px of layout-unit precision; the production minimum remains 44 px. The final commit's required checks are verified separately before handoff.

The next Windows run exposed a pre-existing rate-limit fixture crossing a minute boundary: it set only the request count, so an old window correctly returned 200 after resetting. The test now seeds the current window and retries only if a real rollover occurs during that request; it still requires 429 in a stable window, and separately requires 200 when an older window resets. Production rate-limit behavior is unchanged.


## Ninth owner pass — consistent controls, bulk review and background artwork

The follow-up remains in [GUI #26](https://github.com/itsmrma/scrobbl-rs/issues/26) and PR #47. Subsequent owner messages explicitly add server-side import artwork queuing and automatic chart scaling.

- A yellow warning beside the track title replaces the extra duplicate metadata line. Hover/focus explains it and links to review; its transparent target remains 44 px. The regression initially measured 17 px of extra row height. The primary history cells now have a separate grid, with their divider outside the animated details body; expanding evidence leaves that divider and arrow anchored.
- Automatic duplicate review previews isolated pairs less than 1,000 ms apart, with the same certain Spotify identity or complete exact title/album/ordered artist credits, known sources (including same-source import duplicates), consistent countability and no unresolved evidence conflict. Competing neighbours, repeats, distinct known identities and contradictory actual durations remain manual. The stronger time evidence wins; the other record goes to trash. Revalidation and publication are atomic, original observations stay intact, and one persistent Undo reverses the entire batch. No owner record was resolved by the agent.
- All native select menus, including the timezone datalist, use the shared site dropdown. It supports keyboard selection, typeahead, Escape/Tab/outside dismissal, scrollable options and an editable timezone search. Mobile Settings exposes a category dropdown. Popups are clamped to available screen space. Impeccable's final independent assessments found and corrected low-contrast selected text on light surfaces and a specificity conflict that still scaled pressed triggers.
- Grid columns distribute the remaining width through flexible tracks. The size controls stay mounted and reserve identical toolbar space outside Grid, with hidden controls inert. The album fixture verifies the last card in the first row reaches the grid edge. Generic icon controls share stationary accent feedback; the sidebar arrow no longer translates. ListenBrainz mode buttons use a uniform selected border/fill. Saved Spotify app credentials have a green check separately from account authorization. Activity state badges group with their titles.
- Charts have no manual Day/Week/Month/Year control. Short ranges request days; longer ranges request months, with years only beyond 25 years. Legacy GUI grain choices are ignored. Every displayed series is compacted to the measured width when necessary, including on phones. Totals, exact date ranges and drilldown remain intact; quiet adjacent intervals combine and significant peaks survive while space permits.
- Spotify and ListenBrainz import completion append missing album/artist images to one durable queue. Images already saved, custom images and valid negative-cache entries are excluded. The worker shares the existing single download slot and catalogue request pacing, adds a second between background images, respects persisted API/CDN Retry-After, waits for Spotify authorization, and checkpoints each item. It survives browser closure/restart and exposes progress, cancel/resume and paginated readable image names in Activities. Completed imports are independent of downloading. No schema migration was needed: the existing durable job/member structure represents the queue.

The preview query exposed an SQLite planner choice scanning Spotify namespace entries for each candidate. Selecting the existing entity identity index reduced the read-only lookup over the real archive to about 70 ms for 717 candidate rows. The same indexed lookup is used for artwork identity resolution. No real review or image-download batch was executed by the agent for this measurement.

Impeccable 4.7.0: context once, audit/layout/adapt/clarify/harden/optimize/polish follow-through, two isolated layout/mechanical assessments, and final source detector `[]` (exit 0). The source result supplements rendered and interaction checks; physical assistive-technology/device acceptance remains in #31. The broader established design and earlier evidence remain above.

Backend coverage includes stale-preview rejection without partial writes, bulk Undo, reimport stability, original preservation, queue coalescing, cache reuse, restart checkpoints, cancellation/maintenance fencing and a mocked Spotify 429 with a 1,200-second Retry-After. A previous import test counted members from every job; it now scopes its five source files to the import and separately verifies two queued artworks.

Preview restart: all **160,821 pre-existing listen IDs** and identical bytes for **136 saved images** are present, and embedded JavaScript/CSS hashes match the final build. The current browser/CI verification is recorded in #26 and the PR handoff. GUI acceptance remains open; no merge was performed.


Final audit follow-up: grouped date labels now include years when the displayed timeline crosses years. Automatic charts never request assigned-week buckets (daily/monthly/yearly defaults are regression-tested), so overlapping week assignments cannot make a grouped date-range link disagree with its total. The explicit weekly API remains compatible and retains its existing week-assignment drilldown tests. Firefox also exposed the timezone list covering Save after an exact value was typed; exact matches now close suggestions while prefix search and keyboard/touch selection remain available. CI run 40024633391 passed Linux/Windows backend, Docker smoke, and Chromium; Firefox and WebKit exposed a 1.7-pixel horizontal shift when the Period filter dropdown selected 'This month'. The Overview filter label and select controls now allocate a 140 px minimum width (matching the music period controls), and the period notice check allows subpixel coordinate precision. Manual review cards now dismiss optimistically without reloading or blocking the whole page; active track filters render as an inline chip with a circular dismiss button; artist ranking rows prevent composition wrapping to preserve equal row heights; and artist subtitles explicitly display the count of different tracks listened.

Synthetic evidence for this pass: [fluid grid](screenshots/gui-grid-fluid.png), [mobile dropdown](screenshots/gui-custom-dropdown-mobile.png), [compact warning](screenshots/gui-history-warning-compact.png), [server artwork queue](screenshots/gui-artwork-server-queue.png), [automatic mobile chart](screenshots/gui-chart-auto-mobile.png).

## PR #47 follow-up — 24 October 2026

The owner's last PR comment asks for a visible trash view in duplicate review and square artwork immediately on entering artist, album and track pages.

- Review now has Comparisons and Trash views. Trash lists removed listens with their music links, source, playback date/time, duration and removal date, in pages of 50. Restore decision uses the existing persistent Undo: it returns every record involved in that decision to review, including records on other pages. This scope is explained above the records. Session, CSRF, maintenance and changed-source-evidence checks remain in the existing restoration path. No automatic decision or restoration was executed on the owner's archive.
- Reproduced the artwork flash with a frame-by-frame browser test: the initial placeholder was 280 × 294.0625 px. The inherited grid `align-self: stretch` stretched the placeholder before the introduction measurement and image replacement. Setting the music cover to `align-self: start` preserves the square from its first frame without removing responsive sizing. The regression samples placeholder and delayed non-square source-image frames for artists/albums/tracks at 1440, 768 and 390 px.

Red checks before the fix: `npx playwright test tests/gui-audit.spec.cjs -g 'music artwork is square from|duplicate review supports' --reporter=list` failed on the missing Trash button and the non-square first frame. `cargo test --locked --test library duplicate_review_is_explicit_recoverable_and_keeps_originals` failed on the missing trash endpoint (404).

Validation: the full Rust all-targets suite passed (five opt-in workloads remain ignored), including authenticated trash listing, out-of-range paging, manual/automatic decision mapping and restoration. Build/TypeScript, release build, Clippy with warnings denied, rustfmt, Prettier and diff checks passed. All 44 Chromium GUI scenarios were covered: 43 passed in the full run; the remaining copy-action scenario hit an artifact-directory collision between concurrent browser runs and passed in isolation together with both new regressions. The three targeted scenarios (trash/recovery, existing artwork geometry and first-frame geometry) passed in WebKit and Firefox. The final Chromium rerun also covers the selected-view contrast/spacing refinement. Impeccable source detector returned `[]`, exit 0. Synthetic evidence: [trash desktop](screenshots/gui-review-trash.png), [trash mobile](screenshots/gui-review-trash-mobile.png).

Preview uses yesterday's existing review archive, outside the prototype `data/` directory. Read-only checks confirmed all 160,861 pre-existing listen IDs and identical bytes for all 171 saved images after restart; no schema migration was added. GUI #26 remains open for owner acceptance; physical-device/live-service checks remain in #31. The new push's CI is followed by the owner, without waiting for its rerun.

### Further owner feedback — compact review and page back

- Comparisons/Trash now share one compact toolbar with flagged history, refresh and the review count. The automatic-review card starts collapsed, keeps the match count visible, and reuses the app's native disclosure animation (160 ms, keyboard support and reduced-motion handling). The title, explanatory paragraphs and detailed counts all start at the same horizontal position; the nested action row no longer adds a second layer of padding. [Desktop with comparison records](screenshots/gui-review-compact.png), [phone](screenshots/gui-review-compact-mobile.png).
- Artist, album and track pages have a small Back action above the music header. Application history entries retain their page origin through reloads; Back skips filter-only entries on the current page, returning to the source page's exact filters. Native forward/back remains usable. Direct entries without an application predecessor fall back to that music category's rankings.
- Collection/import reports and duplicate review in Connections use the same secondary action-button style as Edit details, with icons and no link underlining. The equivalent standalone history action uses that style too; music-name links retain their contextual presentation.

Preview restart preserved all 160,861 pre-existing listen IDs and identical bytes for 174 saved images. Embedded JS/CSS match the final build. No schema or backend behavior changes were needed. Owner acceptance remains open in #26.

Validation: all **47 Chromium GUI scenarios** passed in one complete run. Five targeted scenarios passed in both **WebKit and Firefox**: compact review/disclosure alignment, page-back through filters/reload/forward/direct entry, styled report/review links, actual duplicate decisions/trash/Undo, and the pre-existing browser-back/cache flow. Disclosure coverage includes 160 ms motion, native keyboard activation, reduced motion and mobile overflow. TypeScript/Vite, release build, Prettier and diff checks passed; Impeccable source detection returned `[]`, exit 0. Browser runs use separate artifact directories. The owner follows the new push's CI; no merge or physical-device acceptance is claimed.

### Header placement follow-up

On the owner's next request, Review possible duplicates and Filters share the Listening history title row. The full filter form appears underneath only when expanded; its existing animation, saved open/closed preference and selected filters remain intact. Embedded music history retains its existing disclosure. Back now sits at the far right of the music title row, aligned with the top of the cover, without a separate row above the introduction. Header actions wrap on narrow screens.

Five targeted scenarios passed in each of Chromium, Firefox and WebKit: exact header placement and page-back, phone/tablet routes in both orientations, filter collapse/persistence and embedded history, compact filter selection, and first-frame square artwork. Build/TypeScript, release, Prettier, diff checks and Impeccable source detection (`[]`) passed. Synthetic evidence: [history title actions](screenshots/gui-history-heading-actions.png), [Back at right](screenshots/gui-music-back-right.png). Preview baseline: 160,862 existing listen IDs and 176 saved image payloads. No schema or backend changes; GUI acceptance stays open in #26.

### Listening patterns and CI follow-up

Music covers now start at the normal main-content inset, without the extra 16 px music-header margin. Period and Rank by match the respective Edit details and View on Spotify button widths, including their shared right edge; controls adapt to narrow layouts and custom date selection.

Artist, album and track pages share one compact Listening patterns panel. Its selector switches between start hour, weekday, reported device/app platform, shuffle, offline, skipped, listening country, playback start/end reason and duration evidence. Hour/day charts are fixed-height; categories are bounded to 29 values plus a combined remainder. Counts use the same canonical selection and counted-listen policy as the music page. Each listen contributes once per view; unknown and conflicting facts remain explicit, and coverage is computed before category compaction. Hours/weekdays follow each assigned IANA timezone, including repeated hours at DST transitions. Pointer, touch and focus expose the selected bar's count and share.

Migration 15 adds a compact context object to observation variants. Only selected string/boolean Spotify export fields are retained; IP addresses, usernames and full raw rows are excluded. Reimporting the original export fills contexts for matching fingerprints without replacing listen IDs or adding duplicates. Earlier compact archives cannot reconstruct discarded device facts without that source export; the panel explains this. Snapshot/backup storage preserves the fields, and JSON export includes them with their original variant. A regression checks DST, missing/false/conflicting values, authentication, scoped artist/album/track totals, source/date/trash predicates, enrichment on replay and exported context without identifying raw fields.

The prior CI failure was reproduced by its logs: three standalone tests still clicked the hidden legacy filter summary. Those tests now use the visible Filters action. All 58 Chromium scenarios passed locally. The WebKit set passed 51/52 before exposing a form-initialization race that submitted a source name concatenated with a personal name; initializing name/rating in a layout effect prevents interaction before initialization. That full edit/image/rating path subsequently passed five consecutive WebKit repetitions and a Firefox run. The new patterns/header scenario passed all three engines. Rust all-targets, the additional snapshot/export regression, Clippy with warnings denied, TypeScript/Vite, release, rustfmt, Prettier, diff checks and Impeccable detection (`[]`) passed. The final GitHub run and merge state are recorded in #26 and the PR after verification.

[Synthetic listening-patterns page](screenshots/gui-listening-patterns.png). The review preview migrated with its verified pre-upgrade copy, preserving all 160,866 existing listen IDs and identical payloads for 176 images; served JS/CSS match the final build. No owner record was resolved or reimported by the agent. GUI acceptance and physical/live-provider validation remain open in #26/#31.
