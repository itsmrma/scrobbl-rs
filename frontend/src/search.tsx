import { useEffect, useState } from "react";
import { api } from "./api";
import { useLoad } from "./load";
import { Link, navigate, useSearch } from "./navigation";
import { kinds } from "./selection";
import type { Kind } from "./selection";
import { Cover, ErrorText, Heading, Icon, number } from "./ui";
import { SkeletonList } from "./skeleton";

const labels: Record<Kind, string> = {
  artists: "Artists",
  albums: "Albums",
  tracks: "Tracks",
};

function SearchResults({ kind, query }: { kind: Kind; query: string }) {
  const [retry, setRetry] = useState(0);
  const result = useLoad(
    () => api.search(kind, query),
    `search:${kind}:${query}:${retry}`,
    true,
    undefined,
    true,
  );
  return (
    <section
      className="search-result-group"
      aria-labelledby={`search-${kind}`}
      aria-busy={result.loading}
    >
      <div className="search-group-heading">
        <h2 id={`search-${kind}`}>{labels[kind]}</h2>
        {result.data && (
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
        <SkeletonList
          count={3}
          label={`Searching ${labels[kind].toLowerCase()}…`}
        />
      )}
      {result.data?.length === 0 && (
        <p className="search-no-matches">
          No {labels[kind].toLowerCase()} match “{query}”. Try another name.
        </p>
      )}
      {!!result.data?.length && (
        <ul className="search-result-list">
          {result.data.map((item) => (
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
      {result.data?.length === 50 && (
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
  useEffect(() => setDraft(query), [query]);
  function change(q: string, type: Kind | null) {
    const next = new URLSearchParams();
    if (q) next.set("q", q);
    if (type) next.set("kind", type);
    navigate(`/search${next.size ? `?${next}` : ""}`);
  }
  return (
    <div className="library-search">
      <Heading title="Search your library">
        Find an artist, album or track in your archive.
      </Heading>
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
        <button type="submit" className="secondary">
          Search
        </button>
      </form>
      <div
        className="search-type-filters"
        role="group"
        aria-label="Search categories"
      >
        {[null, ...kinds].map((type) => (
          <button
            key={type || "all"}
            type="button"
            className="secondary"
            aria-pressed={kind === type}
            onClick={() => change(query, type)}
          >
            {type ? labels[type] : "All"}
          </button>
        ))}
      </div>
      {query ? (
        <>
          <p className="search-query-summary" role="status">
            Results for <strong>“{query}”</strong> · Most listened first
          </p>
          <div
            className={`search-result-groups ${kind ? "single-category" : ""}`}
          >
            {(kind ? [kind] : kinds).map((type) => (
              <SearchResults key={type} kind={type} query={query} />
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
