import type { ImageCredit } from "./generated";
import { useEffect, useId, useRef, useState } from "react";
import { api } from "./api";
import { useLoad } from "./load";

// Commons attribution metadata contains untrusted HTML. Display its text, never its markup.
export function plainCredit(value: string | null) {
  if (!value) return "";
  const template = document.createElement("template");
  template.innerHTML = value;
  return template.content.textContent || "";
}
export function ImageCredits({ credit }: { credit?: ImageCredit | null }) {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLSpanElement>(null);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (!open) return;
    const close = (event: Event) => {
      if (event.target instanceof Node && panel.current?.contains(event.target))
        return;
      panel.current?.hidePopover();
    };
    window.addEventListener("resize", close);
    window.addEventListener("scroll", close, true);
    return () => {
      window.removeEventListener("resize", close);
      window.removeEventListener("scroll", close, true);
    };
  }, [open]);
  if (!credit) return null;
  const close = () => {
    panel.current?.hidePopover();
    trigger.current?.focus();
  };
  return (
    <>
      <button
        type="button"
        className="image-credit-trigger icon-button"
        aria-label="Image credits"
        aria-expanded={open}
        popoverTarget={id}
        ref={trigger}
        onClick={(event) => {
          event.currentTarget.focus();
          const rect = event.currentTarget.getBoundingClientRect();
          const width = Math.min(352, window.innerWidth - 28);
          const node = panel.current;
          if (node) {
            node.style.left = `${Math.max(14, Math.min(rect.left, window.innerWidth - width - 14))}px`;
            const top = Math.min(rect.bottom + 8, window.innerHeight - 220);
            node.style.top = `${Math.max(14, top)}px`;
            node.style.maxHeight = `${Math.max(120, window.innerHeight - Math.max(14, top) - 14)}px`;
          }
        }}
      >
        <span className="material-icons-outlined" aria-hidden="true">
          info
        </span>
      </button>
      <span
        popover="auto"
        id={id}
        role="region"
        aria-label="Image credits"
        className="image-credits"
        ref={panel}
        onToggle={(event) => setOpen(event.newState === "open")}
      >
        <span className="image-credit-heading">
          <strong>Image credits</strong>
          <button
            type="button"
            className="icon-button"
            aria-label="Close image credits"
            onClick={close}
          >
            <span className="material-icons-outlined" aria-hidden="true">
              close
            </span>
          </button>
        </span>
        <span className="image-credit-provider">
          {{
            cover_art_archive: "Cover Art Archive",
            musicbrainz_image: "MusicBrainz · Wikimedia Commons",
            wikidata: "Wikidata · Wikimedia Commons",
          }[credit.provider] || credit.provider}
        </span>
        {credit.author && (
          <span className="image-credit-copy">
            {plainCredit(credit.author)}
          </span>
        )}
        {credit.credit && (
          <span className="image-credit-copy">
            {plainCredit(credit.credit)}
          </span>
        )}
        <span className="image-credit-links">
          <a href={credit.source_url} target="_blank" rel="noreferrer">
            View source
          </a>
          {credit.license &&
            (credit.license_url ? (
              <a href={credit.license_url} target="_blank" rel="noreferrer">
                {plainCredit(credit.license)}
              </a>
            ) : (
              <span>{plainCredit(credit.license)}</span>
            ))}
        </span>
      </span>
    </>
  );
}

// Read the provenance of the displayed image: a track uses its album's URL.
// Refresh when a cold download finishes or artwork is replaced elsewhere.
export function ArtworkCredits({ url }: { url: string | null }) {
  const [version, setVersion] = useState(0);
  useEffect(() => {
    const update = () => setVersion((value) => value + 1);
    const downloaded = (event: Event) => {
      if (
        (event as CustomEvent<{ url: string }>).detail.url.split("?")[0] ===
        url?.split("?")[0]
      )
        update();
    };
    window.addEventListener("artwork-downloaded", downloaded);
    window.addEventListener("artwork-updated", update);
    window.addEventListener("artwork-cache-changed", update);
    return () => {
      window.removeEventListener("artwork-downloaded", downloaded);
      window.removeEventListener("artwork-updated", update);
      window.removeEventListener("artwork-cache-changed", update);
    };
  }, [url]);
  const result = useLoad(
    () => api.artworkCredit(url!),
    `${url}:${version}`,
    !!url?.startsWith("/api/v1/artwork/"),
  );
  return <ImageCredits credit={result.data} />;
}
