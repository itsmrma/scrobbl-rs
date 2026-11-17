import { useEffect, useRef, useState } from "react";
import { api } from "./api";
import { useLoad } from "./load";
import { Link, navigate, useSearch } from "./navigation";
import { kinds } from "./selection";
import type { Kind } from "./selection";
import {
  Cover,
  ErrorText,
  Heading,
  Icon,
  LoadingDots,
  SegmentedControl,
  Spinner,
  number,
} from "./ui";
import { SkeletonList } from "./skeleton";

const labels: Record<Kind, string> = {
  artists: "Artists",
  albums: "Albums",
  tracks: "Tracks",
};

const OVERVIEW_LIMIT = 8;
const AUTO_SEARCH_MS = 380;

function SearchResults({
  kind,
  query,
  overview = false,
  onShowAll,
}: {
  kind: Kind;
  query: string;
  overview?: boolean;
  onShowAll?: (kind: Kind) => void;
}) {
  const [retry, setRetry] = useState(0);
  const result = useLoad(
    () => api.search(kind, query),
    `search:${kind}:${query}:${retry}`,
    true,
    undefined,
    true,
  );
  const empty = result.data?.length === 0;
  const shown =
    overview && result.data
      ? result.data.slice(0, OVERVIEW_LIMIT)
      : result.data;
  const hidden = (result.data?.length ?? 0) - (shown?.length ?? 0);
  return (
    <section
      className={`search-result-group${empty && overview ? " is-empty" : ""}`}
      aria-labelledby={`search-${kind}`}
      aria-busy={result.loading}
    >
      <div className="search-group-heading">
        <h2 id={`search-${kind}`} tabIndex={-1}>
          {labels[kind]}
        </h2>
        {result.data && !(empty && overview) && (
          <span>
            {number.format(result.data.length)}
            {result.data.length === 50 ? "+" : ""}
          </span>
        )}
      </div>
      <ErrorText>{result.error}</ErrorText>
      {result.error && (
        <button
          type="button"
          className="secondary"
          onClick={() => setRetry((value) => value + 1)}
        >
          Try again
        </button>
      )}
      {!result.data && !result.error && (
        <>
          <div className="search-group-loading" role="status">
            <Spinner />
            <span>Searching {labels[kind].toLowerCase()}</span>
            <LoadingDots />
          </div>
          <SkeletonList
            count={3}
            label={`Searching ${labels[kind].toLowerCase()}…`}
          />
        </>
      )}
      {empty && (
        <p className="search-no-matches">
          No {labels[kind].toLowerCase()} match “{query}”.
          {!overview && " Try another name."}
        </p>
      )}
      {!!shown?.length && (
        <ul className="search-result-list">
          {shown.map((item) => (
            <li key={item.id}>
              <Link
                to={`/music/${kind}/${encodeURIComponent(item.id)}?period=all`}
              >
                <Cover url={item.cover_url} />
                <span className="search-result-copy">
                  <strong>{item.name}</strong>
                  <small>{item.subtitle}</small>
                </span>
                <span className="search-result-plays">
                  <strong>{number.format(item.plays)}</strong>
                  <small>{item.plays === 1 ? "listen" : "listens"}</small>
                </span>
                <Icon>chevron_right</Icon>
              </Link>
              {item.editions.length > 1 && (
                <details className="search-editions">
                  <summary>
                    {item.editions.length} editions · open an edition
                  </summary>
                  <p>Most listened edition shown above.</p>
                  <ul>
                    {item.editions.map((edition) => (
                      <li key={edition.id}>
                        <Link
                          to={`/music/${kind}/${encodeURIComponent(edition.id)}?period=all`}
                        >
                          <Cover url={edition.cover_url} />
                          <span className="search-result-copy">
                            {edition.subtitle}
                          </span>
                          <small>
                            {number.format(edition.plays)}{" "}
                            {edition.plays === 1 ? "listen" : "listens"}
                          </small>
                          <Icon>chevron_right</Icon>
                        </Link>
                      </li>
                    ))}
                  </ul>
                </details>
              )}
            </li>
          ))}
        </ul>
      )}
      {hidden > 0 && onShowAll && (
        <button
          type="button"
          className="secondary search-show-all"
          onClick={() => onShowAll(kind)}
        >
          Show all {number.format(result.data!.length)}
          {result.data!.length === 50 ? "+" : ""} {labels[kind].toLowerCase()}
        </button>
      )}
      {!overview && result.data?.length === 50 && (
        <p className="subtle">
          Showing the first 50 matches. Use a more specific name to narrow the
          results.
        </p>
      )}
    </section>
  );
}

export function SearchPage() {
  const search = useSearch();
  const params = new URLSearchParams(search);
  const query = (params.get("q") || "").trim();
  const selected = params.get("kind");
  const kind = kinds.includes(selected as Kind) ? (selected as Kind) : null;
  const [draft, setDraft] = useState(query);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => setDraft(query), [query]);

  function change(q: string, type: Kind | null) {
    const next = new URLSearchParams();
    if (q) next.set("q", q);
    if (type) next.set("kind", type);
    const target = `/search${next.size ? `?${next}` : ""}`;
    if (target !== `${location.pathname}${location.search}`) {
      navigate(target);
    }
  }

  // Results update shortly after typing pauses. There is no separate
  // suggestion menu: the result groups below are the suggestions.
  useEffect(() => {
    const trimmed = draft.trim();
    if (trimmed === query) return;
    const timer = window.setTimeout(() => {
      change(trimmed, kind);
    }, AUTO_SEARCH_MS);
    return () => window.clearTimeout(timer);
  }, [draft, query, kind]);

  const pendingAutoSearch = draft.trim() !== query;

  return (
    <div className="library-search">
      <Heading title="Search your library">
        Find an artist, album or track in your archive.
      </Heading>
      <div className="library-search-box">
        <form
          role="search"
          className="library-search-form"
          onSubmit={(event) => {
            event.preventDefault();
            change(draft.trim(), kind);
          }}
        >
          <label htmlFor="library-query" className="visually-hidden">
            Artist, album or track name
          </label>
          <Icon>search</Icon>
          <input
            ref={inputRef}
            id="library-query"
            type="search"
            name="q"
            value={draft}
            maxLength={300}
            autoFocus
            placeholder="Artist, album or track name"
            autoComplete="off"
            onChange={(event) => setDraft(event.target.value)}
          />
          <div className="library-search-controls">
            {pendingAutoSearch && <Spinner />}
            {draft.length > 0 && (
              <button
                type="button"
                className="secondary search-clear-btn"
                aria-label="Clear search"
                onClick={() => {
                  setDraft("");
                  change("", kind);
                  inputRef.current?.focus();
                }}
              >
                <Icon>close</Icon>
              </button>
            )}
            <button type="submit" className="secondary search-submit-btn">
              Search
            </button>
          </div>
        </form>
      </div>
      <SegmentedControl
        className="search-type-filters"
        ariaLabel="Search categories"
        value={kind ?? "all"}
        onChange={(val) => change(query, val === "all" ? null : (val as Kind))}
        options={[
          { value: "all", label: "All" },
          ...kinds.map((type) => ({
            value: type,
            label: labels[type],
          })),
        ]}
      />
      {query ? (
        <>
          <p className="search-query-summary" role="status">
            Results for <strong>“{query}”</strong> · Most listened first
          </p>
          <div className="search-result-groups">
            {(kind ? [kind] : kinds).map((type) => (
              <SearchResults
                key={type}
                kind={type}
                query={query}
                overview={!kind}
                onShowAll={(value) => {
                  change(query, value);
                  // The overview unmounts with the button; keep focus on the
                  // group the person asked to expand.
                  requestAnimationFrame(() =>
                    document.getElementById(`search-${value}`)?.focus(),
                  );
                }}
              />
            ))}
          </div>
        </>
      ) : (
        <div className="search-start">
          <Icon>search</Icon>
          <h2>Find music you’ve listened to.</h2>
          <p>
            Search by name to open a music page, explore its listening history
            and see all its details.
          </p>
        </div>
      )}
    </div>
  );
}
