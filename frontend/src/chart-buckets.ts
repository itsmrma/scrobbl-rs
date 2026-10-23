import type { ActivityBucket } from "./generated";

export type ChartBucket = ActivityBucket & { periods: number };
// Preserve exact totals and the largest peaks as long as possible. Adjacent
// low-activity periods are the cheapest pair to combine; zero periods add no bar.
export function compactBuckets(
  input: ActivityBucket[],
  limit: number,
): ChartBucket[] {
  const groups = input
    .filter(
      (b) => b.listens || b.actual_ms || b.estimated_ms || b.unknown_durations,
    )
    .map((b) => ({ ...b, periods: 1 }));
  const total = groups.reduce((sum, b) => sum + b.listens, 0);
  const low = Math.max(1, total * 0.02);
  while (groups.length > limit) {
    let best = 0,
      cost = Infinity;
    for (let i = 0; i < groups.length - 1; i++) {
      const a = groups[i],
        b = groups[i + 1];
      const score =
        a.listens +
        b.listens +
        (Math.max(a.listens, b.listens) > low ? total : 0);
      if (score < cost) {
        cost = score;
        best = i;
      }
    }
    const a = groups[best],
      b = groups[best + 1];
    groups.splice(best, 2, {
      start: a.start,
      through: b.through,
      listens: a.listens + b.listens,
      actual_ms: a.actual_ms + b.actual_ms,
      estimated_ms: a.estimated_ms + b.estimated_ms,
      unknown_durations: a.unknown_durations + b.unknown_durations,
      partial: a.partial || b.partial,
      periods: a.periods + b.periods,
    });
  }
  return groups;
}
