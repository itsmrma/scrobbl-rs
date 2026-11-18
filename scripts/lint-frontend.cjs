const fs = require("node:fs");
const path = require("node:path");

const ROOT = path.resolve(__dirname, "..", "frontend", "src");

function listTsFiles(dir) {
  const results = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      results.push(...listTsFiles(full));
    } else if (entry.isFile() && /\.(ts|tsx)$/.test(entry.name)) {
      results.push(full);
    }
  }
  return results;
}

/**
 * Replace comment characters with spaces while preserving newlines so reported
 * line numbers remain exact without flagging explanatory prose in comments.
 */
function stripCommentsPreservingLines(source) {
  let out = "";
  let i = 0;
  while (i < source.length) {
    const ch = source[i];
    const next = source[i + 1];
    if (ch === "/" && next === "/") {
      out += "  ";
      i += 2;
      while (i < source.length && source[i] !== "\n") {
        out += " ";
        i += 1;
      }
    } else if (ch === "/" && next === "*") {
      out += "  ";
      i += 2;
      while (
        i < source.length &&
        !(source[i] === "*" && source[i + 1] === "/")
      ) {
        out += source[i] === "\n" ? "\n" : " ";
        i += 1;
      }
      if (i < source.length) {
        out += "  ";
        i += 2;
      }
    } else if (ch === '"' || ch === "'" || ch === "`") {
      // Keep string contents intact for JSX / attribute checks while avoiding
      // naive splitting across escaped quotes.
      const quote = ch;
      out += ch;
      i += 1;
      while (i < source.length && source[i] !== quote) {
        if (source[i] === "\\" && i + 1 < source.length) {
          out += source[i] + source[i + 1];
          i += 2;
        } else {
          out += source[i];
          i += 1;
        }
      }
      if (i < source.length) {
        out += source[i];
        i += 1;
      }
    } else {
      out += ch;
      i += 1;
    }
  }
  return out;
}

const violations = [];
for (const file of listTsFiles(ROOT)) {
  const rel = path
    .relative(path.resolve(__dirname, ".."), file)
    .replace(/\\/g, "/");
  const raw = fs.readFileSync(file, "utf8");
  const sanitized = stripCommentsPreservingLines(raw);
  const lines = sanitized.split(/\r?\n/);

  lines.forEach((line, idx) => {
    const lineNo = idx + 1;

    if (rel !== "frontend/src/api.ts" && /\bfetch\s*\(/.test(line)) {
      violations.push(
        `${rel}:${lineNo}: [no-raw-fetch] Use centralized helpers in frontend/src/api.ts instead of calling fetch() directly. Raw fetch() bypasses CSRF tokens, session-expiry handling, and navigation abort cleanup.`,
      );
    }

    if (
      /\.(innerHTML|outerHTML)\b/.test(line) ||
      /\binsertAdjacentHTML\s*\(/.test(line) ||
      /\bdangerouslySetInnerHTML\b/.test(line) ||
      /\beval\s*\(/.test(line)
    ) {
      violations.push(
        `${rel}:${lineNo}: [no-unsafe-dom-sink] Never use .innerHTML, .outerHTML, insertAdjacentHTML, dangerouslySetInnerHTML, or eval(). Render JSX text nodes or strip untrusted markup with pure string helpers (see plainCredit in image-credit.tsx).`,
      );
    }

    const mapMatch = line.match(/\b([A-Z][A-Z0-9_]{2,})\s*\[/);
    if (mapMatch) {
      const name = mapMatch[1];
      violations.push(
        `${rel}:${lineNo}: [no-unchecked-uppercase-map-index] Avoid direct bracket indexing ${name}[key] on constant lookup maps (which can return Object.prototype properties). Use lookupRecord(${name}, key) from "./ui", or .at(index) for arrays.`,
      );
    }
  });
}

if (violations.length > 0) {
  console.error("Frontend architectural lint failed:\n");
  for (const v of violations) {
    console.error(`  - ${v}`);
  }
  process.exit(1);
}

console.log(
  `Frontend lint passed (${listTsFiles(ROOT).length} TypeScript files checked).`,
);
