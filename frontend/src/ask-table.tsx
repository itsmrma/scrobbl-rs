import type {
  AskResult,
  GroupBy,
  Metric,
  Query,
  ResultRow,
  TimeCoverage,
} from "./generated";
import { Link } from "./navigation";
import { Icon, number } from "./ui";

const temporalGroups: GroupBy[] = [
  "day",
  "week",
  "month",
  "year",
  "hour",
  "weekday",
];

const metricLabels: Record<Metric, string> = {
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

const groupLabels: Record<GroupBy, string> = {
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

const dateFormatters = new Map<string, Intl.DateTimeFormat>();
function formatDate(value: string, options: Intl.DateTimeFormatOptions) {
  const key = JSON.stringify(options);
  let formatter = dateFormatters.get(key);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat("en-GB", {
      timeZone: "UTC",
      ...options,
    });
    dateFormatters.set(key, formatter);
  }
  const date = new Date(`${value}T00:00:00Z`);
  return Number.isNaN(date.getTime()) ? value : formatter.format(date);
}

export function bucketLabel(row: ResultRow, groupBy: GroupBy) {
  switch (groupBy) {
    case "day":
      return formatDate(row.label, {
        day: "numeric",
        month: "short",
        year: "numeric",
      });
    case "week":
      return `Week of ${formatDate(row.label, { day: "numeric", month: "short" })}`;
    case "month":
      return formatDate(`${row.label}-01`, { month: "short", year: "numeric" });
    default:
      return row.label;
  }
}

function coverageText(time: TimeCoverage | null) {
  if (!time) return "—";
  const minutes = Math.round(time.known_ms / 60_000);
  return time.unknown_listens > 0
    ? `${number.format(minutes)} min · ${number.format(time.unknown_listens)} unknown`
    : `${number.format(minutes)} min`;
}

/** Grouped Ask results: a table for entities, a chart for assigned-calendar periods. */
export function AskResultRows({
  result,
  query,
}: {
  result: AskResult;
  query: Query;
}) {
  if (result.rows.length === 0) return null;
  const groupBy = query.group_by;
  if (groupBy && temporalGroups.includes(groupBy)) {
    // A ranking ("which day…") is a short list to read and open, not a
    // timeline: only chronological or long results are drawn as bars.
    if (query.sort_by === "metric" && result.rows.length <= 12) {
      return (
        <CategoricalTable
          rows={result.rows.map((row) => ({
            ...row,
            label: bucketLabel(row, groupBy),
          }))}
          metric={result.metric}
          groupBy={groupBy}
          nameHeader={groupLabels[groupBy]}
        />
      );
    }
    return (
      <TemporalRows
        rows={result.rows}
        groupBy={groupBy}
        metric={result.metric}
      />
    );
  }
  return (
    <CategoricalTable
      rows={result.rows}
      metric={result.metric}
      groupBy={groupBy ?? "track"}
    />
  );
}

function CategoricalTable({
  rows,
  metric,
  groupBy,
  nameHeader = "Name",
}: {
  rows: ResultRow[];
  metric: Metric;
  groupBy: GroupBy;
  nameHeader?: string;
}) {
  const metricLabel = metricLabels[metric];
  const noun = groupLabels[groupBy];
  return (
    <div
      className="ask-table-wrap"
      role="region"
      aria-label={`${metricLabel} by ${noun}`}
      tabIndex={0}
    >
      <table className="ask-table">
        <caption className="visually-hidden">
          {metricLabel} grouped by {noun}
        </caption>
        <thead>
          <tr>
            <th scope="col" className="ask-table-name">
              {nameHeader}
            </th>
            <th scope="col">{metricLabel}</th>
            <th scope="col">Known time</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.key}>
              <th scope="row">
                {row.href ? (
                  <Link to={row.href}>
                    {row.label}
                    <Icon>chevron_right</Icon>
                  </Link>
                ) : (
                  row.label
                )}
              </th>
              <td className="ask-table-value">{number.format(row.plays)}</td>
              <td className="ask-table-time">{coverageText(row.time)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function TemporalRows({
  rows,
  groupBy,
  metric,
}: {
  rows: ResultRow[];
  groupBy: GroupBy;
  metric: Metric;
}) {
  const max = Math.max(1, ...rows.map((row) => row.plays));
  const knownMs = rows.reduce((sum, row) => sum + (row.time?.known_ms ?? 0), 0);
  const unknown = rows.reduce(
    (sum, row) => sum + (row.time?.unknown_listens ?? 0),
    0,
  );
  return (
    <div className="ask-chart">
      <div
        className="chart-scroll"
        tabIndex={0}
        aria-label={`${metricLabels[metric]} by ${groupLabels[groupBy]}`}
      >
        <div className="chart-bars">
          {rows.map((row) => {
            const label = bucketLabel(row, groupBy);
            const height = Math.max(2, (row.plays / max) * 100);
            const segment = (
              <>
                <span className="bar-space">
                  <span className="bar" style={{ height: `${height}%` }}>
                    <span className="actual-bar" style={{ flex: 1 }} />
                  </span>
                </span>
                <small title={label}>{label}</small>
                <small className="chart-bar-value">
                  {number.format(row.plays)}
                </small>
              </>
            );
            return row.href ? (
              <Link
                className="chart-segment ask-chart-link"
                key={row.key}
                to={row.href}
                aria-label={`${label}: ${number.format(row.plays)} ${metricLabels[metric]}. Open these listens`}
              >
                {segment}
              </Link>
            ) : (
              <div
                className="chart-segment"
                key={row.key}
                role="group"
                aria-label={`${label}: ${number.format(row.plays)} ${metricLabels[metric]}`}
              >
                {segment}
              </div>
            );
          })}
        </div>
      </div>
      {knownMs > 0 || unknown > 0 ? (
        <p className="ask-coverage">
          Known time {number.format(Math.round(knownMs / 60_000))} minutes
          {unknown > 0
            ? ` · ${number.format(unknown)} listens with unknown duration`
            : ""}
        </p>
      ) : null}
    </div>
  );
}
