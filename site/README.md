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
