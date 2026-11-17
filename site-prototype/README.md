# PROTOTYPE — marketing site variants (throwaway)

Four radically different landing pages for scrobbl.rs, switchable via `?variant=`
on a single route, served by a static-assets-only Cloudflare Worker.

```sh
npm run site:prototype        # wrangler dev → http://localhost:8787/?variant=A
```

| key | name | idea |
| --- | --- | --- |
| A | Ledger | Project design system: dark slate, lime accent, panels, live accent retint |
| B | Liner notes | Editorial LP sleeve: serif, paper, features as a tracklist, credits |
| C | Console | Monospace split pane: live listening log, illustrative output blocks |
| D | Ask | Ask-first hero demo, then full-bleed bands in the six accent presets |

All numbers, artists and tracks are fictional demo data.
Use ← / → or the bottom bar to switch. Delete this folder once a direction wins.
