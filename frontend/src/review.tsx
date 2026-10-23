import { useState } from "react";
import { api } from "./api";
import type { ReviewPage as ReviewData } from "./api";
import type { Preferences } from "./generated";
import { useLoad } from "./load";
import { Link } from "./navigation";
import { musicLink } from "./selection";
import { Cover, date, duration, ErrorText, Heading, Icon, number } from "./ui";

function Comparison({
  group,
  preferences,
  busy,
  decide,
}: {
  group: ReviewData["groups"][number];
  preferences: Preferences;
  busy: boolean;
  decide: (keep: string | null) => Promise<void>;
}) {
  const [selected, setSelected] = useState<string | null>(null);
  return (
    <article className="panel review-comparison">
      <header>
        <div>
          <h2>{group.items[0]?.title}</h2>
          <p>
            {group.items.length > 1
              ? "Compare the source and playback time, then choose what to keep."
              : "Check this record and mark it reviewed when it looks correct."}
          </p>
        </div>
        <span className="subtle">{group.items.length} records</span>
      </header>
      <div className="review-records">
        {group.items.map((item) => (
          <label
            className={`review-record ${selected === item.id ? "selected" : ""}`}
            key={item.id}
          >
            {group.items.length > 1 && (
              <input
                type="radio"
                name={`keep-${group.id}`}
                aria-label={`Keep ${item.sources.join(" + ")} record`}
                checked={selected === item.id}
                disabled={busy}
                onChange={() => setSelected(item.id)}
              />
            )}
            <Cover url={item.cover_url} />
            <span className="review-identity">
              <Link to={musicLink("tracks", item.track_id)}>{item.title}</Link>
              <small>{item.artists.map((a) => a.name).join(", ")}</small>
              <small>{item.album || "Unknown album"}</small>
            </span>
            <span className="review-evidence">
              <strong>
                {date(item.played_at_ms, {
                  ...preferences,
                  timezone: item.timezone,
                })}
              </strong>
              <small>{item.timezone}</small>
              <small>
                {item.sources
                  .map(
                    (source) =>
                      ({
                        spotify_api: "Spotify collection",
                        spotify_import: "Spotify import",
                        listenbrainz: "ListenBrainz",
                      })[source] || source,
                  )
                  .join(" + ")}
              </small>
            </span>
            <span className="review-duration">
              {item.duration_kind === "estimated" ? "≈ " : ""}
              {duration(item.listened_ms)}
              <small>
                {item.duration_kind === "actual"
                  ? "Actual playback"
                  : item.duration_kind === "estimated"
                    ? "Estimated"
                    : "No duration"}
              </small>
              <small>{new Date(item.played_at_ms).toISOString()}</small>
            </span>
          </label>
        ))}
      </div>
      <div className="review-actions">
        <button
          className="secondary"
          disabled={busy}
          onClick={() => void decide(null)}
        >
          <Icon>done_all</Icon>
          {group.items.length > 1
            ? "Keep all as separate listens"
            : "Mark reviewed"}
        </button>
        {group.items.length > 1 && (
          <button
            className="primary"
            disabled={busy || !selected}
            onClick={() => void decide(selected)}
          >
            <Icon>check</Icon>Keep selected · move {group.items.length - 1} to
            trash
          </button>
        )}
      </div>
    </article>
  );
}

export function ReviewPage({
  preferences,
  refresh,
}: {
  preferences: Preferences;
  refresh: () => Promise<void>;
}) {
  const [page, setPage] = useState(0);
  const [decisionsPage, setDecisionsPage] = useState(0);
  const [version, setVersion] = useState(0);
  const result = useLoad(
    () => api.review(page, decisionsPage),
    `review:${page}:${decisionsPage}:${version}`,
    true,
    undefined,
    true,
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const automatic = useLoad(
    () => api.automaticReview(),
    `automatic:${version}`,
    true,
  );
  async function action(run: () => Promise<unknown>, message: string) {
    setBusy(true);
    setError("");
    setMessage("");
    try {
      await run();
      setMessage(message);
      setVersion((v) => v + 1);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <>
      <Heading title="Review possible duplicates">
        Decide which records describe the same listen. Original source records
        stay saved, and you can undo each decision below.
      </Heading>
      <div className="review-toolbar">
        <Link
          className="button secondary"
          to="/history?status=review&period=all"
        >
          <Icon>arrow_back</Icon>View flagged history
        </Link>
        <button
          className="secondary"
          disabled={busy}
          onClick={() => {
            setError("");
            setVersion((v) => v + 1);
          }}
        >
          <Icon>refresh</Icon>Refresh comparisons
        </button>
        {result.data && (
          <strong>{number.format(result.data.total)} records to review</strong>
        )}
      </div>
      <ErrorText>{error || result.error}</ErrorText>
      <section
        className="panel padded automatic-review"
        aria-label="Automatic duplicate review"
      >
        <h2>Resolve matching duplicates automatically</h2>
        <p>
          Only isolated pairs starting less than one second apart qualify. They
          must describe the same Spotify track, or have exactly matching title,
          album and artist credits, and come from different sources. Repeats,
          competing nearby records, incomplete metadata and conflicting playback
          evidence stay for manual review.
        </p>
        <p>
          The record with actual playback time takes priority, followed by
          estimated time. Other records move to trash; every original stays
          saved. You can undo the whole batch below.
        </p>
        <ErrorText>{automatic.error}</ErrorText>
        {automatic.data ? (
          <div className="review-actions">
            <span role="status">
              <strong>
                {number.format(automatic.data.groups)} matching{" "}
                {automatic.data.groups === 1 ? "pair" : "pairs"}
              </strong>{" "}
              · {number.format(automatic.data.records)} records ·{" "}
              {number.format(automatic.data.remaining)} remain for manual review
            </span>
            <button
              className="primary"
              disabled={busy || !automatic.data.groups}
              onClick={() =>
                void action(
                  () => api.executeAutomaticReview(automatic.data!.token),
                  "Matching duplicates resolved. Undo below restores the entire batch.",
                )
              }
            >
              <Icon>done_all</Icon>Resolve{" "}
              {number.format(automatic.data.groups)}{" "}
              {automatic.data.groups === 1 ? "pair" : "pairs"} automatically
            </button>
          </div>
        ) : (
          !automatic.error && <p role="status">Checking automatic matches…</p>
        )}
      </section>
      {message && (
        <p className="review-feedback" role="status">
          <Icon>check_circle</Icon>
          {message}
        </p>
      )}
      {!result.data && !result.error && (
        <p role="status">Loading comparisons…</p>
      )}
      {result.data?.groups.map((group) => (
        <Comparison
          key={`${group.id}:${group.items.map((i) => i.revision).join(":")}`}
          group={group}
          preferences={preferences}
          busy={busy}
          decide={(keep) =>
            action(
              () =>
                api.decideReview(
                  group.items.map((i) => i.id),
                  group.items.map((i) => i.revision),
                  keep,
                ),
              keep
                ? "Selected record kept. The other records are in trash; Undo restores them."
                : "Records marked reviewed and kept as separate listens.",
            )
          }
        />
      ))}
      {result.data && !result.data.groups.length && (
        <div className="panel empty">
          <Icon>task_alt</Icon>
          <h2>
            {result.data.total
              ? "No more records on this page"
              : "All caught up"}
          </h2>
          <p>
            {result.data.total
              ? "Go back to review the remaining records."
              : "There are no flagged records to review."}
          </p>
        </div>
      )}
      {result.data && (page > 0 || result.data.total > 50) && (
        <div className="pagination">
          <button
            className="secondary"
            disabled={busy || page === 0}
            onClick={() => setPage((p) => p - 1)}
          >
            Previous records
          </button>
          <span>Page {page + 1}</span>
          <button
            className="secondary"
            disabled={busy || (page + 1) * 50 >= result.data.total}
            onClick={() => setPage((p) => p + 1)}
          >
            Next records
          </button>
        </div>
      )}
      {!!result.data &&
        (result.data.decisions_total > 0 || decisionsPage > 0) && (
          <details className="panel review-decisions">
            <summary>Review decisions · Undo</summary>
            <div>
              {result.data.recent.map((decision) => (
                <div className="review-decision" key={decision.id}>
                  <span>
                    <strong>
                      {decision.removed
                        ? `${decision.removed} moved to trash`
                        : `${decision.count} kept separately`}
                    </strong>
                    <small>{date(decision.at_ms, preferences)}</small>
                  </span>
                  <button
                    className="secondary"
                    disabled={busy}
                    onClick={() =>
                      void action(
                        () => api.undoReview(decision.id),
                        "Decision undone. The records are back in the review list.",
                      )
                    }
                  >
                    <Icon>undo</Icon>Undo
                  </button>
                </div>
              ))}
              {(result.data.decisions_total > 20 || decisionsPage > 0) && (
                <div className="pagination">
                  <button
                    className="secondary"
                    disabled={busy || decisionsPage === 0}
                    onClick={() => setDecisionsPage((p) => p - 1)}
                  >
                    Newer decisions
                  </button>
                  <span>Page {decisionsPage + 1}</span>
                  <button
                    className="secondary"
                    disabled={
                      busy ||
                      (decisionsPage + 1) * 20 >= result.data.decisions_total
                    }
                    onClick={() => setDecisionsPage((p) => p + 1)}
                  >
                    Older decisions
                  </button>
                </div>
              )}
            </div>
          </details>
        )}
    </>
  );
}
