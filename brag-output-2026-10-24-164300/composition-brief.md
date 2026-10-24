# Hyperframes Composition Brief: scrobbl.rs

## Objective

Create a short launch-style brag video for scrobbl.rs, the self-hosted single-owner music listening archive.

## Output

- Composition directory: `brag-output-2026-10-24-164300/composition/`
- Rendered video: `brag-output-2026-10-24-164300/brag.mp4`
- Format: landscape — 1920x1080
- Duration: 22.37 seconds

## Source Material

- Project root: repository root
- Primary files read: `README.md`, `Cargo.toml`, `package.json`, `frontend/index.html`, `frontend/src/style.css`, `static/favicon.svg`
- Product name: scrobbl.rs
- Tagline / strongest claim: “Your listening history, kept on your server.”
- Key UI or visual moment to recreate: the Overview dashboard, import report, and music/history consultation screens, grounded in the shipped app and using fictional demo content
- Copy that must appear verbatim:
  - “Your listening history.”
  - “Import extended listening history.”
  - “History. Rankings. Activity.”
  - “A listening archive, on your server.”

## Creative Direction

- Tone preset: polished
- Creative direction: quiet, personal music archive; restrained motion with the product doing the talking
- Interpretation: match the dark application surfaces and lime accent, with generous holds and an unobtrusive music bed
- Angle: treat listening history as something worth keeping; show import and discovery in the app before landing on its self-hosted promise
- Hook: waveform resolves into the Overview dashboard with “Your listening history.”
- Outro / punchline: logo and “A listening archive, on your server.”
- Avoid: generic SaaS language, abstract filler visuals, unsupported feature claims, real listening data, credentials, and account information

## Visual Identity

- Background: `#14171c` (derived from the frontend's dark background token)
- Text: `#f2f3f7`
- Accent: `#c9ee8b`
- Display font: system sans-serif
- Body font: system sans-serif
- Visual references from the project: `frontend/src/style.css`, `static/favicon.svg`

## Storyboard

Use `brag-plan.md` as the creative contract.

1. Keep the history — 3.55s — logo, demo dashboard, headline
2. Bring the archive in — 4.24s — fictional ZIP import and completed report
3. See the archive — 4.77s — dashboard and activity
4. Browse the detail — 5.99s — artist page and demo history rows
5. Keep it close — 3.82s — logo and self-hosted message

## Audio

- Audio role: warm, restrained rhythmic bed
- Audio arc: gentle opening, supports the UI reveals, fades under the final logo
- Music: `assets/music/happy-beats-business-moves-vol-10-by-ende-dot-app.mp3`
- Music treatment: low bed, short fade-in and fade-out
- Music cue guidance: bundled cue file; 4.64s, 9.83s, 15.82s and 20.19s are optional timing cues
- Audio-reactive treatment: none; not needed for the understated brand treatment
- Audio-coupled moments: UI reveal accents and the final logo near 20.19s
- SFX selection guidance: sparse, soft interface movement only
- SFX analysis guidance: `C:/Users/mbena/.codex/skills/brag/assets/sfx/sfx-analysis.md`
- Audio files: bundled music copied into `composition/assets/music/`

## Hyperframes Instructions

Build a deterministic, seekable composition that shows the real product's information architecture through clearly labeled fictional demo data. Keep text large and settled long enough to read. Use the project logo, colors, and actual feature names. Check the rendered preview before final rendering.
