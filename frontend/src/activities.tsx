import type { ArtworkActivity } from "./artwork-activity";
import { api } from "./api";
import type { Activity, ActivityDetail, Preferences } from "./generated";
import { date, ErrorText, Heading, Icon, number, useToast } from "./ui";
import { Link } from "./navigation";
import { useLoad } from "./library";
import { useEffect, useState } from "react";
import { ListenBrainzOriginals } from "./listenbrainz";
const jobMessage = (code: string) => {
  const messages: Record<string, string> = {
    coverage_uncertain_use_official_zip:
      "Coverage is uncertain. Saved listens are retained. Upload the official ListenBrainz ZIP to complete this interval safely.",
    partial_import_review_report:
      "Import finished with rejected rows or unreadable parts. Valid rows are saved; review the report and originals below.",
    listenbrainz_username_not_found:
      "This public ListenBrainz username was not found. Check the name and start another fetch.",
    listenbrainz_rate_wait:
      "Waiting for ListenBrainz rate limits. This activity will resume automatically.",
    listenbrainz_network_wait:
      "ListenBrainz could not be reached. Saved progress is safe; retrying automatically.",
    listenbrainz_service_wait:
      "ListenBrainz is temporarily unavailable or limiting requests. Waiting before retrying.",
    invalid_archive_or_configured_limit_review_original:
      "The archive is unreadable, has an account conflict or exceeds a configured limit. Saved rows and the original file are retained. Review the files and retry.",
    listenbrainz_processing_failed_saved_rows_retained:
      "Processing stopped. Confirm server storage is available and retry. Confirmed rows remain saved.",
    malformed_page:
      "ListenBrainz returned a malformed page. Saved rows remain; retry or upload the official ZIP.",
    nonadvancing_or_unsorted_page:
      "ListenBrainz pagination did not advance safely. Saved rows remain; use the official ZIP for complete coverage.",
  };
  return (
    messages[code] ||
    `Activity reported: ${code}. Saved rows are retained; retry or use the official ZIP.`
  );
};
function SourceInterval({ params }: { params: unknown }) {
  if (params && typeof params === "object" && "account" in params) {
    return (
      <p>
        Source account:{" "}
        {typeof params.account === "string" ? params.account : "unknown"}.
        Changed payloads without a verified playback identifier stay separate.
        Matches requiring an account assumption are flagged for review.
      </p>
    );
  }
  if (
    !params ||
    typeof params !== "object" ||
    !("start" in params) ||
    !("end" in params) ||
    !("timezone" in params) ||
    !("username" in params) ||
    typeof params.start !== "number" ||
    typeof params.end !== "number"
  )
    return null;
  return (
    <p>
      Public account: {String(params.username)} · saved interval:{" "}
      {new Date(params.start * 1000).toISOString()} through{" "}
      {new Date(params.end * 1000 - 1).toISOString()} · selected calendar:{" "}
      {String(params.timezone)}. This interval stays fixed on retry.
    </p>
  );
}
export const isActive = (job: Activity) =>
  ["queued", "running", "waiting"].includes(job.state);
export const activityName = (job: Activity) =>
  job.kind === "calendar"
    ? "Recalculate assigned calendar"
    : job.filename || `${job.kind.replaceAll("_", " ")} activity`;
const stateLabel = (job: Activity) =>
  ({
    queued: "Queued",
    running: "In progress",
    waiting: "Waiting to retry",
    completed:
      job.outcome === "partial"
        ? "Partially complete — review report"
        : job.outcome === "no_music"
          ? "No music found"
          : "Complete",
    failed: "Needs attention",
    cancelled: "Cancelled",
  })[job.state] || job.state;
const phaseLabel = (job: Activity) =>
  ({
    receiving: "Receiving your files",
    inventory: "Finding files in the archive",
    processing: "Adding source rows to your archive",
    finished: "Report saved",
    queued: "Waiting for the server to start",
  })[job.phase] ||
  job.phase?.replaceAll("_", " ") ||
  "Server activity";
function StateBadge({ job }: { job: Activity }) {
  return (
    <span className={`state-badge ${job.state}`}>
      <Icon>
        {job.state === "completed"
          ? "check_circle"
          : job.state === "failed"
            ? "error_outline"
            : job.state === "cancelled"
              ? "cancel"
              : "schedule"}
      </Icon>
      {stateLabel(job)}
    </span>
  );
}
export function Progress({ job }: { job: Activity }) {
  const text =
    job.expected_rows === null
      ? isActive(job)
        ? `${job.processed} processed · total not yet known`
        : `${job.processed} source rows processed`
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
export function SidebarActivity({ job }: { job: Activity }) {
  return (
    <Link to={"/activities/" + job.id} className="sidebar-job">
      <span className="sidebar-job-heading">
        <Icon>{job.state === "waiting" ? "schedule" : "sync"}</Icon>
        <strong>{activityName(job)}</strong>
        <Icon>chevron_right</Icon>
      </span>
      <small>{phaseLabel(job)}</small>
      <Progress job={job} />
    </Link>
  );
}
export function ArtworkWork({
  work,
  compact = false,
}: {
  work: ArtworkActivity;
  compact?: boolean;
}) {
  if (!work.total) return null;
  const progress = (
    <div className="activity-progress">
      {work.pending > 0 && (
        <progress
          aria-label="Artwork download progress"
          value={work.loaded + work.failed}
          max={work.total}
        />
      )}
      <small>
        {number.format(work.loaded)} saved · {number.format(work.pending)}{" "}
        remaining
        {work.failed > 0
          ? " · " + number.format(work.failed) + " could not load"
          : ""}
      </small>
    </div>
  );
  return compact ? (
    <Link to="/activities" className="sidebar-job artwork-work">
      <span className="sidebar-job-heading">
        <Icon>downloading</Icon>
        <strong>Downloading artwork</strong>
        <Icon>chevron_right</Icon>
      </span>
      {progress}
    </Link>
  ) : (
    <article
      className="activity-row panel artwork-work"
      aria-label="Artwork downloads"
    >
      <div className="activity-symbol">
        <Icon>{work.pending ? "downloading" : "photo_library"}</Icon>
      </div>
      <div className="activity-content">
        <div className="activity-row-heading">
          <strong>
            {work.pending ? "Downloading artwork" : "Artwork downloads"}
          </strong>
          <span className="state-badge">
            {work.pending
              ? "In progress"
              : work.failed
                ? "Finished with unavailable images"
                : "Complete"}
          </span>
        </div>
        <p>Artwork is saved locally and reused across your archive.</p>
        {progress}
      </div>
    </article>
  );
}
export function Activities({
  jobs,
  artworkWork,
  error,
  path,
  preferences,
  refresh,
}: {
  jobs: Activity[];
  artworkWork: ArtworkActivity;
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
      {!id && (
        <Heading eyebrow="YOUR ARCHIVE AT WORK" title="Activities">
          Your imports, collection and calendar updates. Follow the progress or
          open a saved report.
        </Heading>
      )}
      <ErrorText>{error}</ErrorText>
      {id ? (
        <>
          <Link to="/activities" className="back-link button secondary">
            <Icon>arrow_back</Icon>All activities
          </Link>
          <ErrorText>{detail.error}</ErrorText>
          <ErrorText>{actionError}</ErrorText>
          {job && (
            <article className="panel padded activity-detail">
              <header className="activity-detail-heading">
                <div>
                  <p className="eyebrow">ACTIVITY REPORT</p>
                  <h1>{activityName(job)}</h1>
                </div>
                <span role="status">
                  <StateBadge job={job} />
                </span>
              </header>
              <div className="activity-detail-status" role="status">
                <span>
                  {phaseLabel(job)} · Attempt {job.attempt}
                </span>
              </div>
              <Progress job={job} />
              <div className="activity-results">
                <div>
                  <strong>{number.format(job.added)}</strong>
                  <span>New listens</span>
                </div>
                <div>
                  <strong>{number.format(job.duplicates)}</strong>
                  <span>Already present</span>
                </div>
                <div>
                  <strong>{number.format(job.reconciled)}</strong>
                  <span>Reconciled</span>
                </div>
                <div>
                  <strong>{number.format(job.invalid)}</strong>
                  <span>Invalid source rows</span>
                </div>
              </div>
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
              {!!(
                detail.data?.source_parameters ||
                job.kind === "spotify_import" ||
                job.error_code ||
                job.retry_at_ms
              ) && (
                <section className="activity-context">
                  <h3>About this activity</h3>
                  <SourceInterval params={detail.data?.source_parameters} />
                  {job.kind === "spotify_import" && (
                    <p>
                      {job.inventory_complete
                        ? "Inventory complete"
                        : "Inventory not yet complete"}
                      . Saved rows remain available after errors or
                      cancellation. Row counts and original counts are not
                      totals of new listens.
                    </p>
                  )}
                  {job.error_code && (
                    <p className="form-error">
                      {job.error_code === "calendar_recalculation_failed"
                        ? "Calendar recalculation failed. The previous calendar is still active. Check server storage and retry."
                        : job.kind.startsWith("listenbrainz_")
                          ? jobMessage(job.error_code)
                          : `Activity reported: ${job.error_code}`}
                    </p>
                  )}
                  {job.retry_at_ms && (
                    <p>Next attempt: {date(job.retry_at_ms, preferences)}</p>
                  )}
                </section>
              )}
              <details className="activity-diagnostics">
                <summary>Detailed counts</summary>
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
              </details>
              {[
                "calendar",
                "spotify_import",
                "listenbrainz_file",
                "listenbrainz_fetch",
              ].includes(job.kind) &&
                job.phase !== "receiving" && (
                  <div className="connection-actions">
                    {(isActive(job)
                      ? ["cancel"]
                      : ["failed", "cancelled"].includes(job.state) ||
                          (job.outcome === "partial" &&
                            job.kind !== "listenbrainz_fetch")
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
              {detail.data && detail.data.members.length > 0 && (
                <section className="activity-files">
                  <h3>Files in this activity</h3>
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
                </section>
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
              {job.kind.startsWith("listenbrainz_") && (
                <ListenBrainzOriginals key={job.id} id={job.id} />
              )}
            </article>
          )}
        </>
      ) : (
        <div className="activities-panel">
          <div className="activities-summary">
            <div>
              <Icon>pending_actions</Icon>
              <strong>
                {jobs.filter(isActive).length +
                  (artworkWork.pending > 0 ? 1 : 0)}
              </strong>
              <span>In progress</span>
            </div>
            <div>
              <Icon>error_outline</Icon>
              <strong>
                {
                  jobs.filter(
                    (j) => j.state === "failed" || j.outcome === "partial",
                  ).length
                }
              </strong>
              <span>To review</span>
            </div>
            <p>
              From the latest {jobs.length} activities. Open a report for saved
              results and recovery actions.
            </p>
          </div>
          <ArtworkWork work={artworkWork} />
          <div className="activities-toolbar">
            <h2>Activity history</h2>
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
                <option value="listenbrainz_file">
                  ListenBrainz file imports
                </option>
                <option value="listenbrainz_fetch">
                  ListenBrainz public fetches
                </option>
                <option value="calendar">Calendar</option>
              </select>
            </label>
          </div>
          <ErrorText>{history.error}</ErrorText>
          {visibleJobs.length ? (
            visibleJobs.map((job) => (
              <article
                className={
                  "activity-row panel " + (isActive(job) ? "is-active" : "")
                }
                key={job.id}
              >
                <div className="activity-symbol">
                  <Icon>
                    {job.kind === "calendar"
                      ? "calendar_month"
                      : job.kind === "spotify_collection"
                        ? "sync"
                        : "file_upload"}
                  </Icon>
                </div>
                <div className="activity-content">
                  <div className="activity-row-heading">
                    <Link to={`/activities/${job.id}`}>
                      <strong>{activityName(job)}</strong>
                    </Link>
                    <StateBadge job={job} />
                  </div>
                  <p>{phaseLabel(job)}</p>
                  <Progress job={job} />
                  {!isActive(job) && (
                    <small>
                      {number.format(job.added)} new listens ·{" "}
                      {number.format(job.duplicates)} already present ·{" "}
                      {number.format(job.invalid)} invalid rows
                    </small>
                  )}
                  {job.error_code && (
                    <p className="form-error">
                      {job.error_code}. Open the report for details and
                      available actions.
                    </p>
                  )}
                  {job.retry_at_ms && (
                    <p>Next attempt: {date(job.retry_at_ms, preferences)}</p>
                  )}
                </div>
                <span className="activity-open" aria-hidden="true">
                  <Icon>arrow_forward</Icon>
                </span>
              </article>
            ))
          ) : (
            <div className="empty activity-empty">
              <Icon>task_alt</Icon>
              <h2>
                {artworkWork.total
                  ? "No other activities"
                  : "No activities to show"}
              </h2>
              <p>
                Imports, collection and calendar changes leave a saved report
                here. You can leave the page while the server works.
              </p>
              <Link to="/import" className="button secondary">
                Import listening history
              </Link>
              <Link to="/settings/data">
                Earlier import reports in Data &amp; recovery
              </Link>
            </div>
          )}
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
        </div>
      )}
    </>
  );
}
