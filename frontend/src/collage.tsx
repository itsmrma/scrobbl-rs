import { useRef, useState } from "react";
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
// Split each rectangle near half its listening weight. The result fills the
// square without gaps or overlap and stays deterministic when totals tie.
export function collageTiles(items: Ranking[], byTime: boolean): Tile[] {
  const weight = (item: Ranking) =>
    Math.max(1, byTime ? item.listened_ms : item.plays);
  function split(
    rows: Ranking[],
    x: number,
    y: number,
    width: number,
    height: number,
  ): Tile[] {
    if (!rows.length) return [];
    if (rows.length === 1) return [{ item: rows[0], x, y, width, height }];
    const total = rows.reduce((sum, item) => sum + weight(item), 0);
    let at = 1,
      sum = weight(rows[0]);
    while (
      at < rows.length - 1 &&
      Math.abs(total / 2 - sum - weight(rows[at])) < Math.abs(total / 2 - sum)
    )
      sum += weight(rows[at++]);
    const ratio = sum / total;
    return width >= height
      ? [
          ...split(rows.slice(0, at), x, y, width * ratio, height),
          ...split(
            rows.slice(at),
            x + width * ratio,
            y,
            width * (1 - ratio),
            height,
          ),
        ]
      : [
          ...split(rows.slice(0, at), x, y, width, height * ratio),
          ...split(
            rows.slice(at),
            x,
            y + height * ratio,
            width,
            height * (1 - ratio),
          ),
        ];
  }
  return split(items, 0, 0, 1, 1);
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
  const tiles = collageTiles(
    items.slice(0, count),
    params.get("sort") === "time",
  );
  const title = `${kind === "albums" ? "Album" : "Artist"} collage`;
  async function download() {
    setBusy(true);
    setError("");
    try {
      const canvas = document.createElement("canvas");
      canvas.width = 1200;
      canvas.height = 1320;
      const ctx = canvas.getContext("2d");
      if (!ctx) throw new Error("This browser cannot export a collage.");
      ctx.fillStyle = "#171b20";
      ctx.fillRect(0, 0, 1200, 1320);
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
          const scale = Math.max(w / img.naturalWidth, h / img.naturalHeight);
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
        1307,
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
      <div className="collage-preview" ref={preview}>
        {tiles.map((tile) => (
          <Link
            key={tile.item.id}
            className="collage-tile"
            to={musicLink(kind, tile.item.id, params)}
            aria-label={tile.item.name}
            style={{
              left: `${tile.x * 100}%`,
              top: `${tile.y * 100}%`,
              width: `${tile.width * 100}%`,
              height: `${tile.height * 100}%`,
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
