import { useState } from "react";
import { api } from "./api";
import type { ReviewPage as ReviewData } from "./api";
import type { Preferences } from "./generated";
import { useLoad } from "./load";
import { Link } from "./navigation";
import { musicLink } from "./selection";
import { Cover, date, duration, ErrorText, Heading, Icon, number } from "./ui";

function Trash({
  preferences,
  version,
  busy,
  restore,
}: {
  preferences: Preferences;
  version: number;
  busy: boolean;
  restore: (id: string) => void;
}) {
  const [page, setPage] = useState(0);
  const result = useLoad(
    () => api.reviewTrash(page),
    `trash:${page}:${version}`,
    true,
    undefined,
    true,
  );
  return (
    <section aria-label="Recoverable listens">
      <h2>Trash</h2>
      <p className="subtle">
        Records removed during duplicate review. Restoring a decision returns
        all of its records to review, including records on other pages. Original
        source records stay saved.
      </p>
      <ErrorText>{result.error}</ErrorText>
      {!result.data && !result.error && <p role="status">Loading trash…</p>}
      {result.data && (
        <>
          <p className="subtle">
            {number.format(result.data.total)}{" "}
            {result.data.total === 1 ? "record" : "records"} in trash
          </p>
          {!result.data.items.length && (
            <div className="panel empty">
              <Icon>delete_outline</Icon>
              <h2>
                {result.data.total
                  ? "No records on this page"
                  : "Trash is empty"}
              </h2>
              <p>
                {result.data.total
                  ? "Go back to see the remaining records."
                  : "Removed duplicates will appear here for recovery."}
              </p>
            </div>
          )}
          {result.data.items.map((item) => (
            <article className="panel trash-record" key={item.id}>
              <Cover url={item.cover_url} />
              <div className="review-identity">
                <Link to={musicLink("tracks", item.track_id)}>
                  {item.title}
                </Link>
                <small>
                  {item.artists.map((a) => a.name).join(", ")} ·{" "}
                  {item.album || "Unknown album"}
                </small>
                <small>
                  {date(item.played_at_ms, {
                    ...preferences,
                    timezone: item.timezone,
                  })}{" "}
                  · {item.sources.join(" + ")}
                </small>
              </div>
              <div>
                <strong>{duration(item.listened_ms)}</strong>
                <small>Removed {date(item.trashed_at_ms, preferences)}</small>
              </div>
              <button
                className="secondary"
                disabled={busy || !item.decision_id}
                onClick={() => item.decision_id && restore(item.decision_id)}
              >
                <Icon>restore_from_trash</Icon>Restore decision
              </button>
            </article>
          ))}
          {(page > 0 || result.data.total > 50) && (
            <div className="pagination">
              <button
                className="secondary"
                disabled={busy || page === 0}
                onClick={() => setPage((p) => p - 1)}
              >
                Previous trash records
              </button>
              <span>Page {page + 1}</span>
              <button
                className="secondary"
                disabled={busy || (page + 1) * 50 >= result.data.total}
                onClick={() => setPage((p) => p + 1)}
              >
                Next trash records
              </button>
            </div>
          )}
        </>
      )}
    </section>
  );
}

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
  const [view, setView] = useState<"comparisons" | "trash">("comparisons");
  const [decisionsPage, setDecisionsPage] = useState(0);
  const [version, setVersion] = useState(0);
  const [autoVersion, setAutoVersion] = useState(0);
  const [resolvedIds, setResolvedIds] = useState<Set<string>>(new Set());
  const [decidingId, setDecidingId] = useState<string | null>(null);
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
    `automatic:${autoVersion}`,
    true,
  );

  const visibleGroups = (result.data?.groups || []).filter(
    (g) => !resolvedIds.has(g.id),
  );
  const resolvedItemsCount = (result.data?.groups || [])
    .filter((g) => resolvedIds.has(g.id))
    .reduce((sum, g) => sum + g.items.length, 0);
  const currentTotal = Math.max(
    0,
    (result.data?.total ?? 0) - resolvedItemsCount,
  );

  async function decideOne(
    group: ReviewData["groups"][number],
    keep: string | null,
  ) {
    setDecidingId(group.id);
    setError("");
    setMessage("");
    try {
      await api.decideReview(
        group.items.map((i) => i.id),
        group.items.map((i) => i.revision),
        keep,
      );
      setResolvedIds((prev) => new Set(prev).add(group.id));
      setMessage(
        keep
          ? "Selected record kept. The other records are in trash; Undo restores them."
          : "Records marked reviewed and kept as separate listens.",
      );
      void refresh();
      if (visibleGroups.length <= 1) {
        setVersion((v) => v + 1);
        setAutoVersion((v) => v + 1);
        setResolvedIds(new Set());
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setDecidingId(null);
    }
  }

  async function action(run: () => Promise<unknown>, message: string) {
    setBusy(true);
    setError("");
    setMessage("");
    try {
      await run();
      setMessage(message);
      setResolvedIds(new Set());
      setVersion((v) => v + 1);
      setAutoVersion((v) => v + 1);
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
      <div className="review-toolbar review-controls">
        <Link
          className="button secondary"
          to="/history?status=review&period=all"
        >
          <Icon>arrow_back</Icon>Flagged history
        </Link>
        <div className="review-views" role="group" aria-label="Review views">
          <button
            className="secondary"
            aria-pressed={view === "comparisons"}
            onClick={() => setView("comparisons")}
          >
            <Icon>compare_arrows</Icon>Comparisons
          </button>
          <button
            className="secondary"
            aria-pressed={view === "trash"}
            onClick={() => setView("trash")}
          >
            <Icon>delete_outline</Icon>Trash
          </button>
        </div>
        <button
          className="secondary icon-button"
          aria-label="Refresh review"
          disabled={busy || !!decidingId}
          onClick={() => {
            setError("");
            setResolvedIds(new Set());
            setVersion((v) => v + 1);
            setAutoVersion((v) => v + 1);
          }}
        >
          <Icon>refresh</Icon>
        </button>
        {view === "comparisons" && result.data && (
          <strong className="review-count">
            {number.format(currentTotal)} records to review
          </strong>
        )}
      </div>
      {message && (
        <p className="review-feedback" role="status">
          <Icon>check_circle</Icon>
          {message}
        </p>
      )}
      <ErrorText>{error}</ErrorText>
      {view === "trash" ? (
        <Trash
          preferences={preferences}
          version={version}
          busy={busy || !!decidingId}
          restore={(id) =>
            void action(
              () => api.undoReview(id),
              "Decision restored. All associated records are back in the review list.",
            )
          }
        />
      ) : (
        <>
          <ErrorText>{result.error}</ErrorText>
          <details
            className="panel automatic-review"
            role="region"
            aria-label="Automatic duplicate review"
          >
            <summary>
              <h2>Resolve matching duplicates automatically</h2>
              <span className="automatic-preview-count">
                {automatic.error
                  ? "Preview unavailable"
                  : automatic.data
                    ? `${number.format(automatic.data.groups)} matching ${automatic.data.groups === 1 ? "pair" : "pairs"}`
                    : "Checking…"}
              </span>
              <Icon>expand_more</Icon>
            </summary>
            <div className="automatic-review-body">
              <p>
                Only isolated pairs starting less than one second apart qualify.
                They must describe the same Spotify track, or have exactly
                matching title, album and artist credits. Repeats, competing
                nearby records, incomplete metadata and conflicting playback
                evidence stay for manual review.
              </p>
              <p>
                The record with actual playback time takes priority, followed by
                estimated time. Other records move to trash; every original
                stays saved. You can undo the whole batch below.
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
                    {number.format(automatic.data.remaining)} remain for manual
                    review
                  </span>
                  <button
                    className="primary"
                    disabled={busy || !!decidingId || !automatic.data.groups}
                    onClick={() =>
                      void action(
                        () => api.executeAutomaticReview(automatic.data!.token),
                        "Matching duplicates resolved. Undo below restores the entire batch.",
                      )
                    }
                  >
                    <Icon>done_all</Icon>Resolve{" "}
                    {number.format(automatic.data.groups)}{" "}
                    {automatic.data.groups === 1 ? "pair" : "pairs"}{" "}
                    automatically
                  </button>
                </div>
              ) : (
                !automatic.error && (
                  <p role="status">Checking automatic matches…</p>
                )
              )}
            </div>
          </details>
          {!result.data && !result.error && (
            <p role="status">Loading comparisons…</p>
          )}
          {visibleGroups.map((group) => (
            <Comparison
              key={`${group.id}:${group.items.map((i) => i.revision).join(":")}`}
              group={group}
              preferences={preferences}
              busy={busy || decidingId === group.id}
              decide={(keep) => decideOne(group, keep)}
            />
          ))}
          {result.data && !visibleGroups.length && (
            <div className="panel empty">
              <Icon>task_alt</Icon>
              <h2>
                {currentTotal
                  ? "No more records on this page"
                  : "All caught up"}
              </h2>
              <p>
                {currentTotal
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
                          (decisionsPage + 1) * 20 >=
                            result.data.decisions_total
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
      )}
    </>
  );
}
