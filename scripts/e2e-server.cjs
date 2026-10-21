const { spawn } = require("node:child_process");
const { mkdtempSync, rmSync, existsSync } = require("node:fs");
const { tmpdir, homedir } = require("node:os");
const { join } = require("node:path");
const dataDir = mkdtempSync(join(tmpdir(), "scrobbl-e2e-"));
const cargo = existsSync(join(homedir(), ".cargo/bin/cargo"))
  ? join(homedir(), ".cargo/bin/cargo")
  : "cargo";
const child = spawn(cargo, ["run", "--locked", "--bin", "scrobbl-rs"], {
  stdio: "inherit",
  env: {
    ...process.env,
    SCROBBL_PORT: process.env.SCROBBL_E2E_PORT || "3211",
    SCROBBL_DATA_DIR: dataDir,
    SPOTIFY_CLIENT_ID: "",
    SPOTIFY_CLIENT_SECRET: "",
  },
});
for (const signal of ["SIGTERM", "SIGINT"])
  process.on(signal, () => child.kill("SIGINT"));
child.on("exit", (code) => {
  rmSync(dataDir, { recursive: true, force: true });
  process.exit(code || 0);
});
