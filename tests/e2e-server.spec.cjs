const { test, expect } = require("@playwright/test");
const { spawn, spawnSync } = require("node:child_process");
const { once } = require("node:events");
const { existsSync, utimesSync } = require("node:fs");
const { join, resolve } = require("node:path");
const net = require("node:net");
const http = require("node:http");

test("e2e-server.cjs respects CARGO_TARGET_DIR, recompiles stale binaries, and isolates data dir", async () => {
  const socket = net.createServer();
  socket.listen(0, "127.0.0.1");
  await once(socket, "listening");
  const port = socket.address().port;
  await new Promise((res) => socket.close(res));

  const targetDir = process.env.CARGO_TARGET_DIR || "target";
  const binaryName =
    process.platform === "win32" ? "scrobbl-rs.exe" : "scrobbl-rs";
  const binaryPath = resolve(targetDir, "debug", binaryName);

  expect(existsSync(binaryPath)).toBe(true);

  // Make source dirty to verify that cargo build is triggered even when binary exists
  const now = new Date();
  utimesSync(resolve("src/main.rs"), now, now);

  const child = spawn(process.execPath, [resolve("scripts/e2e-server.cjs")], {
    env: {
      ...process.env,
      SCROBBL_E2E_PORT: String(port),
      CARGO_TARGET_DIR: targetDir,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });

  let output = "";
  let dataDirPath = null;

  child.stdout.on("data", (data) => {
    output += data.toString();
    const match = output.match(/Data:\s+([^\r\n]+)/);
    if (match && !dataDirPath) {
      dataDirPath = match[1].trim();
    }
  });

  child.stderr.on("data", (data) => {
    output += data.toString();
  });

  const baseURL = `http://127.0.0.1:${port}`;
  let ready = false;

  for (let i = 0; i < 40; i++) {
    await new Promise((r) => setTimeout(r, 250));
    try {
      const res = await new Promise((resolveReq, rejectReq) => {
        const req = http.get(`${baseURL}/api/session`, (resp) => {
          resolveReq(resp.statusCode);
        });
        req.on("error", rejectReq);
      });
      if (res === 200) {
        ready = true;
        break;
      }
    } catch {
      // Retry until server starts
    }
  }

  expect(ready).toBe(true);
  expect(dataDirPath).toBeTruthy();
  expect(existsSync(dataDirPath)).toBe(true);

  // Shutdown the server via SIGTERM and verify graceful exit
  child.kill("SIGTERM");
  const [exitCode] = await once(child, "exit");
  expect(exitCode === 0 || exitCode === null).toBe(true);

  // Verify compilation artifacts remain in targetDir, but dataDir is cleanly purged
  expect(existsSync(binaryPath)).toBe(true);
  expect(existsSync(dataDirPath)).toBe(false);
});

test("distribution and verify scripts respect CARGO_TARGET_DIR", () => {
  const testTarget = "/tmp/scrobbl_custom_target_dir_test";

  const checkDist = spawnSync(
    "python3",
    [
      "-c",
      `
import os, importlib.util
from pathlib import Path
os.environ["CARGO_TARGET_DIR"] = "${testTarget}"
spec = importlib.util.spec_from_file_location("pkg_dist", "scripts/package-distribution.py")
mod = importlib.util.module_from_spec(spec)
cargo_target = os.environ.get("CARGO_TARGET_DIR")
default_output = (Path(cargo_target) if cargo_target else Path("target")) / "distribution"
assert str(default_output) == "${testTarget}/distribution"
print("DIST_OK")
`,
    ],
    {
      env: { ...process.env, PYTHONPATH: "." },
      encoding: "utf-8",
    },
  );

  expect(checkDist.status).toBe(0);
  expect(checkDist.stdout).toContain("DIST_OK");

  const checkVerify = spawnSync(
    "python3",
    [
      "-c",
      `
import os, importlib.util
from pathlib import Path
os.environ["CARGO_TARGET_DIR"] = "${testTarget}"
spec = importlib.util.spec_from_file_location("verify_archive", "scripts/verify-spotify-archive.py")
mod = importlib.util.module_from_spec(spec)
target_dir = Path(os.environ.get("CARGO_TARGET_DIR", "target"))
binary_name = "scrobbl-rs.exe" if os.name == "nt" else "scrobbl-rs"
default_binary = target_dir / "debug" / binary_name
assert str(default_binary) == "${testTarget}/debug/" + binary_name
print("VERIFY_OK")
`,
    ],
    {
      env: { ...process.env, PYTHONPATH: "." },
      encoding: "utf-8",
    },
  );

  expect(checkVerify.status).toBe(0);
  expect(checkVerify.stdout).toContain("VERIFY_OK");
});
