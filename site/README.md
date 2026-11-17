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

## Add a custom domain later

Add a `routes` entry to `wrangler.jsonc` and deploy. The zone must be on the
same Cloudflare account.

```jsonc
{
  "routes": [{ "pattern": "scrobbl.rs", "custom_domain": true }]
}
```

Use `"www.scrobbl.rs"` for a subdomain entry as well. Cloudflare creates the
DNS record and certificate on the next deploy.

## Set the production origin

Share metadata needs absolute URLs, so the production origin is written in
several places as the placeholder `https://scrobbl-site.example.workers.dev`:

- `../README.md` (repository root, outside `site/`): the "Website" link
- `public/index.html`: `canonical`, `og:url`, `og:image`, `twitter:image`
- `public/robots.txt`: the `Sitemap:` line
- `public/sitemap.xml`: the single `<loc>`

After the first deploy, replace it with the real address (the
`scrobbl-site.<account>.workers.dev` URL, or the custom domain once added):

```sh
grep -rl 'scrobbl-site.example.workers.dev' public ../README.md | xargs sed -i '' 's#https://scrobbl-site.example.workers.dev#https://YOUR-ORIGIN#g'
```

(On Linux use `sed -i` without the empty string.) The tests only require that
these URLs share one origin, so they pass before and after the swap. Then add
the same URL to the repository's "About" field.

The social image is `public/social.jpg`, the `brag.jpg` frame of the app
README (demo data only) scaled to 1200 px wide.
