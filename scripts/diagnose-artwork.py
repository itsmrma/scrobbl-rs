#!/usr/bin/env python3
"""
Diagnostic tool for issue #134: artwork collection failures and on-the-fly download drops.
Tests sample artists and albums from the database against the exact provider rules
implemented in scrobbl-rs (src/artwork_sources.rs), and simulates concurrent on-the-fly requests.
"""

import json
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
    return s.strip().lower()

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

def test_artist(artist_name: str) -> dict:
    result = {
        "entity": artist_name,
        "kind": "artist",
        "mb_search_status": "ok",
        "mbid": None,
        "final_status": "unknown",
        "failure_reason": None,
        "disambiguation": None,
        "candidates_count": 0,
    }
    
    # 1. Search MusicBrainz artist
    query = f"artist:{quoted(artist_name)}"
    url = f"https://musicbrainz.org/ws/2/artist/?query={urllib.parse.quote(query)}&limit=100&fmt=json"
    
    data, code, err = http_get_json(url)
    if not data:
        result["mb_search_status"] = err
        result["final_status"] = "search_error"
        result["failure_reason"] = f"MB search failed: {err}"
        return result

    items = data.get("artists", [])
    count = data.get("count", 0)
    result["candidates_count"] = count

    # scrobbl-rs rule: count > items.len() => Ambiguous!
    if count > len(items):
        result["final_status"] = "ambiguous_rule_count_exceeded"
        result["failure_reason"] = f"Rule: count ({count}) > items.len ({len(items)})"
        return result

    # matching exact name
    matching = [
        it for it in items
        if normalized(it.get("name", "")) == normalized(artist_name)
    ]

    if len(matching) == 0:
        result["final_status"] = "missing_exact_name"
        result["failure_reason"] = "No candidate with exact matching name"
        return result

    candidate = matching[0]
    result["disambiguation"] = candidate.get("disambiguation", "")

    # scrobbl-rs rule: matching.len() != 1 => Ambiguous
    if len(matching) != 1:
        result["final_status"] = "ambiguous_multiple_exact_matches"
        result["failure_reason"] = f"Rule: multiple exact matches ({len(matching)})"
        return result

    # scrobbl-rs rule: score != 100 => Ambiguous
    if candidate.get("score") != 100:
        result["final_status"] = "ambiguous_score_not_100"
        result["failure_reason"] = f"Rule: score is {candidate.get('score')}, not 100"
        return result

    # scrobbl-rs rule: disambiguation not empty => Ambiguous!
    if candidate.get("disambiguation"):
        result["final_status"] = "ambiguous_has_disambiguation"
        result["failure_reason"] = f"Rule: candidate has disambiguation '{candidate.get('disambiguation')}'"
        return result

    mbid = candidate.get("id")
    result["mbid"] = mbid

    # 2. Check relations for image / wikidata
    time.sleep(1.0) # respect MB rate limit
    rel_url = f"https://musicbrainz.org/ws/2/artist/{mbid}?inc=url-rels&fmt=json"
    rel_data, rel_code, rel_err = http_get_json(rel_url)
    if not rel_data:
        result["final_status"] = "rel_fetch_error"
        result["failure_reason"] = f"MB url-rels failed: {rel_err}"
        return result

    relations = rel_data.get("relations", [])
    img_candidates = [r["url"]["resource"] for r in relations if r.get("type") == "image"]
    wiki_candidates = [r["url"]["resource"] for r in relations if r.get("type") == "wikidata"]

    if len(img_candidates) == 1:
        result["final_status"] = "found_mb_image"
        return result
    elif len(wiki_candidates) == 1:
        result["final_status"] = "has_wikidata"
        return result
    elif len(img_candidates) > 1 or len(wiki_candidates) > 1:
        result["final_status"] = "ambiguous_multiple_relations"
        result["failure_reason"] = "Multiple image or wikidata relations"
        return result
    else:
        result["final_status"] = "missing_no_image_relation"
        result["failure_reason"] = "No image or wikidata relation on MusicBrainz"
        return result

def test_album(album_title: str, artist_name: str) -> dict:
    result = {
        "entity": f"{artist_name} - {album_title}",
        "kind": "album",
        "mb_search_status": "ok",
        "mbid": None,
        "final_status": "unknown",
        "failure_reason": None,
        "disambiguation": None,
        "candidates_count": 0,
    }

    # 1. Search MusicBrainz release-group
    query = f"releasegroup:{quoted(album_title)} AND artist:{quoted(artist_name)}"
    url = f"https://musicbrainz.org/ws/2/release-group/?query={urllib.parse.quote(query)}&limit=100&fmt=json"

    data, code, err = http_get_json(url)
    if not data:
        result["mb_search_status"] = err
        result["final_status"] = "search_error"
        result["failure_reason"] = f"MB search failed: {err}"
        return result

    items = data.get("release-groups", [])
    count = data.get("count", 0)
    result["candidates_count"] = count

    # scrobbl-rs rule: count > items.len() => Ambiguous!
    if count > len(items):
        result["final_status"] = "ambiguous_rule_count_exceeded"
        result["failure_reason"] = f"Rule: count ({count}) > items.len ({len(items)})"
        return result

    matching = []
    for it in items:
        if normalized(it.get("title", "")) != normalized(album_title):
            continue
        credits = it.get("artist-credit", [])
        if len(credits) == 1 and normalized(credits[0].get("artist", {}).get("name", "")) == normalized(artist_name):
            matching.append(it)

    if len(matching) == 0:
        result["final_status"] = "missing_exact_name"
        result["failure_reason"] = "No candidate with exact matching title & single artist credit"
        return result

    candidate = matching[0]
    result["disambiguation"] = candidate.get("disambiguation", "")

    if len(matching) != 1:
        result["final_status"] = "ambiguous_multiple_exact_matches"
        result["failure_reason"] = f"Rule: multiple exact matches ({len(matching)})"
        return result

    if candidate.get("score") != 100:
        result["final_status"] = "ambiguous_score_not_100"
        result["failure_reason"] = f"Rule: score is {candidate.get('score')}, not 100"
        return result

    if candidate.get("disambiguation"):
        result["final_status"] = "ambiguous_has_disambiguation"
        result["failure_reason"] = f"Rule: candidate has disambiguation '{candidate.get('disambiguation')}'"
        return result

    mbid = candidate.get("id")
    result["mbid"] = mbid

    # 2. Check CoverArtArchive for release group
    time.sleep(0.5)
    caa_url = f"https://coverartarchive.org/release-group/{mbid}"
    caa_data, caa_code, caa_err = http_get_json(caa_url)
    if caa_code == 404:
        result["final_status"] = "caa_404_no_artwork"
        result["failure_reason"] = "Cover Art Archive has no artwork for this release group"
        return result
    if not caa_data:
        result["final_status"] = "caa_error"
        result["failure_reason"] = f"CAA error: {caa_err}"
        return result

    images = caa_data.get("images", [])
    front = [img for img in images if img.get("front") is True and img.get("approved") is True]
    if len(front) == 1:
        result["final_status"] = "found_caa_front"
        return result
    elif len(front) == 0:
        result["final_status"] = "caa_no_approved_front"
        result["failure_reason"] = f"CAA has images ({len(images)}) but 0 approved front covers"
        return result
    else:
        result["final_status"] = "ambiguous_multiple_front_covers"
        result["failure_reason"] = f"CAA has {len(front)} approved front covers (scrobbl-rs requires exactly 1)"
        return result

def main():
    db_path = sys.argv[1] if len(sys.argv) > 1 else "./data/scrobbl.db"
    print(f"=== Diagnosi Raccolta Artwork (Issue #134) ===")
    print(f"Database: {db_path}")

    conn = sqlite3.connect(db_path)
    cur = conn.cursor()

    # Sample top 10 artists by listens
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

    # Sample top 10 albums by listens
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
        time.sleep(1.0)
        res = test_artist(a)
        artist_results.append(res)
        print(f"[{res['final_status']}] {res['entity']} -> {res['failure_reason'] or 'OK'}")

    print(f"\n--- 2. Test Campione Album ({len(top_albums)}) ---")
    album_results = []
    for alb, art in top_albums:
        time.sleep(1.0)
        res = test_album(alb, art)
        album_results.append(res)
        print(f"[{res['final_status']}] {res['entity']} -> {res['failure_reason'] or 'OK'}")

    print("\n" + "="*60)
    print("=== RIEPILOGO STATISTICHE CAMPIONE ===")
    print("="*60)

    art_counter = Counter(r["final_status"] for r in artist_results)
    alb_counter = Counter(r["final_status"] for r in album_results)

    total_tested = len(artist_results) + len(album_results)
    total_failed = sum(1 for r in artist_results if not r["final_status"].startswith("found")) + \
                   sum(1 for r in album_results if not r["final_status"].startswith("found"))
    fail_rate = (total_failed / total_tested) * 100

    print(f"\nTotale testati: {total_tested}")
    print(f"Totale falliti: {total_failed} ({fail_rate:.1f}%)")

    if fail_rate > 10.0:
        print(f"\n[ROSSO]: Tasso di fallimento {fail_rate:.1f}% supera la soglia di tolleranza (10%)!")
        sys.exit(1)
    else:
        print("\n[VERDE]: Raccolta artwork soddisfacente.")
        sys.exit(0)

if __name__ == "__main__":
    main()
