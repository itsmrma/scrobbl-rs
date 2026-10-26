import { api } from "./api";
import { plainCredit } from "./image-credit";
import { Select } from "./select";
import { useMemo, useRef, useState } from "react";
import type { Ranking } from "./generated";
import { Link } from "./navigation";
import { musicLink } from "./selection";
import type { Kind } from "./selection";
import { Cover, ErrorText, Icon, duration, number } from "./ui";

type Tile = {
  item: Ranking;
  x: number;
  y: number;
  width: number;
  height: number;
};
// Partition by the selected metric: each tile's area equals its listening share.
export function collageTiles(items: Ranking[], byTime: boolean): Tile[] {
  const weight = (item: Ranking) =>
    Math.max(1, byTime ? item.listened_ms : item.plays);
  const rows = [...items].sort((a, b) => weight(b) - weight(a));
  function partition(
    rows: Ranking[],
    x: number,
    y: number,
    width: number,
    height: number,
  ): Tile[] {
    if (!rows.length) return [];
    if (rows.length === 1) return [{ item: rows[0], x, y, width, height }];
    const total = rows.reduce((sum, row) => sum + weight(row), 0);
    let split = 1,
      sum = weight(rows[0]);
    while (
      split < rows.length - 1 &&
      Math.abs(sum + weight(rows[split]) - total / 2) <
        Math.abs(sum - total / 2)
    )
      sum += weight(rows[split++]);
    const share = sum / total;
    return width >= height
      ? [
          ...partition(rows.slice(0, split), x, y, width * share, height),
          ...partition(
            rows.slice(split),
            x + width * share,
            y,
            width * (1 - share),
            height,
          ),
        ]
      : [
          ...partition(rows.slice(0, split), x, y, width, height * share),
          ...partition(
            rows.slice(split),
            x,
            y + height * share,
            width,
            height * (1 - share),
          ),
        ];
  }
  return partition(rows, 0, 0, 1, 1);
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
  const total = useMemo(
    () =>
      tiles.reduce(
        (sum, t) => sum + (byTime ? t.item.listened_ms : t.item.plays),
        0,
      ),
    [tiles, byTime],
  );
  const height = Math.max(1 / 25, ...tiles.map((tile) => tile.y + tile.height));
  const title = `${kind === "albums" ? "Album" : "Artist"} collage`;
  async function download() {
    setBusy(true);
    setError("");
    try {
      const credits = await Promise.all(
        tiles.map((tile) =>
          tile.item.cover_url?.startsWith("/api/v1/artwork/")
            ? api.artworkCredit(tile.item.cover_url)
            : Promise.resolve(null),
        ),
      );
      const creditText = [
        ...new Set(
          credits
            .filter((credit) => credit !== null)
            .map((credit) =>
              [
                credit.provider,
                plainCredit(credit.author),
                plainCredit(credit.credit),
                plainCredit(credit.license),
                credit.license_url,
                credit.source_url,
              ]
                .filter(Boolean)
                .join(" | "),
            ),
        ),
      ];
      const canvas = document.createElement("canvas");
      canvas.width = 1200;
      canvas.height = Math.round(height * 1200) + 120;
      const ctx = canvas.getContext("2d");
      if (!ctx) throw new Error("This browser cannot export a collage.");
      // Wrap attribution without dropping authors or licence/source links from the PNG.
      ctx.font = "16px sans-serif";
      const creditLines = creditText.flatMap((text) => {
        const lines: string[] = [];
        let line = "";
        for (const word of text.split(/\s+/)) {
          const candidate = line ? `${line} ${word}` : word;
          if (ctx.measureText(candidate).width <= 1152) {
            line = candidate;
            continue;
          }
          if (line) {
            lines.push(line);
            line = "";
          }
          // Keep ordinary source/licence URLs intact. Very long single tokens
          // still wrap without dropping any attribution characters.
          for (const character of word) {
            if (ctx.measureText(line + character).width > 1152) {
              lines.push(line);
              line = "";
            }
            line += character;
          }
        }
        if (line) lines.push(line);
        return [...lines, ""];
      });
      const originalHeight = canvas.height;
      canvas.height += creditLines.length * 22;
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
          const scale = Math.min(
            w / img.naturalWidth,
            Math.max(1, h - 44) / img.naturalHeight,
          );
          ctx.drawImage(
            img,
            x + (w - img.naturalWidth * scale) / 2,
            y + (Math.max(1, h - 44) - img.naturalHeight * scale) / 2,
            img.naturalWidth * scale,
            img.naturalHeight * scale,
          );
        }
        if (w >= 75 && h >= 48) {
          ctx.fillStyle = "#000b";
          ctx.fillRect(x, y + h - 44, w, 44);
          ctx.fillStyle = "#fff";
          ctx.font = "600 17px sans-serif";
          const chars = Array.from(tile.item.name);
          let end = chars.length;
          while (
            end > 1 &&
            ctx.measureText(chars.slice(0, end).join("")).width > w - 16
          )
            end--;
          const label = chars.slice(0, end).join("");
          ctx.fillText(
            end === chars.length ? label : `${label}…`,
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
      ctx.fillStyle = "#bec5cf";
      ctx.font = "16px sans-serif";
      creditLines.forEach((line, index) =>
        ctx.fillText(line, 24, originalHeight - 10 + index * 22),
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
          <Select
            aria-label={`Number of ${kind} in collage`}
            value={count}
            onChange={(value) => setCount(Number(value))}
          >
            {[9, 16, 25].map((n) => (
              <option key={n}>{n}</option>
            ))}
          </Select>
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
        Tile area is proportional to{" "}
        {params.get("sort") === "time" ? "listening time" : "listens"}. The
        download includes the artwork currently displayed. Rectangles make
        differences in listening share visible.
      </p>
      <ErrorText>{error}</ErrorText>
      <div className="collage-body">
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
        <ol className="collage-legend" aria-label="Collage listening shares">
          {tiles.map(({ item }, i) => {
            const value = byTime ? item.listened_ms : item.plays;
            return (
              <li key={item.id}>
                <Link to={musicLink(kind, item.id, params)}>
                  <span>
                    {i + 1}. {item.name}
                  </span>
                  <strong>
                    {byTime
                      ? duration(value)
                      : `${number.format(value)} listens`}{" "}
                    · {total ? ((value / total) * 100).toFixed(1) : "0"}%
                  </strong>
                </Link>
                <progress
                  value={value}
                  max={byTime ? tiles[0].item.listened_ms : tiles[0].item.plays}
                  aria-label={`${item.name} relative listening`}
                />
              </li>
            );
          })}
        </ol>
      </div>
    </section>
  );
}
