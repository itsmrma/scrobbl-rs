# Image providers

Open **Settings → Image providers** to enable sources and change their order.
The settings belong to the instance and survive restarts and portable backups.
Concurrent saves are rejected until the older page reloads its saved settings.
No source calls the Spotify API: image retrieval never spends the quota used for
listening history and now playing.

## Sources

| Order | Source | Albums | Artists | Pace |
| --- | --- | --- | --- | --- |
| 1 | Deezer public search | covers up to 1000 × 1000 | photos | 50 requests / 5 s allowed, paced at 120 ms |
| 2 | Apple iTunes Search API | covers up to 1000 × 1000 | — | about 20 searches / min, paced at 3 s |
| 3 | MusicBrainz → Cover Art Archive | covers | — | MusicBrainz 1 request / s |
| 4 | MusicBrainz → Wikimedia Commons (direct or via Wikidata P18) | — | attributed photos | MusicBrainz 1 request / s |

Tracks show their album's cover. Fast sources come first; the MusicBrainz
sources catch what stores list differently, for example original-script titles
such as `狂言` that Deezer and Apple publish in romaji.

## Matching

- Names are the library's own, after local corrections and name grouping.
- Album searches use the **album artists** (primary credits, see the glossary),
  never a guest on one song, so `MAP OF THE SOUL : 7` is searched as BTS.
- Titles are compared ignoring case, accents, spacing and punctuation, and
  common edition suffixes such as `(Deluxe Edition)` or `[Remastered]`.
- A compilation by "Various Artists" matches any album artist.
- Deezer homonym artists resolve to the most followed one; placeholder
  silhouettes are skipped.
- MusicBrainz release groups need a search score of at least 90 and matching
  title and artist; MusicBrainz artists need a unique exact name or a clear
  winner. Otherwise the next source is asked.
- Commons files without author and licence are skipped.

## Engine

The engine was rewritten for the 0.1 alpha. One resolver serves pages and the
background queue:

- One pooled HTTPS client. A custom DNS resolver refuses private, loopback and
  special addresses on every connection, including redirects, so personal
  image links stay safe too.
- Politeness is per host and in memory. A Retry-After or server error pauses
  only that host (5 s growing to 10 min, Retry-After capped at one hour);
  other sources continue.
- A page that shows a missing image starts its download in a task that
  survives the request and waits up to 8 seconds; a second request for the
  same image waits for the first download instead of starting another. If the
  download takes longer, the page retries after the time the server suggests.
- The background queue resolves four images at once and starts by itself on
  startup, after imports and after Spotify collection adds listens.
- A temporary provider failure is never cached as "no image". A confirmed
  absence (every enabled source answered without a match) is retried after
  three days, or at once after **Retry missing images** or a settings change.
- Saved images are never replaced. A personal image always wins, including
  when it is saved while an automatic download is running. Changing settings
  during a download discards that download's result.

Typical timings measured against the public services: 0.1–0.3 s per image when
Deezer has it, about 2.5 s when MusicBrainz has to answer.

## Credits and rights

The image source, author, credits and licence are retained where supplied.
**Image credits** on the music/edit page exposes them, and collage PNG exports
carry attribution text and source/licence URLs. Commons metadata is displayed as
text, never executable HTML. Album artwork may be copyrighted: MusicBrainz's
metadata licence does not license its images. Last.fm is unavailable as an image
source because its API data licence excludes artwork.

Legacy permanent Spotify images remain saved; portable backups and their
isolated restores omit all Spotify images, and personal images are preserved.

## Upgrading

Migration `0022_artwork_engine` adds the new sources, removes placeholder rows
without bytes (they hid work), clears saved absences and the old persisted
pacing keys, and wakes any waiting artwork activity. A user pause is kept.
Saved settings from older versions move to the new order and keep their
choices: a disabled legacy source disables its successor.

## Smoke test

A manual public-service smoke test is available for maintainers:
`cargo test --locked --lib public_artwork_smoke -- --ignored --nocapture`.
It creates a temporary archive with Abbey Road, `狂言` by Ado and a BTS album
with a featured guest, downloads every album and artist image and prints the
time each took. It never opens the personal archive and is excluded from
routine CI because public services can change.
