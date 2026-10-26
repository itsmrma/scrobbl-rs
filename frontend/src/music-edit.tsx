import { NameGroup } from "./name-groups";
import { ImageCredits } from "./image-credit";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { api } from "./api";
import type { Details } from "./generated";
import { useLoad } from "./load";
import { Link, returnTo } from "./navigation";
import { Cover, ErrorText, Heading, Icon, useToast } from "./ui";
import type { Kind } from "./selection";
import { SkeletonCardList } from "./skeleton";

function Stars({
  value,
  change,
  disabled = false,
}: {
  value: number | null;
  change: (value: number | null) => void;
  disabled?: boolean;
}) {
  const [preview, setPreview] = useState<number | null>(null);
  const displayed = preview ?? value ?? 0;
  function pointedRating(star: number, clientX: number, element: HTMLElement) {
    const rect = element.getBoundingClientRect();
    return clientX - rect.left < rect.width / 2
      ? Math.max(0.5, star - 0.5)
      : star;
  }
  return (
    <fieldset className="rating-control" disabled={disabled}>
      <legend className="visually-hidden">Personal rating</legend>
      <div className="rating-options">
        <div
          className="rating-stars"
          aria-label="Star rating"
          onPointerLeave={() => setPreview(null)}
        >
          {[1, 2, 3, 4, 5].map((star) => (
            <button
              key={star}
              type="button"
              className="star-button"
              aria-label={`Rate ${star} stars; left half selects ${Math.max(0.5, star - 0.5)}`}
              aria-pressed={value !== null && value === star}
              onPointerMove={(event) => {
                if (event.pointerType === "mouse" && !disabled)
                  setPreview(
                    pointedRating(star, event.clientX, event.currentTarget),
                  );
              }}
              onFocus={() => setPreview(star)}
              onBlur={() => setPreview(null)}
              onKeyDown={(event) => {
                if (
                  [
                    "ArrowLeft",
                    "ArrowDown",
                    "ArrowRight",
                    "ArrowUp",
                    "Home",
                    "End",
                  ].includes(event.key)
                ) {
                  event.preventDefault();
                  const next =
                    event.key === "Home"
                      ? 0.5
                      : event.key === "End"
                        ? 5
                        : Math.max(
                            0.5,
                            Math.min(
                              5,
                              (value ?? 0.5) +
                                (["ArrowLeft", "ArrowDown"].includes(event.key)
                                  ? -0.5
                                  : 0.5),
                            ),
                          );
                  setPreview(null);
                  change(next);
                }
              }}
              onClick={(event) => {
                setPreview(null);
                change(
                  event.detail > 0
                    ? pointedRating(star, event.clientX, event.currentTarget)
                    : star,
                );
              }}
            >
              <Icon>
                {displayed >= star
                  ? "star"
                  : displayed >= star - 0.5
                    ? "star_half"
                    : "star_outline"}
              </Icon>
            </button>
          ))}
        </div>
        <output aria-label="Rating in stars" aria-live="polite">
          {preview ?? value ?? "Not rated"}
          {(preview ?? value) !== null && " / 5"}
        </output>
        {value !== null && (
          <button
            className="icon-button"
            type="button"
            aria-label="Clear rating"
            onClick={() => {
              setPreview(null);
              change(null);
            }}
          >
            <Icon>close</Icon>
          </button>
        )}
      </div>
    </fieldset>
  );
}

export function MusicActions({ kind, id }: { kind: Kind; id: string }) {
  const actions = useRef<HTMLDivElement>(null);
  const [version, setVersion] = useState(0);
  const details = useLoad<Details>(
    () => api.libraryDetails(kind, id),
    `${kind}:${id}:${version}`,
  );
  const [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  const toast = useToast();
  useEffect(() => {
    const update = () => setVersion((v) => v + 1);
    const loaded = (event: Event) => {
      const url = (event as CustomEvent<{ url: string }>).detail.url;
      if (url.split("?")[0] === `/api/v1/artwork/${kind}/${id}`) update();
    };
    window.addEventListener("artwork-credit-loaded", loaded);
    window.addEventListener("artwork-cache-changed", update);
    window.addEventListener("artwork-updated", update);
    return () => {
      window.removeEventListener("artwork-credit-loaded", loaded);
      window.removeEventListener("artwork-cache-changed", update);
      window.removeEventListener("artwork-updated", update);
    };
  }, []);

  useLayoutEffect(() => {
    const node = actions.current;
    const header = node?.closest<HTMLElement>(".music-header-content");
    const buttons = node?.querySelectorAll<HTMLAnchorElement>(":scope > a");
    if (!header || !buttons?.length) return;
    const measure = () => {
      const first = buttons[0].getBoundingClientRect().width;
      const second = buttons[1]?.getBoundingClientRect().width || first;
      header.style.setProperty("--music-edit-width", `${first}px`);
      header.style.setProperty("--music-spotify-width", `${second}px`);
      header.style.setProperty(
        "--music-actions-width",
        `${first + second + 12}px`,
      );
    };
    const observer = new ResizeObserver(measure);
    buttons.forEach((button) => observer.observe(button));
    measure();
    return () => observer.disconnect();
  }, [details.data?.spotify_url, kind]);
  return (
    <div className="music-personal" ref={actions}>
      {details.data && (
        <Stars
          value={details.data.rating}
          disabled={busy}
          change={async (rating) => {
            setBusy(true);
            setError("");
            try {
              await api.editMusic(kind, id, {
                name:
                  details.data!.name === details.data!.original_name
                    ? null
                    : details.data!.name,
                rating,
              });
              setVersion((v) => v + 1);
              toast(
                rating === null
                  ? "Rating cleared."
                  : `Rated ${rating} out of 5 stars.`,
              );
            } catch (e) {
              setError(e instanceof Error ? e.message : String(e));
            } finally {
              setBusy(false);
            }
          }}
        />
      )}
      <Link className="button secondary" to={`/music/${kind}/${id}/edit`}>
        <Icon>edit</Icon>Edit details{kind !== "tracks" && " & image"}
      </Link>
      {details.data?.spotify_url && (
        <a
          className="spotify-attribution"
          href={details.data.spotify_url}
          target="_blank"
          rel="noreferrer"
        >
          View on Spotify<Icon>open_in_new</Icon>
        </a>
      )}
      <ImageCredits credit={details.data?.image_credit} />
      <ErrorText>{error || details.error}</ErrorText>
    </div>
  );
}

export function MusicEdit({ kind, id }: { kind: Kind; id: string }) {
  const [version, setVersion] = useState(0);
  const details = useLoad<Details>(
    () => api.libraryDetails(kind, id),
    `${kind}:${id}:${version}`,
  );
  const [name, setName] = useState(""),
    [rating, setRating] = useState<number | null>(null);
  const [file, setFile] = useState<File | null>(null),
    [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  const [preview, setPreview] = useState("");
  const initialized = useRef("");
  const toast = useToast();
  const back = `/music/${kind}/${id}`;
  useLayoutEffect(() => {
    if (details.data && initialized.current !== `${kind}:${id}`) {
      initialized.current = `${kind}:${id}`;
      setName(details.data.name);
      setRating(details.data.rating);
    }
  }, [details.data]);
  useEffect(() => {
    const objectUrl = file ? URL.createObjectURL(file) : "";
    setPreview(objectUrl);
    return () => {
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [file]);
  return (
    <>
      <Link to={back} back className="back-link button secondary">
        <Icon>arrow_back</Icon>Back to{" "}
        {kind === "artists" ? "artist" : kind === "albums" ? "album" : "track"}
      </Link>
      <Heading
        title={`Edit ${kind === "artists" ? "artist" : kind === "albums" ? "album" : "track"}`}
      >
        Personal corrections are kept across future imports and Spotify updates.
      </Heading>
      <ErrorText>{details.error}</ErrorText>
      {details.data ? (
        <div className={`edit-layout ${kind === "tracks" ? "edit-track" : ""}`}>
          <article className="panel padded details-editor">
            <h2>Details &amp; rating</h2>
            <form
              className="details-form"
              onSubmit={async (e) => {
                e.preventDefault();
                setBusy(true);
                setError("");
                try {
                  await api.editMusic(kind, id, {
                    name: name === details.data!.original_name ? null : name,
                    rating,
                  });
                  toast("Details saved.");
                  returnTo(back);
                } catch (e) {
                  setError(e instanceof Error ? e.message : String(e));
                } finally {
                  setBusy(false);
                }
              }}
            >
              <label>
                {kind === "artists" ? "Artist name" : "Title"}
                <input
                  required
                  maxLength={300}
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  disabled={busy}
                />
              </label>
              <div className="source-name">
                <p className="original-value">
                  From your sources:{" "}
                  <strong>{details.data.original_name}</strong>
                </p>
                <button
                  className="secondary"
                  type="button"
                  disabled={busy || name === details.data.original_name}
                  onClick={() => setName(details.data!.original_name)}
                >
                  <Icon>restart_alt</Icon>Use original name
                </button>
              </div>
              <Stars value={rating} change={setRating} disabled={busy} />
              <ErrorText>{error}</ErrorText>
              <div className="connection-actions">
                <button className="primary" disabled={busy}>
                  {busy ? "Saving…" : "Save details"}
                </button>
                <Link className="button secondary" to={back} back>
                  Cancel
                </Link>
              </div>
            </form>
          </article>
          {kind !== "tracks" && (
            <article className="panel padded artwork-editor">
              <h2>{kind === "artists" ? "Artist image" : "Album artwork"}</h2>
              <p>
                Upload a file or paste a direct image link. Your choice replaces
                automatic artwork until you reset it.
              </p>
              {preview ? (
                <img
                  className="artwork-preview"
                  src={preview}
                  alt="Selected image preview"
                />
              ) : (
                <Cover
                  url={`/api/v1/artwork/${kind}/${id}?v=${version}`}
                  large
                />
              )}
              <ImageCredits credit={details.data.image_credit} />
              <form
                onSubmit={async (e) => {
                  e.preventDefault();
                  setBusy(true);
                  setError("");
                  try {
                    if (file) await api.uploadArtwork(kind, id, file);
                    else await api.artworkUrl(kind, id, url);
                    setFile(null);
                    setUrl("");
                    setVersion((v) => v + 1);
                    window.dispatchEvent(
                      new CustomEvent("artwork-updated", {
                        detail: { kind, id },
                      }),
                    );
                    toast("Image saved locally.");
                  } catch (e) {
                    setError(e instanceof Error ? e.message : String(e));
                  } finally {
                    setBusy(false);
                  }
                }}
              >
                <label>
                  Upload an image
                  <input
                    type="file"
                    accept="image/png,image/jpeg,image/webp"
                    disabled={busy}
                    onChange={(e) => {
                      setFile(e.target.files?.[0] || null);
                      setUrl("");
                    }}
                  />
                </label>
                <p className="subtle">
                  PNG, JPEG or WebP · up to 5 MB. Images are stored on your
                  server.
                </p>
                <label>
                  Or paste an image URL
                  <input
                    type="url"
                    placeholder="https://…"
                    value={url}
                    disabled={busy}
                    onChange={(e) => {
                      setUrl(e.target.value);
                      setFile(null);
                    }}
                  />
                </label>
                <div className="connection-actions">
                  <button
                    className="primary"
                    disabled={busy || (!file && !url)}
                  >
                    {busy ? "Saving image…" : "Save image"}
                  </button>
                  <button
                    type="button"
                    className="secondary"
                    disabled={busy || !details.data.custom_image}
                    onClick={async () => {
                      setBusy(true);
                      setError("");
                      try {
                        await api.artworkUrl(kind, id, null);
                        setFile(null);
                        setUrl("");
                        setVersion((v) => v + 1);
                        window.dispatchEvent(
                          new CustomEvent("artwork-updated", {
                            detail: { kind, id },
                          }),
                        );
                        toast(
                          "Custom image removed. Available provider images will be used.",
                        );
                      } catch (e) {
                        setError(e instanceof Error ? e.message : String(e));
                      } finally {
                        setBusy(false);
                      }
                    }}
                  >
                    <Icon>restart_alt</Icon>Use Spotify image
                  </button>
                </div>
              </form>
              <ErrorText>{error}</ErrorText>
            </article>
          )}
          {kind !== "tracks" && <NameGroup kind={kind} id={id} />}
        </div>
      ) : (
        !details.error && (
          <SkeletonCardList count={2} label="Loading details…" />
        )
      )}
    </>
  );
}
