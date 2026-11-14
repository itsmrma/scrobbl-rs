import { useEffect, useId, useRef, useState } from "react";
import { api } from "./api";
import type { SearchItem } from "./generated";
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

const singularLabels: Record<Kind, string> = {
  artists: "Artist",
  albums: "Album",
  tracks: "Track",
};

const OVERVIEW_LIMIT = 8;
const AUTO_SEARCH_MS = 380;
const AUTOCOMPLETE_MS = 160;

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

type Suggestion = {
  kind: Kind;
  item: SearchItem;
};

export function SearchPage() {
  const search = useSearch();
  const params = new URLSearchParams(search);
  const query = (params.get("q") || "").trim();
  const selected = params.get("kind");
  const kind = kinds.includes(selected as Kind) ? (selected as Kind) : null;
  const [draft, setDraft] = useState(query);
  const [suggestOpen, setSuggestOpen] = useState(false);
  const [suggestions, setSuggestions] = useState<Suggestion[]>([]);
  const [suggestLoading, setSuggestLoading] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const formWrapRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const listboxId = useId();

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

  // Auto-search shortly after typing pauses.
  useEffect(() => {
    const trimmed = draft.trim();
    if (trimmed === query) return;
    const timer = window.setTimeout(() => {
      change(trimmed, kind);
    }, AUTO_SEARCH_MS);
    return () => window.clearTimeout(timer);
  }, [draft, query, kind]);

  // Fetch autocomplete suggestions with cancellation only while actively interacting.
  useEffect(() => {
    const trimmed = draft.trim();
    if (!suggestOpen || !trimmed) {
      if (!trimmed) setSuggestions([]);
      setSuggestLoading(false);
      setActiveIndex(-1);
      return;
    }
    const controller = new AbortController();
    setSuggestLoading(true);
    const timer = window.setTimeout(() => {
      const scope = kind ? [kind] : kinds;
      void Promise.all(
        scope.map((type) =>
          api
            .search(type, trimmed, controller.signal)
            .then((items) =>
              items
                .slice(0, kind ? 8 : 4)
                .map((item) => ({ kind: type, item })),
            )
            .catch(() => [] as Suggestion[]),
        ),
      )
        .then((groups) => {
          if (controller.signal.aborted) return;
          setSuggestions(groups.flat());
          setActiveIndex(-1);
          setSuggestLoading(false);
        })
        .catch(() => {
          if (!controller.signal.aborted) setSuggestLoading(false);
        });
    }, AUTOCOMPLETE_MS);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [draft, kind, suggestOpen]);

  const showSuggestions =
    suggestOpen &&
    draft.trim().length > 0 &&
    (suggestLoading || suggestions.length > 0);
  const pendingAutoSearch = draft.trim() !== query;

  function openSuggestion(entry: Suggestion) {
    setSuggestOpen(false);
    setActiveIndex(-1);
    navigate(
      `/music/${entry.kind}/${encodeURIComponent(entry.item.id)}?period=all`,
    );
  }

  return (
    <div className="library-search">
      <Heading title="Search your library">
        Find an artist, album or track in your archive.
      </Heading>
      <div
        ref={formWrapRef}
        className="library-search-box"
        onBlur={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget)) {
            setSuggestOpen(false);
          }
        }}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            setSuggestOpen(false);
            setActiveIndex(-1);
          } else if (
            (event.key === "ArrowDown" || event.key === "ArrowUp") &&
            draft.trim().length > 0
          ) {
            event.preventDefault();
            setSuggestOpen(true);
            if (suggestions.length > 0) {
              setActiveIndex((prev) =>
                event.key === "ArrowDown"
                  ? (prev + 1) % suggestions.length
                  : (prev - 1 + suggestions.length) % suggestions.length,
              );
            }
          }
        }}
      >
        <form
          role="search"
          className="library-search-form"
          onSubmit={(event) => {
            event.preventDefault();
            if (activeIndex >= 0 && suggestions[activeIndex]) {
              openSuggestion(suggestions[activeIndex]);
              return;
            }
            setSuggestOpen(false);
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
            aria-autocomplete="list"
            aria-expanded={showSuggestions}
            aria-controls={listboxId}
            aria-activedescendant={
              activeIndex >= 0 ? `${listboxId}-opt-${activeIndex}` : undefined
            }
            onPointerDown={() => {
              if (draft.trim().length > 0) setSuggestOpen(true);
            }}
            onChange={(event) => {
              const next = event.target.value;
              setDraft(next);
              setSuggestOpen(next.trim().length > 0);
            }}
          />
          <div className="library-search-controls">
            {(suggestLoading || pendingAutoSearch) && <Spinner />}
            {draft.length > 0 && (
              <button
                type="button"
                className="btn-icon icon-button search-clear-btn"
                aria-label="Clear search"
                onClick={() => {
                  setDraft("");
                  setSuggestions([]);
                  setSuggestOpen(false);
                  setActiveIndex(-1);
                  change("", kind);
                  inputRef.current?.focus();
                }}
              >
                <Icon>close</Icon>
              </button>
            )}
            <button type="submit" className="secondary">
              Search
            </button>
          </div>
        </form>
        {showSuggestions && (
          <ul
            id={listboxId}
            role="listbox"
            className="search-autocomplete"
            aria-label="Search suggestions"
          >
            {suggestLoading && suggestions.length === 0 ? (
              <li className="search-autocomplete-loading" role="status">
                <Spinner />
                <span>Looking for matches</span>
                <LoadingDots />
              </li>
            ) : (
              suggestions.map((entry, idx) => (
                <li key={`${entry.kind}:${entry.item.id}`}>
                  <button
                    id={`${listboxId}-opt-${idx}`}
                    type="button"
                    role="option"
                    aria-selected={idx === activeIndex}
                    className={`search-autocomplete-item ${idx === activeIndex ? "is-active" : ""}`}
                    onPointerDown={(event) => event.preventDefault()}
                    onClick={() => openSuggestion(entry)}
                  >
                    <Cover url={entry.item.cover_url} />
                    <span className="search-autocomplete-copy">
                      <strong>{entry.item.name}</strong>
                      {entry.item.subtitle && (
                        <small>{entry.item.subtitle}</small>
                      )}
                    </span>
                    <span className="search-autocomplete-kind">
                      {singularLabels[entry.kind]}
                    </span>
                  </button>
                </li>
              ))
            )}
          </ul>
        )}
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
