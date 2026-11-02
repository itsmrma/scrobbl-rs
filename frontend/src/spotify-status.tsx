import type { Preferences, SpotifyStatus } from "./generated";
import { date, Notice } from "./ui";
export function SpotifyNotice({
  status,
  preferences,
}: {
  status: SpotifyStatus | null;
  preferences?: Preferences;
}) {
  if (!status) return null;
  const waiting = !!status.retry_at_ms && status.retry_at_ms > Date.now();
  if (waiting) {
    const minutes = Math.max(
      1,
      Math.ceil((status.retry_at_ms! - Date.now()) / 60000),
    );
    const isQuota = status.rate_limit_reason === "QUOTA_EXCEEDED";
    return (
      <Notice
        tone="warning"
        title={
          isQuota
            ? "Spotify developer quota exceeded"
            : "Spotify rate limit cooldown active"
        }
      >
        <p>
          {isQuota
            ? "Spotify developer application quota has been exceeded."
            : "Spotify has asked this app to wait due to a request rate limit."}{" "}
          Collection and now playing will resume automatically in about{" "}
          {minutes} {minutes === 1 ? "minute" : "minutes"}
          {preferences ? ` (${date(status.retry_at_ms, preferences)})` : ""}.
        </p>
        <p>
          Your saved listens and images are available. You can wait for the
          cooldown to expire or retry connection now in Connections.
        </p>
      </Notice>
    );
  }
  if (status.connection === "action_required")
    return (
      <Notice title="Spotify needs your attention">
        <p>
          {status.last_error ||
            "Reconnect your account and check the app permissions in Spotify."}
        </p>
        <p>Open Connections to reconnect or check your Spotify app settings.</p>
      </Notice>
    );
  if (status.last_error)
    return (
      <Notice title="Spotify couldn’t finish the last update">
        <p>{status.last_error}</p>
        <p>
          Collection will retry automatically. Your saved listening history is
          available.
        </p>
      </Notice>
    );
  return null;
}
