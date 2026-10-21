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

### Refresh intervals

Use **Settings → Connections → Spotify → Refresh intervals** to change collection
and now-playing intervals in seconds. Collection accepts 60–86400 seconds; now
playing accepts 15–86400 seconds. Save applies immediately without restarting or
recreating the container. Values are stored with the archive's instance settings,
shared by browsers and preserved across restarts and archive snapshots.
Changing them does not bypass Spotify's shared request-limit wait.

Existing `SCROBBL_SYNC_SECONDS` / `SCROBBL_NOW_PLAYING_SECONDS` values remain initial
fallbacks until intervals are saved through Settings. Saved values then take
precedence even if the old Docker variables are still present. Now playing still
polls only while the page is visible. Intervals are polling settings, not a promise
of provider availability or end-to-end latency.

API keys is now a section under **Settings → API keys**. The previous
`/integrations/api-keys` address redirects there; key creation/revocation remains
part of the separate read-API implementation.

### Authorization and recovery

Connect/Reconnect Spotify opens a separate browser tab. Keep the scrobbl.rs tab:
its dialog waits for this specific authorization attempt, then shows **Connected**
with a check mark or the refusal/error and a retry action. A previously saved
authorization does not count as success for a new attempt. Spotify may return
immediately when consent has already been granted.

If the browser blocks the tab, allow new tabs for this site and choose **Try again**.
Closing the Spotify tab shows an interrupted attempt where the browser can detect
closure; browser isolation may make the tab unreachable instead. The original tab
continues checking the server in either case, so a real callback can still complete.
**Cancel** invalidates the pending attempt. Retrying supersedes it. Attempts expire
after ten minutes; the UI also bounds its wait if a callback is interrupted.
If the original tab is gone, the return tab displays the result with a link back
to Connections. No access token or refresh token is sent to either tab.

The server preserves the original authenticated session, CSRF checks and single-use
OAuth state. A new private-database migration records the attempt outcome; it does
not alter listening data. Live Firefox/Safari acceptance remains separate from the
automated browser checks.

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
