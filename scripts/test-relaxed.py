#!/usr/bin/env python3
"""
Test the relaxed matching rules against the same top 10 artists and albums
to measure the exact improvement in artwork resolution.
"""

import json
import re
import sqlite3
import sys
import time
import urllib.parse
import urllib.request
from collections import Counter

USER_AGENT = "scrobbl-rs-diagnosis/0.1.0 ( https://github.com/itsmrma/scrobbl-rs )"

def quoted(val: str) -> str:
    escaped = []
    for c in val:
        if c in "+-!(){}[]^\"~*?:\\/|&":
            escaped.append("\\")
        escaped.append(c)
    return f'"{("".join(escaped))}"'

def normalized(s: str) -> str:
    # Normalize whitespace, quotes, and case
    s = s.strip().lower()
    s = re.sub(r"[\u2018\u2019']", "'", s)
    s = re.sub(r'[\u201c\u201d"]', '"', s)
    return s

def clean_album_title(title: str) -> str:
    # Strip (Deluxe Edition), (Remastered), - Single, etc.
    t = re.sub(r"\s*[\(\[](deluxe|remastered|anniversary|expanded|bonus|special|\d+th).*?[\)\]]", "", title, flags=re.IGNORECASE)
    t = re.sub(r"\s*-\s*(single|ep)\s*$", "", t, flags=re.IGNORECASE)
    return t.strip()

def http_get_json(url: str, timeout: int = 10):
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT, "Accept": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            if resp.status == 200:
                return json.loads(resp.read().decode("utf-8")), 200, None
            return None, resp.status, f"HTTP {resp.status}"
    except urllib.error.HTTPError as e:
        return None, e.code, f"HTTP {e.code}"
    except Exception as e:
        return None, 0, str(e)

def test_artist_relaxed(artist_name: str) -> dict:
    result = {
        "entity": artist_name,
        "kind": "artist",
        "final_status": "unknown",
        "failure_reason": None,
        "mbid": None,
    }

    # 1. Search MusicBrainz artist
    query = f"artist:{quoted(artist_name)}"
    url = f"https://musicbrainz.org/ws/2/artist/?query={urllib.parse.quote(query)}&limit=25&fmt=json"

    data, code, err = http_get_json(url)
    if not data:
        result["final_status"] = "search_error"
        result["failure_reason"] = f"MB search: {err}"
        return result

    items = data.get("artists", [])
    if not items:
        result["final_status"] = "not_found"
        result["failure_reason"] = "No artists returned by search"
        return result

    # Relaxed rule: matching name, score >= 90
    exact_matches = [
        it for it in items
        if normalized(it.get("name", "")) == normalized(artist_name)
    ]

    if not exact_matches:
        # Check if top item score is 100 (e.g. alias match like Sawano Hiroyuki -> 澤野弘之)
        if items[0].get("score") == 100:
            candidate = items[0]
        else:
            result["final_status"] = "missing_exact_name"
            result["failure_reason"] = f"Top result: '{items[0].get('name')}' (score {items[0].get('score')})"
            return result
    elif len(exact_matches) == 1:
        candidate = exact_matches[0]
    else:
        # Multiple exact matches: pick candidate with highest score, or the one with wikidata/image
        # If the first has score 100 and others < 100:
        if exact_matches[0].get("score") == 100 and exact_matches[1].get("score", 0) < 100:
            candidate = exact_matches[0]
        else:
            # Let's inspect them:
            candidate = exact_matches[0]

    mbid = candidate.get("id")
    result["mbid"] = mbid
    result["disambiguation"] = candidate.get("disambiguation", "")

    # 2. Check relations for image / wikidata
    time.sleep(1.1) # MB rate limit
    rel_url = f"https://musicbrainz.org/ws/2/artist/{mbid}?inc=url-rels&fmt=json"
    rel_data, rel_code, rel_err = http_get_json(rel_url)
    if not rel_data:
        result["final_status"] = "rel_fetch_error"
        result["failure_reason"] = f"url-rels: {rel_err}"
        return result

    relations = rel_data.get("relations", [])
    img_candidates = [r["url"]["resource"] for r in relations if r.get("type") == "image"]
    wiki_candidates = [r["url"]["resource"] for r in relations if r.get("type") == "wikidata"]

    if img_candidates:
        result["final_status"] = "found_mb_image"
        result["source_url"] = img_candidates[0]
        return result
    elif wiki_candidates:
        result["final_status"] = "found_wikidata"
        result["source_url"] = wiki_candidates[0]
        return result
    else:
        result["final_status"] = "missing_no_image_relation"
        result["failure_reason"] = "Artist found on MB, but has no image/wikidata relation"
        return result

def test_album_relaxed(album_title: str, artist_name: str) -> dict:
    result = {
        "entity": f"{artist_name} - {album_title}",
        "kind": "album",
        "final_status": "unknown",
        "failure_reason": None,
        "mbid": None,
    }

    clean_title = clean_album_title(album_title)

    # 1. Search MusicBrainz release-group
    query = f"releasegroup:{quoted(clean_title)} AND artist:{quoted(artist_name)}"
    url = f"https://musicbrainz.org/ws/2/release-group/?query={urllib.parse.quote(query)}&limit=25&fmt=json"

    data, code, err = http_get_json(url)
    if not data:
        result["final_status"] = "search_error"
        result["failure_reason"] = f"MB search: {err}"
        return result

    items = data.get("release-groups", [])
    if not items:
        # Fallback query: search by title only and check artist in credits
        time.sleep(1.1)
        q2 = f"releasegroup:{quoted(clean_title)}"
        url2 = f"https://musicbrainz.org/ws/2/release-group/?query={urllib.parse.quote(q2)}&limit=25&fmt=json"
        data, code, err = http_get_json(url2)
        items = data.get("release-groups", []) if data else []

    if not items:
        result["final_status"] = "not_found"
        result["failure_reason"] = "Release group not found"
        return result

    # Find matching candidate: title matches and artist is in credits
    matching = []
    for it in items:
        it_title = normalized(it.get("title", ""))
        if it_title != normalized(album_title) and it_title != normalized(clean_title):
            continue
        credits = it.get("artist-credit", [])
        # Check if artist is in any credit
        has_artist = any(
            normalized(artist_name) in normalized(c.get("artist", {}).get("name", ""))
            for c in credits
        )
        if has_artist:
            matching.append(it)

    if not matching:
        # If top result has high score and title matches
        top = items[0]
        if normalized(top.get("title", "")) in (normalized(album_title), normalized(clean_title)):
            matching.append(top)
        else:
            result["final_status"] = "missing_matching_title"
            result["failure_reason"] = f"Top candidate title was '{top.get('title')}'"
            return result

    candidate = matching[0]
    mbid = candidate.get("id")
    result["mbid"] = mbid

    # 2. Check CoverArtArchive for release group
    time.sleep(0.5)
    caa_url = f"https://coverartarchive.org/release-group/{mbid}"
    caa_data, caa_code, caa_err = http_get_json(caa_url)
    if caa_code == 404:
        result["final_status"] = "caa_404_no_artwork"
        result["failure_reason"] = f"CAA 404 for release-group {mbid}"
        return result
    if not caa_data:
        result["final_status"] = "caa_error"
        result["failure_reason"] = f"CAA error: {caa_err}"
        return result

    images = caa_data.get("images", [])
    # Relaxed rule: pick any front image, approved or unapproved, or first image
    front = [img for img in images if img.get("front") is True]
    if front:
        result["final_status"] = "found_caa_front"
        result["image_url"] = front[0].get("thumbnails", {}).get("500") or front[0].get("image")
        return result
    elif images:
        result["final_status"] = "found_caa_image"
        result["image_url"] = images[0].get("thumbnails", {}).get("500") or images[0].get("image")
        return result
    else:
        result["final_status"] = "caa_empty_images"
        result["failure_reason"] = "CAA returned 0 images"
        return result

def main():
    db_path = "./data/scrobbl.db"
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

    print("=== Test con Regole Rilassate (MusicBrainz / CAA) ===")

    print(f"\n--- 1. Artisti ({len(top_artists)}) ---")
    art_results = []
    for a in top_artists:
        time.sleep(1.2) # pacing
        res = test_artist_relaxed(a)
        art_results.append(res)
        status = res["final_status"]
        extra = res.get("source_url") or res.get("failure_reason") or ""
        print(f"[{status}] {res['entity']} -> {extra}")

    print(f"\n--- 2. Album ({len(top_albums)}) ---")
    alb_results = []
    for alb, art in top_albums:
        time.sleep(1.2) # pacing
        res = test_album_relaxed(alb, art)
        alb_results.append(res)
        status = res["final_status"]
        extra = res.get("image_url") or res.get("failure_reason") or ""
        print(f"[{status}] {res['entity']} -> {extra}")

    print("\n" + "="*60)
    print("=== RISULTATI CONFRONTO REGOLA RILASSATA ===")
    print("="*60)
    art_ok = sum(1 for r in art_results if r["final_status"].startswith("found"))
    alb_ok = sum(1 for r in alb_results if r["final_status"].startswith("found"))
    print(f"Artisti trovati: {art_ok} / {len(art_results)} ({art_ok*10}%)")
    print(f"Album trovati:   {alb_ok} / {len(alb_results)} ({alb_ok*10}%)")
    total_ok = art_ok + alb_ok
    total_tested = len(art_results) + len(alb_results)
    print(f"TOTALE:          {total_ok} / {total_tested} ({(total_ok/total_tested)*100:.1f}%)")

if __name__ == "__main__":
    main()
