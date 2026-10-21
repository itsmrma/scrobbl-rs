import { api } from "./api";
import type { Activity, ActivityDetail, Preferences } from "./generated";
import { date, ErrorText, Heading, useToast } from "./ui";
import { Link } from "./navigation";
import { useLoad } from "./library";
import { useEffect, useState } from "react";
export const isActive = (job: Activity) =>
  ["queued", "running", "waiting"].includes(job.state);
export const activityName = (job: Activity) =>
  job.kind === "calendar"
    ? "Recalculate assigned calendar"
    : job.filename || `${job.kind.replaceAll("_", " ")} activity`;
export function Progress({ job }: { job: Activity }) {
  const text =
    job.expected_rows === null
      ? `${job.processed} processed · total not yet known`
      : `${job.processed} of ${job.expected_rows} processed`;
  return (
    <div className="activity-progress">
      {isActive(job) && (
        <progress
          aria-label={`${activityName(job)} progress`}
          max={job.expected_rows || undefined}
          value={
            job.expected_rows
              ? Math.min(job.processed, job.expected_rows)
              : undefined
          }
        />
      )}
      <small>{text}</small>
    </div>
  );
}
export function Activities({
  jobs,
  error,
  path,
  preferences,
  refresh,
}: {
  jobs: Activity[];
  error: string;
  path: string;
  preferences: Preferences;
  refresh: () => Promise<void>;
}) {
  const id = path.startsWith("/activities/")
    ? path.slice("/activities/".length)
    : "";
  const [listPage, setListPage] = useState(0);
  const [kind, setKind] = useState("");
  const history = useLoad<Activity[]>(
    () => api.activities(listPage, kind),
    `${listPage}:${kind}`,
    !id && (!!listPage || !!kind),
    3000,
  );
  const visibleJobs = listPage || kind ? history.data || [] : jobs;
  const [issuesPage, setIssuesPage] = useState(0);
  const detail = useLoad<ActivityDetail>(
    () => api.activity(id, issuesPage),
    `${id}:${issuesPage}`,
    !!id,
    2000,
  );
  const toast = useToast();
  const [actionError, setActionError] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    setActionError("");
    setIssuesPage(0);
  }, [id]);
  const job = detail.data?.activity;
  return (
    <>
      <Heading eyebrow="YOUR ARCHIVE AT WORK" title="Activities">
        Progress and saved outcomes remain available across pages and browser
        visits.
      </Heading>
      <ErrorText>{error}</ErrorText>
      {id ? (
        <>
          <Link to="/activities">All activities</Link>
          <ErrorText>{detail.error}</ErrorText>
          <ErrorText>{actionError}</ErrorText>
          {job && (
            <article className="panel padded">
              <h2>{activityName(job)}</h2>
              <p role="status">
                {job.outcome || job.state} · phase {job.phase} · attempt{" "}
                {job.attempt}
              </p>
              {job.target_timezone && (
                <p>
                  Requested calendar: {job.target_timezone} · week starts{" "}
                  {
                    [
                      "Monday",
                      "Tuesday",
                      "Wednesday",
                      "Thursday",
                      "Friday",
                      "Saturday",
                      "Sunday",
                    ][job.target_week_start || 0]
                  }
                </p>
              )}
              <Progress job={job} />
              {job.kind === "spotify_import" && (
                <p>
                  {job.inventory_complete
                    ? "Inventory complete"
                    : "Inventory not yet complete"}
                  . Saved rows remain available after errors or cancellation.
                  Row counts and original counts are not totals of new listens.
                </p>
              )}
              {job.error_code && (
                <p className="form-error">
                  {job.error_code === "calendar_recalculation_failed"
                    ? "Calendar recalculation failed. The previous calendar is still active. Check server storage and retry."
                    : `Activity reported: ${job.error_code}`}
                </p>
              )}
              {job.retry_at_ms && (
                <p>Next attempt: {date(job.retry_at_ms, preferences)}</p>
              )}
              <dl className="diagnostics">
                {(
                  [
                    "originals",
                    "variants",
                    "below_threshold",
                    "added",
                    "reconciled",
                    "duplicates",
                    "invalid",
                    "skipped",
                    "ambiguous",
                  ] as const
                ).map((key) => (
                  <div key={key}>
                    <dt>
                      {
                        {
                          originals: "New original observations",
                          variants: "New original variants",
                          below_threshold: "Below-threshold music rows",
                          added: "New listens",
                          reconciled: "Reconciliations",
                          duplicates: "Known source rows",
                          invalid: "Invalid source rows",
                          skipped: "Non-musical source rows",
                          ambiguous: "Ambiguous matches",
                        }[key]
                      }
                    </dt>
                    <dd>{job[key]}</dd>
                  </div>
                ))}
              </dl>
              {["calendar", "spotify_import"].includes(job.kind) &&
                job.phase !== "receiving" && (
                  <div className="connection-actions">
                    {(isActive(job)
                      ? ["cancel"]
                      : ["failed", "cancelled"].includes(job.state) ||
                          job.outcome === "partial"
                        ? ["retry"]
                        : []
                    ).map((action) => (
                      <button
                        key={action}
                        className="secondary"
                        disabled={busy}
                        onClick={async () => {
                          setBusy(true);
                          setActionError("");
                          try {
                            await api.activityAction(job.id, action);
                            await refresh();
                            toast(
                              action === "cancel"
                                ? "Cancellation requested. Confirmed changes remain saved."
                                : "Activity queued again. Confirmed rows will not be repeated.",
                            );
                          } catch (e) {
                            setActionError(
                              e instanceof Error ? e.message : String(e),
                            );
                            toast(
                              e instanceof Error ? e.message : String(e),
                              true,
                            );
                          } finally {
                            setBusy(false);
                          }
                        }}
                      >
                        {action === "cancel"
                          ? job.kind === "calendar"
                            ? "Cancel recalculation"
                            : "Cancel import"
                          : job.kind === "calendar"
                            ? "Retry recalculation"
                            : "Retry import"}
                      </button>
                    ))}
                  </div>
                )}
              {detail.data && detail.data.issues?.length > 0 && (
                <div>
                  <h3>Invalid source rows (page {issuesPage + 1})</h3>
                  <div className="connection-actions">
                    <button
                      className="secondary"
                      disabled={issuesPage === 0}
                      onClick={() => setIssuesPage((p) => p - 1)}
                    >
                      Previous errors
                    </button>
                    <button
                      className="secondary"
                      disabled={(issuesPage + 1) * 100 >= job.invalid}
                      onClick={() => setIssuesPage((p) => p + 1)}
                    >
                      Next errors
                    </button>
                  </div>
                  {detail.data.issues.map((issue) => (
                    <p key={`${issue.member}:${issue.row_number}`}>
                      {issue.member}, row {issue.row_number}: {issue.reason}
                    </p>
                  ))}
                </div>
              )}
              {detail.data && (
                <p>
                  Files discovered:{" "}
                  {
                    detail.data.members.filter((m) => m.kind !== "directory")
                      .length
                  }
                  . Processed:{" "}
                  {
                    detail.data.members.filter((m) =>
                      ["completed", "partial"].includes(m.state),
                    ).length
                  }
                  . Excluded:{" "}
                  {
                    detail.data.members.filter(
                      (m) => m.state === "excluded" && m.kind !== "directory",
                    ).length
                  }
                  . Failed:{" "}
                  {
                    detail.data.members.filter((m) => m.state === "failed")
                      .length
                  }
                  . Still to process:{" "}
                  {
                    detail.data.members.filter((m) =>
                      ["pending", "received", "processing"].includes(m.state),
                    ).length
                  }
                  .
                </p>
              )}
              {detail.data?.members.map((member) => (
                <div className="job" key={member.member}>
                  <strong>{member.member}</strong>
                  <span>
                    {member.kind} · {member.state} · {member.processed}{" "}
                    processed
                    {member.expected_rows === null
                      ? " · total unknown"
                      : ` of ${member.expected_rows}`}
                  </span>
                  <p>
                    {member.originals} new originals | {member.variants} new
                    variants | {member.duplicates} known rows | {member.added}{" "}
                    new listens | {member.reconciled} reconciled |{" "}
                    {member.invalid} invalid | {member.skipped} non-music |{" "}
                    {member.below_threshold} below threshold
                  </p>
                  {member.error_code && (
                    <p className="form-error">{member.error_code}</p>
                  )}
                </div>
              ))}
            </article>
          )}
        </>
      ) : (
        <div className="panel padded">
          <label>
            Activity type
            <select
              value={kind}
              onChange={(e) => {
                setKind(e.target.value);
                setListPage(0);
              }}
            >
              <option value="">All activities</option>
              <option value="spotify_import">Spotify imports</option>
              <option value="spotify_collection">Spotify collection</option>
              <option value="calendar">Calendar</option>
            </select>
          </label>
          <ErrorText>{history.error}</ErrorText>
          <div className="connection-actions">
            <button
              className="secondary"
              disabled={listPage === 0}
              onClick={() => setListPage((p) => p - 1)}
            >
              Previous activities
            </button>
            <span>Page {listPage + 1}</span>
            <button
              className="secondary"
              disabled={visibleJobs.length < 100}
              onClick={() => setListPage((p) => p + 1)}
            >
              Next activities
            </button>
          </div>
          {visibleJobs.length ? (
            visibleJobs.map((job) => (
              <article className="activity-row" key={job.id}>
                <Link to={`/activities/${job.id}`}>
                  <strong>{activityName(job)}</strong>
                </Link>
                <span className={job.state === "failed" ? "form-error" : ""}>
                  {job.outcome || job.state}
                </span>
                <Progress job={job} />
              </article>
            ))
          ) : (
            <p>
              No persistent activities yet. Existing JSON import reports are in{" "}
              <Link to="/settings/data">Data & recovery</Link>.
            </p>
          )}
        </div>
      )}
    </>
  );
}
