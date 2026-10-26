import { useState } from "react";
import { api } from "./api";
import { useLoad } from "./load";
import { navigate } from "./navigation";
import { ErrorText, Icon, number, useToast } from "./ui";
import { SkeletonList } from "./skeleton";
export function NameGroup({ kind, id }: { kind: string; id: string }) {
  const [query, setQuery] = useState("");
  const [draft, setDraft] = useState("");
  const [page, setPage] = useState(1);
  const [selected, setSelected] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [version, setVersion] = useState(0);
  const result = useLoad(
    () => api.nameGroup(kind, id, query, page),
    `${kind}:${id}:${query}:${page}:${version}`,
  );
  const toast = useToast();
  async function change(action: "separate" | "rejoin") {
    setBusy(true);
    setError("");
    try {
      const saved = await api.changeNameGroup(kind, id, action, selected);
      setSelected([]);
      setVersion((v) => v + 1);
      toast(
        action === "separate"
          ? "Selected entries now form a separate group. Future imports keep this choice."
          : "Selected entries rejoined their name groups.",
      );
      navigate(`/music/${kind}/${saved.group_id}/edit`);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <article className="panel padded name-group-editor">
      <h2>Grouped by name</h2>
      <p>
        {kind === "artists"
          ? "Artists with the same source name appear together."
          : "Albums with the same source title and artist appear together."}{" "}
        Grouping is local and doesn’t contact Spotify.
      </p>
      <p>
        For namesakes or different editions, select the source entries that
        belong together and separate them into another group. You can rejoin
        them later. Original identities and listens are kept.
      </p>
      <form
        className="search-inline"
        onSubmit={(e) => {
          e.preventDefault();
          setQuery(draft);
          setPage(1);
          setSelected([]);
        }}
      >
        <label>
          Find a source entry
          <input
            value={draft}
            maxLength={500}
            onChange={(e) => setDraft(e.target.value)}
          />
        </label>
        <button className="secondary" disabled={busy}>
          Search entries
        </button>
      </form>
      <ErrorText>{error || result.error}</ErrorText>
      {result.data && (
        <>
          <p className="subtle">
            {number.format(result.data.total)} source entries ·{" "}
            {selected.length} selected
          </p>
          <ul className="group-members">
            {result.data.members.map((member) => (
              <li key={member.id}>
                <label className="check">
                  <input
                    type="checkbox"
                    checked={selected.includes(member.id)}
                    disabled={busy}
                    onChange={(e) =>
                      setSelected((ids) =>
                        e.target.checked
                          ? [...ids, member.id]
                          : ids.filter((value) => value !== member.id),
                      )
                    }
                  />
                  <span>
                    <strong>{member.name}</strong>
                    <small>
                      {member.sample}
                      {member.tracks > 1
                        ? ` + ${member.tracks - 1} other tracks`
                        : ""}{" "}
                      · {number.format(member.listens)} listens
                    </small>
                  </span>
                </label>
              </li>
            ))}
          </ul>
          {!result.data.members.length && (
            <p>No source entries match your search.</p>
          )}
          <div className="pagination">
            <button
              className="secondary"
              disabled={busy || page === 1}
              onClick={() => {
                setPage((p) => p - 1);
                setSelected([]);
              }}
            >
              Previous entries
            </button>
            <span>
              Page {page} of {Math.max(1, Math.ceil(result.data.total / 24))}
            </span>
            <button
              className="secondary"
              disabled={busy || page * 24 >= result.data.total}
              onClick={() => {
                setPage((p) => p + 1);
                setSelected([]);
              }}
            >
              Next entries
            </button>
          </div>
          <div className="connection-actions">
            <button
              className="secondary"
              disabled={busy || !selected.length}
              onClick={() => void change("separate")}
            >
              <Icon>call_split</Icon>Separate selected entries
            </button>
            {result.data.can_rejoin && (
              <button
                className="secondary"
                disabled={busy || !selected.length}
                onClick={() => void change("rejoin")}
              >
                <Icon>merge</Icon>Rejoin selected by name
              </button>
            )}
          </div>
        </>
      )}
      {!result.data && !result.error && (
        <SkeletonList count={3} label="Loading source entries…" />
      )}
    </article>
  );
}
