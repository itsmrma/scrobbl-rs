import type { CSSProperties } from "react";
import { ErrorText } from "./ui";

export function SkeletonBlock({
  width,
  height,
  borderRadius,
  className = "",
  style,
  ...props
}: {
  width?: string | number;
  height?: string | number;
  borderRadius?: string | number;
  className?: string;
  style?: CSSProperties;
  [key: string]: unknown;
}) {
  return (
    <div
      className={`skeleton-block ${className}`}
      style={{
        width,
        height,
        borderRadius,
        ...style,
      }}
      {...props}
    />
  );
}

export function SkeletonText({
  lines = 1,
  widths = ["100%"],
  height = 14,
  className = "",
}: {
  lines?: number;
  widths?: (string | number)[];
  height?: string | number;
  className?: string;
}) {
  return (
    <div className={`skeleton-text ${className}`}>
      {Array.from({ length: lines }).map((_, i) => (
        <SkeletonBlock
          key={i}
          width={widths[i % widths.length] ?? "100%"}
          height={height}
          borderRadius={4}
          style={{ marginBottom: i < lines - 1 ? 6 : 0 }}
        />
      ))}
    </div>
  );
}

export function SkeletonListenRow() {
  return (
    <div className="listen-row skeleton-listen-row" aria-hidden="true">
      <div className="listen-main">
        <SkeletonBlock className="cover" borderRadius={6} />
        <div className="listen-title">
          <SkeletonBlock width="75%" height={15} borderRadius={4} />
          <SkeletonBlock
            width="45%"
            height={12}
            borderRadius={4}
            style={{ marginTop: 6 }}
          />
        </div>
        <div className="listen-album">
          <SkeletonBlock width="60%" height={13} borderRadius={4} />
        </div>
        <div className="listen-date">
          <SkeletonBlock width="80px" height={13} borderRadius={4} />
          <SkeletonBlock
            width="50px"
            height={11}
            borderRadius={4}
            style={{ marginTop: 4 }}
          />
        </div>
        <div className="listen-duration">
          <SkeletonBlock width="40px" height={13} borderRadius={4} />
        </div>
      </div>
    </div>
  );
}

export function ListenListSkeleton({
  count = 6,
  label = "Loading listening history…",
}: {
  count?: number;
  label?: string;
}) {
  return (
    <div className="skeleton-container" role="status" aria-busy="true">
      <span className="visually-hidden">{label}</span>
      <div className="skeleton-list">
        {Array.from({ length: count }).map((_, i) => (
          <SkeletonListenRow key={i} />
        ))}
      </div>
    </div>
  );
}

export function SkeletonRankingRow({ grid = false }: { grid?: boolean }) {
  if (grid) {
    return (
      <li className="music-card skeleton-music-card" aria-hidden="true">
        <SkeletonBlock className="cover large" borderRadius={8} />
        <div className="rank-name" style={{ marginTop: 8 }}>
          <SkeletonBlock width="80%" height={14} borderRadius={4} />
          <SkeletonBlock
            width="50%"
            height={12}
            borderRadius={4}
            style={{ marginTop: 4 }}
          />
        </div>
      </li>
    );
  }
  return (
    <li className="ranking-row skeleton-ranking-row" aria-hidden="true">
      <SkeletonBlock
        width={20}
        height={18}
        borderRadius={4}
        className="rank-number"
      />
      <SkeletonBlock
        width={38}
        height={38}
        borderRadius={6}
        className="cover"
      />
      <div className="rank-name">
        <SkeletonBlock width="70%" height={14} borderRadius={4} />
        <SkeletonBlock
          width="45%"
          height={12}
          borderRadius={4}
          style={{ marginTop: 4 }}
        />
      </div>
      <div className="rank-value">
        <SkeletonBlock width={36} height={14} borderRadius={4} />
      </div>
    </li>
  );
}

export function RankingsSkeleton({
  count = 6,
  label = "Loading rankings…",
  grid = false,
}: {
  count?: number;
  label?: string;
  grid?: boolean;
}) {
  return (
    <div className="skeleton-container" role="status" aria-busy="true">
      <span className="visually-hidden">{label}</span>
      <ol className={grid ? "music-grid" : "ranking-list"}>
        {Array.from({ length: count }).map((_, i) => (
          <SkeletonRankingRow key={i} grid={grid} />
        ))}
      </ol>
    </div>
  );
}

export function ChartSkeleton({
  label = "Loading activity…",
  standalone = true,
}: {
  label?: string;
  standalone?: boolean;
}) {
  const bars = [
    35, 60, 20, 75, 45, 90, 30, 70, 50, 85, 40, 65, 80, 55, 95, 30, 70, 45,
  ];
  const content = (
    <article className="panel padded skeleton-chart-panel">
      <div className="panel-title" style={{ marginBottom: 16 }}>
        <SkeletonBlock width="25%" height={20} borderRadius={4} />
        <SkeletonBlock width="15%" height={14} borderRadius={4} />
      </div>
      <div className="chart-scroll">
        <div className="activity-chart skeleton-activity-chart">
          {bars.map((height, i) => (
            <div key={i} className="chart-segment skeleton-chart-segment">
              <span className="chart-bar-area">
                <SkeletonBlock
                  className="skeleton-bar"
                  height={`${height}%`}
                  borderRadius="3px 3px 0 0"
                />
              </span>
              <SkeletonBlock
                width="70%"
                height={10}
                borderRadius={2}
                style={{ marginTop: 8 }}
              />
            </div>
          ))}
        </div>
      </div>
    </article>
  );
  if (!standalone) return content;
  return (
    <div className="skeleton-container" role="status" aria-busy="true">
      <span className="visually-hidden">{label}</span>
      {content}
    </div>
  );
}

export function OverviewSkeleton({
  label = "Loading your archive…",
  standalone = true,
}: {
  label?: string;
  standalone?: boolean;
}) {
  const content = (
    <>
      <span className="visually-hidden">{label}</span>
      <div className="stats-grid">
        {[1, 2, 3].map((i) => (
          <div className="stat skeleton-stat" key={i}>
            <SkeletonBlock width="45%" height={13} borderRadius={4} />
            <SkeletonBlock
              width="60%"
              height={32}
              borderRadius={6}
              style={{ margin: "6px 0" }}
            />
            <SkeletonBlock width="35%" height={11} borderRadius={4} />
          </div>
        ))}
      </div>
      <div className="rankings-grid overview-rankings">
        {(["artists", "albums", "tracks"] as const).map((kind) => (
          <article className="panel padded" key={kind}>
            <div className="panel-title">
              <SkeletonBlock width="35%" height={20} borderRadius={4} />
              <SkeletonBlock width="25%" height={13} borderRadius={4} />
            </div>
            <ol className="ranking-list" style={{ marginTop: 12 }}>
              {[1, 2, 3, 4, 5].map((j) => (
                <SkeletonRankingRow key={j} />
              ))}
            </ol>
          </article>
        ))}
      </div>
      <ChartSkeleton label="Loading activity…" standalone={false} />
      <article className="panel">
        <div className="panel-title padded">
          <SkeletonBlock width="30%" height={22} borderRadius={4} />
          <SkeletonBlock width="20%" height={13} borderRadius={4} />
        </div>
        <div>
          {[1, 2, 3, 4, 5].map((i) => (
            <SkeletonListenRow key={i} />
          ))}
        </div>
      </article>
    </>
  );

  if (!standalone) return <>{content}</>;
  return (
    <div className="skeleton-container" role="status" aria-busy="true">
      {content}
    </div>
  );
}

export function MusicPageSkeleton({
  label = "Loading music page…",
}: {
  label?: string;
}) {
  return (
    <div className="skeleton-container" role="status" aria-busy="true">
      <span className="visually-hidden">{label}</span>
      <div className="music-heading" style={{ marginBottom: 24 }}>
        <SkeletonBlock className="cover large" borderRadius={12} />
        <div className="music-title" style={{ flex: 1 }}>
          <div className="music-heading-title">
            <div style={{ flex: 1 }}>
              <SkeletonBlock width="60%" height={28} borderRadius={6} />
              <SkeletonBlock
                width="35%"
                height={16}
                borderRadius={4}
                style={{ marginTop: 10 }}
              />
            </div>
          </div>
          <div style={{ display: "flex", gap: 12, marginTop: 16 }}>
            <SkeletonBlock width={100} height={36} borderRadius={6} />
            <SkeletonBlock width={120} height={36} borderRadius={6} />
          </div>
        </div>
      </div>
      <div className="stats-grid">
        {[1, 2, 3].map((i) => (
          <div className="stat skeleton-stat" key={i}>
            <SkeletonBlock width="45%" height={13} borderRadius={4} />
            <SkeletonBlock
              width="60%"
              height={32}
              borderRadius={6}
              style={{ margin: "6px 0" }}
            />
            <SkeletonBlock width="35%" height={11} borderRadius={4} />
          </div>
        ))}
      </div>
    </div>
  );
}

export function SkeletonCardList({
  count = 3,
  label,
}: {
  count?: number;
  label: string;
}) {
  return (
    <div className="skeleton-container" role="status" aria-busy="true">
      <span className="visually-hidden">{label}</span>
      <div className="skeleton-card-list">
        {Array.from({ length: count }).map((_, i) => (
          <article
            className="panel padded skeleton-panel"
            key={i}
            style={{ marginBottom: 16 }}
          >
            <div
              style={{
                display: "flex",
                justifyContent: "space-between",
                alignItems: "center",
                marginBottom: 14,
              }}
            >
              <SkeletonBlock width="40%" height={20} borderRadius={4} />
              <SkeletonBlock width={80} height={28} borderRadius={6} />
            </div>
            <SkeletonBlock
              width="85%"
              height={14}
              borderRadius={4}
              style={{ marginBottom: 8 }}
            />
            <SkeletonBlock width="60%" height={14} borderRadius={4} />
          </article>
        ))}
      </div>
    </div>
  );
}

export function SkeletonList({
  count = 4,
  label,
}: {
  count?: number;
  label: string;
}) {
  return (
    <div className="skeleton-container" role="status" aria-busy="true">
      <span className="visually-hidden">{label}</span>
      <div className="skeleton-list">
        {Array.from({ length: count }).map((_, i) => (
          <div
            key={i}
            className="skeleton-list-item"
            style={{
              display: "flex",
              alignItems: "center",
              gap: 14,
              padding: "14px 16px",
              borderBottom: "1px solid var(--border)",
            }}
          >
            <SkeletonBlock width={20} height={20} borderRadius={4} />
            <div style={{ flex: 1 }}>
              <SkeletonBlock width="65%" height={15} borderRadius={4} />
              <SkeletonBlock
                width="40%"
                height={12}
                borderRadius={4}
                style={{ marginTop: 6 }}
              />
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

export function SkeletonPatterns({
  label = "Loading listening patterns…",
}: {
  label?: string;
}) {
  return (
    <div className="skeleton-container" role="status" aria-busy="true">
      <span className="visually-hidden">{label}</span>
      <div className="stats-grid" style={{ marginBottom: 16 }}>
        {[1, 2].map((i) => (
          <div className="stat skeleton-stat" key={i}>
            <SkeletonBlock width="50%" height={12} borderRadius={4} />
            <SkeletonBlock
              width="40%"
              height={26}
              borderRadius={6}
              style={{ margin: "4px 0" }}
            />
          </div>
        ))}
      </div>
      <article className="panel padded skeleton-panel">
        <SkeletonBlock
          width="35%"
          height={18}
          borderRadius={4}
          style={{ marginBottom: 16 }}
        />
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "repeat(12, 1fr)",
            gap: 6,
            height: 100,
            alignItems: "end",
          }}
        >
          {[30, 50, 70, 40, 80, 60, 90, 45, 65, 85, 55, 75].map((h, i) => (
            <SkeletonBlock
              key={i}
              width="100%"
              height={`${h}%`}
              borderRadius="4px 4px 0 0"
            />
          ))}
        </div>
      </article>
    </div>
  );
}

export function SkeletonCard({ label }: { label: string }) {
  return (
    <div className="skeleton-container" role="status" aria-busy="true">
      <span className="visually-hidden">{label}</span>
      <article className="panel padded skeleton-panel">
        <SkeletonBlock
          width="35%"
          height={18}
          borderRadius={4}
          style={{ marginBottom: 12 }}
        />
        <SkeletonBlock
          width="70%"
          height={13}
          borderRadius={4}
          style={{ marginBottom: 8 }}
        />
        <SkeletonBlock width="50%" height={13} borderRadius={4} />
      </article>
    </div>
  );
}

export function SkeletonPlayback() {
  return (
    <div className="sidebar-now">
      <div
        className="skeleton-container skeleton-playback"
        role="status"
        aria-busy="true"
      >
        <span className="visually-hidden">Loading now playing…</span>
        <SkeletonBlock width={40} height={40} borderRadius={6} />
        <SkeletonText
          lines={2}
          widths={["85%", "60%"]}
          className="skeleton-playback-copy"
        />
      </div>
    </div>
  );
}

export function AppShellSkeleton({
  label = "Opening your archive…",
  error,
  onReload,
}: {
  label?: string;
  error?: string;
  onReload?: () => void;
}) {
  return (
    <main className="app-skeleton" role="status" aria-busy="true">
      <span className="visually-hidden">{label}</span>
      <aside className="app-skeleton-sidebar">
        <div className="app-skeleton-logo">
          <SkeletonBlock width={110} height={28} borderRadius={6} />
        </div>
        <div className="app-skeleton-nav">
          {[1, 2, 3, 4, 5].map((i) => (
            <div key={i} className="app-skeleton-nav-item">
              <SkeletonBlock width={22} height={22} borderRadius={4} />
              <SkeletonBlock width="60%" height={14} borderRadius={4} />
            </div>
          ))}
        </div>
      </aside>
      <div className="app-skeleton-main">
        <header className="app-skeleton-header">
          <SkeletonBlock width={180} height={24} borderRadius={6} />
          <div style={{ display: "flex", gap: 12 }}>
            <SkeletonBlock width={36} height={36} borderRadius="50%" />
          </div>
        </header>
        {error ? (
          <div className="loading" style={{ padding: "40px 0" }}>
            <ErrorText>{error}</ErrorText>
            {onReload && (
              <button
                type="button"
                className="secondary"
                onClick={onReload}
                style={{ marginTop: 16 }}
              >
                Reload
              </button>
            )}
          </div>
        ) : (
          <OverviewSkeleton label={label} standalone={false} />
        )}
      </div>
    </main>
  );
}
