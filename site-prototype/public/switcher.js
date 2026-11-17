// PROTOTYPE switcher: mounts one variant fragment and shows a floating bar.
// Switching reloads the page so each variant starts clean (timers, styles).
(() => {
  const VARIANTS = [
    { key: "A", name: "Ledger", file: "a-ledger.html" },
    { key: "B", name: "Liner notes", file: "b-liner.html" },
    { key: "C", name: "Console", file: "c-console.html" },
    { key: "D", name: "Ask", file: "d-ask.html" },
  ];
  const params = new URLSearchParams(location.search);
  const requested = (params.get("variant") || "A").toUpperCase();
  const index = Math.max(0, VARIANTS.findIndex((v) => v.key === requested));
  const current = VARIANTS[index];

  // The bar is a prototype tool: never show it on a real production host.
  const host = location.hostname;
  const showBar = host === "localhost" || host === "127.0.0.1" || host.endsWith(".workers.dev") || params.has("variant");

  async function mount() {
    const root = document.getElementById("root");
    const html = await fetch(`/variants/${current.file.replace(/\.html$/, "")}`, { cache: "no-store" }).then((r) => r.text());
    root.innerHTML = html;
    // innerHTML does not execute scripts: re-create them in order.
    for (const old of root.querySelectorAll("script")) {
      const s = document.createElement("script");
      s.textContent = old.textContent;
      old.replaceWith(s);
    }
    document.title = `scrobbl.rs — ${current.name} (prototype ${current.key})`;
    if (showBar) renderBar();
  }

  function go(delta) {
    const next = VARIANTS[(index + delta + VARIANTS.length) % VARIANTS.length];
    params.set("variant", next.key);
    location.search = params.toString();
  }

  function renderBar() {
    const bar = document.createElement("div");
    bar.setAttribute("role", "toolbar");
    bar.setAttribute("aria-label", "Prototype variant switcher");
    bar.innerHTML = `
      <style>
        .proto-bar{position:fixed;left:50%;bottom:16px;transform:translateX(-50%);z-index:2147483647;
          display:flex;align-items:center;gap:4px;padding:4px;border-radius:999px;
          background:#fff;color:#111;border:2px dashed #111;box-shadow:0 8px 28px rgba(0,0,0,.35);
          font:600 13px/1 ui-monospace,SFMono-Regular,Menlo,monospace;white-space:nowrap}
        .proto-bar button{all:unset;cursor:pointer;width:40px;height:40px;display:grid;place-items:center;border-radius:999px;font-size:18px}
        .proto-bar button:hover{background:#111;color:#fff}
        .proto-bar button:focus-visible{outline:3px solid #ff3d7f;outline-offset:2px}
        .proto-bar span{padding:0 10px;min-width:150px;text-align:center}
        .proto-bar small{display:block;font-size:9px;letter-spacing:.12em;opacity:.6;margin-bottom:3px}
      </style>
      <div class="proto-bar">
        <button type="button" aria-label="Previous variant" data-d="-1">←</button>
        <span><small>PROTOTYPE ${index + 1}/${VARIANTS.length}</small>${current.key} (${current.name})</span>
        <button type="button" aria-label="Next variant" data-d="1">→</button>
      </div>`;
    bar.addEventListener("click", (e) => {
      const b = e.target.closest("button[data-d]");
      if (b) go(Number(b.dataset.d));
    });
    document.body.appendChild(bar);
  }

  document.addEventListener("keydown", (e) => {
    if (!showBar || e.metaKey || e.ctrlKey || e.altKey) return;
    const t = e.target;
    if (t.closest && t.closest("input, textarea, select, [contenteditable]")) return;
    if (e.key === "ArrowLeft") go(-1);
    if (e.key === "ArrowRight") go(1);
  });

  mount();
})();
