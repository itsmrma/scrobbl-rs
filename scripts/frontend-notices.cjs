// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 itsmrma and scrobbl.rs contributors
const { readFileSync, writeFileSync } = require("node:fs");
const { join } = require("node:path");
const root = join(__dirname, "..");
const lock = JSON.parse(readFileSync(join(root, "package-lock.json"), "utf8"));
const sections = [
  "scrobbl.rs — GPL-3.0-or-later\nCopyright (c) 2026 itsmrma and contributors.\n" +
    "GNU GPL version 3 or any later version; without warranty. Full license: /license\n" +
    "Matching source is included in the Docker image and its GitHub Actions distribution artifact.\n" +
    "Source and build instructions: https://github.com/itsmrma/scrobbl-rs\n\n" +
    "The browser bundle also contains Vite's MIT modulepreload helper.\n",
];
const packages = Object.entries(lock.packages).filter(
  ([path, pkg]) => path && !pkg.dev,
);
// Vite injects its modulepreload helper into the distributed frontend.
packages.push(["node_modules/vite", lock.packages["node_modules/vite"]]);
for (const [path, pkg] of packages) {
  const folder = join(root, path);
  const meta = JSON.parse(readFileSync(join(folder, "package.json"), "utf8"));
  if (meta.version !== pkg.version)
    throw new Error(`Unexpected ${path} version`);
  const license = readFileSync(
    join(folder, meta.name === "vite" ? "LICENSE.md" : "LICENSE"),
    "utf8",
  );
  sections.push(`${meta.name} ${meta.version} — ${meta.license}\n\n${license}`);
}
writeFileSync(
  join(root, "static/dist/third-party-notices.txt"),
  sections
    .join("\n\n--------------------\n\n")
    .split("\n")
    .map((line) => line.trimEnd())
    .join("\n"),
);
