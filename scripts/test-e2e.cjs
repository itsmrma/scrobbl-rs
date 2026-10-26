const { spawnSync } = require("node:child_process");

const args = process.argv.slice(2);
const hasSpecificTarget = args.some(
  (arg) =>
    arg.endsWith(".cjs") ||
    arg.endsWith(".spec.cjs") ||
    arg.includes(".spec.cjs:"),
);

if (hasSpecificTarget) {
  const res = spawnSync("npx", ["playwright", "test", ...args], {
    stdio: "inherit",
    shell: true,
  });
  process.exit(res.status ?? 0);
}

const serialFiles = [
  "tests/local.spec.cjs",
  "tests/settings.spec.cjs",
  "tests/spotify-connect.spec.cjs",
  "tests/spotify-import.spec.cjs",
  "tests/spotify-settings.spec.cjs",
];

const isolatedFiles = [
  "tests/skeleton.spec.cjs",
  "tests/api-keys.spec.cjs",
  "tests/artwork-providers.spec.cjs",
  "tests/consultation.spec.cjs",
  "tests/gui-audit.spec.cjs",
  "tests/recovery-confirm.spec.cjs",
  "tests/spotify-import-total.spec.cjs",
  "tests/z-listenbrainz.spec.cjs",
];

console.log("==> Running serial journey suite (workers: 1)...");
const res1 = spawnSync(
  "npx",
  ["playwright", "test", ...serialFiles, "--workers=1", ...args],
  { stdio: "inherit", shell: true },
);

if (res1.status !== 0) {
  process.exit(res1.status ?? 1);
}

const workers = process.env.CI ? "4" : undefined;
const workerArgs = workers ? [`--workers=${workers}`] : [];

console.log(
  `==> Running isolated suites in parallel (workers: ${workers || "default"})...`,
);
const res2 = spawnSync(
  "npx",
  ["playwright", "test", ...isolatedFiles, ...workerArgs, ...args],
  { stdio: "inherit", shell: true },
);

process.exit(res2.status ?? 0);
