import type { AskResult, Preferences, Query } from "./generated";
import { Link } from "./navigation";
import { number } from "./ui";

const thresholds = [
  10, 50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000, 25_000, 50_000, 100_000,
];

const dayFormatters = new Map<string, Intl.DateTimeFormat>();

function day(ms: number, preferences: Preferences) {
  let formatter = dayFormatters.get(preferences.timezone);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat("en-GB", {
      timeZone: preferences.timezone,
      day: "2-digit",
      month: "short",
      year: "numeric",
    });
    dayFormatters.set(preferences.timezone, formatter);
  }
  return formatter.format(ms);
}

function range(start: string | null, end: string | null) {
  if (!start || !end) return null;
  return start === end ? start : `${start} – ${end}`;
}

function days(count: number) {
  return `${number.format(count)} ${count === 1 ? "day" : "days"}`;
}

function entityName(query: Query) {
  return query.entity?.name ?? "your archive";
}

function nextThreshold(total: number) {
  return thresholds.find((threshold) => threshold > total) ?? null;
}

export function StreakResultCard({
  query,
  result,
  preferences,
}: {
  query: Query;
  result: AskResult;
  preferences: Preferences;
}) {
  const entity = entityName(query);
  const headline =
    result.metric === "streak"
      ? (result.streak?.current.days ?? 0) > 0
        ? `${entity}: ${days(result.streak!.current.days)} in a row.`
        : `No current listening streak for ${entity}.`
      : `${entity}: ${result.milestones.length} listening ${
          result.milestones.length === 1 ? "milestone" : "milestones"
        } reached.`;

  return (
    <article className="panel padded ask-result">
      <p className="ask-eyebrow">Result</p>
      <p className="ask-result-headline">{headline}</p>
      {result.metric === "streak" ? (
        <StreakDetail result={result} />
      ) : (
        <MilestoneDetail result={result} preferences={preferences} />
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

function StreakDetail({ result }: { result: AskResult }) {
  const streak = result.streak;
  if (!streak) {
    return <p className="ask-empty">No streak data available.</p>;
  }
  const currentRange = range(streak.current.start, streak.current.end);
  const longestRange = range(streak.longest.start, streak.longest.end);
  return (
    <>
      <div className="ask-listen">
        <p className="ask-eyebrow">Current streak</p>
        {streak.current.days > 0 ? (
          <>
            <p className="ask-listen-title">
              {days(streak.current.days)} in a row
            </p>
            {currentRange && <p className="ask-listen-date">{currentRange}</p>}
          </>
        ) : (
          <p className="ask-empty">
            No current streak. It ends after a day without a counted listen.
          </p>
        )}
      </div>
      <div className="ask-listen">
        <p className="ask-eyebrow">Longest streak</p>
        {streak.longest.days > 0 ? (
          <>
            <p className="ask-listen-title">
              {days(streak.longest.days)} in a row
            </p>
            {longestRange && <p className="ask-listen-date">{longestRange}</p>}
          </>
        ) : (
          <p className="ask-empty">No listening days yet.</p>
        )}
      </div>
    </>
  );
}

function MilestoneDetail({
  result,
  preferences,
}: {
  result: AskResult;
  preferences: Preferences;
}) {
  const total = result.scalar ?? 0;
  const next = nextThreshold(total);
  const reached = [...result.milestones].sort((a, b) => a.count - b.count);
  return (
    <>
      <p className="ask-result-value" aria-label="Counted listens">
        {number.format(total)}
      </p>
      <p className="ask-listen-date">
        counted listens in this entity&apos;s whole history.
      </p>
      {reached.length === 0 ? (
        <p className="ask-empty">
          No milestones reached yet. The first milestone is{" "}
          {number.format(thresholds[0])} listens.
        </p>
      ) : (
        <ul className="ask-rows">
          {reached.map((milestone) => (
            <li key={milestone.count}>
              <span>{number.format(milestone.count)} listens</span>
              <span>{day(milestone.reached_at_ms, preferences)}</span>
            </li>
          ))}
        </ul>
      )}
      {next !== null ? (
        <>
          <ul className="ask-rows">
            <li>
              <strong>{number.format(next)} listens</strong>
              <span>Next milestone</span>
            </li>
          </ul>
          <p className="ask-hint">
            {number.format(next - total)} more{" "}
            {next - total === 1 ? "listen" : "listens"} to reach it.
          </p>
        </>
      ) : total > 0 ? (
        <p className="ask-hint">Every milestone has been reached.</p>
      ) : null}
    </>
  );
}
