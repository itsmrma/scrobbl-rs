import { useState } from "react";
import type { CSSProperties } from "react";
import "./listening-patterns.css";
import { api } from "./api";
import { useLoad } from "./load";
import { Select } from "./select";
import { ErrorText, number } from "./ui";
import { SkeletonPatterns } from "./skeleton";

const views = [
  ["hours", "Time of day"],
  ["weekdays", "Day of week"],
  ["devices", "Device / app"],
  ["shuffle", "Shuffle"],
  ["offline", "Offline listening"],
  ["skipped", "Skipped"],
  ["countries", "Listening country"],
  ["start", "Playback start reason"],
  ["end", "Playback end reason"],
  ["evidence", "Duration evidence"],
] as const;
export function ListeningPatterns({
  params,
  version,
}: {
  params: URLSearchParams;
  version: number;
}) {
  const [view, setView] = useState("hours");
  const [inspected, setInspected] = useState<string | null>(null);
  const result = useLoad(
    () => api.listeningPatterns(params),
    `patterns:${params}:${version}`,
    true,
    undefined,
    true,
  );
  const bins = result.data?.series[view] || [];
  const max = Math.max(1, ...bins.map((bin) => bin.count));
  const unknown =
    (result.data?.coverage[view]?.unknown || 0) +
    (result.data?.coverage[view]?.conflicting || 0);
  const temporal = view === "hours" || view === "weekdays";
  const selected =
    bins.find((bin) => bin.label === inspected) ||
    bins.reduce<(typeof bins)[number] | undefined>(
      (best, bin) => (!best || bin.count > best.count ? bin : best),
      undefined,
    );
  return (
    <article
      className="panel padded listening-patterns"
      aria-label="Listening patterns"
    >
      <div className="panel-title">
        <h2>Listening patterns</h2>
        <Select
          aria-label="Listening pattern"
          value={view}
          onChange={(value) => {
            setView(value);
            setInspected(null);
          }}
        >
          {views.map(([key, label]) => (
            <option value={key} key={key}>
              {label}
            </option>
          ))}
        </Select>
      </div>
      <p className="subtle">
        Counted listens in the selected period. Hours and weekdays use listen
        start times in their assigned timezones. Spotify export details are
        shown only when available.
      </p>
      <ErrorText>{result.error}</ErrorText>
      {!result.data && !result.error && (
        <SkeletonPatterns label="Loading listening patterns…" />
      )}
      {result.data && (
        <>
          <p className="pattern-summary" role="status">
            {number.format(result.data.total)} listens ·{" "}
            {!result.data.total
              ? "No data in this period"
              : unknown
                ? `${number.format(result.data.total - unknown)} with known ${views.find(([key]) => key === view)?.[1].toLowerCase()} · ${number.format(unknown)} unknown or conflicting`
                : "Complete coverage"}
          </p>
          {view === "devices" && (
            <p className="subtle">
              Spotify’s platform field describes the app or platform, not
              necessarily a specific physical device.
            </p>
          )}
          {unknown > 0 && (
            <p className="subtle">
              Older compact imports did not retain these fields. Reimport the
              original Spotify export to add missing details without duplicating
              listens. Missing or conflicting values remain explicit.
            </p>
          )}
          {selected && result.data.total > 0 && (
            <p className="pattern-readout" aria-live="polite">
              {view === "weekdays" ? selected.label.slice(2) : selected.label} ·{" "}
              {number.format(selected.count)} listens ·{" "}
              {((selected.count / result.data.total) * 100).toFixed(1)}%
            </p>
          )}
          {!result.data.total ? (
            <p>No counted listens in this selection.</p>
          ) : (
            <div
              className={`pattern-bars ${temporal ? "pattern-temporal" : ""} ${view === "hours" ? "pattern-hours" : ""}`}
              style={
                temporal
                  ? ({
                      "--pattern-columns": bins.length,
                    } as CSSProperties)
                  : undefined
              }
              role="list"
              aria-label={views.find(([key]) => key === view)?.[1]}
            >
              {bins.map((bin) => {
                const label =
                  view === "weekdays" ? bin.label.slice(2) : bin.label;
                return (
                  <div
                    className="pattern-row"
                    role="listitem"
                    tabIndex={0}
                    onFocus={() => setInspected(bin.label)}
                    onMouseEnter={() => setInspected(bin.label)}
                    onClick={() => setInspected(bin.label)}
                    key={bin.label}
                    aria-label={`${label}: ${number.format(bin.count)} listens`}
                  >
                    <span className="pattern-label">
                      {temporal
                        ? view === "hours"
                          ? bin.label.slice(0, 2)
                          : label.slice(0, 3)
                        : label}
                    </span>
                    <span className="pattern-track" aria-hidden="true">
                      <span
                        style={
                          temporal
                            ? { height: `${(bin.count / max) * 100}%` }
                            : { width: `${(bin.count / max) * 100}%` }
                        }
                      />
                    </span>
                    {!temporal && (
                      <strong>
                        {number.format(bin.count)}
                        <small>
                          {((bin.count / result.data!.total) * 100).toFixed(1)}%
                        </small>
                      </strong>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </>
      )}
    </article>
  );
}
