const { spawn, spawnSync } = require("node:child_process");
const { mkdtempSync, rmSync, existsSync } = require("node:fs");
const { tmpdir, homedir } = require("node:os");
const { join } = require("node:path");

function getCargoCmd() {
  if (process.env.CARGO) return process.env.CARGO;
  const cargoHomeBin = join(
    homedir(),
    ".cargo",
    "bin",
    process.platform === "win32" ? "cargo.exe" : "cargo",
  );
  if (existsSync(cargoHomeBin)) return cargoHomeBin;
  return "cargo";
}

const cargo = getCargoCmd();
const build = spawnSync(cargo, ["build", "--locked", "--bin", "scrobbl-rs"], {
  stdio: "inherit",
  env: process.env,
});
if (build.error) {
  console.error("Failed to execute cargo:", build.error);
  process.exit(1);
}
if (build.status !== 0) {
  process.exit(build.status ?? 1);
}

const targetDir = process.env.CARGO_TARGET_DIR || "target";
const binary = join(
  targetDir,
  "debug",
  process.platform === "win32" ? "scrobbl-rs.exe" : "scrobbl-rs",
);

if (!existsSync(binary)) {
  console.error(`Binary not found at expected location: ${binary}`);
  process.exit(1);
}

const dataDir = mkdtempSync(join(tmpdir(), "scrobbl-e2e-"));
const child = spawn(binary, [], {
  stdio: "inherit",
  env: {
    ...process.env,
    SCROBBL_PORT: process.env.SCROBBL_E2E_PORT || "3211",
    SCROBBL_DATA_DIR: dataDir,
    SPOTIFY_CLIENT_ID: "",
    SPOTIFY_CLIENT_SECRET: "",
  },
});
for (const signal of ["SIGTERM", "SIGINT"]) {
  process.on(signal, () => {
    if (!child.killed) {
      child.kill(signal);
    }
  });
}
child.on("exit", (code) => {
  rmSync(dataDir, { recursive: true, force: true });
  process.exit(code || 0);
});
