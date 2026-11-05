import { useEffect, useState } from "react";
import type { FormEvent } from "react";
import type {
  AskResponse,
  AskStatusResponse,
  EntityCandidate,
  Preferences,
  Query,
} from "./generated";
import { api } from "./api";
import { InterpretationChips } from "./ask-chips";
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
};

function entityName(query: Query): string {
  return query.entity?.name ?? "your archive";
}

function headline(response: AskResponse, preferences: Preferences): string {
  const query = response.query;
  const result = response.result;
  if (!query || !result) return "";
  const entity = entityName(query);
  const period = periodCopy[query.period] ?? "in this period";
  const value = result.scalar;
  if (query.metric === "plays") {
    const plays = value ?? 0;
    return `You listened to ${entity} ${number.format(plays)} ${plays === 1 ? "time" : "times"} ${period}.`;
  }
  if (query.metric === "first_play" || query.metric === "last_play") {
    const which = query.metric === "first_play" ? "first" : "last";
    return value === null
      ? `No ${which} listen found for ${entity} ${period}.`
      : `Your ${which} listen to ${entity} was on ${date(value, preferences)}.`;
  }
  return `${query.metric} for ${entity} ${period}.`;
}

export function AskPage({ preferences }: { preferences: Preferences }) {
  const [status, setStatus] = useState<AskStatusResponse | null>(null);
  const [statusError, setStatusError] = useState("");
  const [question, setQuestion] = useState("");
  const [response, setResponse] = useState<AskResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    let current = true;
    void api
      .askStatus()
      .then((data) => {
        if (current) {
          setStatus(data);
          setStatusError("");
        }
      })
      .catch((cause) => {
        if (current) setStatusError(cause.message);
      });
    return () => {
      current = false;
    };
  }, []);

  async function submitQuestion(event: FormEvent) {
    event.preventDefault();
    if (!question.trim() || loading) return;
    setLoading(true);
    setError("");
    try {
      setResponse(await api.ask(question.trim(), null));
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
    void runQuery({
      ...response.query,
      entity: {
        kind: candidate.kind,
        name: candidate.name,
        id: candidate.id,
      },
    });
  }

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
        from your own listens.
      </Heading>

      {statusError && <Notice title="Ask is unavailable">{statusError}</Notice>}

      {!status && !statusError && (
        <article className="panel padded ask-loading" aria-busy="true">
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
          Download and select a model in AI settings before asking questions.
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
          Ask will be ready in a moment. Nothing else in the application is
          blocked.
        </Notice>
      )}

      <form className="ask-form panel padded" onSubmit={submitQuestion}>
        <label>
          Your question
          <input
            value={question}
            onChange={(event) => setQuestion(event.target.value)}
            placeholder="How many times did I listen to Björk?"
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
            <Icon>send</Icon>
            {loading ? "Asking…" : "Ask"}
          </Button>
          <span className="field-hint">
            Only play counts, first plays and last plays are available in this
            version.
          </span>
        </div>
      </form>

      {error && <Notice title="The question could not be answered">{error}</Notice>}

      {loading && (
        <article className="panel padded ask-loading" aria-busy="true">
          <SkeletonBlock height={18} />
          <SkeletonBlock height={44} />
          <SkeletonBlock height={120} />
        </article>
      )}

      {!loading && response && (
        <>
          {response.chips.length > 0 && response.query && (
            <section className="ask-interpretation" aria-label="Interpretation">
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
                    <span className="ask-candidate-kind">{candidate.kind}</span>
                  </button>
                ))}
              </div>
            </Notice>
          )}

          {response.status === "unsupported" && (
            <Notice title="That question is outside the current scope" tone="info">
              {response.message}
              <p className="ask-hint">
                Try: “How many times did I listen to Björk?” · “When did I first
                listen to Radiohead?” · “Quante volte ho ascoltato Björk?”
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
                <Link to="/settings/ai" className="btn btn-secondary secondary">
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

          {response.status === "ok" && response.result && response.query && (
            <ResultCard
              response={response}
              query={response.query}
              preferences={preferences}
            />
          )}
        </>
      )}
    </>
  );
}

function ResultCard({
  response,
  query,
  preferences,
}: {
  response: AskResponse;
  query: Query;
  preferences: Preferences;
}) {
  const result = response.result!;
  const point =
    query.metric === "first_play" ? result.first : result.last;
  const isPoint = query.metric === "first_play" || query.metric === "last_play";
  return (
    <article className="panel padded ask-result">
      <p className="ask-eyebrow">Result</p>
      <p className="ask-result-headline">{headline(response, preferences)}</p>
      {!isPoint && (
        <p className="ask-result-value" aria-label="Result value">
          {number.format(result.scalar ?? 0)}
        </p>
      )}
      {isPoint && point && (
        <div className="ask-listen">
          <p className="ask-listen-title">{point.title}</p>
          <p className="ask-listen-meta">
            {point.artist}
            {point.album ? ` · ${point.album}` : ""}
          </p>
          <p className="ask-listen-date">{date(point.played_at_ms, preferences)}</p>
        </div>
      )}
      {isPoint && !point && (
        <p className="ask-empty">No matching listen in this selection.</p>
      )}
      {result.time && (
        <p className="ask-coverage">
          Known time {number.format(result.time.known_ms / 60_000)} minutes
          {result.time.unknown_listens > 0
            ? ` · ${number.format(result.time.unknown_listens)} listens with unknown duration`
            : ""}
        </p>
      )}
      {result.rows.length > 0 && (
        <ul className="ask-rows">
          {result.rows.map((row) => (
            <li key={row.key}>
              {row.href ? <Link to={row.href}>{row.label}</Link> : row.label}
              <span>{number.format(row.plays)}</span>
            </li>
          ))}
        </ul>
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
