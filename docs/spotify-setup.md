# Connect your Spotify account

1. Start scrobbl.rs and open your configured origin (locally **http://127.0.0.1:3210**).
   On a private server, first follow the generic [Docker and HTTPS guide](docker.md).
   Set the owner password and open **Settings → Connections**.
2. Sign in to the [Spotify Developer Dashboard](https://developer.spotify.com/dashboard) using your
   Premium account. Create an app named `scrobbl.rs` with a description such as “My personal listening
   archive”. Select **Web API** if asked. If a website is required, use your repository URL.
3. Register this exact redirect URI in the app settings:

   ```text
   http://127.0.0.1:3210/auth/spotify/callback
   ```

   This is the local default. On a server, copy the exact HTTPS URI displayed in Connections,
   derived from `SCROBBL_EXTERNAL_ORIGIN`, for example `https://music.example.org/auth/spotify/callback`.
   Changing the external port/name also changes the callback. Spotify
   permits HTTP for explicit loopback addresses; `localhost` is not accepted as a redirect URI.
4. Save the app and copy its **Client ID** and **Client Secret** into the local form. Save the form.
   These values belong in your private local configuration; do not paste them into a chat or commit
   them to Git. Alternatively, set both credential variables in a local `.env`, then restart.
5. If the account that will listen differs from the developer account, add it under the app's
   **Users Management** allowlist. Development mode requires the app owner to have Premium and
   supports up to five authorized users; scrobbl.rs itself is for a single owner.
6. Click **Connect Spotify**. Grant permission to read recently played tracks and currently playing
   content. Spotify redirects back to the local app. The browser and server must be on the same
   computer for this loopback configuration.
7. Check **Connected** and **Last successful sync**. Play music in Spotify. Return to Overview to
   inspect now playing; after playback, allow time for Spotify to publish the recent-history entry
   and for the next collection cycle. **Sync now** can request an earlier check.
8. Close the browser while leaving the Rust process running. Play another track, reopen the app and
   verify collection. Stop/restart the service and confirm the account connection and history persist.

An authorized session and a single-use state value protect the callback. Authorization tokens are
stored locally and refreshed by the server; you should not need to reconnect on every restart.

## Import the past

Request **Extended streaming history** from [Spotify privacy settings](https://www.spotify.com/account/privacy/).
After the archive arrives, select the ZIP directly or multiple music JSON files on **Import history**.
The default upload limit is 256 MiB; processing continues as a persistent background activity.
See [Spotify import](spotify-import.md) for expanded limits and saved reports.
Reimporting preserves provenance without multiplying known listens.

The account-data export (often `StreamingHistory*.json` with `endTime` and `trackName`) is not the
extended format supported here. Podcast/audiobook entries are skipped. Missing fields and invalid
rows are reported in the import results and recent import log.

To validate reconciliation on your account, import an extended-history file covering some plays
already collected via the API and compare original source timestamps in the local database. Check
that confident matches become one listen with actual duration, and investigate unmatched repeats
before relying on lifetime totals. The accepted initial rule uses API `played_at` and the export's
inferred start; Spotify does not guarantee identical timing across those records.

## Connection diagnostics

- **403:** check Premium, app access/allowlist and permissions, then reconnect.
- **401 or authorization rejected:** reconnect; app changes or revoked authorization can invalidate
  tokens. The server attempts one refresh for an expired API token.
- **429:** collection and now playing wait for the shared retry time. Do not reconnect to work
  around the quota. Retry timing is persisted across process restarts.
- **Connection failure or 5xx:** the background process retries on a later collection cycle.
- **Possible history gap:** recent history did not reach the previous checkpoint or recovery hit its
  bounded page limit. Keep the warning in mind and use extended history to repair the archive.

## Official references

- [Create and configure an app](https://developer.spotify.com/documentation/web-api/concepts/apps)
- [Redirect URI requirements](https://developer.spotify.com/documentation/web-api/concepts/redirect_uri)
- [Development mode and request quotas](https://developer.spotify.com/documentation/web-api/concepts/quota-modes)
- [Authorization code flow](https://developer.spotify.com/documentation/web-api/tutorials/code-flow)
- [Recently played tracks](https://developer.spotify.com/documentation/web-api/reference/get-recently-played)
- [Currently playing track](https://developer.spotify.com/documentation/web-api/reference/get-the-users-currently-playing-track)

Checked against the official documentation on 20 October 2026. Spotify may change its app-creation
requirements, access rules and endpoint availability.
