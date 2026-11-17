// Accent picker: swatches set --accent and --accent-ink on the root element.
// Nothing is persisted: every visit starts from lime.
(() => {
  // Same rule as the app: relative luminance picks dark or white text.
  const ink = (hex) => {
    const n = parseInt(hex.slice(1), 16), r = n >> 16, g = (n >> 8) & 255, b = n & 255;
    return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255 > 0.55 ? "#192912" : "#ffffff";
  };
  const group = document.querySelector(".swatches");
  if (!group) return;
  const buttons = Array.from(group.querySelectorAll("button[data-color]"));
  group.addEventListener("click", (e) => {
    const chosen = e.target.closest("button[data-color]");
    if (!chosen) return;
    const root = document.documentElement.style;
    root.setProperty("--accent", chosen.dataset.color);
    root.setProperty("--accent-ink", ink(chosen.dataset.color));
    buttons.forEach((b) => b.setAttribute("aria-pressed", String(b === chosen)));
  });
})();
