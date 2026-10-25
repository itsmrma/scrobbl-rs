import { Select } from "./select";
import type { ArtworkActivity } from "./artwork-activity";
import { api } from "./api";
import type { Activity, ActivityDetail, Preferences } from "./generated";
import { date, ErrorText, Heading, Icon, number, useToast } from "./ui";
import { Link } from "./navigation";
import { useLoad } from "./load";
import { useEffect, useState } from "react";
import { ListenBrainzOriginals } from "./listenbrainz";
const jobMessage = (code: string) => {
  const messages: Record<string, string> = {
    artwork_connection_required:
      "Connect Spotify in Settings to download the queued artwork. The queue is saved and resumes automatically.",
    artwork_provider_wait:
      "Spotify or its image server requested a pause, or could not be reached. Saved images remain available; downloads resume automatically.",
    artwork_paused:
      "Downloads are paused. Saved images and progress are kept, including after an application restart.",
    artwork_pacing:
      "Downloading slowly in the background to leave capacity for live listening updates.",
    artwork_capacity_wait: "Waiting for another artwork download to finish.",
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
  job.error_code === "artwork_paused"
    ? "Paused"
    : {
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
      }[job.state] || job.state;
export const phaseLabel = (job: Activity) =>
  job.kind === "artwork" && job.state === "waiting" && job.error_code
    ? {
        artwork_connection_required: "Waiting for Spotify connection",
        artwork_provider_wait: "Waiting before the next download",
        artwork_paused: "Paused",
        artwork_pacing: "Slow background download",
        artwork_capacity_wait: "Waiting for the download slot",
      }[job.error_code] || "Waiting to resume artwork"
    : {
        receiving: "Receiving your files",
        inventory: "Finding files in the archive",
        counting: "Counting source rows across all files",
        processing: "Adding source rows to your archive",
        finished: "Report saved",
        queued: "Waiting for the server to start",
        artwork: "Saving missing album and artist images",
      }[job.phase] ||
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
  const preparingSpotify =
    job.kind === "spotify_import" &&
    ["receiving", "inventory", "counting"].includes(job.phase);
  const text = preparingSpotify
    ? "Total available after all files are counted"
    : job.expected_rows === null
      ? job.kind === "spotify_import"
        ? `${number.format(job.processed)} source rows processed · total unavailable: some files could not be counted`
        : isActive(job)
          ? `${job.processed} processed · total not yet known`
          : `${job.processed} source rows processed`
      : `${number.format(job.processed)} of ${number.format(job.expected_rows)} processed`;
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
      <small>
        {job.kind === "artwork"
          ? `${number.format(job.processed)} of ${number.format(job.expected_rows || 0)} images checked`
          : text}
      </small>
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
function ArtworkSchedule({
  job,
  params,
  preferences,
}: {
  job: Activity;
  params: unknown;
  preferences: Preferences;
}) {
  const remaining = Math.max(0, (job.expected_rows || 0) - job.processed);
  const schedule = params as {
    interval_ms?: number;
    next_at_ms?: number;
  } | null;
  const interval = schedule?.interval_ms;
  if (!remaining || !interval) return null;
  const hours = Math.ceil((remaining * interval) / 3600000);
  const estimate =
    Math.max(Date.now(), job.retry_at_ms || 0, schedule?.next_at_ms || 0) +
    remaining * interval;
  return (
    <p className="artwork-estimate">
      {job.error_code === "artwork_paused"
        ? `About ${hours} hours of background work remaining after you resume.`
        : `Estimated finish: ${date(estimate, preferences)} (about ${hours} hours remaining).`}{" "}
      This is an estimate at one image per minute; Spotify cooldowns, time
      offline and additional imports can extend it.
    </p>
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
  const collectionRuns = visibleJobs.filter(
    (j) =>
      j.kind === "spotify_collection" &&
      j.state === "completed" &&
      j.outcome !== "partial" &&
      !j.error_code,
  );
  const separateJobs = visibleJobs.filter((j) => !collectionRuns.includes(j));
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
                  <div className="activity-report-title">
                    <h1>{activityName(job)}</h1>
                    <span role="status">
                      <StateBadge job={job} />
                    </span>
                  </div>
                </div>
              </header>
              <div className="activity-detail-status" role="status">
                <span>
                  {phaseLabel(job)} · Attempt {job.attempt}
                  {job.created_at_ms
                    ? ` · ${date(job.created_at_ms, preferences)}`
                    : ""}
                </span>
              </div>
              <Progress job={job} />
              <div className="activity-results">
                <div>
                  <strong>{number.format(job.added)}</strong>
                  <span>
                    {job.kind === "artwork" ? "Images saved" : "New listens"}
                  </span>
                </div>
                <div>
                  <strong>{number.format(job.duplicates)}</strong>
                  <span>
                    {job.kind === "artwork"
                      ? "Already cached"
                      : "Already present"}
                  </span>
                </div>
                <div>
                  <strong>
                    {number.format(
                      job.kind === "artwork" ? job.skipped : job.reconciled,
                    )}
                  </strong>
                  <span>
                    {job.kind === "artwork"
                      ? "No image available"
                      : "Reconciled"}
                  </span>
                </div>
                <div>
                  <strong>
                    {number.format(
                      job.kind === "artwork"
                        ? Math.max(0, (job.expected_rows || 0) - job.processed)
                        : job.invalid,
                    )}
                  </strong>
                  <span>
                    {job.kind === "artwork"
                      ? "Remaining"
                      : "Invalid source rows"}
                  </span>
                </div>
              </div>
              {job.kind === "artwork" &&
                job.error_code === "artwork_connection_required" && (
                  <Link className="button secondary" to="/settings/connections">
                    Connect Spotify
                  </Link>
                )}
              {job.kind === "artwork" && (
                <>
                  <p>
                    Artwork downloads run slowly in the background, at most one
                    image per minute, to reduce the risk of Spotify limiting
                    requests and delaying live listening updates. Large
                    libraries can take several days. Spotify shares its limits
                    across requests, so additional cooldowns are always
                    respected.
                  </p>
                  <p>
                    You can close the browser. Saved images, progress and your
                    pause choice survive application restarts.
                  </p>
                  <ArtworkSchedule
                    job={job}
                    params={detail.data?.source_parameters}
                    preferences={preferences}
                  />
                </>
              )}
              {job.state === "cancelled" && (
                <p className="subtle">
                  Cancellation requested. Any current file or network step
                  finishes safely before cleanup; saved progress and completed
                  copies are kept.
                </p>
              )}
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
                    <p
                      className={
                        job.kind === "artwork" ? "subtle" : "form-error"
                      }
                    >
                      {job.error_code === "calendar_recalculation_failed"
                        ? "Calendar recalculation failed. The previous calendar is still active. Check server storage and retry."
                        : job.kind.startsWith("listenbrainz_") ||
                            job.kind === "artwork"
                          ? jobMessage(job.error_code)
                          : `Activity reported: ${job.error_code}`}
                    </p>
                  )}
                  {job.retry_at_ms && (
                    <p>Next attempt: {date(job.retry_at_ms, preferences)}</p>
                  )}
                </section>
              )}
              {job.kind !== "artwork" && (
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
              )}
              {(isActive(job) ||
                [
                  "calendar",
                  "spotify_import",
                  "listenbrainz_file",
                  "listenbrainz_fetch",
                  "artwork",
                ].includes(job.kind)) && (
                <div className="connection-actions">
                  {(isActive(job)
                    ? job.kind === "artwork"
                      ? [
                          job.error_code === "artwork_paused"
                            ? "resume"
                            : "pause",
                          "cancel",
                        ]
                      : ["cancel"]
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
                          if (action === "cancel") void refresh();
                          else await refresh();
                          toast(
                            action === "pause"
                              ? "Artwork downloads paused. Saved progress is kept."
                              : action === "resume"
                                ? "Artwork downloads resumed from saved progress."
                                : action === "cancel"
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
                      {action === "pause"
                        ? "Pause downloads"
                        : action === "resume"
                          ? "Resume downloads"
                          : action === "cancel"
                            ? job.kind === "artwork"
                              ? "Cancel downloads"
                              : job.kind === "calendar"
                                ? "Cancel recalculation"
                                : job.kind === "spotify_import" ||
                                    job.kind.startsWith("listenbrainz_")
                                  ? "Cancel import"
                                  : "Cancel activity"
                            : job.kind === "artwork"
                              ? "Resume downloads"
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
              {job.kind === "artwork" && (job.expected_rows || 0) > 100 && (
                <div className="pagination">
                  <button
                    className="secondary"
                    disabled={issuesPage === 0}
                    onClick={() => setIssuesPage((p) => p - 1)}
                  >
                    Previous images
                  </button>
                  <span>Page {issuesPage + 1}</span>
                  <button
                    className="secondary"
                    disabled={
                      (issuesPage + 1) * 100 >= (job.expected_rows || 0)
                    }
                    onClick={() => setIssuesPage((p) => p + 1)}
                  >
                    Next images
                  </button>
                </div>
              )}
              {job.kind !== "artwork" &&
                detail.data &&
                detail.data.members.length > 0 && (
                  <section className="activity-files">
                    <h3>Files in this activity</h3>
                    <p>
                      Files discovered:{" "}
                      {
                        detail.data.members.filter(
                          (m) => m.kind !== "directory",
                        ).length
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
                          (m) =>
                            m.state === "excluded" && m.kind !== "directory",
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
                          ["pending", "received", "processing"].includes(
                            m.state,
                          ),
                        ).length
                      }
                      .
                    </p>
                  </section>
                )}
              {detail.data?.members.map((member, index) => (
                <div className="job" key={`${index}:${member.member}`}>
                  <strong>{member.member}</strong>
                  {job.kind === "artwork" ? (
                    <span>
                      {member.state === "pending"
                        ? "Queued"
                        : member.skipped
                          ? "No image available"
                          : member.duplicates
                            ? "Already cached"
                            : "Saved"}
                    </span>
                  ) : (
                    <>
                      <span>
                        {member.kind} · {member.state} · {member.processed}{" "}
                        processed
                        {member.expected_rows === null
                          ? " · total unknown"
                          : ` of ${member.expected_rows}`}
                      </span>
                      <p>
                        {member.originals} new originals | {member.variants} new
                        variants | {member.duplicates} known rows |{" "}
                        {member.added} new listens | {member.reconciled}{" "}
                        reconciled | {member.invalid} invalid | {member.skipped}{" "}
                        non-music | {member.below_threshold} below threshold
                      </p>
                    </>
                  )}
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
              <Select
                aria-label="Activity type"
                value={kind}
                onChange={(value) => {
                  setKind(value);
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
                <option value="artwork">Artwork downloads</option>
              </Select>
            </label>
          </div>
          <ErrorText>{history.error}</ErrorText>
          {visibleJobs.length ? (
            separateJobs.map((job) => (
              <article
                className={
                  "activity-row panel " + (isActive(job) ? "is-active" : "")
                }
                key={job.id}
              >
                <div className="activity-symbol">
                  <Icon>
                    {job.kind === "artwork"
                      ? "photo_library"
                      : job.kind === "calendar"
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
                  {!isActive(job) && job.kind === "artwork" && (
                    <small>
                      {number.format(job.added)} images saved ·{" "}
                      {number.format(job.duplicates)} already cached ·{" "}
                      {number.format(job.skipped)} unavailable
                    </small>
                  )}
                  {!isActive(job) && job.kind !== "artwork" && (
                    <small>
                      {number.format(job.added)} new listens ·{" "}
                      {number.format(job.duplicates)} already present ·{" "}
                      {number.format(job.invalid)} invalid rows
                    </small>
                  )}
                  {job.error_code && (
                    <p className="form-error">
                      {job.kind === "artwork"
                        ? jobMessage(job.error_code)
                        : `${job.error_code}. Open the report for details and available actions.`}
                    </p>
                  )}
                  {job.retry_at_ms && (
                    <p>Next attempt: {date(job.retry_at_ms, preferences)}</p>
                  )}
                </div>
                <div className="activity-row-actions">
                  {isActive(job) && (
                    <button
                      className="secondary icon-button activity-cancel"
                      aria-label={`Cancel ${activityName(job)}`}
                      title="Cancel activity"
                      disabled={busy}
                      onClick={async () => {
                        setBusy(true);
                        setActionError("");
                        try {
                          await api.activityAction(job.id, "cancel");
                          await refresh();
                          toast(
                            "Cancellation requested. Saved progress is kept; the current step stops at a safe boundary.",
                          );
                        } catch (error) {
                          toast(
                            error instanceof Error
                              ? error.message
                              : String(error),
                            true,
                          );
                        } finally {
                          setBusy(false);
                        }
                      }}
                    >
                      <Icon>close</Icon>
                    </button>
                  )}
                  <span className="activity-open" aria-hidden="true">
                    <Icon>arrow_forward</Icon>
                  </span>
                </div>
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
          {!!collectionRuns.length && (
            <details className="panel collection-runs">
              <summary>
                <Icon>sync</Icon>
                <span>
                  <strong>Spotify background collection</strong>
                  <small>
                    {collectionRuns.length} completed runs on this page ·{" "}
                    {number.format(
                      collectionRuns.reduce((sum, j) => sum + j.added, 0),
                    )}{" "}
                    new listens
                  </small>
                </span>
                <Icon>expand_more</Icon>
              </summary>
              <div>
                {collectionRuns.map((job) => (
                  <Link
                    key={job.id}
                    to={`/activities/${job.id}`}
                    className="collection-run"
                  >
                    <time
                      dateTime={
                        job.created_at_ms
                          ? new Date(job.created_at_ms).toISOString()
                          : undefined
                      }
                    >
                      {job.created_at_ms
                        ? date(job.created_at_ms, preferences)
                        : "Open collection report"}
                    </time>
                    <span>
                      {number.format(job.added)} new listens ·{" "}
                      {number.format(job.processed)} processed
                    </span>
                    <Icon>chevron_right</Icon>
                  </Link>
                ))}
              </div>
            </details>
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
