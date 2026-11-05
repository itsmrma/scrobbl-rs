import type { TimeCoverage } from "./generated";
import { duration, number } from "./ui";

/**
 * Deterministic time card. Known time is actual + estimated; unknown duration
 * is always reported as a count and a share, never as zero time.
 */
export function AskTimeCard({ time }: { time: TimeCoverage }) {
  const hasKnown = time.known_ms > 0;
  const unknown = time.unknown_listens;
  const percent = Math.round(time.unknown_percent * 10) / 10;
  return (
    <div className="ask-time">
      {hasKnown ? (
        <p className="ask-result-value" aria-label="Known listening time">
          {duration(time.known_ms, true)}
        </p>
      ) : (
        <p className="ask-empty">No known listening time in this selection.</p>
      )}
      {hasKnown && (
        <p className="ask-listen-meta">
          {duration(time.actual_ms, true)} actual ·{" "}
          {duration(time.estimated_ms, true)} estimated
        </p>
      )}
      <p className="ask-coverage">
        {unknown > 0
          ? `Unknown time: ${number.format(unknown)} ${unknown === 1 ? "listen" : "listens"} · ${number.format(percent)}% of counted listens`
          : "All counted listens have a known duration."}
      </p>
    </div>
  );
}
