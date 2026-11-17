# scrobbl.rs site

Static presentation site served by a static-assets-only Cloudflare Worker
named `scrobbl-site` (see `wrangler.jsonc`). Assets live in `public/`.

## Run and test locally

```sh
cd site
npm ci
npm run dev           # wrangler dev on http://127.0.0.1:8788
npm test              # Playwright tests
npm run deploy:dry-run  # validate the Worker bundle without deploying
```

## One-time Cloudflare Workers Builds setup

Done once by the owner in the Cloudflare dashboard (Workers & Pages >
`scrobbl-site` > Settings > Builds > Connect to Git). The repository is
`itsmrma/scrobbl-rs`.

| Setting | Value |
| --- | --- |
| Worker name | `scrobbl-site` (must match `name` in `wrangler.jsonc`) |
| Root directory | `site` |
| Production branch | `main` |
| Build command | none (leave empty; dependencies are installed automatically) |
| Deploy command | `npx wrangler deploy` |
| Non-production branch deploy command | `npx wrangler versions upload` |
| Build watch paths | `site/*` |

Pushes to `main` that touch `site/*` deploy to production. Other branches
get a preview version via `versions upload`.

## Custom domain

The site is served at `https://scrobblrs.benassimatteo.dev`, declared as a
Custom Domain in `wrangler.jsonc` (`routes`). The `benassimatteo.dev` zone must
be on the same Cloudflare account; Cloudflare creates the DNS record and the
certificate on the next `wrangler deploy` (production builds only: preview
versions keep their own URLs).

## Production origin

Share metadata needs absolute URLs, so `https://scrobblrs.benassimatteo.dev`
is written in:

- `../README.md` (repository root): the "Website" link
- `public/index.html`: `canonical`, `og:url`, `og:image`, `twitter:image`
- `public/robots.txt`: the `Sitemap:` line
- `public/sitemap.xml`: the single `<loc>`

To change domain, update those files and the `routes` entry. Also add the URL
to the repository's "About" field.

The social image is `public/social.jpg`, the `brag.jpg` frame of the app
README (demo data only) scaled to 1200 px wide.
