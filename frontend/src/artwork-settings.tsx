import { useState } from "react";
import type { ArtworkSettings, ImageProvider } from "./generated";
import { api } from "./api";
import { useLoad } from "./load";
import { ErrorText, Icon, useToast } from "./ui";
import { SkeletonCardList } from "./skeleton";

const providers: Record<
  ImageProvider,
  {
    name: string;
    coverage: string;
    access: string;
    limits: string;
    rights: string;
    sources: [string, string][];
  }
> = {
  cover_art_archive: {
    name: "MusicBrainz → Cover Art Archive",
    coverage:
      "Album covers from an unambiguous MusicBrainz release group. No artist portraits; some releases have no cover.",
    access: "No account or API key required.",
    limits:
      "MusicBrainz requests are paced to at most one per second. Archive availability varies; request limits and temporary failures pause retrieval.",
    rights:
      "Artwork can be copyrighted. The MusicBrainz metadata licence does not grant image reuse rights. The source page is kept with each image.",
    sources: [
      ["MusicBrainz API", "https://musicbrainz.org/doc/MusicBrainz_API"],
      [
        "Cover Art Archive API",
        "https://musicbrainz.org/doc/Cover_Art_Archive/API",
      ],
      [
        "Copyright",
        "https://musicbrainz.org/doc/Copyright_and_DMCA_Compliance",
      ],
    ],
  },
  musicbrainz_image: {
    name: "MusicBrainz → Commons image",
    coverage:
      "Artist portraits explicitly linked from MusicBrainz to a Commons file. No album covers; coverage is incomplete. Multiple candidates are left unresolved.",
    access: "No account or API key required.",
    limits:
      "Uses the shared MusicBrainz pace, then serial Commons requests with an identifying client name. Provider Retry-After is respected.",
    rights:
      "The individual Commons file licence applies. Author, credits, licence and source page are saved and available in Image credits. Images without attribution metadata are skipped.",
    sources: [
      [
        "MusicBrainz image relationship",
        "https://musicbrainz.org/relationship/221132e9-e30e-43f2-a741-15afc4c5fa7c",
      ],
      [
        "Commons reuse",
        "https://commons.wikimedia.org/wiki/Commons:Reusing_content_outside_Wikimedia",
      ],
      [
        "Wikimedia request limits",
        "https://www.mediawiki.org/wiki/Wikimedia_APIs/Rate_limits",
      ],
    ],
  },
  wikidata: {
    name: "MusicBrainz → Wikidata → Commons",
    coverage:
      "Artist portraits from P18 on the Wikidata entity linked by MusicBrainz. No album covers. Missing or multiple images remain unresolved; coverage is not guaranteed.",
    access: "No account or API key required.",
    limits:
      "Shares the MusicBrainz pace and makes serial Wikidata/Commons requests. Temporary failures are retried rather than recorded as missing images.",
    rights:
      "Commons file terms apply, separately from Wikidata metadata. Author, credits, licence and source page are retained with the image.",
    sources: [
      ["Wikidata image property", "https://www.wikidata.org/wiki/Property:P18"],
      ["Wikidata access", "https://www.wikidata.org/wiki/Wikidata:Data_access"],
      [
        "Commons reuse",
        "https://commons.wikimedia.org/wiki/Commons:Reusing_content_outside_Wikimedia",
      ],
    ],
  },
};

function ProviderForm({
  initial,
  reload,
}: {
  initial: ArtworkSettings;
  reload: () => void;
}) {
  const [draft, setDraft] = useState(initial);
  const [baseline, setBaseline] = useState(initial);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const toast = useToast();
  const dirty = JSON.stringify(draft) !== JSON.stringify(baseline);
  function move(index: number, direction: number) {
    setDraft((current) => {
      const next = [...current.providers];
      [next[index], next[index + direction]] = [
        next[index + direction],
        next[index],
      ];
      return { ...current, providers: next };
    });
  }
  return (
    <form
      className="panel padded artwork-provider-form"
      onSubmit={async (event) => {
        event.preventDefault();
        setBusy(true);
        setError("");
        try {
          const result = await api.saveArtworkSettings(draft);
          setDraft(result);
          setBaseline(result);
          window.dispatchEvent(new Event("artwork-cache-changed"));
          toast("Image providers saved.");
        } catch (e) {
          setError(e instanceof Error ? e.message : String(e));
        } finally {
          setBusy(false);
        }
      }}
    >
      <h2>Image providers</h2>
      <p>
        Choose sources for missing album and artist images, in the order shown.
        These settings belong to this instance and apply across browsers. Image
        retrieval uses public sources and does not call the Spotify API.
      </p>
      <p>
        Saved images stay in your library. Personal images always take priority.
        Changing sources does not replace or delete them. Ambiguous identities
        are left without an automatic image.
      </p>
      <ol className="artwork-provider-list">
        {draft.providers.map((item, index) => {
          const info = providers[item.provider];
          return (
            <li key={item.provider} className="artwork-provider">
              <div className="artwork-provider-header">
                <label className="checkbox-label">
                  <input
                    type="checkbox"
                    checked={item.enabled}
                    disabled={busy}
                    onChange={(event) =>
                      setDraft((current) => ({
                        ...current,
                        providers: current.providers.map((p) =>
                          p.provider === item.provider
                            ? { ...p, enabled: event.target.checked }
                            : p,
                        ),
                      }))
                    }
                  />
                  {info.name}
                </label>
                <div className="artwork-provider-order">
                  <button
                    type="button"
                    className="button secondary"
                    aria-label={`Move ${info.name} up`}
                    disabled={busy || index === 0}
                    onClick={() => move(index, -1)}
                  >
                    <Icon>arrow_upward</Icon>
                  </button>
                  <button
                    type="button"
                    className="button secondary"
                    aria-label={`Move ${info.name} down`}
                    disabled={busy || index >= draft.providers.length - 1}
                    onClick={() => move(index, 1)}
                  >
                    <Icon>arrow_downward</Icon>
                  </button>
                </div>
              </div>
              <p>{info.coverage}</p>
              <details>
                <summary>Access, limits &amp; image rights</summary>
                <dl>
                  <dt>Access</dt>
                  <dd>{info.access}</dd>
                  <dt>Limits &amp; retention</dt>
                  <dd>{info.limits}</dd>
                  <dt>Reuse &amp; attribution</dt>
                  <dd>{info.rights}</dd>
                </dl>
                <div className="artwork-provider-links">
                  {info.sources.map(([title, href]) => (
                    <a key={href} href={href} target="_blank" rel="noreferrer">
                      {title}
                      <Icon>open_in_new</Icon>
                    </a>
                  ))}
                </div>
              </details>
            </li>
          );
        })}
      </ol>
      <p className="subtle">
        Last.fm is unavailable as an image provider: its API data licence
        excludes artwork.{" "}
        <a href="https://www.last.fm/api/tos" target="_blank" rel="noreferrer">
          Last.fm API terms
        </a>
      </p>
      <ErrorText>{error}</ErrorText>
      <div className="form-actions">
        <button className="button" disabled={busy || !dirty}>
          {busy ? "Saving…" : "Save image providers"}
        </button>
        <button
          type="button"
          className="button secondary"
          disabled={busy}
          onClick={reload}
        >
          Reload saved settings
        </button>
      </div>
    </form>
  );
}
export function ArtworkProviderSettings() {
  const [revision, setRevision] = useState(0);
  const settings = useLoad(
    () => api.artworkSettings(),
    `artwork-settings:${revision}`,
  );
  return (
    <>
      <ErrorText>{settings.error}</ErrorText>
      {settings.data ? (
        <ProviderForm
          key={`${settings.data.version}:${revision}`}
          initial={settings.data}
          reload={() => setRevision((v) => v + 1)}
        />
      ) : (
        !settings.error && (
          <SkeletonCardList count={3} label="Loading image providers…" />
        )
      )}
    </>
  );
}
