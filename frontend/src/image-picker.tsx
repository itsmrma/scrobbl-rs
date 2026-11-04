import { useEffect, useRef, useState } from "react";
import type { ArtworkCandidate, ArtworkCandidatesResponse } from "./api";
import { api, artworkPreviewUrl } from "./api";
import type { Kind } from "./selection";
import { ErrorText, Icon, Modal, SegmentedControl, useToast } from "./ui";

type PickerTab = "search" | "upload" | "url";

export interface ImagePickerModalProps {
  kind: Kind;
  id: string;
  entityName: string;
  customImage?: boolean;
  onClose: () => void;
  onSaved?: () => void;
}

export function ImagePickerModal({
  kind,
  id,
  entityName,
  customImage = false,
  onClose,
  onSaved,
}: ImagePickerModalProps) {
  const [tab, setTab] = useState<PickerTab>("search");
  const [query, setQuery] = useState("");
  const [searchData, setSearchData] =
    useState<ArtworkCandidatesResponse | null>(null);
  const [searching, setSearching] = useState(false);
  const [selectedCandidate, setSelectedCandidate] =
    useState<ArtworkCandidate | null>(null);
  const [file, setFile] = useState<File | null>(null);
  const [filePreview, setFilePreview] = useState("");
  const [dragOver, setDragOver] = useState(false);
  const [directUrl, setDirectUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const fileInputRef = useRef<HTMLInputElement>(null);
  const toast = useToast();

  // Resolve target storage entity (artists/albums, or release album for tracks)
  const targetKind =
    searchData?.target_kind || (kind === "tracks" ? "albums" : kind);
  const targetId = searchData?.target_id || id;
  const targetName = searchData?.target_name || entityName;

  async function runSearch(customQuery?: string) {
    setSearching(true);
    setError("");
    try {
      const response = await api.artworkCandidates(kind, id, customQuery);
      setSearchData(response);
      if (customQuery === undefined) {
        setQuery(response.query);
      }
      setSelectedCandidate(response.candidates[0] ?? null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSearching(false);
    }
  }

  useEffect(() => {
    void runSearch();
  }, [kind, id]);

  useEffect(() => {
    const objectUrl = file ? URL.createObjectURL(file) : "";
    setFilePreview(objectUrl);
    return () => {
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [file]);

  function notifyArtworkSaved() {
    window.dispatchEvent(
      new CustomEvent("artwork-updated", {
        detail: { kind: targetKind, id: targetId },
      }),
    );
    window.dispatchEvent(new Event("artwork-cache-changed"));
    onSaved?.();
  }

  async function applyCandidate(candidateOverride?: ArtworkCandidate) {
    const chosen = candidateOverride ?? selectedCandidate;
    if (!chosen) return;
    setBusy(true);
    setError("");
    try {
      await api.artworkUrl(
        targetKind,
        targetId,
        chosen.image_url,
        chosen.credit,
      );
      notifyArtworkSaved();
      toast(`Saved artwork from ${chosen.provider_label}.`);
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function applyFileUpload() {
    if (!file) return;
    setBusy(true);
    setError("");
    try {
      await api.uploadArtwork(targetKind, targetId, file);
      notifyArtworkSaved();
      toast("Custom image uploaded and saved locally.");
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function applyDirectUrl() {
    const trimmed = directUrl.trim();
    if (!trimmed) return;
    setBusy(true);
    setError("");
    try {
      await api.artworkUrl(targetKind, targetId, trimmed);
      notifyArtworkSaved();
      toast("Custom image downloaded and saved locally.");
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function resetToAutomatic() {
    setBusy(true);
    setError("");
    try {
      await api.artworkUrl(targetKind, targetId, null);
      notifyArtworkSaved();
      toast("Custom image removed. Automatic provider artwork restored.");
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  const isHttpsUrl = /^https:\/\/.+/i.test(directUrl.trim());

  return (
    <Modal
      title={`Choose artwork · ${entityName}`}
      close={() => {
        if (!busy) onClose();
      }}
      className="image-picker-dialog"
    >
      <div className="image-picker">
        {kind === "tracks" && searchData && (
          <div className="image-picker-scope-banner">
            <Icon>album</Icon>
            <span>
              Tracks display their release cover. Changes apply to{" "}
              <strong>{targetName}</strong>.
            </span>
          </div>
        )}

        <div className="image-picker-toolbar">
          <SegmentedControl
            ariaLabel="Image source"
            value={tab}
            onChange={(next) => {
              setTab(next);
              setError("");
            }}
            options={[
              {
                value: "search",
                label: "Search providers",
                icon: "travel_explore",
              },
              { value: "upload", label: "Upload file", icon: "upload_file" },
              { value: "url", label: "Image URL", icon: "link" },
            ]}
          />
        </div>

        <ErrorText>{error}</ErrorText>

        {tab === "search" && (
          <div className="image-picker-section">
            <form
              className="image-picker-search-bar"
              onSubmit={(e) => {
                e.preventDefault();
                void runSearch(query);
              }}
            >
              <label htmlFor="image-picker-query" className="visually-hidden">
                Search image providers
              </label>
              <div className="search-input-wrap">
                <Icon>search</Icon>
                <input
                  id="image-picker-query"
                  type="search"
                  value={query}
                  placeholder="Search artist or release title…"
                  disabled={searching || busy}
                  onChange={(e) => setQuery(e.target.value)}
                />
              </div>
              <button
                type="submit"
                className="btn btn-secondary secondary"
                disabled={searching || busy}
              >
                {searching ? "Searching…" : "Search"}
              </button>
            </form>

            {searchData?.warnings.map((w, idx) => (
              <p key={idx} className="image-picker-warning">
                <Icon>info</Icon>
                <span>{w}</span>
              </p>
            ))}

            {searching ? (
              <div className="image-picker-loading" role="status">
                <span className="spinner-dot" aria-hidden="true" />
                <span>Querying enabled artwork providers…</span>
              </div>
            ) : searchData && searchData.candidates.length > 0 ? (
              <div
                className="image-picker-grid"
                role="listbox"
                aria-label="Artwork candidates"
              >
                {searchData.candidates.map((candidate) => {
                  const selected = selectedCandidate?.id === candidate.id;
                  return (
                    <button
                      key={candidate.id}
                      type="button"
                      role="option"
                      aria-selected={selected}
                      className={`image-candidate-card ${selected ? "selected" : ""}`}
                      onClick={() => setSelectedCandidate(candidate)}
                      onDoubleClick={() => {
                        setSelectedCandidate(candidate);
                        void applyCandidate(candidate);
                      }}
                    >
                      <span className="image-candidate-thumb">
                        <img
                          src={artworkPreviewUrl(candidate.preview_url)}
                          alt=""
                          loading="lazy"
                          referrerPolicy="no-referrer"
                        />
                        <span className="image-candidate-provider">
                          {candidate.provider_label}
                        </span>
                        {selected && (
                          <span
                            className="image-candidate-check"
                            aria-hidden="true"
                          >
                            <Icon>check</Icon>
                          </span>
                        )}
                      </span>
                      <span className="image-candidate-meta">
                        <strong>{candidate.title}</strong>
                        <small>{candidate.subtitle}</small>
                      </span>
                    </button>
                  );
                })}
              </div>
            ) : (
              <div className="empty compact-empty">
                <Icon>image_search</Icon>
                <strong>No provider images matched this query</strong>
                <span>
                  Try refining the search terms above, uploading a local image,
                  or pasting a direct HTTPS link.
                </span>
              </div>
            )}
          </div>
        )}

        {tab === "upload" && (
          <div className="image-picker-section">
            <label
              className={`image-dropzone ${dragOver ? "drag-over" : ""}`}
              onDragOver={(e) => {
                e.preventDefault();
                setDragOver(true);
              }}
              onDragLeave={() => setDragOver(false)}
              onDrop={(e) => {
                e.preventDefault();
                setDragOver(false);
                const dropped = e.dataTransfer.files?.[0];
                if (dropped) setFile(dropped);
              }}
            >
              <input
                ref={fileInputRef}
                type="file"
                aria-label="Upload an image"
                accept="image/png,image/jpeg,image/webp"
                disabled={busy}
                onChange={(e) => setFile(e.target.files?.[0] || null)}
              />
              {filePreview ? (
                <div className="image-upload-preview">
                  <img src={filePreview} alt="Selected image preview" />
                  <div>
                    <strong>{file?.name}</strong>
                    <small>
                      {file ? `${(file.size / 1024).toFixed(0)} KB` : ""} ·
                      Click or drop another file to replace
                    </small>
                  </div>
                </div>
              ) : (
                <div className="image-dropzone-copy">
                  <Icon>cloud_upload</Icon>
                  <strong>Drop a PNG, JPEG or WebP image here</strong>
                  <span>or click to browse your device · up to 5 MB</span>
                </div>
              )}
            </label>
          </div>
        )}

        {tab === "url" && (
          <div className="image-picker-section">
            <label>
              Direct HTTPS image URL
              <input
                type="url"
                placeholder="https://example.org/cover.jpg"
                value={directUrl}
                disabled={busy}
                onChange={(e) => setDirectUrl(e.target.value)}
              />
            </label>
            <p className="subtle">
              The server downloads and verifies PNG, JPEG or WebP bytes (up to 5
              MB) so your browser never contacts external hosts directly.
            </p>
            {isHttpsUrl && (
              <div className="image-upload-preview">
                <img
                  src={artworkPreviewUrl(directUrl.trim())}
                  alt="URL image preview"
                />
                <div>
                  <strong>Live server-proxied preview</strong>
                  <small>{directUrl.trim()}</small>
                </div>
              </div>
            )}
          </div>
        )}

        <div className="modal-footer image-picker-footer">
          <div className="modal-footer-left">
            {customImage && (
              <button
                type="button"
                className="btn btn-danger danger"
                disabled={busy}
                onClick={() => void resetToAutomatic()}
              >
                <Icon>restart_alt</Icon>Reset custom image
              </button>
            )}
          </div>
          <div className="modal-footer-right">
            <button
              type="button"
              className="btn btn-secondary secondary"
              disabled={busy}
              onClick={onClose}
            >
              Cancel
            </button>
            {tab === "search" && (
              <button
                type="button"
                className="btn btn-primary primary"
                disabled={busy || !selectedCandidate}
                onClick={() => void applyCandidate()}
              >
                <Icon>check</Icon>
                {busy ? "Saving…" : "Use selected image"}
              </button>
            )}
            {tab === "upload" && (
              <button
                type="button"
                className="btn btn-primary primary"
                disabled={busy || !file}
                onClick={() => void applyFileUpload()}
              >
                <Icon>save</Icon>
                {busy ? "Uploading…" : "Save uploaded image"}
              </button>
            )}
            {tab === "url" && (
              <button
                type="button"
                className="btn btn-primary primary"
                disabled={busy || !isHttpsUrl}
                onClick={() => void applyDirectUrl()}
              >
                <Icon>download_done</Icon>
                {busy ? "Saving…" : "Save image URL"}
              </button>
            )}
          </div>
        </div>
      </div>
    </Modal>
  );
}
