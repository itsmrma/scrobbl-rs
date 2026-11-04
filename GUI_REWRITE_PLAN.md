# `scrobbl-rs` Ground-Up GUI Rewrite & Standardization Plan

## Audit Findings (Issues #155, #158, #163 & Layout Bloat)

1. **Issue #158 (Invisible Startup / Boot / Onboarding Screen):**
   - **Root cause:** `frontend/index.html` rendered a minimal pre-splash div, and `mount()` in `frontend/src/main.tsx` checked `/api/boot-status`. Because local SQLite opens in milliseconds, `/api/boot-status` immediately returned `ready: true`, causing React to mount `<App />` and overwrite the splash screen with grey skeleton placeholders (`<AppShellSkeleton>`) in `< 5ms` while `/api/session` and `/api/v1/preferences` loaded. Furthermore, `src/boot.rs` (`standalone_splash_html`) and `frontend/index.html` only rendered a tiny centered wordmark floating in an empty viewport.
   - **Fix:** Replaced early bootstrap fallbacks with a rich, viewport-balanced **Precision Atelier Startup Screen** shared across `src/boot.rs`, `frontend/index.html`, `frontend/src/startup-screen.tsx`, `frontend/src/main.tsx`, and `frontend/src/lifecycle-screen.tsx`, surfacing real initialization stages (`HTTP listener`, `SQLite archive`, `Session security`, `Workspace preferences`), `?boot=preview` inspection, and a `"Preview startup screen"` trigger in Settings → Application.

2. **Issue #163 (Practically Unnoticeable Row Separation & Hierarchy):**
   - **Root cause:** `.ranking-row` and `.listen-row` were assigned `background: var(--surface)` inside `.panel` containers whose background was also `var(--surface)` (`1.00:1` contrast) with `border-bottom: none`, making rows visually indistinguishable from the panel background until hovered.
   - **Fix:** Introduced dedicated `--row-bg`, `--row-border`, `--row-hover`, and `--row-hover-border` OKLab surface tokens that visibly contrast against `--surface` in both dark and light modes, paired with `4px` vertical separation, `6px` border-radius, crisp `tabular-nums` columns, desktop hover elevation + left accent edge bar, and `160ms` micro-transitions (`0s` under `prefers-reduced-motion: reduce`).

3. **Issue #155 (Missing Interactive Image Picker & Uncoordinated Artwork Queue):**
   - **Root cause:** No interactive provider search / visual image picker existed in the GUI (only raw file/URL inputs buried in `/music/{kind}/{id}/edit`, with zero Track custom image access on track headers), and `frontend/src/artwork-activity.ts` (`queueArtwork`) continued firing browse-time artwork downloads even while a full-library `artwork` job was running on the server.
   - **Fix:**
     - Added backend provider candidate search endpoint `GET /api/v1/artwork/{kind}/{id}/candidates` and SSRF-safe strict-CSP preview proxy `GET /api/v1/artwork-preview` (`src/artwork_sources.rs`, `src/library.rs`, `src/web.rs`) querying enabled providers (`iTunes`, `Deezer`, `MusicBrainz / Cover Art Archive`, `Wikimedia Commons / Wikidata`) across `artists`, `albums`, and `tracks`, while preserving provider credit provenance when a candidate is selected.
     - Built a dedicated 3-tab `ImagePickerModal` (`frontend/src/image-picker.tsx`) accessible directly from Artist, Album, and Track headers (cover overlay button + entity toolbar button + `/music/{kind}/{id}/edit`) supporting **Search providers**, **Upload file** (with revocable `blob:` preview), **Image URL**, and **Reset custom image**.
     - Coordinated `frontend/src/artwork-activity.ts` (`syncServerArtworkJobs`) with active server `artwork` jobs so browse-time downloads pause automatically while a library artwork sync is running, surfaced prominently via the topbar `.artwork-sync-pill`, `/activities` (`ArtworkWork`), and `/settings/images` (`ArtworkProviderSettings`).

4. **Dead / Empty Space & Fragmented CSS:**
   - **Root cause:** `frontend/src/style.css` (`8,419` lines) and `frontend/src/dashboard.css` (`1,468` lines) accumulated conflicting overrides, mismatched side-by-side widget heights (`288px` vs `576px`), blank `min-height: 88px` panel title headers, and a `max-width: 1200px` cap on `.page-container` that wasted 40%+ of wide desktop monitors.
   - **Fix:** Tore down and rewrote `frontend/src/style.css`, `frontend/src/dashboard.css`, and `frontend/src/listening-patterns.css` from scratch using a modular OKLab token + component system, fluid full-width desktop containers (`padding-inline: 24px` desktop, `14px` mobile `<= 600px`), dense 6-column Bento grids with matched row slot heights, and unified `40px`/`44px` control variants while preserving the `Name.` signature accent period across all headers.

---

## Macro-Phase Checklist

### Phase 1: Design System Foundation
- [x] Tear down legacy CSS bloat and write clean, modular `frontend/src/style.css` from scratch (OKLab dynamic tokens for dark/light + custom `--accent`, CSS reset, typography with `tabular-nums`, signature `Name.` accent period).
- [x] Define strict unified component variants: buttons (`btn` / `btn-primary` / `btn-secondary` / `btn-ghost` / `btn-icon` / `btn-danger` with clean semantic aliases), inputs, custom `Select`, `SegmentedControl`, `Modal`, `Notice`, `Toast`, `Badge`, and `Skeleton`.
- [x] Rebuild `frontend/src/ui.tsx`, `frontend/src/select.tsx`, and `frontend/src/skeleton.tsx` on top of the unified design primitives.

### Phase 2: App Shell, Navigation, Startup Screen (#158 Fix), & High-Density Grid System
- [x] Redesign early-boot HTML (`src/boot.rs`), pre-hydration shell (`frontend/index.html`), React bootstrap startup screen (`frontend/src/startup-screen.tsx` & `frontend/src/main.tsx`), onboarding (`frontend/src/account.tsx`), and `LifecycleScreen` (`frontend/src/lifecycle-screen.tsx`) so `#158` is genuinely visible, architectural, and utilizes viewport space cleanly.
- [x] Rebuild the App Shell (`Sidebar` with Now Playing card & resizer, `Topbar` with breadcrumbs, prominent Artwork Sync status pill, Library Tools menu, Activities flyout, Account menu) and fluid full-width `.page-container` layout grid (zero dead desktop margins, crisp `<= 600px` single-column mobile baseline).

### Phase 3: Dashboard, Charts, Activity Views & Prominent #155 / #163 Integration
- [x] Rewrite `frontend/src/dashboard.css` and `frontend/src/listening-patterns.css` from scratch with compact widget headers, proportional bento spans, zero-CLS loading geometry, and zero artificial empty vertical gaps.
- [x] Apply prominent `#163` dedicated row surfaces (`--row-bg` / `--row-border` / `--row-hover`), `4px` row separation, `6px` radius, right-aligned tabular metrics, and `160ms` micro-transitions across `Rankings`, `ListenList` (`History`), `Activities`, `Search`, and `Review`.
- [x] Coordinate client `artwork-activity.ts` with server `artwork` jobs (`#155`) to pause concurrent browse-time fetches during library artwork runs and surface prominent artwork sync progress + refresh actions in the Shell, Library tools, Settings, and Activities.

### Phase 4: Artist, Album, and Track Detail Pages
- [x] Rebuild `MusicPage` (`frontend/src/consultation.tsx`) for Artists, Albums, and Tracks with consistent signature `Title.` (`<span className="accent">.</span>`) headers, square interactive cover hero, inline half-star rating, dense KPI strip, inline period/filter controls, and full-width tracklists, discography, and history panels.
- [x] Surface direct **Change artwork** (`ImagePickerModal`), **Edit details**, and **Customize template** controls right in the entity header.

### Phase 5: Interactive GUI Image Picker & Full GUI Customization/Template Modals
- [x] Add backend artwork candidate search (`GET /api/v1/artwork/{kind}/{id}/candidates` and `GET /api/v1/artwork-preview` in `src/artwork_sources.rs`, `src/library.rs`, `src/web.rs`) across enabled providers (`iTunes`, `Deezer`, `MusicBrainz / Cover Art Archive`, `Wikimedia Commons / Wikidata`) supporting `artists`, `albums`, and `tracks`.
- [x] Implement `frontend/src/image-picker.tsx` (`ImagePickerModal`) with interactive provider candidate grid, drag-and-drop file upload with revocable blob preview, direct HTTPS URL preview, and reset-to-automatic, integrated into `MusicPage` and `MusicEdit` (`frontend/src/music-edit.tsx`).
- [x] Standardize `DashboardEditToolbar`, `WidgetEditControls`, and `WidgetCatalogDialog` for seamless full-GUI layout customization across Overview, Artist, Album, and Track templates.

### Phase 6: Settings, Admin, Cleanup, and Build Verification
- [x] Standardize `Settings` (`frontend/src/settings.tsx`, `artwork-settings.tsx`, `account-settings.tsx`, `api-keys.tsx`, `recovery.tsx`), `ImportPage` (`library.tsx`, `listenbrainz.tsx`), `ReviewPage` (`review.tsx`), and `SearchPage` (`search.tsx`) onto the unified card/form/button system.
- [x] Remove dead CSS (`style.css` reduced from `8,419` lines of fragmented overrides to a modular `4,339`-line token + component sheet), unused styles, and legacy layout hacks.
- [x] Run `npm run format:web`, `npm run build`, `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked --test deployment --test library`, and Playwright verification suites (`skeleton`, `artwork-providers`, `settings`, `consultation`, `gui-audit`); commit cleanly and open PR.
