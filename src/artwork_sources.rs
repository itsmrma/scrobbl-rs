//! Public image providers, matching rules and the manual image picker.
//!
//! Every provider answers one question for one [`Target`]: "do you have an
//! image for exactly this album or artist?". The resolver in `artwork.rs`
//! asks them in the owner's order. Providers never guess between homonyms:
//! when the evidence is not good enough the answer is `Missing` and the next
//! provider is asked.
use crate::{
    AppState,
    artwork_http::{self, build_url},
    artwork_settings::{self, ImageProvider},
    ask::names::fold,
    auth,
    error::{AppError, Result},
    library,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use std::time::Duration;

#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
pub struct ImageCredit {
    pub provider: String,
    pub source_url: String,
    pub image_url: String,
    pub author: Option<String>,
    pub license: Option<String>,
    pub license_url: Option<String>,
    pub credit: Option<String>,
}
pub(crate) struct FoundImage {
    pub bytes: Vec<u8>,
    pub credit: ImageCredit,
}
pub(crate) enum Lookup {
    Found(FoundImage),
    Missing,
}

/// What the providers search for: the library's own names, after local
/// corrections and name grouping.
#[derive(Clone, Debug, Default)]
pub(crate) struct Target {
    pub kind: String,
    pub name: String,
    /// For albums: the album artists (primary credits), most frequent first.
    pub artists: Vec<String>,
    /// MusicBrainz id when an import linked one.
    pub mbid: Option<String>,
}

pub(crate) trait ImageLookup: Sync {
    async fn lookup(&self, provider: ImageProvider, target: &Target) -> Result<Lookup>;
}
pub(crate) struct PublicSources;

const COPYRIGHT_NOTICE: &str = "Artwork may be copyrighted by its rights holders.";

// ---------------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------------

fn is_edition_keyword(content: &str) -> bool {
    let lower = content.trim().to_lowercase();
    if lower == "single" || lower == "ep" {
        return true;
    }
    [
        "deluxe",
        "remaster",
        "expanded",
        "edition",
        "anniversary",
        "bonus",
        "version",
        "special",
        "soundtrack",
        "ost",
        "score",
        "explicit",
        "clean",
    ]
    .iter()
    .any(|keyword| lower.contains(keyword))
}
fn strip_edition_suffix(s: &str) -> &str {
    let trimmed = s.trim();
    for (open, close) in [('(', ')'), ('[', ']')] {
        if trimmed.ends_with(close)
            && let Some(start) = trimmed.rfind(open)
            && is_edition_keyword(&trimmed[start + 1..trimmed.len() - 1])
        {
            return trimmed[..start].trim_end();
        }
    }
    for sep in [" - ", " – ", " — "] {
        if let Some((base, suffix)) = trimmed.rsplit_once(sep)
            && is_edition_keyword(suffix)
        {
            return base.trim_end();
        }
    }
    trimmed
}
/// `Map of the Soul : 7 (Deluxe Edition) [Remastered]` → `Map of the Soul : 7`.
pub(crate) fn normalize_album_title(title: &str) -> String {
    let mut current = title.trim();
    loop {
        let previous = current;
        current = strip_edition_suffix(current).trim();
        if current == previous || current.is_empty() {
            break;
        }
    }
    if current.is_empty() {
        title.trim().to_string()
    } else {
        current.to_string()
    }
}
/// Compare names ignoring case, accents, spacing and punctuation spacing.
fn key(value: &str) -> String {
    fold(value)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}
pub(crate) fn album_title_matches(candidate: &str, target: &str) -> bool {
    let (c, t) = (key(candidate), key(target));
    !t.is_empty()
        && (c == t || key(&normalize_album_title(candidate)) == key(&normalize_album_title(target)))
}
/// A credit like "BTS & Halsey" or "Ado, Vaundy" matches each of its artists.
pub(crate) fn artist_matches(candidate: &str, target: &str) -> bool {
    let t = key(target);
    if t.is_empty() {
        return false;
    }
    if key(candidate) == t {
        return true;
    }
    candidate
        .split([',', '&', '、', '×'])
        .flat_map(|part| part.split(" feat. "))
        .flat_map(|part| part.split(" and "))
        .any(|part| key(part) == t)
}
fn various(name: &str) -> bool {
    matches!(
        key(name).as_str(),
        "variousartists" | "various" | "soundtrack" | "artistesdivers" | "variartisti"
    )
}
fn mbid(s: &str) -> bool {
    s.len() == 36
        && s.chars().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}
pub(crate) fn secure_url(value: &str) -> Option<String> {
    let mut url = url::Url::parse(value).ok()?;
    // Cover Art Archive still returns legacy HTTP links.
    if url.scheme() == "http"
        && url.host_str().is_some_and(|h| {
            h == "coverartarchive.org" || h == "archive.org" || h.ends_with(".archive.org")
        })
    {
        let _ = url.set_port(None);
        let _ = url.set_scheme("https");
    }
    (url.scheme() == "https" && url.username().is_empty() && url.password().is_none())
        .then(|| url.into())
}
fn quoted(value: &str) -> String {
    let mut escaped = String::new();
    for c in value.chars() {
        if "+-!(){}[]^\"~*?:\\/|&".contains(c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    format!("\"{escaped}\"")
}

/// Choose the Deezer album whose title matches and whose artist is one of the
/// album artists (or any artist for a compilation).
pub(crate) fn pick_deezer_album<'a>(data: &'a Value, target: &Target) -> Option<&'a Value> {
    let items = data["data"].as_array()?;
    items.iter().find(|item| {
        let title_ok = item["title"]
            .as_str()
            .is_some_and(|title| album_title_matches(title, &target.name));
        let artist = item["artist"]["name"].as_str().unwrap_or_default();
        let artist_ok = target.artists.is_empty()
            || various(artist)
            || target.artists.iter().any(|a| artist_matches(artist, a));
        title_ok && artist_ok && item["cover_xl"].as_str().is_some()
    })
}
/// Deezer returns a grey silhouette for artists without a picture; its URL
/// has an empty image hash (`/images/artist//`).
fn deezer_placeholder(url: &str) -> bool {
    url.contains("/artist//") || url.ends_with("/artist/")
}
/// Exact-name Deezer artist; among homonyms the most followed one, which is
/// the one a listener's library almost always means.
pub(crate) fn pick_deezer_artist<'a>(data: &'a Value, name: &str) -> Option<&'a Value> {
    data["data"]
        .as_array()?
        .iter()
        .filter(|item| item["name"].as_str().is_some_and(|n| key(n) == key(name)))
        .filter(|item| {
            item["picture_xl"]
                .as_str()
                .is_some_and(|url| !deezer_placeholder(url))
        })
        .max_by_key(|item| item["nb_fan"].as_u64().unwrap_or(0))
}
pub(crate) fn pick_itunes_album<'a>(data: &'a Value, target: &Target) -> Option<&'a Value> {
    data["results"].as_array()?.iter().find(|item| {
        let title_ok = item["collectionName"]
            .as_str()
            .is_some_and(|title| album_title_matches(title, &target.name));
        let artist = item["artistName"].as_str().unwrap_or_default();
        let artist_ok = target.artists.is_empty()
            || various(artist)
            || target.artists.iter().any(|a| artist_matches(artist, a));
        title_ok && artist_ok && item["artworkUrl100"].as_str().is_some()
    })
}
pub(crate) fn upgrade_itunes_artwork_url(url: &str) -> String {
    url.replace("/100x100bb.", "/1000x1000bb.")
        .replace("/100x100.", "/1000x1000.")
}
/// The best MusicBrainz release group: score ≥ 90, matching title and artist.
pub(crate) fn pick_release_group<'a>(data: &'a Value, target: &Target) -> Option<&'a str> {
    data["release-groups"]
        .as_array()?
        .iter()
        .filter(|group| group["score"].as_u64().unwrap_or(0) >= 90)
        .find(|group| {
            let title_ok = group["title"]
                .as_str()
                .is_some_and(|title| album_title_matches(title, &target.name));
            let artist_ok = target.artists.is_empty()
                || group["artist-credit"].as_array().is_some_and(|credits| {
                    credits.iter().any(|credit| {
                        let name = credit["name"]
                            .as_str()
                            .or_else(|| credit["artist"]["name"].as_str())
                            .unwrap_or_default();
                        various(name) || target.artists.iter().any(|a| artist_matches(name, a))
                    })
                });
            title_ok && artist_ok
        })
        .and_then(|group| group["id"].as_str())
        .filter(|id| mbid(id))
}
/// A unique exact-name MusicBrainz artist with score 100, or nothing.
pub(crate) fn pick_mb_artist<'a>(data: &'a Value, name: &str) -> Option<&'a str> {
    let exact: Vec<_> = data["artists"]
        .as_array()?
        .iter()
        .filter(|artist| artist["name"].as_str().is_some_and(|n| key(n) == key(name)))
        .collect();
    match exact.as_slice() {
        [only] => only["id"].as_str().filter(|id| mbid(id)),
        // Homonyms: trust only a clear winner by search score.
        [first, second, ..]
            if first["score"].as_u64() == Some(100) && second["score"].as_u64() < Some(90) =>
        {
            first["id"].as_str().filter(|id| mbid(id))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Providers
// ---------------------------------------------------------------------------

async fn found(image_url: String, credit: ImageCredit) -> Result<Lookup> {
    Ok(match artwork_http::image(&image_url).await? {
        Some(bytes) => Lookup::Found(FoundImage { bytes, credit }),
        None => Lookup::Missing,
    })
}

async fn deezer_json(target: &str) -> Result<Option<Value>> {
    let data = artwork_http::json(target).await?;
    // Deezer reports its quota as `{"error":{"code":4,...}}` with HTTP 200.
    if let Some(error) = data.as_ref().and_then(|d| d.get("error")) {
        if error["code"].as_i64() == Some(4) {
            return Err(artwork_http::cool_down("api.deezer.com", Duration::from_secs(5)).await);
        }
        return Ok(None);
    }
    Ok(data)
}

async fn deezer(target: &Target) -> Result<Lookup> {
    if target.kind == "artists" {
        let Some(data) = deezer_json(&build_url(
            "https://api.deezer.com/search/artist",
            &[("q", &target.name), ("limit", "25")],
        ))
        .await?
        else {
            return Ok(Lookup::Missing);
        };
        let Some(item) = pick_deezer_artist(&data, &target.name) else {
            return Ok(Lookup::Missing);
        };
        let Some(image_url) = item["picture_xl"].as_str().and_then(secure_url) else {
            return Ok(Lookup::Missing);
        };
        let source_url = item["link"]
            .as_str()
            .and_then(secure_url)
            .unwrap_or_else(|| image_url.clone());
        return found(
            image_url.clone(),
            ImageCredit {
                provider: ImageProvider::Deezer.key().into(),
                source_url,
                image_url,
                author: None,
                license: Some(COPYRIGHT_NOTICE.into()),
                license_url: None,
                credit: Some("Deezer".into()),
            },
        )
        .await;
    }
    let title = normalize_album_title(&target.name);
    let mut queries = Vec::new();
    if let Some(artist) = target.artists.first() {
        // Deezer's advanced search is precise when the names are exact.
        queries.push(format!("artist:\"{artist}\" album:\"{title}\""));
        queries.push(format!("{title} {artist}"));
    }
    queries.push(title.clone());
    for query in queries {
        let Some(data) = deezer_json(&build_url(
            "https://api.deezer.com/search/album",
            &[("q", &query), ("limit", "25")],
        ))
        .await?
        else {
            continue;
        };
        let Some(item) = pick_deezer_album(&data, target) else {
            continue;
        };
        let Some(image_url) = item["cover_xl"].as_str().and_then(secure_url) else {
            continue;
        };
        let source_url = item["link"]
            .as_str()
            .and_then(secure_url)
            .unwrap_or_else(|| image_url.clone());
        return found(
            image_url.clone(),
            ImageCredit {
                provider: ImageProvider::Deezer.key().into(),
                source_url,
                image_url,
                author: None,
                license: Some(COPYRIGHT_NOTICE.into()),
                license_url: None,
                credit: Some("Deezer".into()),
            },
        )
        .await;
    }
    Ok(Lookup::Missing)
}

async fn itunes(target: &Target) -> Result<Lookup> {
    let title = normalize_album_title(&target.name);
    let term = match target.artists.first() {
        Some(artist) => format!("{title} {artist}"),
        None => title,
    };
    let Some(data) = artwork_http::json(&build_url(
        "https://itunes.apple.com/search",
        &[("term", &term), ("entity", "album"), ("limit", "25")],
    ))
    .await?
    else {
        return Ok(Lookup::Missing);
    };
    let Some(item) = pick_itunes_album(&data, target) else {
        return Ok(Lookup::Missing);
    };
    let Some(image_url) = item["artworkUrl100"]
        .as_str()
        .map(upgrade_itunes_artwork_url)
        .and_then(|url| secure_url(&url))
    else {
        return Ok(Lookup::Missing);
    };
    let source_url = item["collectionViewUrl"]
        .as_str()
        .and_then(secure_url)
        .unwrap_or_else(|| image_url.clone());
    found(
        image_url.clone(),
        ImageCredit {
            provider: ImageProvider::Itunes.key().into(),
            source_url,
            image_url,
            author: None,
            license: Some(COPYRIGHT_NOTICE.into()),
            license_url: None,
            credit: Some("Apple iTunes".into()),
        },
    )
    .await
}

async fn cover_art_archive(target: &Target) -> Result<Lookup> {
    let title = normalize_album_title(&target.name);
    let query = match target.artists.first() {
        Some(artist) => format!(
            "releasegroup:{} AND artist:{}",
            quoted(&title),
            quoted(artist)
        ),
        None => format!("releasegroup:{}", quoted(&title)),
    };
    let Some(data) = artwork_http::json(&build_url(
        "https://musicbrainz.org/ws/2/release-group/",
        &[("query", &query), ("limit", "10"), ("fmt", "json")],
    ))
    .await?
    else {
        return Ok(Lookup::Missing);
    };
    let Some(group) = pick_release_group(&data, target) else {
        return Ok(Lookup::Missing);
    };
    // The front-500 shortcut redirects straight to the image: one request.
    let image_url = format!("https://coverartarchive.org/release-group/{group}/front-500");
    found(
        image_url.clone(),
        ImageCredit {
            provider: ImageProvider::CoverArtArchive.key().into(),
            source_url: format!("https://musicbrainz.org/release-group/{group}"),
            image_url,
            author: None,
            license: Some(
                "Cover artwork may be copyrighted; metadata licensing does not license the image."
                    .into(),
            ),
            license_url: Some("https://musicbrainz.org/doc/Copyright_and_DMCA_Compliance".into()),
            credit: Some("Cover Art Archive / MusicBrainz".into()),
        },
    )
    .await
}

fn commons_file(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    if url.host_str()? != "commons.wikimedia.org" {
        return None;
    }
    let title = url.path().strip_prefix("/wiki/")?;
    let decoded =
        url::form_urlencoded::parse(format!("v={}", title.replace('+', "%2B")).as_bytes())
            .next()?
            .1
            .into_owned();
    decoded.starts_with("File:").then_some(decoded)
}
fn text(value: &Value, key: &str) -> Option<String> {
    value[key]["value"]
        .as_str()
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}
/// Commons image with its author and licence; files without them are skipped.
async fn commons(file: &str) -> Result<Lookup> {
    let Some(data) = artwork_http::json(&build_url(
        "https://commons.wikimedia.org/w/api.php",
        &[
            ("action", "query"),
            ("format", "json"),
            ("prop", "imageinfo"),
            ("titles", file),
            ("iiprop", "url|mime|extmetadata"),
            ("iiurlwidth", "800"),
            ("maxlag", "5"),
        ],
    ))
    .await?
    else {
        return Ok(Lookup::Missing);
    };
    let Some(info) = data["query"]["pages"].as_object().and_then(|pages| {
        pages
            .values()
            .find_map(|p| p["imageinfo"].as_array()?.first())
    }) else {
        return Ok(Lookup::Missing);
    };
    let meta = &info["extmetadata"];
    let (Some(author), Some(license), Some(source_url), Some(image_url)) = (
        text(meta, "Artist"),
        text(meta, "LicenseShortName"),
        info["descriptionurl"].as_str().and_then(secure_url),
        info["thumburl"]
            .as_str()
            .or_else(|| info["url"].as_str())
            .and_then(secure_url),
    ) else {
        return Ok(Lookup::Missing);
    };
    found(
        image_url.clone(),
        ImageCredit {
            provider: ImageProvider::Wikimedia.key().into(),
            source_url,
            image_url,
            author: Some(author),
            license: Some(license),
            license_url: text(meta, "LicenseUrl").and_then(|v| secure_url(&v)),
            credit: text(meta, "Credit"),
        },
    )
    .await
}

async fn wikimedia(target: &Target) -> Result<Lookup> {
    let artist = match &target.mbid {
        Some(id) => id.clone(),
        None => {
            let Some(data) = artwork_http::json(&build_url(
                "https://musicbrainz.org/ws/2/artist/",
                &[
                    ("query", &format!("artist:{}", quoted(&target.name))),
                    ("limit", "10"),
                    ("fmt", "json"),
                ],
            ))
            .await?
            else {
                return Ok(Lookup::Missing);
            };
            let Some(id) = pick_mb_artist(&data, &target.name) else {
                return Ok(Lookup::Missing);
            };
            id.to_owned()
        }
    };
    let Some(data) = artwork_http::json(&build_url(
        &format!("https://musicbrainz.org/ws/2/artist/{artist}"),
        &[("inc", "url-rels"), ("fmt", "json")],
    ))
    .await?
    else {
        return Ok(Lookup::Missing);
    };
    let relations = data["relations"].as_array().cloned().unwrap_or_default();
    let resource = |kind: &str| -> Vec<String> {
        relations
            .iter()
            .filter(|r| r["type"] == kind)
            .filter_map(|r| r["url"]["resource"].as_str().map(str::to_owned))
            .collect()
    };
    if let [image] = resource("image").as_slice()
        && let Some(file) = commons_file(image)
        && let Lookup::Found(image) = commons(&file).await?
    {
        return Ok(Lookup::Found(image));
    }
    let wikidata = resource("wikidata");
    let Some(qid) = wikidata
        .first()
        .and_then(|url| url::Url::parse(url).ok())
        .filter(|url| url.host_str() == Some("www.wikidata.org"))
        .and_then(|url| url.path().strip_prefix("/wiki/").map(str::to_owned))
        .filter(|q| {
            q.starts_with('Q') && q[1..].bytes().all(|c| c.is_ascii_digit()) && q.len() > 1
        })
    else {
        return Ok(Lookup::Missing);
    };
    let Some(data) = artwork_http::json(&format!(
        "https://www.wikidata.org/wiki/Special:EntityData/{qid}.json"
    ))
    .await?
    else {
        return Ok(Lookup::Missing);
    };
    let files: std::collections::BTreeSet<_> = data["entities"][&qid]["claims"]["P18"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|v| v["rank"] != "deprecated")
        .filter_map(|v| v["mainsnak"]["datavalue"]["value"].as_str())
        .collect();
    match files.first() {
        Some(file) if files.len() == 1 => commons(&format!("File:{file}")).await,
        _ => Ok(Lookup::Missing),
    }
}

impl ImageLookup for PublicSources {
    async fn lookup(&self, provider: ImageProvider, target: &Target) -> Result<Lookup> {
        if !provider.supports(&target.kind) {
            return Ok(Lookup::Missing);
        }
        match provider {
            ImageProvider::Deezer => deezer(target).await,
            ImageProvider::Itunes => itunes(target).await,
            ImageProvider::CoverArtArchive => cover_art_archive(target).await,
            ImageProvider::Wikimedia => wikimedia(target).await,
        }
    }
}

// ---------------------------------------------------------------------------
// Library side
// ---------------------------------------------------------------------------

/// Names to search for one artist or album group, after local corrections.
pub(crate) async fn target(
    conn: &mut sqlx::SqliteConnection,
    kind: &str,
    id: &str,
) -> Result<Option<Target>> {
    let (table, singular) = match kind {
        "artists" => ("artists", "artist"),
        "albums" => ("albums", "album"),
        _ => return Ok(None),
    };
    let name: Option<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "{} SELECT name FROM {table} WHERE id=?",
        library::METADATA
    )))
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(name) = name.filter(|n| !n.trim().is_empty()) else {
        return Ok(None);
    };
    let artists = if kind == "albums" {
        library::album_artists(&mut *conn, id)
            .await?
            .into_iter()
            .map(|artist| artist.name)
            .collect()
    } else {
        Vec::new()
    };
    let links: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT external_id FROM entity_links WHERE namespace='musicbrainz' AND kind=? AND entity_id IN (SELECT entity_id FROM name_group_members WHERE group_id=? UNION SELECT ?)",
    )
    .bind(singular)
    .bind(id)
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    let mbid = match links.as_slice() {
        [only] if mbid(only) => Some(only.clone()),
        _ => None,
    };
    Ok(Some(Target {
        kind: kind.into(),
        name,
        artists,
        mbid,
    }))
}

pub async fn credit(state: &AppState, kind: &str, id: &str) -> Result<Option<ImageCredit>> {
    let mut conn = state.read_db.acquire().await?;
    credit_in(&mut conn, kind, id).await
}
pub(crate) async fn credit_in(
    conn: &mut sqlx::SqliteConnection,
    kind: &str,
    id: &str,
) -> Result<Option<ImageCredit>> {
    let row = sqlx::query(
        "SELECT source,provenance_json FROM artwork WHERE kind=? AND entity_id=? AND bytes IS NOT NULL ORDER BY CASE source WHEN 'custom' THEN 0 ELSE 1 END LIMIT 1",
    )
    .bind(kind)
    .bind(id)
    .fetch_optional(conn)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let Some(value) = row.get::<Option<String>, _>("provenance_json") else {
        return Ok(None);
    };
    let mut credit: ImageCredit = serde_json::from_str(&value).map_err(|_| AppError::Internal)?;
    // Restored metadata is untrusted too: never publish executable links.
    credit.provider = row.get("source");
    credit.source_url = secure_url(&credit.source_url).ok_or(AppError::Internal)?;
    credit.image_url = secure_url(&credit.image_url).ok_or(AppError::Internal)?;
    credit.license_url = credit.license_url.and_then(|v| secure_url(&v));
    Ok(Some(credit))
}

// ---------------------------------------------------------------------------
// Manual picker
// ---------------------------------------------------------------------------

#[derive(Default, Deserialize)]
pub struct CandidateQuery {
    pub q: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtworkCandidate {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub provider: String,
    pub provider_label: String,
    pub preview_url: String,
    pub image_url: String,
    pub source_url: String,
    pub credit: ImageCredit,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtworkCandidatesResponse {
    pub target_kind: String,
    pub target_id: String,
    pub target_name: String,
    pub default_query: String,
    pub query: String,
    pub current_image_url: Option<String>,
    pub current_provider: Option<String>,
    pub candidates: Vec<ArtworkCandidate>,
    pub warnings: Vec<String>,
}

fn candidate(
    provider: ImageProvider,
    id: String,
    title: String,
    subtitle: String,
    preview_url: String,
    image_url: String,
    source_url: String,
) -> ArtworkCandidate {
    ArtworkCandidate {
        id,
        title,
        subtitle,
        provider: provider.key().into(),
        provider_label: provider.label().into(),
        preview_url,
        image_url: image_url.clone(),
        source_url: source_url.clone(),
        credit: ImageCredit {
            provider: provider.key().into(),
            source_url,
            image_url,
            author: None,
            license: Some(COPYRIGHT_NOTICE.into()),
            license_url: None,
            credit: Some(provider.label().into()),
        },
    }
}

async fn search_candidates(
    provider: ImageProvider,
    kind: &str,
    query: &str,
) -> Result<Vec<ArtworkCandidate>> {
    let mut out = Vec::new();
    match (kind, provider) {
        ("albums", ImageProvider::Deezer) => {
            let data = deezer_json(&build_url(
                "https://api.deezer.com/search/album",
                &[("q", query), ("limit", "12")],
            ))
            .await?;
            for (i, item) in data
                .as_ref()
                .and_then(|d| d["data"].as_array())
                .into_iter()
                .flatten()
                .enumerate()
            {
                let (Some(image), Some(preview)) = (
                    item["cover_xl"].as_str().and_then(secure_url),
                    item["cover_medium"].as_str().and_then(secure_url),
                ) else {
                    continue;
                };
                let source = item["link"]
                    .as_str()
                    .and_then(secure_url)
                    .unwrap_or_else(|| image.clone());
                out.push(candidate(
                    provider,
                    format!("deezer-album-{i}"),
                    item["title"].as_str().unwrap_or_default().into(),
                    item["artist"]["name"].as_str().unwrap_or_default().into(),
                    preview,
                    image,
                    source,
                ));
            }
        }
        ("artists", ImageProvider::Deezer) => {
            let data = deezer_json(&build_url(
                "https://api.deezer.com/search/artist",
                &[("q", query), ("limit", "12")],
            ))
            .await?;
            for (i, item) in data
                .as_ref()
                .and_then(|d| d["data"].as_array())
                .into_iter()
                .flatten()
                .enumerate()
            {
                let (Some(image), Some(preview)) = (
                    item["picture_xl"]
                        .as_str()
                        .filter(|u| !deezer_placeholder(u))
                        .and_then(secure_url),
                    item["picture_medium"].as_str().and_then(secure_url),
                ) else {
                    continue;
                };
                let source = item["link"]
                    .as_str()
                    .and_then(secure_url)
                    .unwrap_or_else(|| image.clone());
                let fans = item["nb_fan"].as_u64().unwrap_or(0);
                out.push(candidate(
                    provider,
                    format!("deezer-artist-{i}"),
                    item["name"].as_str().unwrap_or_default().into(),
                    format!("{fans} fans"),
                    preview,
                    image,
                    source,
                ));
            }
        }
        ("albums", ImageProvider::Itunes) => {
            let data = artwork_http::json(&build_url(
                "https://itunes.apple.com/search",
                &[("term", query), ("entity", "album"), ("limit", "12")],
            ))
            .await?;
            for (i, item) in data
                .as_ref()
                .and_then(|d| d["results"].as_array())
                .into_iter()
                .flatten()
                .enumerate()
            {
                let Some(raw) = item["artworkUrl100"].as_str() else {
                    continue;
                };
                let (Some(image), Some(preview)) = (
                    secure_url(&upgrade_itunes_artwork_url(raw)),
                    secure_url(&raw.replace("/100x100bb.", "/300x300bb.")),
                ) else {
                    continue;
                };
                let source = item["collectionViewUrl"]
                    .as_str()
                    .and_then(secure_url)
                    .unwrap_or_else(|| image.clone());
                let year = item["releaseDate"]
                    .as_str()
                    .and_then(|d| d.get(..4))
                    .unwrap_or("");
                let artist = item["artistName"].as_str().unwrap_or_default();
                out.push(candidate(
                    provider,
                    format!("itunes-album-{i}"),
                    item["collectionName"].as_str().unwrap_or_default().into(),
                    if year.is_empty() {
                        artist.into()
                    } else {
                        format!("{artist} · {year}")
                    },
                    preview,
                    image,
                    source,
                ));
            }
        }
        ("albums", ImageProvider::CoverArtArchive) => {
            let data = artwork_http::json(&build_url(
                "https://musicbrainz.org/ws/2/release-group/",
                &[("query", query), ("limit", "6"), ("fmt", "json")],
            ))
            .await?;
            for group in data
                .as_ref()
                .and_then(|d| d["release-groups"].as_array())
                .into_iter()
                .flatten()
            {
                let Some(id) = group["id"].as_str().filter(|id| mbid(id)) else {
                    continue;
                };
                let artist = group["artist-credit"]
                    .as_array()
                    .and_then(|credits| credits.first())
                    .and_then(|c| c["name"].as_str())
                    .unwrap_or("MusicBrainz");
                out.push(candidate(
                    provider,
                    format!("caa-{id}"),
                    group["title"].as_str().unwrap_or_default().into(),
                    artist.into(),
                    format!("https://coverartarchive.org/release-group/{id}/front-250"),
                    format!("https://coverartarchive.org/release-group/{id}/front-500"),
                    format!("https://musicbrainz.org/release-group/{id}"),
                ));
            }
        }
        _ => {}
    }
    Ok(out)
}

pub async fn candidates(
    axum::extract::State(state): axum::extract::State<AppState>,
    headers: axum::http::HeaderMap,
    axum::extract::Path((kind, id)): axum::extract::Path<(String, String)>,
    axum::extract::Query(params): axum::extract::Query<CandidateQuery>,
) -> Result<axum::Json<ArtworkCandidatesResponse>> {
    auth::session(&state, &headers).await?;
    let mut conn = state.read_db.acquire().await?;
    // A track's artwork is its album's.
    let (target_kind, target_id) = match kind.as_str() {
        "artists" | "albums" => (
            kind.clone(),
            crate::name_groups::canonical(&mut conn, &kind, &id).await?,
        ),
        "tracks" => {
            let album: Option<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "{} SELECT album_id FROM tracks WHERE id=?",
                library::METADATA
            )))
            .bind(&id)
            .fetch_optional(&mut *conn)
            .await?
            .flatten();
            let album = album.ok_or_else(|| {
                AppError::BadRequest(
                    "This track is not linked to an album for artwork storage.".into(),
                )
            })?;
            ("albums".to_string(), album)
        }
        _ => {
            return Err(AppError::BadRequest(
                "Choose artists, albums or tracks.".into(),
            ));
        }
    };
    let found = target(&mut conn, &target_kind, &target_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let current_credit = credit_in(&mut conn, &target_kind, &target_id)
        .await
        .ok()
        .flatten();
    let current_source: Option<String> = sqlx::query_scalar(
        "SELECT source FROM artwork WHERE kind=? AND entity_id=? AND bytes IS NOT NULL ORDER BY CASE source WHEN 'custom' THEN 0 ELSE 1 END LIMIT 1",
    )
    .bind(&target_kind)
    .bind(&target_id)
    .fetch_optional(&mut *conn)
    .await
    .ok()
    .flatten();
    let settings = artwork_settings::read(&mut conn).await?;
    drop(conn);
    let default_query = match found.artists.first() {
        Some(artist) => format!("{} {artist}", normalize_album_title(&found.name)),
        None => found.name.clone(),
    };
    let query = params
        .q
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(200).collect::<String>())
        .unwrap_or_else(|| default_query.clone());

    let enabled: Vec<ImageProvider> = settings.enabled(&target_kind).collect();
    let mut warnings = Vec::new();
    if enabled.is_empty() {
        warnings.push("All image providers are disabled in Settings → Image providers.".into());
    }
    // Search every provider at once; each keeps its own pace.
    let results = futures_util::future::join_all(
        enabled
            .iter()
            .map(|provider| search_candidates(*provider, &target_kind, &query)),
    )
    .await;
    let mut seen = std::collections::BTreeSet::new();
    let mut candidates = Vec::new();
    for (provider, result) in enabled.iter().zip(results) {
        match result {
            Ok(list) => candidates.extend(
                list.into_iter()
                    .filter(|candidate| seen.insert(candidate.image_url.clone())),
            ),
            Err(_) => warnings.push(format!("{} is temporarily unavailable.", provider.label())),
        }
    }
    Ok(axum::Json(ArtworkCandidatesResponse {
        target_kind,
        target_id,
        target_name: found.name,
        default_query,
        query,
        current_image_url: current_credit.as_ref().map(|c| c.image_url.clone()),
        current_provider: current_source.or_else(|| current_credit.map(|c| c.provider)),
        candidates,
        warnings,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn album(name: &str, artists: &[&str]) -> Target {
        Target {
            kind: "albums".into(),
            name: name.into(),
            artists: artists.iter().map(|a| a.to_string()).collect(),
            mbid: None,
        }
    }

    #[test]
    fn edition_suffixes_and_spacing_do_not_prevent_a_match() {
        assert_eq!(
            normalize_album_title("Map of the Soul : 7 (Deluxe Edition) [Remastered]"),
            "Map of the Soul : 7"
        );
        assert!(album_title_matches(
            "MAP OF THE SOUL : 7",
            "Map of the Soul: 7"
        ));
        assert!(album_title_matches("Kyōgen", "Kyogen"));
        assert!(album_title_matches("狂言", "狂言 (Deluxe)"));
        assert!(!album_title_matches("Ado's Best Adobum", "Kyōgen"));
        assert!(!album_title_matches("", ""));
    }

    #[test]
    fn credits_with_several_artists_match_each_of_them() {
        assert!(artist_matches("BTS & Halsey", "BTS"));
        assert!(artist_matches("Ado, Vaundy", "Vaundy"));
        assert!(artist_matches("Beyoncé", "Beyonce"));
        assert!(!artist_matches("BTS", "Halsey"));
    }

    #[test]
    fn deezer_album_needs_title_and_album_artist() {
        let data = json!({"data":[
            {"title":"Map of the Soul : 7","artist":{"name":"Someone Else"},"cover_xl":"https://e-cdns-images.dzcdn.net/a.jpg"},
            {"title":"MAP OF THE SOUL : 7","artist":{"name":"BTS"},"cover_xl":"https://e-cdns-images.dzcdn.net/b.jpg"}
        ]});
        let picked = pick_deezer_album(&data, &album("MAP OF THE SOUL : 7", &["BTS"])).unwrap();
        assert_eq!(picked["cover_xl"], "https://e-cdns-images.dzcdn.net/b.jpg");
        assert!(pick_deezer_album(&data, &album("Other", &["BTS"])).is_none());
        let compilation = json!({"data":[{"title":"Now 100","artist":{"name":"Various Artists"},"cover_xl":"https://x.test/c.jpg"}]});
        assert!(pick_deezer_album(&compilation, &album("Now 100", &["Ado", "BTS"])).is_some());
    }

    #[test]
    fn deezer_artist_prefers_the_most_followed_homonym_and_skips_placeholders() {
        let data = json!({"data":[
            {"name":"Ado","nb_fan":120,"picture_xl":"https://e-cdns-images.dzcdn.net/images/artist/aaa/1000x1000.jpg"},
            {"name":"Ado","nb_fan":900000,"picture_xl":"https://e-cdns-images.dzcdn.net/images/artist/bbb/1000x1000.jpg"},
            {"name":"Ado","nb_fan":9000000,"picture_xl":"https://e-cdns-images.dzcdn.net/images/artist//1000x1000.jpg"},
            {"name":"Adore","nb_fan":99999999,"picture_xl":"https://e-cdns-images.dzcdn.net/images/artist/ccc/1000x1000.jpg"}
        ]});
        let picked = pick_deezer_artist(&data, "ado").unwrap();
        assert_eq!(picked["nb_fan"], 900000);
        assert!(pick_deezer_artist(&json!({"data":[]}), "Ado").is_none());
    }

    #[test]
    fn itunes_upgrades_resolution_and_matches_romaji_editions() {
        assert_eq!(
            upgrade_itunes_artwork_url("https://is1-ssl.mzstatic.com/x/100x100bb.jpg"),
            "https://is1-ssl.mzstatic.com/x/1000x1000bb.jpg"
        );
        let data = json!({"results":[{"collectionName":"Kyougen (Deluxe Edition)","artistName":"Ado","artworkUrl100":"https://is1-ssl.mzstatic.com/x/100x100bb.jpg"}]});
        assert!(pick_itunes_album(&data, &album("Kyougen", &["Ado"])).is_some());
        assert!(pick_itunes_album(&data, &album("Kyougen", &["Vaundy"])).is_none());
    }

    #[test]
    fn musicbrainz_requires_a_confident_unique_answer() {
        let groups = json!({"release-groups":[
            {"id":"b10bbbfc-cf9e-42e0-be17-e2c3e1d2600d","score":100,"title":"Abbey Road","artist-credit":[{"name":"The Beatles"}]}
        ]});
        assert!(pick_release_group(&groups, &album("Abbey Road", &["The Beatles"])).is_some());
        assert!(pick_release_group(&groups, &album("Abbey Road", &["Someone"])).is_none());
        let homonyms = json!({"artists":[
            {"id":"b10bbbfc-cf9e-42e0-be17-e2c3e1d2600d","score":100,"name":"Nina"},
            {"id":"c10bbbfc-cf9e-42e0-be17-e2c3e1d2600d","score":100,"name":"Nina"}
        ]});
        assert!(pick_mb_artist(&homonyms, "Nina").is_none());
        let clear = json!({"artists":[
            {"id":"b10bbbfc-cf9e-42e0-be17-e2c3e1d2600d","score":100,"name":"Björk"},
            {"id":"c10bbbfc-cf9e-42e0-be17-e2c3e1d2600d","score":40,"name":"Bjork"}
        ]});
        assert!(pick_mb_artist(&clear, "Björk").is_some());
    }

    #[test]
    fn secure_urls_upgrade_archive_links_and_reject_others() {
        assert_eq!(
            secure_url("http://coverartarchive.org/release/x/front.jpg").as_deref(),
            Some("https://coverartarchive.org/release/x/front.jpg")
        );
        assert!(secure_url("http://example.com/a.jpg").is_none());
        assert!(secure_url("https://user:pw@example.com/a.jpg").is_none());
    }
}
