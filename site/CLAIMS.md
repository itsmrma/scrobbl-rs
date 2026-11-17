# Claims in the Features and Numbers you can trust sections (ticket #221)

Each claim on the page with its source in the repository. Demo examples (track
names, counts, file names) are fictional, as the page footer states, and assert
no software behaviour beyond the claim listed next to them.

## Hero

| Where | Claim on the page | Source |
| --- | --- | --- |
| Headline | Your listening history, kept on your server | `README.md` intro ("self-hosted, single-owner archive for music listening"), Data and privacy ("designed for one owner on a private server") |

## Features

| Panel | Claim on the page | Source |
| --- | --- | --- |
| Collect | Connect your own Spotify app once | `README.md` Quick start ("open Settings to connect Spotify"), `docs/spotify-setup.md`; owner-created app: README "Scope and verification" |
| Collect | Listens are collected in the background while the server runs | `README.md` What you can do ("Collect Spotify listens in the background"); Quick start ("The terminal process must stay running to collect listens") |
| Collect | Now playing is shown | `README.md` What you can do ("see now playing") |
| Import | Spotify Extended Streaming History and ListenBrainz JSON, JSONL, ZIP | `README.md` What you can do; `docs/listenbrainz.md`; `docs/spotify-import.md` |
| Import | Interrupted imports resume | `README.md` What you can do ("Imports can resume after interruption") |
| Import | Repeating an import never counts a listen twice | `README.md` What you can do ("safely repeated"), Data and privacy ("Repeated imports and collection are idempotent") |
| Import | Key/value "Import complete" example | Demo data; shape follows the app's import result (`docs/gui-audit.md`), no figures claimed |
| Explore | Searchable full history | `README.md` What you can do ("searchable listening history") |
| Explore | Artist, album and track pages, complete rankings | `README.md` What you can do |
| Explore | Activity by day, week, month or year | `README.md` What you can do ("day/week/month/year activity charts") |
| Explore | Pattern bars (Mornings, Afternoons...) | Demo data; `DESIGN.md` "Listening Patterns & Horizontal Progress Bars" for the component |
| Curate | Correct names and titles | `README.md` What you can do ("correct names and titles"); `GLOSSARY.md` Correzione locale |
| Curate | Pick artwork | `README.md` What you can do ("Personalize artist and album images") |
| Curate | Rate from half a star to five | `README.md` What you can do; `GLOSSARY.md` Voto personale |
| Curate | Reversibly group namesakes, originals kept | `README.md` What you can do ("reversibly group namesakes"); `GLOSSARY.md` Raggruppamento per nome / Separazione dal gruppo ("non cancella ascolti o informazioni originali") |
| Ask | Optional and off by default | `docs/ai.md` Activation and privacy ("AI is optional and off by default") |
| Ask | The model never sees archive rows | `docs/ai.md` ("The model never sees archive rows"; Bring your own AI: "It never receives archive rows") |
| Ask | Rust computes every number | `docs/ai.md` ("Rust executes that DSL ... and computes every number") |
| Ask | Local model | `docs/ai.md` Settings keys (`ai_provider` = `local`, downloaded model) |
| Ask | Own OpenAI-compatible server | `docs/ai.md` Bring your own AI ("Any server that implements the OpenAI `POST {base}/chat/completions` protocol works") |
| Ask | Example question and answer | Demo data; "Ask questions in plain words" per `docs/ai.md` ("natural-language questions" via DSL) |
| Keep | Export JSON or CSV | `README.md` What you can do |
| Keep | Portable backups | `README.md` What you can do; `docs/recovery.md` |
| Keep | Verify a restore ("Restore verified") | `README.md` What you can do ("verify a restore"); `docs/recovery.md` ("Only explicit successful verification sets Restore verified") |
| Keep | Upgrades take a verified safety copy first | `README.md` Configuration ("Release upgrades create verified technical safety copies before schema changes") |

## Numbers you can trust

| Notice | Claim on the page | Source |
| --- | --- | --- |
| Success | Imported plays of 30 seconds or longer count in statistics (notice title: "Imported plays count from 30 seconds"; the threshold applies to imported music plays, so the title does not generalise to collected listens) | `README.md` Data and privacy ("Imported music plays count in statistics at 30,000 ms or longer") |
| Success | Shorter records stay searchable in history | `README.md` Data and privacy ("Shorter records remain searchable in history") |
| Warning | A missing listening time is never shown as zero | `README.md` Data and privacy ("A missing duration stays unknown, not zero"); `GLOSSARY.md` Tempo ascoltato sconosciuto |
| Warning | Estimated time is labelled as an estimate | `README.md` Data and privacy ("API recent-history duration is an estimate, never presented as measured listening time"); `GLOSSARY.md` Tempo ascoltato stimato |
| Info | Uncertain matches stay separate until a review | `README.md` Data and privacy ("uncertain matches remain separate and reviewable"); `GLOSSARY.md` Possibile duplicato di ascolto |
| Info | Removed listens go to a recoverable trash | `GLOSSARY.md` Possibile duplicato di ascolto, Cestino; `docs/gui-audit.md` (review "Comparisons and Trash views") |
| Info | A decision can be undone | `GLOSSARY.md` Possibile duplicato di ascolto ("con possibilità di annullamento"); `docs/gui-audit.md` ("persistent paginated decisions expose Undo") |

## Deliberately not claimed

No performance figures, collection intervals or supported platforms. The
section copy mentions no platform at all (the linux/amd64 image belongs to the
install section).
