import { useEffect, useState } from "react";
import type { FormEvent } from "react";
import type {
  AskPrevious,
  AskResponse,
  AskStep,
  AiAskStatus,
  EntityCandidate,
  Preferences,
  Query,
} from "./generated";
import { api } from "./api";
import { applyCandidate, InterpretationChips } from "./ask-chips";
import { AskResultRows, bucketLabel } from "./ask-table";
import { AskTimeCard } from "./ask-time";
import { StreakResultCard } from "./ask-streak";
import { Link } from "./navigation";
import { SkeletonBlock } from "./skeleton";
import { Button, Heading, Icon, Notice, date, number } from "./ui";

const periodCopy: Record<string, string> = {
  all_time: "all time",
  today: "today",
  yesterday: "yesterday",
  this_week: "this week",
  last_week: "last week",
  this_month: "this month",
  last_month: "last month",
  this_year: "this year",
  last_year: "last year",
  last_7_days: "in the last 7 days",
  last_30_days: "in the last 30 days",
  last_90_days: "in the last 90 days",
  last_180_days: "in the last 180 days",
  last_365_days: "in the last 365 days",
};

const metricCopy: Record<string, string> = {
  plays: "Plays",
  time_listened: "Time listened",
  distinct_tracks: "Distinct tracks",
  distinct_artists: "Distinct artists",
  distinct_albums: "Distinct albums",
  first_play: "First play",
  last_play: "Last play",
  streak: "Listening streak",
  milestones: "Milestones",
  rank: "Ranking position",
  share: "Share of plays",
  listening_days: "Listening days",
};

const groupCopy: Record<string, string> = {
  day: "day",
  week: "week",
  month: "month",
  year: "year",
  hour: "hour",
  weekday: "weekday",
  artist: "artist",
  album: "album",
  track: "track",
};

function entityName(query: Query): string {
  if (!query.entity) return "your archive";
  return query.scope && query.scope.name !== query.entity.name
    ? `${query.entity.name} by ${query.scope.name}`
    : query.entity.name;
}

const kindPlural: Record<string, string> = {
  artist: "artists",
  album: "albums",
  track: "tracks",
};

/** Share of plays as a percentage with one decimal, from integer counts. */
export function sharePercent(plays: number, total: number): string {
  if (total <= 0) return "0%";
  return `${(Math.round((plays / total) * 1000) / 10).toLocaleString(undefined, { maximumFractionDigits: 1 })}%`;
}

const temporalWhere: Record<string, (label: string) => string> = {
  day: (label) => `on ${label}`,
  week: (label) => `in the ${label.charAt(0).toLowerCase()}${label.slice(1)}`,
  month: (label) => `in ${label}`,
  year: (label) => `in ${label}`,
  hour: (label) => `at ${label}`,
  weekday: (label) => `on ${label}s`,
};

/** A plain sentence for "which day/month/… had the most?" questions. */
function rankedHeadline(response: AskResponse, entity: string): string | null {
  const query = response.query;
  const result = response.result;
  const group = query?.group_by;
  if (!query || !result || !group || !temporalWhere[group]) return null;
  if (query.sort_by !== "metric" || result.rows.length === 0) return null;
  const top = result.rows[0];
  const label = bucketLabel(top, group);
  const count = number.format(top.plays);
  const what = query.metric.startsWith("distinct_")
    ? `${count} distinct ${query.metric.replace("distinct_", "")}`
    : `${count} ${top.plays === 1 ? "play" : "plays"}`;
  const extreme = query.order === "asc" ? "least" : "most";
  const subject = query.entity ? entity : "music";
  if (result.rows.length === 1) {
    return `You listened to ${subject} the ${extreme} ${temporalWhere[group](label)}: ${what}.`;
  }
  return `Your ${extreme} listened ${groupCopy[group]}s for ${subject}: ${label} comes first, with ${what}.`;
}

function headline(response: AskResponse, preferences: Preferences): string {
  const query = response.query;
  const result = response.result;
  if (!query || !result) return "";
  const entity = entityName(query);
  const period = periodCopy[query.period] ?? "in this period";
  const value = result.scalar;
  if (query.group_by) {
    const ranked = rankedHeadline(response, entity);
    if (ranked) return ranked;
    const subject = query.entity ? ` for ${entity}` : "";
    return `${metricCopy[query.metric] ?? query.metric} by ${groupCopy[query.group_by] ?? query.group_by}${subject}, ${period}.`;
  }
  if (query.metric.startsWith("distinct_")) {
    const noun = query.metric.replace("distinct_", "");
    return `${number.format(value ?? 0)} distinct ${noun} for ${entity} ${period}.`;
  }
  if (query.metric === "plays") {
    const plays = value ?? 0;
    return `You listened to ${entity} ${number.format(plays)} ${plays === 1 ? "time" : "times"} ${period}.`;
  }
  if (query.metric === "time_listened") {
    const time = result.time;
    return time && time.unknown_listens > 0
      ? `Known listening time for ${entity} ${period}, with unknown durations reported separately.`
      : `Known listening time for ${entity} ${period}.`;
  }
  if (query.metric === "first_play" || query.metric === "last_play") {
    const which = query.metric === "first_play" ? "first" : "last";
    return value === null
      ? `No ${which} listen found for ${entity} ${period}.`
      : `Your ${which} listen to ${entity} was on ${date(value, preferences)}.`;
  }
  if (query.metric === "rank") {
    const kind = kindPlural[query.entity?.kind ?? "artist"];
    return value === null
      ? `${entity} has no plays ${period}.`
      : `${entity} is number ${number.format(value)} of ${number.format(result.total)} ${kind} ${period}.`;
  }
  if (query.metric === "share") {
    return `${entity} is ${sharePercent(value ?? 0, result.total)} of your plays ${period}.`;
  }
  if (query.metric === "listening_days") {
    const days = value ?? 0;
    return `You listened to ${entity} on ${number.format(days)} ${days === 1 ? "day" : "days"} ${period}.`;
  }
  return `${query.metric} for ${entity} ${period}.`;
}

export function AskPage({
  preferences,
  visible = true,
}: {
  preferences: Preferences;
  visible?: boolean;
}) {
  const [status, setStatus] = useState<AiAskStatus | null>(null);
  const [statusError, setStatusError] = useState("");
  const [question, setQuestion] = useState("");
  const [response, setResponse] = useState<AskResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [followUp, setFollowUp] = useState(true);
  const [asked, setAsked] = useState("");

  useEffect(() => {
    if (!visible) return;
    let current = true;
    const fetchStatus = () => {
      void api
        .askStatus()
        .then((data) => {
          if (current) {
            setStatus(data);
            setStatusError("");
          }
        })
        .catch((cause) => {
          if (current) {
            setStatusError(
              cause instanceof Error ? cause.message : String(cause),
            );
          }
        });
    };
    fetchStatus();
    const timer = setInterval(() => {
      if (current && status?.model_loading) {
        fetchStatus();
      }
    }, 1000);
    return () => {
      current = false;
      clearInterval(timer);
    };
  }, [visible, status?.model_loading]);

  async function submitQuestion(event: FormEvent) {
    event.preventDefault();
    if (!question.trim() || loading) return;
    setLoading(true);
    setError("");
    try {
      // A follow-up sends the last question and how it was understood,
      // never its results.
      const previous: AskPrevious | null =
        followUp && canFollowUp && response?.chips.length
          ? { question: asked, chips: response.chips }
          : null;
      setResponse(await api.ask(question.trim(), null, previous));
      setAsked(question.trim());
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  }

  async function runQuery(query: Query) {
    setLoading(true);
    setError("");
    try {
      setResponse(await api.askQuery(query));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  }

  function choose(candidate: EntityCandidate) {
    if (!response?.query) return;
    void runQuery(applyCandidate(response.query, candidate));
  }

  // Older or mocked answers may omit the steps.
  const earlier = response?.earlier ?? [];
  const remote = status?.provider === "remote";
  const canFollowUp = remote && response?.status === "ok";
  const unavailable =
    !status ||
    !status.enabled ||
    !status.runtime_available ||
    status.model_loading;
  const disabled = unavailable || !status?.model_ready;

  return (
    <>
      <Heading title="Ask">
        Ask your listening archive a question. Every number is computed locally
        from your own listens
        {status?.provider === "remote" && status.remote_model
          ? `; ${status.remote_model} only interprets the question.`
          : "."}
      </Heading>

      {statusError && <Notice title="Ask is unavailable">{statusError}</Notice>}

      {!status && !statusError && (
        <article className="panel padded ask-loading" aria-busy="true">
          <div className="ask-loading-status" role="status">
            <span className="spinner-dot spinner-sm" aria-hidden="true" />
            <span>Checking AI status</span>
            <span className="loading-dots" aria-hidden="true">
              <span />
              <span />
              <span />
            </span>
          </div>
          <SkeletonBlock height={18} />
          <SkeletonBlock height={44} />
        </article>
      )}

      {status && !status.enabled && (
        <Notice title="AI is off" tone="info">
          Ask is disabled by default. Enable it in Settings and choose a local
          model to start asking questions.
          <div className="ask-notice-action">
            <Link to="/settings/ai" className="btn btn-secondary secondary">
              <Icon>settings</Icon>
              Open AI settings
            </Link>
          </div>
        </Notice>
      )}

      {status?.enabled && !status.runtime_available && (
        <Notice title="The AI runtime is not available" tone="warning">
          This build does not include the local inference runtime, so Ask cannot
          answer yet. The rest of your archive is unaffected.
        </Notice>
      )}

      {status?.enabled && status.runtime_available && !status.model_ready && (
        <Notice title="No model is ready" tone="warning">
          {status.provider === "remote"
            ? "Enter the address and model name of your AI server in AI settings before asking questions."
            : "Download and select a model in AI settings before asking questions."}
          <div className="ask-notice-action">
            <Link to="/settings/ai" className="btn btn-secondary secondary">
              <Icon>settings</Icon>
              Open AI settings
            </Link>
          </div>
        </Notice>
      )}

      {status?.enabled && status.model_loading && (
        <Notice title="The model is loading" tone="info">
          <span className="inline-loading">
            <span className="spinner-dot spinner-sm" aria-hidden="true" />
            <span>
              Ask will be ready in a moment. Nothing else in the application is
              blocked.
            </span>
          </span>
        </Notice>
      )}

      <form className="ask-form panel padded" onSubmit={submitQuestion}>
        <label>
          Your question
          <input
            value={question}
            onChange={(event) => setQuestion(event.target.value)}
            placeholder={
              canFollowUp && followUp
                ? "Ask a follow-up, for example: and last year?"
                : remote
                  ? "In the month I listened to Ado the most, which track?"
                  : "How many times did I listen to Björk?"
            }
            autoComplete="off"
            disabled={disabled}
          />
        </label>
        <div className="ask-form-actions">
          <Button
            type="submit"
            variant="primary"
            disabled={disabled || loading || !question.trim()}
          >
            {loading ? (
              <span className="spinner-dot spinner-sm" aria-hidden="true" />
            ) : (
              <Icon>send</Icon>
            )}
            {loading ? "Asking…" : "Ask"}
          </Button>
          <span className="field-hint">
            Play counts, listening time, distinct tracks/artists/albums, first
            and last plays, streaks, milestones and grouped top lists are
            available. Grouping covers plays and distinct counts; listening time
            cannot be grouped yet.
            {remote &&
              " Your AI can also chain questions, use exact date ranges and build on the previous answer."}
          </span>
          {canFollowUp && (
            <label className="ask-follow-up">
              <input
                type="checkbox"
                checked={followUp}
                onChange={(event) => setFollowUp(event.target.checked)}
              />
              <span>Continue from the previous answer</span>
            </label>
          )}
        </div>
      </form>

      {error && (
        <Notice title="The question could not be answered">{error}</Notice>
      )}

      <div aria-live="polite" aria-atomic="true">
        {loading && (
          <article className="panel padded ask-loading" aria-busy="true">
            <div className="ask-loading-status" role="status">
              <span className="spinner-dot spinner-sm" aria-hidden="true" />
              <span>Thinking</span>
              <span className="loading-dots" aria-hidden="true">
                <span />
                <span />
                <span />
              </span>
            </div>
            <SkeletonBlock height={18} />
            <SkeletonBlock height={44} />
            <SkeletonBlock height={120} />
          </article>
        )}

        {!loading && response && (
          <>
            {response.chips.length > 0 && response.query && (
              <section
                className="ask-interpretation"
                aria-label="Interpretation"
              >
                <p className="ask-eyebrow">Interpretation</p>
                <InterpretationChips
                  query={response.query}
                  chips={response.chips}
                  onChange={(query) => void runQuery(query)}
                />
              </section>
            )}

            {response.status === "ambiguous" && (
              <Notice title="More than one match" tone="warning">
                <p>{response.message}</p>
                <div className="ask-candidates">
                  {response.candidates.map((candidate) => (
                    <button
                      key={candidate.id}
                      type="button"
                      className="btn btn-secondary secondary ask-candidate"
                      onClick={() => choose(candidate)}
                    >
                      <span>
                        <strong>{candidate.name}</strong>
                        {candidate.subtitle && (
                          <small>{candidate.subtitle}</small>
                        )}
                      </span>
                      <span className="ask-candidate-kind">
                        {candidate.kind}
                      </span>
                    </button>
                  ))}
                </div>
              </Notice>
            )}

            {response.status === "unsupported" && (
              <Notice
                title="That question is outside the current scope"
                tone="info"
              >
                {response.message}
                <p className="ask-hint">
                  Try: “How many times did I listen to Björk?” · “When did I
                  first listen to Radiohead?” · “How many hours did I listen?” ·
                  “What is my listening streak?”
                </p>
              </Notice>
            )}

            {response.status === "model_unavailable" && (
              <Notice title="The model is not available" tone="warning">
                {response.message}
              </Notice>
            )}

            {response.status === "model_missing" && (
              <Notice title="No model is ready" tone="warning">
                {response.message}
                <div className="ask-notice-action">
                  <Link
                    to="/settings/ai"
                    className="btn btn-secondary secondary"
                  >
                    <Icon>settings</Icon>
                    Open AI settings
                  </Link>
                </div>
              </Notice>
            )}

            {response.status === "failed" && (
              <Notice title="I couldn't answer that" tone="error">
                {response.message}
              </Notice>
            )}

            {earlier.map(
              (step, index) =>
                step.result &&
                step.query && (
                  <ResultCard
                    key={index}
                    response={stepResponse(step)}
                    query={step.query}
                    preferences={preferences}
                    label={`Step ${index + 1} of ${earlier.length + 1}`}
                  />
                ),
            )}

            {response.status === "ok" && response.result && response.query && (
              <ResultCard
                response={response}
                query={response.query}
                preferences={preferences}
                label={
                  earlier.length > 0
                    ? `Step ${earlier.length + 1} of ${earlier.length + 1}`
                    : undefined
                }
              />
            )}
          </>
        )}
      </div>
    </>
  );
}

/** An earlier step of a chained question, shaped like a full answer. */
function stepResponse(step: AskStep): AskResponse {
  return {
    status: "ok",
    question: "",
    query: step.query,
    chips: step.chips,
    result: step.result,
    message: null,
    candidates: [],
    runtime_available: true,
    earlier: [],
  };
}

export function ResultCard({
  response,
  query,
  preferences,
  label,
}: {
  response: AskResponse;
  query: Query;
  preferences: Preferences;
  label?: string;
}) {
  const result = response.result!;
  if (query.metric === "streak" || query.metric === "milestones") {
    return (
      <StreakResultCard
        query={query}
        result={result}
        preferences={preferences}
      />
    );
  }
  const point = query.metric === "first_play" ? result.first : result.last;
  const isPoint = query.metric === "first_play" || query.metric === "last_play";
  const isTime = query.metric === "time_listened";
  return (
    <article className="panel padded ask-result">
      <p className="ask-eyebrow">{label ?? "Result"}</p>
      <p className="ask-result-headline">{headline(response, preferences)}</p>
      {!isPoint && !isTime && !query.group_by && (
        <p className="ask-result-value" aria-label="Result value">
          {query.metric === "share"
            ? sharePercent(result.scalar ?? 0, result.total)
            : query.metric === "rank"
              ? result.scalar === null
                ? "—"
                : `#${number.format(result.scalar)}`
              : number.format(result.scalar ?? 0)}
        </p>
      )}
      {query.metric === "share" && (
        <p className="ask-coverage">
          {number.format(result.scalar ?? 0)} of {number.format(result.total)}{" "}
          counted plays
        </p>
      )}
      {isTime && result.time && <AskTimeCard time={result.time} />}
      {isPoint && point && (
        <div className="ask-listen">
          <p className="ask-listen-title">{point.title}</p>
          <p className="ask-listen-meta">
            {point.artist}
            {point.album ? ` · ${point.album}` : ""}
          </p>
          <p className="ask-listen-date">
            {date(point.played_at_ms, preferences)}
          </p>
        </div>
      )}
      {isPoint && !point && (
        <p className="ask-empty">No matching listen in this selection.</p>
      )}
      {result.time && !isTime && (
        <p className="ask-coverage">
          Known time {number.format(result.time.known_ms / 60_000)} minutes
          {result.time.unknown_listens > 0
            ? ` · ${number.format(result.time.unknown_listens)} listens with unknown duration`
            : ""}
        </p>
      )}
      {result.rows.length > 0 && query.metric !== "rank" && (
        <AskResultRows result={result} query={query} />
      )}
      {result.links.length > 0 && (
        <div className="ask-result-links">
          {result.links.map((link) => (
            <Link key={link.href + link.label} to={link.href}>
              {link.label}
            </Link>
          ))}
        </div>
      )}
    </article>
  );
}
