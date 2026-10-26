# Image providers

Open **Settings → Image providers** to enable sources and change their order.
The settings belong to the instance and survive restarts and portable backups.
Concurrent saves are rejected until the older page reloads its saved settings.
All sources can be disabled. Spotify is no longer an image provider: image
retrieval never calls its catalogue API, preserving the app quota for listening
history and now playing.

The default order is MusicBrainz → Cover Art Archive for albums, MusicBrainz
image relationships → Commons for artists, MusicBrainz → Wikidata P18 → Commons
for artists. Public sources need no Spotify login or API key.
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
at least 1.1 seconds between requests, including redirects. The background queue
yields for one second between images; it no longer waits one minute per image.
This is not a coverage or completion-time guarantee. Retrieval shares the existing bounded image slot,
checks public HTTPS/DNS again at each redirect, limits redirects and download
sizes, and accepts PNG/JPEG/WebP. Provider Retry-After and network/server backoff
are retained per host across restarts; one host's pause does not block other
image sources. Network/server failures without Retry-After back off from
30 seconds to one hour. Confirmed absences are cached for one day; errors are not
cached as absences. A cooldown saved before this upgrade did not identify its
host: that existing finite pause is honored until its original expiry; new waits
are scoped per host.

Each queued image has its own persistent retry time. An unavailable source stays
pending while other eligible images continue. New items take priority over
retries so a large queue cannot starve its tail. Changing providers reconsiders
pending items. A confirmed absence completes that item and is cached for one day;
it never waits for a Spotify connection. User pauses, saved bytes and progress
survive restarts. Activities shows provider waits without estimating completion
from a fixed per-image rate.

Upgrading removes Spotify from saved provider settings while retaining the order
and enabled state of public sources. Temporary and expired Spotify artwork rows
are permanently retired, and Spotify provider badges on covers are removed.
Pending items from the retired fallback are reconsidered; user pauses and public host
waits remain respected. Existing Spotify API Retry-After waits are preserved:
removing artwork requests does not shorten a pause already imposed by Spotify.
Collection and now playing resume on their scheduled retry after the pause.
A long interruption can leave a history gap if Spotify no longer returns all listens;
an extended-history import can repair it.

Legacy permanent images remain saved; temporary Spotify image rows are retired by
migration 0020. Portable backups and their isolated restores continue to omit all
Spotify images; personal images are preserved. Technical private upgrade copies
retain their recovery purpose.

The image source, author, credits and licence are retained where supplied.
**Image credits** on the music/edit page exposes them, and collage PNG exports
carry attribution text and source/licence URLs. Commons metadata is displayed as
text, never executable HTML. Album artwork may be copyrighted: MusicBrainz's
metadata licence does not license its images. Last.fm is unavailable as an image
source because its API data licence excludes artwork. Each provider's Settings
entry links its documentation and explains access, limitations and reuse terms.

Queue pacing and recovery originated in [Recuperare le nuove immagini tramite
MusicBrainz con Spotify come fallback](https://github.com/itsmrma/scrobbl-rs/issues/65).
The retirement of its Spotify fallback is tracked in [Problema api Spotify](https://github.com/itsmrma/scrobbl-rs/issues/84).
Coverage of a personal library is not guaranteed by isolated tests. Physical device
and real-library checks remain part of final release validation.

A manual public-service smoke test is available for maintainers:
`cargo test --locked --lib public_artwork_smoke -- --ignored --nocapture`.
It creates a temporary archive, searches Abbey Road, follows the Beatles' known
MusicBrainz identity to Commons, downloads the bytes, and checks persistence
across restart without Spotify authorization. It is excluded from routine CI
because availability and responses of public services can change. It never opens
the personal archive.
