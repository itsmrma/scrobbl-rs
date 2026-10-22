import type { Listen, Preferences } from "./generated";
import { Cover, date, duration } from "./ui";
import { Link } from "./navigation";
import { musicLink } from "./selection";
export function ListenList({
  items,
  preferences,
  id,
}: {
  items: Listen[];
  preferences: Preferences;
  id?: string;
}) {
  return (
    <div id={id}>
      {!items.length ? (
        <div className="empty">
          <strong>No listens in this selection</strong>
          <span>Try another period or change the filters.</span>
          <Link to="/import">Import your Spotify history</Link>
        </div>
      ) : (
        items.map((item) => (
          <article className="listen-row" key={item.id}>
            <Cover url={item.cover_url} />
            <div className="listen-title">
              <Link to={musicLink("tracks", item.track_id)}>{item.title}</Link>
              <small>
                {item.artists.map((a, i) => (
                  <span key={a.id}>
                    {i > 0 && ", "}
                    <Link to={musicLink("artists", a.id)}>{a.name}</Link>
                  </span>
                ))}
                {!item.artists.length && "Unknown artist"}
              </small>
            </div>
            <div className="listen-album">
              {item.album_id ? (
                <Link to={musicLink("albums", item.album_id)}>
                  {item.album}
                </Link>
              ) : (
                "Unknown album"
              )}
            </div>
            <div className="listen-date">
              {date(item.played_at_ms, {
                ...preferences,
                timezone: item.timezone,
              })}
              <small>
                {item.timezone} · {item.local_date}
              </small>
              <small>
                {item.sources
                  .join(" + ")
                  .replaceAll("spotify_import", "Spotify import")
                  .replaceAll("spotify_api", "Spotify collection")}
              </small>
              {item.needs_review && (
                <small className="warning">Possible duplicate</small>
              )}
            </div>
            <div className="listen-duration">
              {item.duration_kind === "estimated" && "≈ "}
              {duration(item.listened_ms)}
              <small>
                {!item.counted
                  ? "Below threshold"
                  : item.duration_kind === "actual"
                    ? "Actual"
                    : item.duration_kind === "estimated"
                      ? "Estimated"
                      : "Unknown duration"}
              </small>
            </div>
          </article>
        ))
      )}
    </div>
  );
}
