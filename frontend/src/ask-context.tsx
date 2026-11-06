import { useEffect, useId, useState } from "react";
import type { FormEvent } from "react";
import type {
  AskResponse,
  AiAskStatus,
  EntityCandidate,
  EntityKind,
  Preferences,
  Query,
} from "./generated";
import { api } from "./api";
import { InterpretationChips } from "./ask-chips";
import { ResultCard } from "./ask";
import { Link } from "./navigation";
import { SkeletonBlock } from "./skeleton";
import { Button, Icon, Notice } from "./ui";

type ContextKind = "artists" | "albums";

const entityKinds: Record<ContextKind, EntityKind> = {
  artists: "artist",
  albums: "album",
};

const entityLabels: Record<ContextKind, string> = {
  artists: "artist",
  albums: "album",
};

/**
 * Contextual Ask box for artist and album pages. The page entity is sent with
 * every question, so the user never has to repeat its name.
 */
export function AskContextPanel({
  preferences,
  kind,
  id,
  name,
}: {
  preferences: Preferences;
  kind: ContextKind;
  id: string;
  name: string;
}) {
  const titleId = useId();
  const bodyId = useId();
  const [open, setOpen] = useState(false);
  const [status, setStatus] = useState<AiAskStatus | null>(null);
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
      setResponse(
        await api.ask(question.trim(), { kind: entityKinds[kind], id }),
      );
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
  const label = entityLabels[kind];

  return (
    <section
      className="panel ask-context"
      data-ask-context={kind}
      aria-labelledby={titleId}
    >
      <button
        type="button"
        className="ask-context-toggle"
        aria-expanded={open}
        aria-controls={bodyId}
        aria-label={`Ask about this ${label}: ${name}`}
        onClick={() => setOpen((value) => !value)}
      >
        <span className="ask-context-heading">
          <span className="ask-eyebrow">Ask</span>
          <strong id={titleId}>Ask about this {label}</strong>
          <span className="ask-context-subject">{name}</span>
        </span>
        <span className="ask-context-action">
          {open ? "Hide" : "Open"}
          <Icon>{open ? "expand_less" : "expand_more"}</Icon>
        </span>
      </button>

      {open && (
        <div id={bodyId} className="ask-context-body">
          <AvailabilityNotices status={status} statusError={statusError} />

          <form className="ask-context-form" onSubmit={submitQuestion}>
            <label>
              Your question
              <input
                value={question}
                onChange={(event) => setQuestion(event.target.value)}
                placeholder="When did I first listen?"
                autoComplete="off"
                disabled={disabled}
              />
            </label>
            <Button
              type="submit"
              variant="primary"
              disabled={disabled || loading || !question.trim()}
            >
              <Icon>send</Icon>
              {loading ? "Asking…" : "Ask"}
            </Button>
          </form>

          <p className="field-hint">
            The {label} on this page is already part of the question.
          </p>

          {error && (
            <Notice title="The question could not be answered">{error}</Notice>
          )}

          {loading && (
            <article className="ask-loading" aria-busy="true">
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
                </Notice>
              )}

              {response.status === "model_unavailable" && (
                <Notice title="The model is not available" tone="warning">
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

              {response.status === "ok" &&
                response.result &&
                response.query && (
                  <ResultCard
                    response={response}
                    query={response.query}
                    preferences={preferences}
                  />
                )}
            </>
          )}
        </div>
      )}
    </section>
  );
}

function AvailabilityNotices({
  status,
  statusError,
}: {
  status: AiAskStatus | null;
  statusError: string;
}) {
  if (statusError) {
    return <Notice title="Ask is unavailable">{statusError}</Notice>;
  }
  if (!status) {
    return (
      <article className="ask-loading" aria-busy="true">
        <SkeletonBlock height={18} />
        <SkeletonBlock height={44} />
      </article>
    );
  }
  return (
    <>
      {!status.enabled && (
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

      {status.enabled && !status.runtime_available && (
        <Notice title="The AI runtime is not available" tone="warning">
          This build does not include the local inference runtime, so Ask cannot
          answer yet. The rest of your archive is unaffected.
        </Notice>
      )}

      {status.enabled && status.runtime_available && !status.model_ready && (
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

      {status.enabled && status.model_loading && (
        <Notice title="The model is loading" tone="info">
          Ask will be ready in a moment. Nothing else in the application is
          blocked.
          <div className="ask-notice-action">
            <Link to="/settings/ai" className="btn btn-secondary secondary">
              <Icon>settings</Icon>
              Open AI settings
            </Link>
          </div>
        </Notice>
      )}
    </>
  );
}
