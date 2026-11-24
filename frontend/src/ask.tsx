import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import type {
  AskPrevious,
  AskResponse,
  AiAskStatus,
  ColumnFormat,
  Preferences,
  SqlAnswer,
  SqlValue,
} from "./generated";
import { api } from "./api";
import { Link } from "./navigation";
import { SkeletonBlock } from "./skeleton";
import { Button, Heading, Icon, Notice, date, duration, number } from "./ui";

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
  const [sqlEditorOpen, setSqlEditorOpen] = useState(false);
  // The model's description of the query the owner edited and re-ran, kept so
  // the explanation never disappears silently after `POST /api/v1/ask/sql`.
  const [editedDescription, setEditedDescription] = useState<string | null>(
    null,
  );
  const abortRef = useRef<AbortController | null>(null);
  const requestIdRef = useRef<string | null>(null);

  useEffect(() => {
    return () => {
      abortRef.current?.abort();
    };
  }, []);

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
    const controller = new AbortController();
    const requestId =
      globalThis.crypto?.randomUUID?.() ??
      `ask-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    abortRef.current = controller;
    requestIdRef.current = requestId;
    setLoading(true);
    setError("");
    setSqlEditorOpen(false);
    try {
      // A follow-up sends the last question and SQL definition, never its results.
      const previous: AskPrevious | null =
        followUp && canFollowUp && response?.sql?.sql
          ? {
              question: asked || response.sql.title || "",
              sql: {
                title: response.sql.title,
                computed: response.sql.computed,
                sql: response.sql.sql,
              },
            }
          : null;
      const next = await api.ask(question.trim(), previous, {
        signal: controller.signal,
        requestId,
      });
      if (requestIdRef.current === requestId) {
        setResponse(next);
        setAsked(question.trim());
        setEditedDescription(null);
      }
    } catch (cause) {
      if (requestIdRef.current !== requestId) return;
      if (
        controller.signal.aborted ||
        (cause instanceof DOMException && cause.name === "AbortError")
      ) {
        setResponse(null);
        setError("The question was cancelled.");
      } else {
        setError(cause instanceof Error ? cause.message : String(cause));
      }
    } finally {
      if (requestIdRef.current === requestId) {
        abortRef.current = null;
        requestIdRef.current = null;
        setLoading(false);
      }
    }
  }

  function cancelQuestion() {
    const id = requestIdRef.current;
    if (id) void api.cancelAsk(id).catch(() => {});
    abortRef.current?.abort();
  }

  async function runEditedSql(editedSql: string, current?: SqlAnswer | null) {
    if (!editedSql.trim() || loading) return;
    setLoading(true);
    setError("");
    setSqlEditorOpen(true);
    try {
      const next = await api.askSql({
        sql: editedSql,
        title: current?.title ?? null,
        computed: null,
        answer_kind:
          current && current.answer_kind !== "unsupported"
            ? current.answer_kind
            : null,
        columns: current?.columns ?? [],
      });
      setResponse(next);
      // The edited query is not the one the model described, so keep its
      // description visible with an explicit note instead of dropping it. A
      // second edit keeps the description from the original answer.
      setEditedDescription(
        current?.computed?.trim() ? current.computed : editedDescription,
      );
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  }

  const canFollowUp = response?.status === "ok" && Boolean(response.sql?.sql);
  const unavailable =
    !status ||
    !status.enabled ||
    !status.runtime_available ||
    status.model_loading;
  const disabled = unavailable || !status?.model_ready;

  return (
    <>
      <Heading title="Ask">
        Ask your listening archive a question. A local GGUF model writes
        read-only SQL against your listening projection, and every number is
        computed locally inside a sandbox.
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
          GGUF model to start asking questions.
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
          Download a local Qwen3.5 model or set a custom local GGUF model path
          in AI settings before asking questions.
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
                : "What are my top 5 albums by listening time in 2024?"
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
          {loading && (
            <Button type="button" variant="secondary" onClick={cancelQuestion}>
              Cancel
            </Button>
          )}
          <span className="field-hint">
            Ask any question about your listening history. A local model writes
            a read-only SQL query over your listening projection and scrobbl.rs
            runs it in a sandbox.
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
                {response.sql?.rule_code &&
                  !response.message?.includes(response.sql.rule_code) && (
                    <p className="ask-coverage">
                      Rule: {response.sql.rule_code}
                    </p>
                  )}
                {!sqlEditorOpen && response.sql?.rule_code !== "cancelled" && (
                  <p className="ask-hint">
                    Smaller local models can occasionally struggle with complex
                    SQL. Try rephrasing or selecting a larger local GGUF model
                    in <Link to="/settings/ai">AI settings</Link>.
                  </p>
                )}
                {response.sql?.sql && (
                  <SqlDisclosure
                    sql={response.sql.sql}
                    running={loading}
                    defaultOpen={sqlEditorOpen}
                    onRunSql={(edited) =>
                      void runEditedSql(edited, response.sql)
                    }
                  />
                )}
              </Notice>
            )}

            {response.status === "ok" && response.sql && (
              <SqlResultCard
                sql={response.sql}
                preferences={preferences}
                running={loading}
                defaultSqlOpen={sqlEditorOpen}
                editedDescription={editedDescription}
                onRunSql={(edited) => void runEditedSql(edited, response.sql)}
              />
            )}
          </>
        )}
      </div>
    </>
  );
}

const SQL_PAGE_SIZE = 100;

const sqlDateFormatter = new Intl.DateTimeFormat("en-GB", {
  timeZone: "UTC",
  day: "numeric",
  month: "short",
  year: "numeric",
});
const sqlMonthFormatter = new Intl.DateTimeFormat("en-GB", {
  timeZone: "UTC",
  month: "short",
  year: "numeric",
});
const sqlDateTzFormatters = new Map<string, Intl.DateTimeFormat>();

function formatDateOnly(ms: number, timezone: string): string {
  let formatter = sqlDateTzFormatters.get(timezone);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat("en-GB", {
      timeZone: timezone,
      day: "2-digit",
      month: "short",
      year: "numeric",
    });
    sqlDateTzFormatters.set(timezone, formatter);
  }
  return formatter.format(ms);
}

function isNumericFormat(
  value: SqlValue,
  format: ColumnFormat | undefined,
): boolean {
  if (typeof value !== "number") return false;
  return (
    format === "count" ||
    format === "duration_ms" ||
    format === "percent" ||
    format === undefined
  );
}

function formatSqlCell(
  value: SqlValue,
  format: ColumnFormat | undefined,
  preferences: Preferences,
): string {
  if (value === null) return "—";
  if (typeof value === "number") {
    if (format === "duration_ms") {
      return duration(value, value >= 60_000 || value === 0);
    }
    if (format === "percent") {
      return `${value.toLocaleString("en-GB", { maximumFractionDigits: 1 })}%`;
    }
    if (format === "datetime") return date(value, preferences);
    if (format === "date") return formatDateOnly(value, preferences.timezone);
    return number.format(value);
  }
  const text = String(value);
  if (format === "date") {
    if (/^\d{4}-\d{2}-\d{2}$/.test(text)) {
      const parsed = new Date(`${text}T00:00:00Z`);
      if (!Number.isNaN(parsed.getTime()))
        return sqlDateFormatter.format(parsed);
    }
    if (/^\d{4}-\d{2}$/.test(text)) {
      const parsed = new Date(`${text}-01T00:00:00Z`);
      if (!Number.isNaN(parsed.getTime()))
        return sqlMonthFormatter.format(parsed);
    }
  }
  if (format === "datetime") {
    if (
      /^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}(:\d{2})?(Z|[+-]\d{2}:?\d{2})$/.test(
        text,
      )
    ) {
      const parsed = Date.parse(text);
      if (!Number.isNaN(parsed)) return date(parsed, preferences);
    }
    const localMatch = text.match(/^(\d{4}-\d{2}-\d{2})[T ](\d{2}:\d{2})/);
    if (localMatch) {
      const parsed = new Date(`${localMatch[1]}T00:00:00Z`);
      if (!Number.isNaN(parsed.getTime())) {
        return `${sqlDateFormatter.format(parsed)}, ${localMatch[2]}`;
      }
    }
  }
  return text;
}

function renderSqlCell(
  value: SqlValue,
  format: ColumnFormat | undefined,
  href: string | null | undefined,
  preferences: Preferences,
) {
  const text = formatSqlCell(value, format, preferences);
  return href ? <Link to={href}>{text}</Link> : text;
}

function chartColumns(
  sql: SqlAnswer,
): { labelCol: number; valueCol: number } | null {
  if (sql.columns.length < 2 || sql.rows.length === 0) return null;
  const numericIndices: number[] = [];
  for (let i = 0; i < sql.columns.length; i += 1) {
    if (sql.rows.some((row) => typeof row[i] === "number")) {
      numericIndices.push(i);
    }
  }
  const preferredValue = numericIndices.find((idx) => {
    const fmt = sql.columns[idx]?.format;
    return fmt === "count" || fmt === "duration_ms" || fmt === "percent";
  });
  const valueCol = preferredValue ?? numericIndices[numericIndices.length - 1];
  if (valueCol === undefined) return null;

  let labelCol = sql.columns.findIndex((col, idx) => {
    if (idx === valueCol) return false;
    return (
      col.format === "artist" ||
      col.format === "album" ||
      col.format === "track" ||
      col.format === "date" ||
      col.format === "datetime" ||
      col.format === "text"
    );
  });
  if (labelCol === -1) {
    labelCol = sql.columns.findIndex((_, idx) => idx !== valueCol);
  }
  if (labelCol === -1) return null;
  return { labelCol, valueCol };
}

function SqlDisclosure({
  sql,
  running,
  defaultOpen,
  onRunSql,
}: {
  sql: string;
  running: boolean;
  defaultOpen: boolean;
  onRunSql: (editedSql: string) => void;
}) {
  const [draft, setDraft] = useState(sql);
  const [open, setOpen] = useState(defaultOpen);
  useEffect(() => {
    setDraft(sql);
  }, [sql]);
  useEffect(() => {
    setOpen(defaultOpen);
  }, [defaultOpen, sql]);
  if (!sql.trim()) return null;
  return (
    <details
      className="ask-sql-disclosure"
      open={open}
      onToggle={(event) => setOpen(event.currentTarget.open)}
    >
      <summary>Show SQL</summary>
      <div className="ask-sql-editor">
        <textarea
          className="ask-sql-code"
          aria-label="SQL query"
          rows={4}
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          spellCheck={false}
          disabled={running}
        />
        <div className="ask-sql-actions">
          <Button
            type="button"
            variant="secondary"
            disabled={running || !draft.trim()}
            onClick={() => onRunSql(draft)}
          >
            {running ? "Running…" : "Run"}
          </Button>
          {draft !== sql && (
            <Button
              type="button"
              variant="secondary"
              disabled={running}
              onClick={() => setDraft(sql)}
            >
              Reset
            </Button>
          )}
        </div>
      </div>
    </details>
  );
}

function SqlTable({
  sql,
  pageRows,
  pageLinks,
  preferences,
}: {
  sql: SqlAnswer;
  pageRows: SqlValue[][];
  pageLinks: (string | null)[][];
  preferences: Preferences;
}) {
  return (
    <div className="ask-table-wrap" tabIndex={0}>
      <table className="ask-table">
        <thead>
          <tr>
            {sql.columns.map((column, index) => (
              <th key={`${column.name}-${index}`} scope="col">
                {column.name}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {pageRows.map((row, rowIndex) => (
            <tr key={rowIndex}>
              {row.map((cell, cellIndex) => {
                const format = sql.columns[cellIndex]?.format;
                const href = pageLinks[rowIndex]?.[cellIndex];
                return (
                  <td
                    key={cellIndex}
                    className={
                      isNumericFormat(cell, format)
                        ? "ask-table-value"
                        : undefined
                    }
                  >
                    {renderSqlCell(cell, format, href, preferences)}
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function SqlChart({
  sql,
  pageRows,
  pageLinks,
  labelCol,
  valueCol,
  preferences,
}: {
  sql: SqlAnswer;
  pageRows: SqlValue[][];
  pageLinks: (string | null)[][];
  labelCol: number;
  valueCol: number;
  preferences: Preferences;
}) {
  const max = Math.max(
    1,
    ...pageRows.map((row) => {
      const n = row[valueCol];
      return typeof n === "number" && Number.isFinite(n) && n > 0 ? n : 0;
    }),
  );
  const labelFormat = sql.columns[labelCol]?.format;
  const valueFormat = sql.columns[valueCol]?.format;
  return (
    <div className="ask-chart">
      <div
        className="chart-scroll"
        tabIndex={0}
        aria-label={sql.title || "Chart result"}
      >
        <div className="chart-bars">
          {pageRows.map((row, rowIndex) => {
            const label = formatSqlCell(
              row[labelCol],
              labelFormat,
              preferences,
            );
            const valueText = formatSqlCell(
              row[valueCol],
              valueFormat,
              preferences,
            );
            const rawNum = row[valueCol];
            const num =
              typeof rawNum === "number" &&
              Number.isFinite(rawNum) &&
              rawNum > 0
                ? rawNum
                : 0;
            const height = Math.max(2, (num / max) * 100);
            const href = pageLinks[rowIndex]?.[labelCol];
            const segment = (
              <>
                <span className="bar-space">
                  <span className="bar" style={{ height: `${height}%` }}>
                    <span className="actual-bar" style={{ flex: 1 }} />
                  </span>
                </span>
                <small title={label}>{label}</small>
                <small className="chart-bar-value">{valueText}</small>
              </>
            );
            return href ? (
              <Link
                key={rowIndex}
                to={href}
                className="chart-segment ask-chart-link"
                aria-label={`${label}: ${valueText}`}
              >
                {segment}
              </Link>
            ) : (
              <div
                key={rowIndex}
                className="chart-segment"
                role="group"
                aria-label={`${label}: ${valueText}`}
              >
                {segment}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function SqlResultCard({
  sql,
  preferences,
  running,
  defaultSqlOpen,
  editedDescription,
  onRunSql,
}: {
  sql: SqlAnswer;
  preferences: Preferences;
  running: boolean;
  defaultSqlOpen: boolean;
  editedDescription: string | null;
  onRunSql: (editedSql: string) => void;
}) {
  const [page, setPage] = useState(1);
  useEffect(() => {
    setPage(1);
  }, [sql]);

  const totalRows = sql.rows.length;
  const totalPages = Math.max(1, Math.ceil(totalRows / SQL_PAGE_SIZE));
  const safePage = Math.min(page, totalPages);
  const start = (safePage - 1) * SQL_PAGE_SIZE;
  const pageRows = sql.rows.slice(start, start + SQL_PAGE_SIZE);
  const pageLinks = (sql.cell_links ?? []).slice(start, start + SQL_PAGE_SIZE);

  const isSingleValue =
    sql.answer_kind === "value" &&
    sql.rows.length === 1 &&
    sql.columns.length >= 1;
  const chartCols =
    sql.answer_kind === "series" || sql.answer_kind === "ranking"
      ? chartColumns(sql)
      : null;

  return (
    <article className="panel padded ask-result">
      <p className="ask-eyebrow">Result</p>
      {sql.title && <p className="ask-result-headline">{sql.title}</p>}
      {sql.computed && (
        <p className="ask-sql-computed">
          <strong>The model says this query computes:</strong> {sql.computed}
        </p>
      )}
      {editedDescription && (
        <p className="ask-sql-computed">
          <strong>You ran edited SQL.</strong> This result comes from the query
          you changed. The model's description of its original query was:{" "}
          {editedDescription}
        </p>
      )}
      {sql.rows.length === 0 ? (
        <p className="ask-empty">No matching rows.</p>
      ) : isSingleValue ? (
        <>
          <p className="ask-result-value" aria-label="Result value">
            {renderSqlCell(
              sql.rows[0][0],
              sql.columns[0]?.format,
              sql.cell_links?.[0]?.[0],
              preferences,
            )}
          </p>
          {sql.columns.length > 1 && (
            <p className="ask-listen-meta">
              {sql.columns.slice(1).map((col, idx) => (
                <span key={`${col.name}-${idx}`}>
                  {idx > 0 ? " · " : ""}
                  {col.name}:{" "}
                  {renderSqlCell(
                    sql.rows[0][idx + 1],
                    col.format,
                    sql.cell_links?.[0]?.[idx + 1],
                    preferences,
                  )}
                </span>
              ))}
            </p>
          )}
        </>
      ) : chartCols ? (
        <>
          <SqlChart
            sql={sql}
            pageRows={pageRows}
            pageLinks={pageLinks}
            labelCol={chartCols.labelCol}
            valueCol={chartCols.valueCol}
            preferences={preferences}
          />
          {(sql.answer_kind === "ranking" || sql.columns.length > 2) && (
            <SqlTable
              sql={sql}
              pageRows={pageRows}
              pageLinks={pageLinks}
              preferences={preferences}
            />
          )}
        </>
      ) : (
        <SqlTable
          sql={sql}
          pageRows={pageRows}
          pageLinks={pageLinks}
          preferences={preferences}
        />
      )}
      {totalRows > SQL_PAGE_SIZE && (
        <div className="pagination">
          <button
            type="button"
            className="secondary"
            disabled={safePage <= 1}
            onClick={() => setPage(safePage - 1)}
          >
            Previous
          </button>
          <span>
            Page {safePage} of {totalPages} · {number.format(totalRows)} rows
          </span>
          <button
            type="button"
            className="secondary"
            disabled={safePage >= totalPages}
            onClick={() => setPage(safePage + 1)}
          >
            Next
          </button>
        </div>
      )}
      {sql.truncated && (
        <p className="ask-coverage">
          Showing the first {number.format(sql.rows.length)} rows (result
          truncated).
        </p>
      )}
      {sql.as_of_ms != null && (
        <p className="ask-coverage">
          Projection as of {date(sql.as_of_ms, preferences)}.
        </p>
      )}
      {sql.history_href && (
        <div className="ask-result-links">
          <Link to={sql.history_href}>
            {sql.history_href.startsWith("/rankings/")
              ? "See in rankings"
              : "See these listens"}
          </Link>
        </div>
      )}
      <SqlDisclosure
        sql={sql.sql}
        running={running}
        defaultOpen={defaultSqlOpen}
        onRunSql={onRunSql}
      />
    </article>
  );
}
