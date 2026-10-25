# Spotify image caching: terms research

Context: [“Quali durata e regole si applicano alla cache temporanea delle immagini Spotify?”](https://github.com/itsmrma/scrobbl-rs/issues/71)

Checked 25 October 2026 against Spotify’s current first-party documents. Spotify Developer Terms currently identify themselves as Version 10, effective 15 May 2025. This is a policy reading, not legal advice.

## Findings

- **No numeric maximum cache duration is stated.** The Developer Terms prohibit indefinite storage, require reasonable efforts to display the most up-to-date data and delete older data, and permit only temporary local caching of metadata and cover art when strictly necessary to improve the SDA’s performance and functionality. They do not define “temporary,” prescribe a TTL, or specify an image-cache duration. Source: [Spotify Developer Terms, Sections II and IV.3](https://developer.spotify.com/terms).
- **Artwork is Spotify Content.** The definition expressly includes cover art and metadata. The temporary exception is not blanket permission for a permanent artwork library: storage beyond what is strictly necessary to operate the SDA remains prohibited unless another express permission applies. The permitted cache must remain temporary; the terms do not provide a separate, longer-lived image allowance.
- **Downloading and caching need to be distinguished.** Spotify’s Web API image endpoint warns “Spotify content may not be downloaded,” while the Developer Terms expressly allow temporary caching of metadata and cover art for necessary functionality. A cautious reading is that the terms permit a constrained technical cache but do not authorize a user-facing download/export or permanent copy. Whether fetching Spotify-hosted image URLs into an application-managed server cache and serving those copies to the app’s user fits the cache exception is not expressly answered by the text. Source: [Get Playlist Cover Image](https://developer.spotify.com/documentation/web-api/reference/get-playlist-cover), [Developer Terms](https://developer.spotify.com/terms).
- **Local server and backup treatment is not specified.** Terms allow transfer of Spotify Content to third-party data processors providing the SDA (e.g., hosting), with developer responsibility for their acts/omissions. They do not expressly discuss app-owned server copies, HTTP proxying, or backups. Inference: a backup is another stored copy, so indefinite or long-retained backups would conflict with the non-indefinite storage rule; a temporary cache should expire from backups too. The Developer Terms require deletion of all Spotify Content, including from servers, upon termination/discontinuance. Sources: [Developer Terms, Sections IV.2.5, IV.3, IX.8.7](https://developer.spotify.com/terms).
- **Refreshing/deleting:** make reasonable efforts to use the latest available data and delete older data; do not retain indefinitely. On termination or discontinuance, immediately stop use and delete all Spotify Content obtained through the Platform, including server copies. There is no image-specific TTL or cache invalidation schedule. Source: [Developer Terms](https://developer.spotify.com/terms).
- **Display conditions:** Spotify metadata (including artwork) must be accompanied by Spotify branding/logo and link back to the Spotify Service. Use Spotify-provided album/podcast art only; preserve its original form (no crop, overlay, animation, blur or distortion). Resizing is allowed by the Terms. Sources: [Design & Branding Guidelines](https://developer.spotify.com/documentation/design), [Developer Terms](https://developer.spotify.com/terms).
- **Source distinction:** these restrictions apply to artwork or metadata obtained through the Spotify Platform, Service, or Spotify. An image obtained independently from MusicBrainz-linked artwork, a user file, or another source is not made Spotify Content just because it depicts the same release; its source’s license/terms govern. If the actual image bytes came from Spotify, changing the record’s selected provider does not change their provenance. The latter is an application of the terms’ definition of Spotify Content.

## What this establishes for the fallback

The terms support only a temporary, functionality/performance-necessary cache for Spotify cover art. They do not support specifying a numeric TTL from the published text. A server-side cache with backup copies, served locally while the upstream source is unavailable, is a plausible technical-cache implementation, but the text leaves that specific pathway uncertain; retaining the bytes as the user’s durable album/artwork library would be at odds with “temporary” and “do not store indefinitely.”

Before choosing a concrete server TTL, cache scope, or backup policy, seek written clarification from Spotify Developer Support for this private, single-user, non-streaming library use case. The exact question should describe: (1) retrieval of Spotify artwork as a fallback after MusicBrainz, (2) copying bytes into the app server’s local cache and serving them from there, (3) the intended TTL and replacement on refresh, and (4) backup/restore treatment. The terms give no safe number to infer.

## Sources

- Spotify, [Developer Terms](https://developer.spotify.com/terms) (Version 10; effective 15 May 2025; Sections II, IV.2, IV.3, IX.8).
- Spotify, [Design & Branding Guidelines](https://developer.spotify.com/documentation/design) (attribution, linking, artwork form).
- Spotify, [Get Playlist Cover Image API reference](https://developer.spotify.com/documentation/web-api/reference/get-playlist-cover) (image endpoint policy notes).

Reviewed 25 October 2026.
