---
name: scrobbl.rs
description: Self-hosted personal listening archive and music library curation
colors:
  primary: "#c9ee8b"
  primary-ink: "#192912"
  neutral-bg-dark: "#13151b"
  neutral-surface-dark: "#1c1f27"
  neutral-surface-raised-dark: "#292d38"
  neutral-sidebar-dark: "#191c24"
  neutral-border-dark: "#454a58"
  neutral-text-dark: "#f2f3f7"
  neutral-muted-dark: "#bac0cc"
  neutral-bg-light: "#f5f6fa"
  neutral-surface-light: "#ffffff"
  neutral-surface-raised-light: "#e9ecf3"
  neutral-sidebar-light: "#eef0f6"
  neutral-border-light: "#9da7b6"
  neutral-text-light: "#18212d"
  neutral-muted-light: "#425065"
  semantic-error-dark: "#ffaaa0"
  semantic-error-light: "#a22620"
  semantic-warning-dark: "#e5c685"
  semantic-warning-light: "#795609"
  semantic-success-dark: "#86dba2"
  semantic-success-light: "#246e3c"
  preset-lime: "#c9ee8b"
  preset-sky: "#91c9f7"
  preset-lavender: "#c5a3ee"
  preset-peach: "#f7b68b"
  preset-rose: "#f099ad"
  preset-mint: "#64d9c3"
typography:
  display:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
    fontSize: "clamp(28px, 3vw, 40px)"
    fontWeight: 600
    lineHeight: 1.2
    letterSpacing: "-0.035em"
  stat:
    fontSize: "38px"
    fontWeight: 500
    letterSpacing: "-0.04em"
  headline:
    fontSize: "20px"
    fontWeight: 600
    lineHeight: 1.2
    letterSpacing: "-0.035em"
  title:
    fontSize: "16px"
    fontWeight: 600
    lineHeight: 1.2
    letterSpacing: "-0.035em"
  body:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
    fontSize: "15px"
    fontWeight: 400
    lineHeight: 1.6
    letterSpacing: "normal"
  body-compact:
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.5
  control:
    fontSize: "13px"
    fontWeight: 600
  caption:
    fontSize: "12px"
    fontWeight: 400
  meta:
    fontSize: "11px"
    fontWeight: 400
  label:
    fontSize: "10px"
    fontWeight: 600
    lineHeight: 1.4
    letterSpacing: "0.15em"
rounded:
  track: "3px"
  xs: "4px"
  bar: "5px"
  sm: "6px"
  field: "7px"
  md: "8px"
  card: "10px"
  panel: "12px"
  modal: "14px"
  full: "50%"
spacing:
  xs: "4px"
  sm: "8px"
  md: "12px"
  lg: "16px"
  xl: "18px"
  2xl: "24px"
  3xl: "26px"
  4xl: "36px"
components:
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.primary-ink}"
    rounded: "{rounded.md}"
    padding: "10px 15px"
    size: "13px"
    height: "44px"
  button-secondary:
    backgroundColor: "{colors.neutral-surface-raised-dark}"
    textColor: "{colors.neutral-text-dark}"
    rounded: "{rounded.md}"
    padding: "10px 15px"
    size: "13px"
    height: "44px"
  button-icon:
    backgroundColor: "transparent"
    textColor: "{colors.neutral-text-dark}"
    rounded: "{rounded.md}"
    padding: "8px"
    height: "44px"
    width: "44px"
  panel:
    backgroundColor: "{colors.neutral-surface-dark}"
    textColor: "{colors.neutral-text-dark}"
    rounded: "{rounded.panel}"
    padding: "26px"
  input-field:
    backgroundColor: "{colors.neutral-bg-dark}"
    textColor: "{colors.neutral-text-dark}"
    rounded: "{rounded.field}"
    padding: "10px 12px"
    height: "44px"
---

# Design System: scrobbl.rs

## Overview

**Creative North Star: "The Precision Listening Ledger"**

scrobbl.rs is a personal, durable music archive and listening ledger. Its visual language merges the quiet authority of an archivist's catalog with the tactile modern clarity of an audiophile dashboard. It rejects generic SaaS trends, heavy saturated gradients, neon card borders, and loud branding. Instead, the interface puts musical evidence—album artwork, track provenance, playtime aggregates, and listening chronologies—at the center of the experience.

The design operates predominantly in dark mode with a cohesive, muted slate palette, paired with a lively chartreuse accent (`#c9ee8b`). A central architectural principle is **dynamic color harmony**: all background, surface, and border tokens are dynamically tinted with a small percentage of the user's selected accent color using CSS `color-mix(in oklab, ...)` and `color-mix(in srgb, ...)`. This gives every surface an organic warmth and deep cohesion, whether the accent is lime, lavender, sky blue, peach, rose, or a custom hue.

Interactive controls and layout structures are disciplined, restrained, and accessible: touch targets never drop below 44px, text contrast strictly complies with WCAG guidelines, motion is concise and respectful of user preferences (`prefers-reduced-motion`), and loading states use YouTube-style subtle shimmer skeletons instead of spinners.

**Key Characteristics:**
- **Dynamic OKLab Surface Tinting:** Backgrounds, surfaces, sidebars, and borders subtly absorb the user's chosen accent color.
- **Tonal Layering over Shadows:** Flat, border-delimited cards at rest; shadows are reserved exclusively for transient floating layers (menus, dropdowns, dialogs, toasts).
- **The Signature Accent Dot:** A punctuation motif (`<span class="accent">.</span>`) terminates primary page titles and the brand wordmark.
- **Compact Data Density:** Tabular numbers, proportional listening bars, and clear hierarchy allow scanning extensive music collections without cognitive fatigue.
- **Touch-First Ergonomics:** 44px minimum touch targets on all buttons, select triggers, and icon actions across desktop and mobile.

---

## Colors

The scrobbl.rs palette combines deep neutral slates with an energizing primary accent and clear semantic signposts. Backgrounds and borders are never pure achromatic grays; they are tinted by the accent.

### Primary
- **Lime / Chartreuse** (`#c9ee8b`): The default signature accent. Used for primary call-to-action buttons, active toggles, chart highlights, progress fills, and the brand punctuation dot.
- **Accent Ink** (`#192912` or dynamic via contrast check): Calculated dynamically via relative luminance to guarantee readable text on top of the accent color.

### Presets Palette
In addition to custom hex selection, the application provides six curated accent presets in Appearance settings:
- **Lime (Default):** `#c9ee8b`
- **Sky Blue:** `#91c9f7`
- **Lavender:** `#c5a3ee`
- **Peach:** `#f7b68b`
- **Rose:** `#f099ad`
- **Mint:** `#64d9c3`

### Neutral (Dark Mode — Default)
- **Background (`--bg`):** `color-mix(in oklab, #13151b 96%, var(--accent))` — The base canvas canvas tone.
- **Surface (`--surface`):** `color-mix(in oklab, #1c1f27 95%, var(--accent))` — Standard container for panels, cards, and widgets.
- **Raised Surface (`--surface-raised`):** `color-mix(in oklab, #292d38 91%, var(--accent))` — Secondary buttons, dropdown popups, active rows, and nested card blocks.
- **Sidebar (`--sidebar`):** `color-mix(in oklab, #191c24 94%, var(--accent))` — The persistent navigation pane.
- **Border (`--border`):** `color-mix(in oklab, #454a58 90%, var(--accent))` — 1px structural framing for cards, dividers, and inputs.
- **Primary Text (`--text`):** `#f2f3f7` — High-contrast off-white for headings, titles, and values.
- **Muted Text (`--muted`):** `#bac0cc` — Medium-contrast secondary text, timestamps, subtitles, and labels.

### Neutral (Light Mode — `:root.theme-light`)
- **Background (`--bg`):** `color-mix(in oklab, #f5f6fa 95%, var(--accent))`
- **Surface (`--surface`):** `color-mix(in oklab, #ffffff 97%, var(--accent))`
- **Raised Surface (`--surface-raised`):** `color-mix(in oklab, #e9ecf3 90%, var(--accent))`
- **Sidebar (`--sidebar`):** `color-mix(in oklab, #eef0f6 94%, var(--accent))`
- **Border (`--border`):** `color-mix(in oklab, #9da7b6 94%, var(--accent))`
- **Primary Text (`--text`):** `#18212d`
- **Muted Text (`--muted`):** `#425065`

### Semantic
- **Error (`--error`):** `#ffaaa0` (Dark) / `#a22620` (Light) — Validation errors, failed imports, destructive actions.
- **Warning (`--warning`):** `#e5c685` (Dark) / `#795609` (Light) — Caution notices, pending reviews, partial sync states.
- **Success (`--success`):** `#86dba2` (Dark) / `#246e3c` (Light) — Completed imports, copy-to-clipboard confirmation, connected status.
- **Focus Ring (`--focus`):** `color-mix(in srgb, var(--accent) 45%, white)` (Dark) / `#354559` (Light) — Accessible focus outline.

### Named Rules
- **The Dynamic Tinting Rule:** Never declare hardcoded background or border grays (`#222`, `#333`, `#e5e5e5`) in component styles. Always use `var(--bg)`, `var(--surface)`, `var(--surface-raised)`, and `var(--border)` so the component participates in accent adaptation.
- **The Rarity Rule:** The primary accent (`var(--accent)`) must occupy $\le 10\%$ of any given view. It serves as an orienting landmark (active tabs, primary action, stat bars, brand dot), never as full card background.
- **The Terminal Accent Dot Rule:** Page headings and the primary wordmark terminate with an accent dot: `<h1>Overview<span class="accent">.</span></h1>`.

---

## Typography

**Primary Font Family:** `-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif`  
**Info Serif Family:** `Georgia, serif` (reserved strictly for the serif "i" glyph in `.info-button`)  
**Icon Font Family:** `"Material Icons Outlined"` (local woff2, `22px` standard, ligatures enabled)

The typography is native, neutral, and crisp. Headings feature tight negative letter-spacing (`-0.035em`) and balanced line breaks (`text-wrap: balance`) for an editorial feel. Data values and durations use tabular numbers.

### Hierarchy
- **Display / H1** (`font-weight: 600`, `font-size: clamp(28px, 3vw, 40px)`, `line-height: 1.2`, `letter-spacing: -0.035em`): Page titles and primary screen headings.
- **Headline / H2** (`font-weight: 600`, `font-size: 20px`, `line-height: 1.2`, `letter-spacing: -0.035em`): Panel and widget titles (`.panel-title h2`).
- **Title / H3** (`font-weight: 600`, `font-size: 16px`, `line-height: 1.2`, `letter-spacing: -0.035em`): Sub-sections and dialog titles.
- **Stat Metric** (`font-weight: 500`, `font-size: 38px`, `letter-spacing: -0.04em`, tabular): Big numeric summaries in stat widgets.
- **Body** (`font-weight: 400`, `font-size: 15px`, `line-height: 1.6`, color: `var(--text)`): Standard body copy and descriptions.
- **Body Compact / Controls** (`font-weight: 600`, `font-size: 13px`): Button labels, select triggers, table headers.
- **Small / Metadata** (`font-weight: 400`, `font-size: 12px`, color: `var(--muted)`): Subtitles, form labels, timestamps, durations.
- **Rank & Badge** (`font-size: 10px–11px`, `tabular-nums`, color: `var(--muted)`): Leaderboard rankings and duration summaries.
- **Eyebrow / Category** (`font-weight: 600`, `font-size: 10px`, `letter-spacing: 0.15em`, `text-transform: uppercase`): Section delimiters like `YOUR LIBRARY` in navigation.

### Named Rules
- **The Balanced Headline Rule:** All `h1`, `h2`, and `h3` tags enforce `text-wrap: balance;` to eliminate typographic orphans.
- **The Unadorned Link Rule:** Inline links retain inherited colors with `text-underline-offset: 3px;`. Navigation links never use underlines.

---

## Layout

### Spatial Model & Responsive Rhythm
The application uses a responsive shell with an adjustable left sidebar and a flexible content canvas.
- **Sidebar Width:** Variable between `216px` and `360px` (stored in preferences, default `248px`), or collapsed to a `64px` icon dock.
- **Mobile Drawer:** Below `900px`, the sidebar turns into a sliding overlay drawer with an animated backdrop (`.sidebar-backdrop`).
- **Main Canvas:** `padding-top: 36px; min-height: 75vh;` with bottom breathing room.

### Spacing Scale
The spacing scale is strictly arithmetic and predictable:
- `4px`: Micro gap (icon gaps, tag borders, indicator spacing)
- `8px`: Tight gap (button internal gaps, music card padding)
- `12px`: Standard gap (grid gaps, panel title flex spacing, notice padding)
- `16px`: Form and section gap (list row vertical padding, mobile panel padding)
- `18px`: Grid column gaps (`.rankings-grid`, `.stats-grid`)
- `24px`: Module separation (bottom margin on `.panel`, two-column layout gap)
- `26px`: Standard desktop panel padding (`.padded`)
- `36px`: Major section / page-level separation

### Breakpoints
- **Mobile (`max-width: 600px`):** Single-column forms, 2-column music card grids, stacked music header, reduced chart bar spacing.
- **Tablet / Responsive Drawer (`max-width: 900px`):** Sidebar transitions from fixed column to modal drawer with hamburger toggle.
- **Wide Desktop (`min-width: 1100px`):** Expanded multi-column listening history and side-by-side rankings.

### Grid Systems
1. **Stats Grid (`.stats-grid`):**
   ```css
   display: grid;
   grid-template-columns: repeat(3, minmax(0, 1fr));
   gap: 18px;
   ```
2. **Rankings Grid (`.rankings-grid`):**
   ```css
   display: grid;
   grid-template-columns: repeat(3, minmax(0, 1fr));
   gap: 18px;
   ```
3. **Music Card Grid (`.music-grid`):**
   ```css
   display: grid;
   grid-template-columns: repeat(auto-fill, minmax(min(var(--music-card-size, 140px), 100%), 1fr));
   gap: 12px;
   ```
4. **Two-Column Form / Detail (`.two-column`):**
   ```css
   display: grid;
   grid-template-columns: minmax(0, 1.6fr) minmax(0, 1fr);
   gap: 24px;
   ```

---

## Elevation & Depth

scrobbl.rs uses a **tonal layering model** rather than ambient shadows. Resting components sit on flat surfaces separated by 1px borders and distinct surface tones (`--bg` $\rightarrow$ `--surface` $\rightarrow$ `--surface-raised`).

Shadows exist strictly for **stateful or floating elevations**: dropdown lists, tooltips, flyout menus, modals, and toasts.

### Shadow Vocabulary
- **Control Hover (`0 2px 8px color-mix(in srgb, var(--text) 10%, transparent)`):** Subtle lift applied to enabled buttons on hover.
- **Primary Glow (`0 4px 12px #0002`):** Discrete glow under primary accent buttons on hover.
- **Dropdown Menu (`0 10px 30px rgba(0, 0, 0, 0.25), 0 2px 6px rgba(0, 0, 0, 0.15)`):** Used on `.select-options` to separate options from content below.
- **Tooltips & Popovers (`0 12px 36px #0005` / `0 16px 40px rgba(0, 0, 0, 0.28)`):** Used on `.time-explanation`, `.activity-menu`, `.account-menu`.
- **Toast Notifications (`0 8px 35px #0004`):** Fixed notifications in bottom right corner.
- **Modal Dialog (`0 16px 48px rgba(0, 0, 0, 0.35)`):** Full modal window with `backdrop-filter: blur(4px);` and `rgba(0, 0, 0, 0.6)` backdrop.
- **Artwork Lightbox (`0 28px 72px rgba(0, 0, 0, 0.65), 0 0 0 1px rgba(255, 255, 255, 0.1)`):** High-contrast focus on enlarged covers.

### Named Rules
- **The Flat-At-Rest Rule:** Cards, panels, and widgets must never have ambient box-shadows when resting on the page. Depth is achieved via `border: 1px solid var(--border)` and surface contrast (`var(--surface)`).
- **The Glass Backdrop Rule:** Modal dialogs always blur the underlying content (`backdrop-filter: blur(4px)`).

---

## Shapes

The geometric vocabulary of scrobbl.rs is clean, rounded, and deliberate:
- **`3px`:** Inner progress bar tracks (`.pattern-track`).
- **`4px`:** Scrollbar thumbs, skeleton text blocks, pill tags.
- **`6px`:** Standard album artwork covers (`.cover`), chart bars (`border-radius: 6px 6px 0 0`).
- **`7px`:** Text fields, textareas (`input`, `textarea`).
- **`8px`:** Buttons (`button`, `.button`), select option containers (`.select-options`), fieldsets.
- **`10px`:** Music cards in tile view (`.music-card`), toast notifications (`.toast`).
- **`12px`:** Panels & widgets (`.panel`), notices (`.notice`), popover boxes.
- **`14px`:** Modals (`dialog`), activity and account dropdown panels, enlarged artwork lightbox.
- **`50%` (Circle):** Avatar placeholders, step numbers (`.step-number`), info button glyph (`.info-button`), activity badges.

---

## Components

### 1. Panels & Dashboard Widgets (Base Blueprint)
Panels are the fundamental visual unit of scrobbl.rs. Any new widget or customizable grid card **must** follow this exact anatomy:
```tsx
<article className="panel padded">
  <div className="panel-title">
    <h2>Widget Title</h2>
    <div className="panel-actions">
      {/* Optional action links, filters, or icon buttons */}
      <Link to="/target">View all</Link>
    </div>
  </div>
  <div className="widget-content">
    {/* Content: list rows, charts, stats, or empty state */}
  </div>
</article>
```
- **Container (`.panel`):** `background: var(--surface); border: 1px solid var(--border); border-radius: 12px; margin-bottom: 24px; min-width: 0;`
- **Padding (`.padded`):** `padding: 26px;` (desktop) / `padding: 16px;` (mobile).
- **Header (`.panel-title`):** `display: flex; justify-content: space-between; align-items: center; gap: 12px; margin-bottom: 16px;`
- **Title (`.panel-title h2`):** `margin: 0; font-size: 20px; font-weight: 600; letter-spacing: -0.035em; text-wrap: balance;`

### 2. Buttons
All buttons enforce a minimum touch target of 44px (`min-height: 44px;`) with `font-size: 13px; font-weight: 600; border-radius: 8px;`.
- **Primary (`.primary`):** `background: var(--accent); color: var(--accent-ink); border: 1px solid transparent;`
  - *Hover:* `filter: brightness(1.1); box-shadow: 0 4px 12px #0002;`
  - *Active:* `transform: scale(0.98); filter: brightness(0.95);`
- **Secondary (`.secondary`):** `background: var(--surface-raised); border: 1px solid var(--border); color: var(--text);`
  - *Hover:* `background: var(--bg); border-color: var(--muted);`
  - *Active:* `transform: scale(0.98);`
- **Icon Button (`.icon-button`):** `background: transparent; color: var(--text); min-width: 44px; min-height: 44px; padding: 8px;`
  - *Hover:* `background: color-mix(in srgb, var(--accent) 9%, var(--surface)); border-color: color-mix(in srgb, var(--accent) 25%, transparent); color: var(--accent);`
- **Focus:** `:focus-visible { outline: 3px solid var(--focus); outline-offset: 3px; }`

### 3. Inputs & Forms
- **Fields (`input`, `textarea`):** `background: var(--bg); border: 1px solid var(--border); border-radius: 7px; padding: 10px 12px; color: var(--text); min-height: 44px; font-size: 14px;`
- **Focus:** `:focus-visible { outline: 3px solid var(--focus); outline-offset: 3px; caret-color: var(--focus); }`
- **Labels:** `font-size: 12px; font-weight: 600; color: var(--muted); display: flex; flex-direction: column; gap: 7px;`

### 4. Custom Select (`.select-control`)
Accessible keyboard-navigable dropdown replacing native select:
- **Trigger (`.select-trigger`):** Matches button/input height (44px), `background: var(--surface-raised); border: 1px solid var(--border); border-radius: 8px; padding: 10px 14px;`
- **Popup (`.select-options`):** Rendered via portal, `background: var(--surface-raised); border: 1px solid var(--border); border-radius: 8px; box-shadow: 0 10px 30px rgba(0,0,0,0.25);`
- **Option Item (`[role="option"]`):** `padding: 10px 14px; border-radius: 6px;` Active item has `background: var(--accent); color: var(--accent-ink);`.

### 5. Artwork Cover (`.cover`)
Album, artist, and track image container:
- **Standard:** `width: 38px; height: 38px; border-radius: 6px; overflow: hidden; object-fit: contain;`
- **Large:** `width: 72px; height: 72px; border-radius: 6px;`
- **Placeholder (`.placeholder`):** `background: var(--surface-raised); color: var(--muted); display: inline-flex; align-items: center; justify-content: center;` with `<Icon>music_note</Icon>`.
- **Zoomable:** Large covers open a fullscreen `<dialog className="artwork-lightbox">` with a close button and smooth backdrop blur.

### 6. Notice Banner (`.notice`)
Structured status callout with icon and bold heading:
- **Container:** `display: flex; align-items: start; gap: 12px; border-radius: 12px; padding: 16px 18px; margin: 16px 0; border: 1px solid color-mix(in srgb, var(--notice-color) 35%, var(--border)); background: color-mix(in srgb, var(--notice-color) 7%, var(--surface));`
- **Variants:**
  - `.notice-error`: `--notice-color: var(--error);` (icon: `error_outline`)
  - `.notice-warning`: `--notice-color: var(--warning);` (icon: `schedule`)
  - `.notice-success`: `--notice-color: var(--success);` (icon: `check_circle`)
  - `.notice-info`: `--notice-color: var(--muted);` (icon: `info`)

### 7. Shimmer Skeletons (`<SkeletonBlock>`, `<SkeletonText>`)
YouTube-style animated placeholders for zero-layout-shift loading:
- **Base Block (`.skeleton-block`):** `background-color: var(--skeleton-base); border-radius: 4px; position: relative; overflow: hidden;`
- **Shimmer Effect:** Pseudo-element `::after` running `@keyframes skeleton-shimmer 1.6s cubic-bezier(0.4, 0, 0.2, 1) infinite;` using linear gradient with `--skeleton-highlight`.
- **Accessibility:** Automatically disabled when `prefers-reduced-motion: reduce`.

### 8. Listening Patterns & Horizontal Progress Bars
- **Track (`.pattern-track`):** `height: 10px; border-radius: 3px; background: var(--surface-raised); overflow: hidden;`
- **Fill (`.pattern-track > span`):** `background: var(--accent); height: 100%; transition: width var(--motion-fast);`
- **Row Grid (`.pattern-row`):** `display: grid; grid-template-columns: minmax(90px, 1fr) minmax(40px, 3fr) 64px; align-items: center; gap: 12px; font-size: 12px;`

---

## Blueprint for Future Customizable Grid & Widgets

To ensure that the future customizable dashboard grid and new widgets look 100% native:

1. **Widget Container Contract:**
   - Every widget must be an `<article className="panel padded widget-[name]">`.
   - Grid cells should use CSS Grid or Subgrid with consistent `gap: 18px;`.
   - If a widget supports resizing (1x1, 2x1, 2x2, full-width), internal layouts should adapt using CSS Container Queries (`container-type: inline-size;`).
2. **Widget Header Uniformity:**
   - Standard header: `<div className="panel-title"><h2>Widget Title</h2> ... </div>`.
   - Action controls in headers should use `<Icon>more_vert</Icon>` or `<Icon>drag_indicator</Icon>` styled as `.icon-button`.
3. **Empty & Error States:**
   - Empty state: `<div className="empty"><p>No listening data in this period.</p></div>`.
   - Error state: `<ErrorText>{errorMessage}</ErrorText>`.
4. **Loading States:**
   - Never show generic spinning loaders. Provide a bespoke skeleton loader composed of `<SkeletonBlock>` matching the widget's final geometry.

---

## Do's and Don'ts

### Do:
- **Do** use `var(--surface)`, `var(--surface-raised)`, `var(--border)`, and `var(--bg)` exclusively for containers and borders.
- **Do** wrap primary page headings with the signature accent dot: `<h1>Title<span className="accent">.</span></h1>`.
- **Do** ensure interactive controls have at least `min-height: 44px;` and `min-width: 44px;` for comfortable touch ergonomics.
- **Do** use `color-mix(in srgb, var(--accent) 9%, var(--surface))` for subtle interactive hover states on neutral items.
- **Do** use `tabular-nums` for numeric counters, timestamps, durations, and rankings.
- **Do** build skeleton loaders (`<SkeletonBlock>`) that mirror the exact height and layout of widget content.
- **Do** respect `prefers-reduced-motion` for all keyframe animations and transitions.

### Don't:
- **Don't** add raw gray hex codes (`#111`, `#222`, `#eee`, `#888`) or CSS frameworks (Tailwind, Bootstrap); scrobbl.rs uses a cohesive pure CSS custom property architecture.
- **Don't** use ambient shadows on resting cards or panels. Reserve shadows strictly for overlays, dropdowns, and dialogs.
- **Don't** introduce external font links or remote CDN scripts. All icons must use the local `Material Icons Outlined` font.
- **Don't** use spinning circle loaders for content panels; always use shimmer skeletons.
- **Don't** create buttons or actionable controls smaller than 44px in either dimension.
- **Don't** let long music titles break the layout; always use `overflow-wrap: anywhere` or ellipsis with `min-width: 0`.
