import { useState } from "react";
import type { ArtworkSettings, ImageProvider } from "./generated";
import { api } from "./api";
import { triggerArtworkBackfill, useArtworkActivity } from "./artwork-activity";
import { useLoad } from "./load";
import { Link } from "./navigation";
import { ErrorText, Icon, Modal, number, useToast } from "./ui";
import { SkeletonCardList } from "./skeleton";

const providers: Record<
  ImageProvider,
  {
    name: string;
    summary: string;
    coverage: string;
    access: string;
    limits: string;
    rights: string;
    sources: [string, string][];
  }
> = {
  deezer: {
    name: "Deezer",
    summary: "Album covers and artist photos",
    coverage:
      "Album covers up to 1000 × 1000 and artist photos from Deezer's public catalogue. Fast and broad, including Japanese and Korean releases. Homonym artists resolve to the most followed one.",
    access: "No account or API key required.",
    limits:
      "Deezer allows 50 requests every 5 seconds; scrobbl.rs stays below that and pauses briefly when Deezer reports its quota.",
    rights:
      "Artwork may be copyrighted by its rights holders. The Deezer page is saved with each image.",
    sources: [["Deezer API", "https://developers.deezer.com/api"]],
  },
  itunes: {
    name: "Apple iTunes",
    summary: "Album covers",
    coverage:
      "Album covers up to 1000 × 1000 from the iTunes Search API, used when Deezer has no match.",
    access: "No account or API key required.",
    limits:
      "Apple asks for about 20 searches per minute, so requests are spaced three seconds apart.",
    rights:
      "Artwork may be copyrighted by its rights holders. The Apple Music page is saved with each image.",
    sources: [
      [
        "iTunes Search API",
        "https://developer.apple.com/library/archive/documentation/AudioVideo/Conceptual/iTuneSearchAPI/",
      ],
    ],
  },
  cover_art_archive: {
    name: "Cover Art Archive",
    summary: "Album covers via MusicBrainz",
    coverage:
      "Album covers from a confident MusicBrainz release-group match. Strong on original-script titles such as 狂言 that other stores list in romaji.",
    access: "No account or API key required.",
    limits:
      "MusicBrainz allows one request per second; scrobbl.rs keeps that pace and waits when MusicBrainz is busy.",
    rights:
      "Artwork can be copyrighted. The MusicBrainz metadata licence does not grant image reuse rights. The release-group page is saved with each image.",
    sources: [
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
  wikimedia: {
    name: "Wikimedia Commons",
    summary: "Attributed artist photos via MusicBrainz",
    coverage:
      "Artist photos linked from MusicBrainz to Commons, directly or through Wikidata. Only files with an author and a licence are used.",
    access: "No account or API key required.",
    limits:
      "Shares the MusicBrainz pace of one request per second, then asks Wikidata and Commons.",
    rights:
      "The licence of each Commons file applies. Author, licence and source page are saved and shown in Image credits.",
    sources: [
      [
        "Commons reuse",
        "https://commons.wikimedia.org/wiki/Commons:Reusing_content_outside_Wikimedia",
      ],
      ["Wikidata image property", "https://www.wikidata.org/wiki/Property:P18"],
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
  const [inspectingProvider, setInspectingProvider] =
    useState<ImageProvider | null>(null);
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
      <div className="settings-section-intro">
        <h2>Image providers</h2>
        <p>
          Sources for missing album and artist images, asked in this order.
          Track pages use their album cover. No source calls the Spotify API.
        </p>
      </div>
      <ol className="artwork-provider-list">
        {draft.providers.map((item, index) => {
          const info = providers[item.provider];
          return (
            <li
              key={item.provider}
              className="artwork-provider compact-provider"
            >
              <div className="artwork-provider-main">
                <label className="checkbox-label">
                  <input
                    type="checkbox"
                    aria-label={info.name}
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
                  <span className="artwork-provider-copy">
                    <span className="artwork-provider-name">{info.name}</span>
                    <small>{info.summary}</small>
                  </span>
                </label>
                <div className="artwork-provider-actions">
                  <button
                    type="button"
                    className="button secondary icon-button"
                    aria-label={`Move ${info.name} up`}
                    disabled={busy || index === 0}
                    onClick={() => move(index, -1)}
                  >
                    <Icon>arrow_upward</Icon>
                  </button>
                  <button
                    type="button"
                    className="button secondary icon-button"
                    aria-label={`Move ${info.name} down`}
                    disabled={busy || index >= draft.providers.length - 1}
                    onClick={() => move(index, 1)}
                  >
                    <Icon>arrow_downward</Icon>
                  </button>
                  <button
                    type="button"
                    className="button secondary icon-button provider-info-trigger"
                    aria-label={`Details and license for ${info.name}`}
                    onClick={() => setInspectingProvider(item.provider)}
                  >
                    <Icon>info</Icon>
                    <span>Details</span>
                  </button>
                </div>
              </div>
            </li>
          );
        })}
      </ol>
      {inspectingProvider && (
        <Modal
          title={`Details & license: ${providers[inspectingProvider].name}`}
          close={() => setInspectingProvider(null)}
        >
          <div className="provider-info-modal">
            <p className="provider-coverage">
              {providers[inspectingProvider].coverage}
            </p>
            <dl className="diagnostics">
              <div>
                <dt>Access</dt>
                <dd>{providers[inspectingProvider].access}</dd>
              </div>
              <div>
                <dt>Limits &amp; retention</dt>
                <dd>{providers[inspectingProvider].limits}</dd>
              </div>
              <div>
                <dt>Reuse &amp; attribution</dt>
                <dd>{providers[inspectingProvider].rights}</dd>
              </div>
            </dl>
            <div className="artwork-provider-links">
              {providers[inspectingProvider].sources.map(([title, href]) => (
                <a key={href} href={href} target="_blank" rel="noreferrer">
                  {title}
                  <Icon>open_in_new</Icon>
                </a>
              ))}
            </div>
          </div>
        </Modal>
      )}
      <p className="subtle">
        Saved and personal images are never replaced when you change sources.
        Last.fm is not offered: its API licence excludes artwork.{" "}
        <a href="https://www.last.fm/api/tos" target="_blank" rel="noreferrer">
          Last.fm API terms
        </a>
      </p>
      <ErrorText>{error}</ErrorText>
      <div className="form-actions">
        <button
          className="btn btn-primary primary button"
          disabled={busy || !dirty}
        >
          {busy ? "Saving…" : "Save image providers"}
        </button>
        <button
          type="button"
          className="btn btn-secondary button secondary"
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
  const [refreshing, setRefreshing] = useState(false);
  const toast = useToast();
  const artworkWork = useArtworkActivity();
  const settings = useLoad(
    () => api.artworkSettings(),
    `artwork-settings:${revision}`,
  );
  return (
    <>
      <article className="panel padded artwork-sync-overview">
        <div className="panel-title">
          <h2>Library artwork</h2>
          <span className="pill">
            {artworkWork.serverSync
              ? artworkWork.serverSync.paused
                ? "Paused"
                : artworkWork.serverSync.expected
                  ? `${number.format(artworkWork.serverSync.processed)} of ${number.format(artworkWork.serverSync.expected)}`
                  : "Running"
              : "Up to date"}
          </span>
        </div>
        <p>
          Missing covers and artist photos download by themselves after imports
          and new listens. To choose an image yourself, open any artist, album
          or track page.
        </p>
        <div className="connection-actions">
          <button
            type="button"
            className="secondary"
            disabled={refreshing}
            onClick={async () => {
              setRefreshing(true);
              try {
                await triggerArtworkBackfill(toast);
              } catch (e) {
                toast(e instanceof Error ? e.message : String(e), true);
              } finally {
                setRefreshing(false);
              }
            }}
          >
            <Icon>refresh</Icon>Retry missing images
          </button>
          <Link to="/activities" className="text-link">
            Artwork activity
            <Icon>arrow_forward</Icon>
          </Link>
        </div>
      </article>
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
