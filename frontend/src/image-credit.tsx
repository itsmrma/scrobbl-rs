import type { ImageCredit } from "./generated";

// Commons attribution metadata contains untrusted HTML. Display its text, never its markup.
export function plainCredit(value: string | null) {
  if (!value) return "";
  const template = document.createElement("template");
  template.innerHTML = value;
  return template.content.textContent || "";
}
export function ImageCredits({ credit }: { credit?: ImageCredit | null }) {
  if (!credit) return null;
  return (
    <details className="image-credits">
      <summary>Image credits</summary>
      <a href={credit.source_url} target="_blank" rel="noreferrer">
        Image source
      </a>
      {credit.author && <p>{plainCredit(credit.author)}</p>}
      {credit.credit && <p>{plainCredit(credit.credit)}</p>}
      {credit.license &&
        (credit.license_url ? (
          <a href={credit.license_url} target="_blank" rel="noreferrer">
            {plainCredit(credit.license)}
          </a>
        ) : (
          <p>{plainCredit(credit.license)}</p>
        ))}
    </details>
  );
}
