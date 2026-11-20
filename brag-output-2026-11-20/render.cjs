// Renders composition/index.html frame by frame (seeking the paused GSAP
// timeline), then encodes brag.mp4 (with the music bed), brag.gif and brag.jpg
// with ffmpeg. Usage, from the repository root:
//   node brag-output-2026-11-20/render.cjs
// Needs the repository's Playwright Chromium, ffmpeg on PATH and network access
// for the GSAP CDN script.
const { chromium } = require("playwright");
const { spawnSync } = require("node:child_process");
const { mkdtempSync, rmSync } = require("node:fs");
const { tmpdir } = require("node:os");
const path = require("node:path");
const { pathToFileURL } = require("node:url");

const FPS = 30;
const DURATION = 30.57;
const here = __dirname;
const composition = path.join(here, "composition", "index.html");
const music = path.join(
  here,
  "composition",
  "assets",
  "music",
  "happy-beats-business-moves-vol-10-by-ende-dot-app.mp3",
);

function ffmpeg(args) {
  const run = spawnSync("ffmpeg", ["-hide_banner", "-loglevel", "error", "-y", ...args], {
    stdio: "inherit",
  });
  if (run.status !== 0) throw new Error(`ffmpeg failed: ${args.join(" ")}`);
}

(async () => {
  const frames = mkdtempSync(path.join(tmpdir(), "scrobbl-brag-"));
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1920, height: 1080 } });
  await page.goto(pathToFileURL(composition).href, { waitUntil: "load" });
  await page.waitForFunction(() => Boolean(window.__timelines && window.__timelines["scrobbl-brag"]), null, { polling: 100 });
  const total = Math.round(DURATION * FPS);
  for (let i = 0; i < total; i++) {
    await page.evaluate((t) => {
      window.__timelines["scrobbl-brag"].seek(t, false);
    }, i / FPS);
    await page.screenshot({
      path: path.join(frames, `f${String(i).padStart(5, "0")}.png`),
    });
    if (i % 60 === 0) console.log(`frame ${i}/${total}`);
  }
  await browser.close();

  const pattern = path.join(frames, "f%05d.png");
  const fadeOut = (DURATION - 1.37).toFixed(2);
  ffmpeg([
    "-framerate", String(FPS), "-i", pattern,
    "-i", music,
    "-filter_complex",
    `[1:a]atrim=0:${DURATION},volume=0.22,afade=t=in:st=0:d=0.8,afade=t=out:st=${fadeOut}:d=1.37[a]`,
    "-map", "0:v", "-map", "[a]",
    "-c:v", "libx264", "-preset", "slow", "-crf", "22", "-pix_fmt", "yuv420p",
    "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart", "-shortest",
    path.join(here, "brag.mp4"),
  ]);
  ffmpeg([
    "-framerate", String(FPS), "-i", pattern,
    "-vf", "fps=12,scale=960:-1:flags=lanczos,split[s0][s1];[s0]palettegen=max_colors=128:stats_mode=diff[p];[s1][p]paletteuse=dither=bayer:bayer_scale=4",
    path.join(here, "brag.gif"),
  ]);
  ffmpeg([
    "-i", path.join(frames, `f${String(Math.round(12.8 * FPS)).padStart(5, "0")}.png`),
    "-q:v", "3", path.join(here, "brag.jpg"),
  ]);
  rmSync(frames, { recursive: true, force: true });
  console.log("brag.mp4, brag.gif and brag.jpg written to", here);
})();
