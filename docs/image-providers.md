# Image providers

Open **Settings → Image providers** to enable sources and change their order.
The settings belong to the instance and survive restarts and portable backups.
Concurrent saves are rejected until the older page reloads its saved settings.
Spotify stays last because it is a fallback. All sources can be disabled.

The default order is MusicBrainz → Cover Art Archive for albums, MusicBrainz
image relationships → Commons for artists, MusicBrainz → Wikidata P18 → Commons
for artists, then Spotify. Public sources need no Spotify login or API key.
Coverage varies; multiple exact identities, disambiguation, incomplete search
results and multiple candidate portraits do not cause a first-result guess.
An uncertain identity stops automatic association. A temporary provider error
does not become a cached absence.

Settings affect missing images. Existing images, including legacy permanent
Spotify images, remain in the active archive. A personal image takes precedence,
including when it arrives during an automatic download. Changing settings during
a download prevents that old result from being saved. Saved absences are retried
under the new settings; confirmed absences otherwise expire after one day.

MusicBrainz requests use an identifying User-Agent and a shared durable pace of
at least 1.1 seconds between requests. The existing background queue still uses
its conservative interval between images; this interval is not a provider limit
or a coverage/speed guarantee. Retrieval shares the existing bounded image slot,
checks public HTTPS/DNS again at each redirect, limits redirects and download
sizes, and accepts PNG/JPEG/WebP. Provider Retry-After is retained across restarts.

New Spotify fallback images have a **14-day app cache policy** and disappear at
expiry or disconnection. This is an application choice, not a numeric permission
or TTL published by Spotify. Portable backups and their isolated restores omit
Spotify images, including legacy Spotify entries; the live archive is untouched.
Technical private upgrade copies retain their existing recovery purpose.

The image source, author, credits and licence are retained where supplied.
**Image credits** on the music/edit page exposes them, and collage PNG exports
carry attribution text and source/licence URLs. Commons metadata is displayed as
text, never executable HTML. Album artwork may be copyrighted: MusicBrainz's
metadata licence does not license its images. Last.fm is unavailable as an image
source because its API data licence excludes artwork. Each provider's Settings
entry links its documentation and explains access, limitations and reuse terms.

The follow-up [Recuperare le nuove immagini tramite MusicBrainz con Spotify come
fallback](https://github.com/itsmrma/scrobbl-rs/issues/65) owns further queue pacing,
provider recovery and real-library coverage verification. Physical device checks
remain part of final release validation.
