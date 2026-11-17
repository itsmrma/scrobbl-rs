import { useState } from "react";
import { api } from "./api";
import type { ImportFile, Preferences } from "./generated";
import { useLoad } from "./load";
import { Button, ErrorText, Icon, Modal, date, number, useToast } from "./ui";

/**
 * The imported files and the listens each one still owns.
 *
 * Deleting a file removes only the listens that came from it: the listens that
 * live collection recorded stay, so a file can be removed and imported again
 * after an import-logic change without losing anything collected from Spotify.
 */
export function ImportFiles({
  preferences,
  active = true,
}: {
  preferences: Preferences;
  active?: boolean;
}) {
  const [version, setVersion] = useState(0);
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [file, setFile] = useState<ImportFile | null>(null);
  const [allOpen, setAllOpen] = useState(false);
  const toast = useToast();
  const result = useLoad(() => api.imports(), `imports:${version}`, active);

  async function remove(run: () => Promise<{ removed: number }>, done: string) {
    setBusy("delete");
    setError("");
    try {
      const removed = await run();
      setFile(null);
      setAllOpen(false);
      setVersion((v) => v + 1);
      toast(done.replace("{n}", number.format(removed.removed)));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy("");
    }
  }

  const items = result.data ?? [];
  return (
    <article className="panel padded settings-form">
      <h2>Imported files</h2>
      <p>
        Every Spotify history file you imported, with what it added and what is
        still attributed to it. Deleting a file removes only the listens it
        brought in: listens collected live from Spotify stay, and the file can
        be imported again.
      </p>
      <ErrorText>{error || result.error}</ErrorText>
      {!result.data && !result.error && (
        <p className="field-hint">Loading imported files…</p>
      )}
      {result.data?.length === 0 && (
        <div className="empty">
          <Icon>upload_file</Icon>
          <p>No Spotify history files have been imported yet.</p>
        </div>
      )}
      {items.length > 0 && (
        <div className="import-files">
          {items.map((item) => (
            <div className="import-file" key={`${item.job_id}:${item.file_id}`}>
              <div className="import-file-main">
                <strong>{item.filename}</strong>
                <small>
                  {date(item.created_at_ms, preferences)} ·{" "}
                  {bytes(item.size_bytes)} · {item.state}
                  {item.outcome ? ` · ${item.outcome}` : ""}
                </small>
                <small className="field-hint">
                  {number.format(item.listens)} listens still attributed ·{" "}
                  {number.format(item.added)} added ·{" "}
                  {number.format(item.duplicates)} duplicates ·{" "}
                  {number.format(item.invalid)} invalid ·{" "}
                  {number.format(item.skipped)} skipped
                </small>
                {item.error_code && (
                  <small className="form-error">{item.error_code}</small>
                )}
              </div>
              <Button
                variant="danger"
                disabled={busy === "delete"}
                onClick={() => setFile(item)}
              >
                <Icon>delete_outline</Icon> Delete listens
              </Button>
            </div>
          ))}
        </div>
      )}
      {items.length > 0 && (
        <div className="connection-actions">
          <Button
            variant="danger"
            disabled={busy === "delete"}
            onClick={() => setAllOpen(true)}
          >
            <Icon>delete_sweep</Icon> Delete all imported listens
          </Button>
        </div>
      )}
      {file && (
        <Modal title="Delete imported listens" close={() => setFile(null)}>
          <p>
            Delete the listens imported from <strong>{file.filename}</strong>?
            {file.listens > 0 &&
              ` ${number.format(file.listens)} listens are still attributed to it.`}
          </p>
          <p>
            Listens collected live from Spotify are kept. You can import the
            same file again afterwards.
          </p>
          <div className="form-actions">
            <Button
              variant="danger"
              disabled={busy === "delete"}
              onClick={() =>
                remove(
                  () => api.deleteImportFile(file.file_id),
                  "Deleted {n} imported listens.",
                )
              }
            >
              <Icon>delete_outline</Icon> Delete listens
            </Button>
            <Button variant="secondary" onClick={() => setFile(null)}>
              Cancel
            </Button>
          </div>
        </Modal>
      )}
      {allOpen && (
        <Modal
          title="Delete every imported listen"
          close={() => setAllOpen(false)}
        >
          <p>
            Delete the listens that came from every Spotify history file you
            imported. Listens collected live from Spotify are kept, and you can
            import the files again.
          </p>
          <div className="form-actions">
            <Button
              variant="danger"
              disabled={busy === "delete"}
              onClick={() =>
                remove(
                  () => api.deleteAllImports(),
                  "Deleted {n} imported listens.",
                )
              }
            >
              <Icon>delete_sweep</Icon> Delete all imported listens
            </Button>
            <Button variant="secondary" onClick={() => setAllOpen(false)}>
              Cancel
            </Button>
          </div>
        </Modal>
      )}
    </article>
  );
}
function bytes(value: number) {
  if (!Number.isFinite(value) || value <= 0) return "Size unknown";
  if (value >= 1024 ** 3) return `${(value / 1024 ** 3).toFixed(1)} GB`;
  if (value >= 1024 ** 2) return `${Math.round(value / 1024 ** 2)} MB`;
  return `${Math.round(value / 1024)} KB`;
}
