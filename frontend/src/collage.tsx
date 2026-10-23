import { useMemo, useRef, useState } from "react";
import type { Ranking } from "./generated";
import { Link } from "./navigation";
import { musicLink } from "./selection";
import type { Kind } from "./selection";
import { Cover, ErrorText, Icon } from "./ui";

type Tile = {
  item: Ranking;
  x: number;
  y: number;
  width: number;
  height: number;
};
// Pack full-width bands of squares. Bands can contain an even row, or a
// dominant square beside a smaller square grid. Minimize the difference between
// tile area shares and listening shares, with a compact overall aspect ratio.
export function collageTiles(items: Ranking[], byTime: boolean): Tile[] {
  if (!items.length) return [];
  const weight = (item: Ranking) =>
    Math.max(1, byTime ? item.listened_ms : item.plays);
  const rows = [...items].sort((a, b) => weight(b) - weight(a));
  const total = rows.reduce((sum, item) => sum + weight(item), 0);
  type Cell = { x: number; y: number; size: number };
  type Band = { cells: Cell[]; height: number };
  type Step = {
    cost: number;
    height: number;
    previous: Step | null;
    band: Band | null;
    offset: number;
  };
  const bands: Band[] = [];
  for (let count = 1; count <= rows.length; count++)
    bands.push({
      height: 1 / count,
      cells: Array.from({ length: count }, (_, i) => ({
        x: i / count,
        y: 0,
        size: 1 / count,
      })),
    });
  for (let side = 2; side < rows.length; side++)
    for (let columns = 1; 1 + side * columns <= rows.length; columns++) {
      const unit = 1 / (side + columns);
      bands.push({
        height: side * unit,
        cells: [
          { x: 0, y: 0, size: side * unit },
          ...Array.from({ length: side * columns }, (_, i) => ({
            x: (side + (i % columns)) * unit,
            y: Math.floor(i / columns) * unit,
            size: unit,
          })),
        ],
      });
    }
  const states = Array.from(
    { length: rows.length + 1 },
    () => new Map<number, Step>(),
  );
  states[0].set(0, {
    cost: 0,
    height: 0,
    previous: null,
    band: null,
    offset: 0,
  });
  for (let index = 0; index < rows.length; index++) {
    for (const band of bands) {
      const end = index + band.cells.length;
      if (end > rows.length) continue;
      const cost = band.cells.reduce((sum, cell, at) => {
        const share = weight(rows[index + at]) / total;
        return sum + share * Math.log(share / (cell.size * cell.size));
      }, 0);
      for (const step of states[index].values()) {
        const height = step.height + band.height;
        if (height > 1.5) continue;
        const bucket = Math.round(height * 240);
        const candidate = {
          cost: step.cost + cost,
          height,
          previous: step,
          band,
          offset: index,
        };
        if (
          !states[end].has(bucket) ||
          candidate.cost < states[end].get(bucket)!.cost
        )
          states[end].set(bucket, candidate);
      }
    }
  }
  const score = (step: Step) =>
    step.cost + Math.log(step.height) + 0.15 * Math.log(step.height) ** 2;
  let best: Step | undefined;
  for (const step of states[rows.length].values())
    if (step.height >= 0.75 && (!best || score(step) < score(best)))
      best = step;
  const ordered: Step[] = [];
  for (let step = best; step?.band; step = step.previous ?? undefined)
    ordered.unshift(step);
  let y = 0;
  return ordered.flatMap((step) => {
    const band = step.band!;
    const tiles = band.cells.map((cell, index) => ({
      item: rows[step.offset + index],
      x: cell.x,
      y: y + cell.y,
      width: cell.size,
      height: cell.size,
    }));
    y += band.height;
    return tiles;
  });
}

export function Collage({
  items,
  kind,
  params,
}: {
  items: Ranking[];
  kind: Kind;
  params: URLSearchParams;
}) {
  const [count, setCount] = useState(16);
  const [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  const preview = useRef<HTMLDivElement>(null);
  const byTime = params.get("sort") === "time";
  const tiles = useMemo(
    () => collageTiles(items.slice(0, count), byTime),
    [items, count, byTime],
  );
  const height = Math.max(1 / 25, ...tiles.map((tile) => tile.y + tile.height));
  const title = `${kind === "albums" ? "Album" : "Artist"} collage`;
  async function download() {
    setBusy(true);
    setError("");
    try {
      const canvas = document.createElement("canvas");
      canvas.width = 1200;
      canvas.height = Math.round(height * 1200) + 120;
      const ctx = canvas.getContext("2d");
      if (!ctx) throw new Error("This browser cannot export a collage.");
      ctx.fillStyle = "#171b20";
      ctx.fillRect(0, 0, 1200, canvas.height);
      ctx.fillStyle = "#ffffff";
      ctx.font = "600 30px sans-serif";
      ctx.fillText(title, 24, 48);
      const cells =
        preview.current?.querySelectorAll<HTMLElement>(".collage-tile");
      tiles.forEach((tile, index) => {
        const x = tile.x * 1200,
          y = 80 + tile.y * 1200,
          w = tile.width * 1200,
          h = tile.height * 1200;
        ctx.save();
        ctx.beginPath();
        ctx.rect(x, y, w, h);
        ctx.clip();
        ctx.fillStyle = "#303943";
        ctx.fillRect(x, y, w, h);
        const img = cells?.[index].querySelector("img");
        if (img?.complete && img.naturalWidth) {
          const scale = Math.min(w / img.naturalWidth, h / img.naturalHeight);
          ctx.drawImage(
            img,
            x + (w - img.naturalWidth * scale) / 2,
            y + (h - img.naturalHeight * scale) / 2,
            img.naturalWidth * scale,
            img.naturalHeight * scale,
          );
        }
        if (w >= 75 && h >= 48) {
          ctx.fillStyle = "#000b";
          ctx.fillRect(x, y + h - 44, w, 44);
          ctx.fillStyle = "#fff";
          ctx.font = "600 17px sans-serif";
          let label = tile.item.name;
          while (ctx.measureText(label).width > w - 16 && label.length > 1)
            label = label.slice(0, -2);
          ctx.fillText(
            label === tile.item.name ? label : `${label}…`,
            x + 8,
            y + h - 17,
          );
        }
        ctx.restore();
      });
      ctx.fillStyle = "#bec5cf";
      ctx.font = "16px sans-serif";
      ctx.fillText(
        `scrobbl.rs · ${params.get("period") || "all"} · ${params.get("sort") === "time" ? "Listening time" : "Listens"}`,
        24,
        canvas.height - 13,
      );
      const blob = await new Promise<Blob>((resolve, reject) =>
        canvas.toBlob(
          (value) =>
            value
              ? resolve(value)
              : reject(new Error("Could not export the collage.")),
          "image/png",
        ),
      );
      const url = URL.createObjectURL(blob),
        link = document.createElement("a");
      link.href = url;
      link.download = `scrobbl-${kind}-collage.png`;
      link.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="collage" aria-label={title}>
      <div className="collage-controls">
        <label>
          Tiles
          <select
            aria-label={`Number of ${kind} in collage`}
            value={count}
            onChange={(event) => setCount(Number(event.target.value))}
          >
            {[9, 16, 25].map((n) => (
              <option key={n}>{n}</option>
            ))}
          </select>
        </label>
        <button
          className="secondary"
          disabled={busy || !tiles.length}
          onClick={download}
        >
          <Icon>download</Icon>
          {busy ? "Preparing…" : "Download PNG"}
        </button>
      </div>
      <p className="subtle">
        From this page, sized by{" "}
        {params.get("sort") === "time" ? "listening time" : "listens"}. The
        download includes the artwork currently displayed.
      </p>
      <ErrorText>{error}</ErrorText>
      <div
        className="collage-preview"
        ref={preview}
        style={{ aspectRatio: 1 / height }}
      >
        {tiles.map((tile) => (
          <Link
            key={tile.item.id}
            className="collage-tile"
            to={musicLink(kind, tile.item.id, params)}
            aria-label={tile.item.name}
            style={{
              left: `${tile.x * 100}%`,
              top: `${(tile.y / height) * 100}%`,
              width: `${tile.width * 100}%`,
              height: `${(tile.height / height) * 100}%`,
            }}
          >
            <Cover url={tile.item.cover_url} large />
            <span>{tile.item.name}</span>
          </Link>
        ))}
      </div>
    </section>
  );
}
