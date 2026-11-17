import type { Listen, Preferences } from "./generated";
import { useId } from "react";
import { Cover, date, duration, formatSources, Icon } from "./ui";
import { Link } from "./navigation";
import { musicLink } from "./selection";
import type { Kind } from "./selection";
function ReviewIndicator() {
  const id = useId();
  return (
    <span className="review-indicator">
      <Link
        to="/review"
        aria-label="Possible duplicate — review record"
        aria-describedby={id}
      >
        <Icon>warning_amber</Icon>
      </Link>
      <span id={id} role="tooltip" aria-label="Possible duplicate">
        Possible duplicate · Review record
      </span>
    </span>
  );
}
export function ListenList({
  items,
  preferences,
  id,
  context,
}: {
  items: Listen[];
  preferences: Preferences;
  id?: string;
  context?: Kind;
}) {
  return (
    <div id={id} className="listens-list">
      {!items.length ? (
        <div className="empty">
          <strong>No listens in this selection</strong>
          <span>Try another period or change the filters.</span>
          <Link to="/import">Import your Spotify history</Link>
        </div>
      ) : (
        items.map((item) => (
          <article
            className={`listen-row ${context ? `listen-context listen-context-${context}` : ""}`}
            key={item.id}
          >
            <div className="listen-main">
              {context !== "tracks" && (
                <>
                  <Cover url={item.cover_url} />
                  <div className="listen-title">
                    <span className="listen-heading">
                      <Link to={musicLink("tracks", item.track_id)}>
                        {item.title}
                      </Link>
                      {item.needs_review && <ReviewIndicator />}
                    </span>
                    {context !== "albums" && (
                      <small className="listen-artists">
                        {item.artists.map((a, i) => (
                          <span key={a.id}>
                            {i > 0 && ", "}
                            <Link to={musicLink("artists", a.id)}>
                              {a.name}
                            </Link>
                          </span>
                        ))}
                        {!item.artists.length && "Unknown artist"}
                      </small>
                    )}
                  </div>
                </>
              )}
              {context !== "tracks" && context !== "albums" && (
                <div className="listen-album">
                  {item.album_id ? (
                    <Link to={musicLink("albums", item.album_id)}>
                      {item.album}
                    </Link>
                  ) : (
                    "Unknown album"
                  )}
                </div>
              )}
              <div className="listen-date">
                {context === "tracks" && item.needs_review && (
                  <ReviewIndicator />
                )}
                {date(item.played_at_ms, {
                  ...preferences,
                  timezone: item.timezone,
                })}
                <small>
                  {item.timezone} · {item.local_date}
                </small>
                <small>{formatSources(item.sources)}</small>
              </div>
              <div className="listen-duration">
                {context && <small>Time listened</small>}
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
            </div>
            {context && (
              <details className="listen-details">
                <summary>
                  <Icon>expand_more</Icon>
                  <span className="visually-hidden">Listen details</span>
                </summary>
                <dl>
                  <div>
                    <dt>Playback timestamp (UTC)</dt>
                    <dd>{new Date(item.played_at_ms).toISOString()}</dd>
                  </div>
                  <div>
                    <dt>Assigned date</dt>
                    <dd>
                      {item.local_date} · {item.timezone}
                    </dd>
                  </div>
                  <div>
                    <dt>Assigned week</dt>
                    <dd>{item.week_date}</dd>
                  </div>
                  <div>
                    <dt>Statistics</dt>
                    <dd>
                      {item.counted
                        ? "Counted listen"
                        : "Below the 30-second threshold"}
                      {item.needs_review
                        ? " · Possible duplicate, needs review"
                        : ""}
                    </dd>
                  </div>
                  <div>
                    <dt>Archived on</dt>
                    <dd>{date(item.first_acquired_at_ms, preferences)}</dd>
                  </div>
                  <div>
                    <dt>Time evidence</dt>
                    <dd>
                      {item.duration_kind === "actual"
                        ? "Playback time reported by the source"
                        : item.duration_kind === "estimated"
                          ? "Estimated time, not confirmed playback"
                          : "No playback duration available"}
                    </dd>
                  </div>
                </dl>
              </details>
            )}
          </article>
        ))
      )}
    </div>
  );
}
