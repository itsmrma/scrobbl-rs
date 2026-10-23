import type { Listen, Preferences } from "./generated";
import { useEffect, useRef } from "react";
import { Cover, date, duration, Icon } from "./ui";
import { Link } from "./navigation";
import { musicLink } from "./selection";
import type { Kind } from "./selection";
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
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let frame = 0;
    const pending = new Map<HTMLElement, number>();
    // Measure the metadata alone, never the expandable row or the image whose
    // dimensions we change. This also shrinks correctly when a badge disappears.
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const row = entry.target.closest<HTMLElement>(".listen-row");
        if (row && entry.contentRect.height > 0)
          pending.set(
            row,
            Math.round(Math.max(38, Math.min(72, entry.contentRect.height))),
          );
      }
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        for (const [row, size] of pending) {
          const value = `${size}px`;
          if (row.style.getPropertyValue("--listen-artwork") !== value)
            row.style.setProperty("--listen-artwork", value);
        }
        pending.clear();
      });
    });
    list.current
      ?.querySelectorAll(".listen-date")
      .forEach((el) => observer.observe(el));
    return () => {
      observer.disconnect();
      cancelAnimationFrame(frame);
    };
  }, [items]);
  return (
    <div id={id} ref={list}>
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
            {context !== "tracks" && (
              <>
                <Cover url={item.cover_url} />
                <div className="listen-title">
                  <Link to={musicLink("tracks", item.track_id)}>
                    {item.title}
                  </Link>
                  {context !== "albums" && (
                    <small className="listen-artists">
                      {item.artists.map((a, i) => (
                        <span key={a.id}>
                          {i > 0 && ", "}
                          <Link to={musicLink("artists", a.id)}>{a.name}</Link>
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
              {date(item.played_at_ms, {
                ...preferences,
                timezone: item.timezone,
              })}
              <small>
                {item.timezone} · {item.local_date}
              </small>
              <small>
                {[...new Set(item.sources)]
                  .join(" + ")
                  .replaceAll("spotify_import", "Spotify import")
                  .replaceAll("spotify_api", "Spotify collection")
                  .replaceAll("listenbrainz", "ListenBrainz")}
              </small>
              {item.needs_review && (
                <small className="warning">Possible duplicate</small>
              )}
            </div>
            {context && (
              <div className="listen-archive">
                <small>Added to archive</small>
                {date(item.first_acquired_at_ms, preferences)}
              </div>
            )}
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
