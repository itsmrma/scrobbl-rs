#!/usr/bin/env python3
"""
Diagnostic tool for issue #134: artwork collection verification.
Tests sample artists and albums from the database against the exact provider rules
implemented in scrobbl-rs (src/artwork_sources.rs):
- MusicBrainz relaxed matching:
  - Disambiguation comments allowed when score is 100 and name matches exactly
  - count > 100 allowed if unique exact match is present
  - Normalized album edition suffixes (Deluxe Edition, Remastered, Single, etc.)
  - Collaboration / multi-artist credits allowed if searched artist is in credits
  - Cover Art Archive selects first approved front cover
- Public fallback providers:
  - Apple iTunes Search API for albums (with 600x600 resolution upgrade)
  - Deezer API for artists
"""

import json
import sqlite3
import sys
import time
import urllib.parse
import urllib.request
from collections import Counter

USER_AGENT = "scrobbl-rs/0.1.0 ( https://github.com/itsmrma/scrobbl-rs )"

def quoted(val: str) -> str:
    escaped = []
    for c in val:
        if c in "+-!(){}[]^\"~*?:\\/|&":
            escaped.append("\\")
        escaped.append(c)
    return f'"{("".join(escaped))}"'

def normalized(s: str) -> str:
    return s.strip().lower()

def is_edition_keyword(content: str) -> bool:
    lower = content.strip().lower()
    if lower in ("single", "ep"):
        return True
    return any(
        k in lower
        for k in (
            "deluxe",
            "remaster",
            "expanded",
            "edition",
            "anniversary",
            "bonus",
            "version",
            "special",
        )
    )

def strip_edition_suffix(s: str) -> str:
    trimmed = s.strip()
    if trimmed.endswith(")"):
        open_idx = trimmed.rfind("(")
        if open_idx != -1:
            inner = trimmed[open_idx + 1:-1]
            if is_edition_keyword(inner):
                return trimmed[:open_idx].rstrip()
    if trimmed.endswith("]"):
        open_idx = trimmed.rfind("[")
        if open_idx != -1:
            inner = trimmed[open_idx + 1:-1]
            if is_edition_keyword(inner):
                return trimmed[:open_idx].rstrip()
    for delim in (" - ", " – ", " — "):
        if delim in trimmed:
            parts = trimmed.rsplit(delim, 1)
            if is_edition_keyword(parts[1]):
                return parts[0].rstrip()
    return trimmed

def normalize_album_title(title: str) -> str:
    current = title.strip()
    while True:
        prev = current
        current = strip_edition_suffix(current).strip()
        if current == prev or not current:
            break
    return current if current else title.strip()

def album_title_matches(candidate: str, target: str) -> bool:
    c_norm = normalized(candidate)
    t_norm = normalized(target)
    c_clean = normalized(normalize_album_title(candidate))
    t_clean = normalized(normalize_album_title(target))
    return c_norm == t_norm or c_clean == t_clean

def artist_matches(candidate: str, target: str) -> bool:
    c = normalized(candidate)
    t = normalized(target)
    return (
        c == t
        or (", " in c and any(part.strip() == t for part in c.split(", ")))
        or (" & " in c and any(part.strip() == t for part in c.split(" & ")))
        or (" and " in c and any(part.strip() == t for part in c.split(" and ")))
        or (", " in t and any(part.strip() == c for part in t.split(", ")))
        or (" & " in t and any(part.strip() == c for part in t.split(" & ")))
        or (" and " in t and any(part.strip() == c for part in t.split(" and ")))
    )

def http_get_json(url: str, timeout: int = 10, retries: int = 2):
    req = urllib.request.Request(
        url,
        headers={"User-Agent": USER_AGENT, "Accept": "application/json"},
    )
    for attempt in range(retries + 1):
        try:
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                if resp.status == 200:
                    return json.loads(resp.read().decode("utf-8")), 200, None
                return None, resp.status, f"HTTP {resp.status}"
        except urllib.error.HTTPError as e:
            if e.code == 503 and attempt < retries:
                time.sleep(2.0)
                continue
            return None, e.code, f"HTTP {e.code}"
        except Exception as e:
            if attempt < retries:
                time.sleep(1.0)
                continue
            return None, 0, str(e)
    return None, 0, "Max retries exceeded"

def test_artist(artist_name: str) -> dict:
    result = {
        "entity": artist_name,
        "kind": "artist",
        "final_status": "unknown",
        "failure_reason": None,
        "source": None,
    }

    # 1. Primary: MusicBrainz
    query = f"artist:{quoted(artist_name)}"
    mb_url = f"https://musicbrainz.org/ws/2/artist/?query={urllib.parse.quote(query)}&limit=100&fmt=json"
    data, code, err = http_get_json(mb_url)
    if data:
        items = data.get("artists", [])
        matching = [
            it for it in items
            if normalized(it.get("name", "")) == normalized(artist_name)
        ]
        if len(matching) == 1 and matching[0].get("score") == 100:
            mbid = matching[0].get("id")
            time.sleep(1.1)
            rel_url = f"https://musicbrainz.org/ws/2/artist/{mbid}?inc=url-rels&fmt=json"
            rel_data, rel_code, rel_err = http_get_json(rel_url)
            if rel_data:
                relations = rel_data.get("relations", [])
                img_candidates = [
                    r["url"]["resource"] for r in relations if r.get("type") == "image"
                ]
                wiki_candidates = [
                    r["url"]["resource"] for r in relations if r.get("type") == "wikidata"
                ]
                if len(img_candidates) == 1:
                    result["final_status"] = "found_mb_image"
                    result["source"] = "musicbrainz_image"
                    return result
                elif len(wiki_candidates) == 1:
                    result["final_status"] = "found_wikidata"
                    result["source"] = "wikidata"
                    return result

    # 2. Fallback: Deezer Search API
    time.sleep(0.5)
    dz_url = f"https://api.deezer.com/search/artist?q={urllib.parse.quote(artist_name)}"
    dz_data, dz_code, dz_err = http_get_json(dz_url)
    if dz_data:
        dz_items = dz_data.get("data", [])
        for item in dz_items:
            if normalized(item.get("name", "")) == normalized(artist_name):
                pic = (
                    item.get("picture_xl")
                    or item.get("picture_big")
                    or item.get("picture_medium")
                    or item.get("picture")
                )
                if pic:
                    result["final_status"] = "found_deezer"
                    result["source"] = "deezer"
                    return result

    result["final_status"] = "missing"
    result["failure_reason"] = "No image found via MusicBrainz, Wikidata or Deezer fallback"
    return result

def test_album(album_title: str, artist_name: str) -> dict:
    result = {
        "entity": f"{artist_name} - {album_title}",
        "kind": "album",
        "final_status": "unknown",
        "failure_reason": None,
        "source": None,
    }

    clean_title = normalize_album_title(album_title)

    # 1. Primary: MusicBrainz Cover Art Archive
    query = f"releasegroup:{quoted(clean_title)} AND artist:{quoted(artist_name)}"
    mb_url = f"https://musicbrainz.org/ws/2/release-group/?query={urllib.parse.quote(query)}&limit=100&fmt=json"
    data, code, err = http_get_json(mb_url)
    if data:
        items = data.get("release-groups", [])
        matching = []
        for it in items:
            title_ok = (
                normalized(it.get("title", "")) == normalized(album_title)
                or normalized(normalize_album_title(it.get("title", "")))
                == normalized(clean_title)
            )
            credits = it.get("artist-credit", [])
            artist_ok = any(
                artist_matches(c.get("artist", {}).get("name", ""), artist_name)
                or artist_matches(c.get("name", ""), artist_name)
                for c in credits
            )
            if title_ok and artist_ok:
                matching.append(it)

        if len(matching) == 1 and matching[0].get("score") == 100:
            mbid = matching[0].get("id")
            time.sleep(0.5)
            caa_url = f"https://coverartarchive.org/release-group/{mbid}"
            caa_data, caa_code, caa_err = http_get_json(caa_url)
            if caa_data:
                images = caa_data.get("images", [])
                front = [
                    img for img in images
                    if img.get("front") is True and img.get("approved") is True
                ]
                if front:
                    result["final_status"] = "found_caa_front"
                    result["source"] = "cover_art_archive"
                    return result

    # 2. Fallback: Apple iTunes Search API
    time.sleep(0.5)
    term = f"{clean_title} {artist_name}"
    it_url = f"https://itunes.apple.com/search?term={urllib.parse.quote(term)}&entity=album&limit=25"
    it_data, it_code, it_err = http_get_json(it_url)
    if it_data:
        results = it_data.get("results", [])
        for item in results:
            c_title = item.get("collectionName", "")
            c_artist = item.get("artistName", "")
            if album_title_matches(c_title, album_title) and artist_matches(c_artist, artist_name):
                artwork = item.get("artworkUrl100", "")
                if artwork:
                    upgraded = (
                        artwork.replace("/100x100bb.", "/600x600bb.")
                        if "/100x100bb." in artwork
                        else artwork.replace("/100x100.", "/600x600.")
                    )
                    result["final_status"] = "found_itunes"
                    result["source"] = "itunes"
                    result["image_url"] = upgraded
                    return result

    result["final_status"] = "missing"
    result["failure_reason"] = "No cover found via CoverArtArchive or iTunes fallback"
    return result

def main():
    db_path = sys.argv[1] if len(sys.argv) > 1 else "./data/scrobbl.db"
    print(f"=== Diagnosi Raccolta Artwork (Issue #134) ===")
    print(f"Database: {db_path}")

    conn = sqlite3.connect(db_path)
    cur = conn.cursor()

    cur.execute("""
        SELECT a.name, count(l.id) as listens
        FROM artists a
        JOIN track_artists ta ON ta.artist_id = a.id
        JOIN listens l ON l.track_id = ta.track_id
        GROUP BY a.id, a.name
        ORDER BY listens DESC
        LIMIT 10;
    """)
    top_artists = [row[0] for row in cur.fetchall()]

    cur.execute("""
        SELECT al.name as album, ar.name as artist, count(l.id) as listens
        FROM albums al
        JOIN tracks t ON t.album_id = al.id
        JOIN listens l ON l.track_id = t.id
        JOIN track_artists ta ON ta.track_id = t.id
        JOIN artists ar ON ar.id = ta.artist_id
        GROUP BY al.id, al.name, ar.name
        ORDER BY listens DESC
        LIMIT 10;
    """)
    top_albums = [(row[0], row[1]) for row in cur.fetchall()]

    conn.close()

    print(f"\n--- 1. Test Campione Artisti ({len(top_artists)}) ---")
    artist_results = []
    for a in top_artists:
        time.sleep(1.2)
        res = test_artist(a)
        artist_results.append(res)
        print(f"[{res['final_status']}] {res['entity']} ({res.get('source') or 'none'}) -> {res['failure_reason'] or 'OK'}")

    print(f"\n--- 2. Test Campione Album ({len(top_albums)}) ---")
    album_results = []
    for alb, art in top_albums:
        time.sleep(1.2)
        res = test_album(alb, art)
        album_results.append(res)
        print(f"[{res['final_status']}] {res['entity']} ({res.get('source') or 'none'}) -> {res['failure_reason'] or 'OK'}")

    print("\n" + "=" * 60)
    print("=== RIEPILOGO STATISTICHE CAMPIONE ===")
    print("=" * 60)

    total_tested = len(artist_results) + len(album_results)
    total_found = sum(1 for r in artist_results if r["final_status"].startswith("found")) + \
                  sum(1 for r in album_results if r["final_status"].startswith("found"))
    total_failed = total_tested - total_found
    success_rate = (total_found / total_tested) * 100
    fail_rate = (total_failed / total_tested) * 100

    print(f"\nTotale testati: {total_tested}")
    print(f"Totale risolti con successo: {total_found} ({success_rate:.1f}%)")
    print(f"Totale falliti: {total_failed} ({fail_rate:.1f}%)")

    sources_count = Counter(r.get("source") for r in artist_results + album_results if r.get("source"))
    print("\nDistribuzione per provider:")
    for src, cnt in sources_count.items():
        print(f"  - {src}: {cnt}")

    if fail_rate > 10.0:
        print(f"\n[ROSSO]: Tasso di fallimento {fail_rate:.1f}% supera la soglia di tolleranza (10%)!")
        sys.exit(1)
    else:
        print(f"\n[VERDE]: Raccolta artwork soddisfacente (successo {success_rate:.1f}% >= 90%).")
        sys.exit(0)

if __name__ == "__main__":
    main()
