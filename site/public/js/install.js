// Install tabs and copy buttons.
(() => {
  const tabs = Array.from(document.querySelectorAll('#install [role="tab"]'));
  if (!tabs.length) return;

  const select = (tab, focus) => {
    for (const t of tabs) {
      const on = t === tab;
      t.setAttribute("aria-selected", String(on));
      document.getElementById(t.getAttribute("aria-controls")).hidden = !on;
    }
    if (focus) tab.focus();
  };
  tabs.forEach((tab, i) => {
    tab.addEventListener("click", () => select(tab, false));
    tab.addEventListener("keydown", (e) => {
      const to = { ArrowRight: i + 1, ArrowLeft: i - 1, Home: 0, End: tabs.length - 1 }[e.key];
      if (to === undefined) return;
      e.preventDefault();
      select(tabs[(to + tabs.length) % tabs.length], true);
    });
  });

  // Only command lines are copied: comment lines carry the "note" class.
  const commandsOf = (block) =>
    Array.from(block.querySelectorAll(".line:not(.note)"), (l) => l.textContent).join("\n");

  const fallbackCopy = (text) => {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.setAttribute("readonly", "");
    ta.className = "copy-buffer";
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand("copy");
    ta.remove();
    return ok;
  };

  for (const btn of document.querySelectorAll("#install [data-copy]")) {
    let timer;
    btn.addEventListener("click", async () => {
      const text = commandsOf(btn.closest("[data-code]"));
      let ok = false;
      try {
        await navigator.clipboard.writeText(text);
        ok = true;
      } catch {
        ok = fallbackCopy(text);
      }
      if (!ok) return;
      btn.textContent = "Copied";
      clearTimeout(timer);
      timer = setTimeout(() => { btn.textContent = "Copy"; }, 1500);
    });
  }
})();
