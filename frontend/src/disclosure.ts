import { useEffect } from "react";

function disclosureBody(details: HTMLDetailsElement) {
  if (details.classList.contains("listen-details"))
    return details.querySelector<HTMLElement>("dl");
  if (details.classList.contains("collection-runs"))
    return details.querySelector<HTMLElement>(":scope > div");
  return details;
}

// Keep native details semantics and leave explicitly controlled disclosures alone.
export function useDisclosureMotion() {
  useEffect(() => {
    const running = new Map<
      HTMLDetailsElement,
      { animation: Animation; opening: boolean; overflow: string }
    >();
    const click = (event: MouseEvent) => {
      if (event.defaultPrevented || !(event.target instanceof Element)) return;
      const summary = event.target.closest("summary");
      const details = summary?.parentElement;
      if (!(details instanceof HTMLDetailsElement) || !summary) return;
      if (
        event.target.closest("a,button,input,select") ||
        details.classList.contains("history-filter-disclosure")
      )
        return;
      if (matchMedia("(prefers-reduced-motion: reduce)").matches) return;
      const body = disclosureBody(details);
      if (!body) return;
      event.preventDefault();
      const previous = running.get(details);
      const opening = previous ? !previous.opening : !details.open;
      const wasOpen = details.open;
      const from =
        body === details || wasOpen ? body.getBoundingClientRect().height : 0;
      previous?.animation.cancel();
      details.open = false;
      const closedHeight =
        body === details ? details.getBoundingClientRect().height : 0;
      details.open = true;
      const fullHeight = body.getBoundingClientRect().height;
      const overflow = previous?.overflow ?? body.style.overflow;
      body.style.overflow = "hidden";
      const animation = body.animate(
        [
          { height: from + "px" },
          { height: (opening ? fullHeight : closedHeight) + "px" },
        ],
        { duration: 160, easing: "ease-out" },
      );
      running.set(details, { animation, opening, overflow });
      void animation.finished
        .then(() => {
          if (running.get(details)?.animation !== animation) return;
          details.open = opening;
          body.style.overflow = overflow;
          running.delete(details);
        })
        .catch(() => {});
    };
    document.addEventListener("click", click);
    return () => {
      document.removeEventListener("click", click);
      for (const [details, { animation, opening, overflow }] of running) {
        animation.cancel();
        details.open = opening;
        const body = disclosureBody(details);
        if (body) body.style.overflow = overflow;
      }
    };
  }, []);
}
