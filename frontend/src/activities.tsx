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
  const detail = useLoad<ActivityDetail>(
    () => api.activity(id),
    id,
    !!id,
    2000,
  );
  const toast = useToast();
  const [actionError, setActionError] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => setActionError(""), [id]);
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
                {job.state} · attempt {job.attempt}
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
                    "added",
                    "reconciled",
                    "duplicates",
                    "invalid",
                    "skipped",
                    "ambiguous",
                  ] as const
                ).map((key) => (
                  <div key={key}>
                    <dt>{key[0].toUpperCase() + key.slice(1)}</dt>
                    <dd>{job[key]}</dd>
                  </div>
                ))}
              </dl>
              {job.kind === "calendar" && (
                <div className="connection-actions">
                  {(isActive(job)
                    ? ["cancel"]
                    : ["failed", "cancelled"].includes(job.state)
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
                              ? "Cancellation requested. The previous calendar remains active."
                              : "Calendar recalculation queued again.",
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
                        ? "Cancel recalculation"
                        : "Retry recalculation"}
                    </button>
                  ))}
                </div>
              )}
              {detail.data?.members.map((member) => (
                <div className="job" key={member.member}>
                  <strong>{member.member}</strong>
                  <span>
                    {member.state} · {member.processed} processed
                    {member.expected_rows === null
                      ? " · total unknown"
                      : ` of ${member.expected_rows}`}
                  </span>
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
          {jobs.length ? (
            jobs.map((job) => (
              <article className="activity-row" key={job.id}>
                <Link to={`/activities/${job.id}`}>
                  <strong>{activityName(job)}</strong>
                </Link>
                <span className={job.state === "failed" ? "form-error" : ""}>
                  {job.state}
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
