//! Public MusicBrainz paths and ordered lookups, shared by browser and worker.
use crate::{
    AppState,
    artwork_settings::{self, ArtworkSettings, ImageProvider},
    auth,
    error::{AppError, Result},
    library,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;

const DAY: i64 = 86_400_000;
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
    Ambiguous,
}
pub(crate) trait ImageLookup {
    async fn lookup(&self, provider: ImageProvider, kind: &str, id: &str) -> Result<Lookup>;
}
pub(crate) struct PublicSources<'a>(pub &'a AppState);

fn url(base: &str, pairs: &[(&str, &str)]) -> String {
    let mut url = url::Url::parse(base).expect("constant provider URL");
    url.query_pairs_mut().extend_pairs(pairs.iter().copied());
    url.into()
}
async fn json(state: &AppState, target: &str) -> Result<Option<Value>> {
    let mut response = library::public_response(target, Some(state)).await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(request_failed(
            state,
            url::Url::parse(target)
                .map_err(|_| AppError::Internal)?
                .host_str()
                .ok_or(AppError::Internal)?,
            None,
        )
        .await?);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| AppError::Internal)? {
        if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err(AppError::BadRequest(
                "Image provider response is too large.".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::BadRequest("Image provider returned an invalid response.".into()))?;
    if value.get("error").is_some() {
        return Err(request_failed(
            state,
            url::Url::parse(target)
                .map_err(|_| AppError::Internal)?
                .host_str()
                .ok_or(AppError::Internal)?,
            None,
        )
        .await?);
    }
    Ok(Some(value))
}
pub(crate) fn retry_backoff_ms(attempts: i64) -> i64 {
    30_000_i64
        .saturating_mul(1_i64 << attempts.clamp(0, 7))
        .min(3_600_000)
}

// The single image slot serializes callers; reservations and cooldowns survive restarts.
pub(crate) async fn request_slot(state: &AppState, host: &str) -> Result<()> {
    let mut tx = state.db.begin().await?;
    let retry: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key=?")
        .bind(format!("artwork_host_retry:{host}"))
        .fetch_optional(&mut *tx)
        .await?;
    let now = auth::now_ms();
    if let Some(at) = retry
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|at| *at > now)
    {
        return Err(AppError::ArtworkWait { retry_at_ms: at });
    }
    if host == "musicbrainz.org" {
        let next: Option<String> =
            sqlx::query_scalar("SELECT value FROM settings WHERE key='musicbrainz_next_at_ms'")
                .fetch_optional(&mut *tx)
                .await?;
        let at = now.max(next.and_then(|v| v.parse::<i64>().ok()).unwrap_or(0));
        crate::db::set_setting(&mut tx, "musicbrainz_next_at_ms", &(at + 1100).to_string()).await?;
        tx.commit().await?;
        if at > now {
            tokio::time::sleep(std::time::Duration::from_millis((at - now) as u64)).await;
        }
    }
    Ok(())
}

pub(crate) async fn request_failed(
    state: &AppState,
    host: &str,
    at: Option<i64>,
) -> Result<AppError> {
    let mut tx = state.db.begin().await?;
    let key = format!("artwork_host_failures:{host}");
    let attempts: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key=?")
        .bind(&key)
        .fetch_optional(&mut *tx)
        .await?;
    let attempts = attempts
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0)
        .clamp(0, 7);
    let retry = at.unwrap_or_else(|| auth::now_ms().saturating_add(retry_backoff_ms(attempts)));
    crate::db::set_setting(&mut tx, &key, &(attempts + 1).to_string()).await?;
    let key = format!("artwork_host_retry:{host}");
    let previous: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key=?")
        .bind(&key)
        .fetch_optional(&mut *tx)
        .await?;
    let retry = retry.max(previous.and_then(|v| v.parse::<i64>().ok()).unwrap_or(0));
    crate::db::set_setting(&mut tx, &key, &retry.to_string()).await?;
    tx.commit().await?;
    Ok(AppError::ArtworkWait { retry_at_ms: retry })
}
pub(crate) async fn request_succeeded(state: &AppState, host: &str) -> Result<()> {
    sqlx::query("DELETE FROM settings WHERE key IN (?,?)")
        .bind(format!("artwork_host_retry:{host}"))
        .bind(format!("artwork_host_failures:{host}"))
        .execute(&state.db)
        .await?;
    Ok(())
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
fn normalized(s: &str) -> String {
    s.trim().to_lowercase()
}
pub(crate) fn normalize_album_title(title: &str) -> String {
    let mut current = title.trim();
    loop {
        let prev = current;
        current = strip_edition_suffix(current).trim();
        if current == prev || current.is_empty() {
            break;
        }
    }
    if current.is_empty() {
        title.trim().to_string()
    } else {
        current.to_string()
    }
}
fn is_edition_keyword(content: &str) -> bool {
    let lower = content.trim().to_lowercase();
    if lower == "single" || lower == "ep" {
        return true;
    }
    lower.contains("deluxe")
        || lower.contains("remaster")
        || lower.contains("expanded")
        || lower.contains("edition")
        || lower.contains("anniversary")
        || lower.contains("bonus")
        || lower.contains("version")
        || lower.contains("special")
}
fn strip_edition_suffix(s: &str) -> &str {
    let trimmed = s.trim();
    if trimmed.ends_with(')') {
        if let Some(open) = trimmed.rfind('(') {
            let inner = &trimmed[open + 1..trimmed.len() - 1];
            if is_edition_keyword(inner) {
                return trimmed[..open].trim_end();
            }
        }
    }
    if trimmed.ends_with(']') {
        if let Some(open) = trimmed.rfind('[') {
            let inner = &trimmed[open + 1..trimmed.len() - 1];
            if is_edition_keyword(inner) {
                return trimmed[..open].trim_end();
            }
        }
    }
    for sep in [" - ", " – ", " — "] {
        if let Some((base, suffix)) = trimmed.rsplit_once(sep) {
            if is_edition_keyword(suffix) {
                return base.trim_end();
            }
        }
    }
    trimmed
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
fn unique_match<'a>(
    data: &'a Value,
    kind: &str,
    name: &str,
    artist: Option<&str>,
) -> std::result::Result<Option<&'a str>, ()> {
    let key = if kind == "artists" {
        "artists"
    } else {
        "release-groups"
    };
    let items = data[key].as_array().ok_or(())?;
    let _count = data["count"].as_u64().ok_or(())?;
    let matching: Vec<_> = items
        .iter()
        .filter(|item| {
            let title_match = if kind == "artists" {
                item["name"]
                    .as_str()
                    .is_some_and(|v| normalized(v) == normalized(name))
            } else {
                item["title"].as_str().is_some_and(|v| {
                    normalized(v) == normalized(name)
                        || normalized(&normalize_album_title(v))
                            == normalized(&normalize_album_title(name))
                })
            };
            title_match
                && artist.is_none_or(|artist| {
                    item["artist-credit"].as_array().is_some_and(|credits| {
                        credits.iter().any(|c| {
                            c["artist"]["name"]
                                .as_str()
                                .is_some_and(|v| normalized(v) == normalized(artist))
                                || c["name"]
                                    .as_str()
                                    .is_some_and(|v| normalized(v) == normalized(artist))
                        })
                    })
                })
        })
        .collect();
    if matching.is_empty() {
        return Ok(None);
    }
    if matching.len() != 1 || matching[0]["score"].as_u64() != Some(100) {
        return Err(());
    }
    matching[0]["id"]
        .as_str()
        .filter(|id| mbid(id))
        .map(Some)
        .ok_or(())
}
async fn entity_name_and_artists(
    state: &AppState,
    kind: &str,
    id: &str,
) -> Result<std::result::Result<(String, Vec<String>), ()>> {
    let table = if kind == "artists" {
        "artists"
    } else {
        "albums"
    };
    // A group deliberately separated from an identically named entity is still ambiguous externally.
    let names: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT DISTINCT e.name FROM main.{table} e JOIN name_group_members gm ON gm.entity_id=e.id WHERE gm.group_id=?"
    )))
    .bind(id)
    .fetch_all(&state.read_db)
    .await?;
    if names.len() != 1 {
        return Ok(Err(()));
    }
    let name = names.into_iter().next().unwrap();
    let artists: Vec<String> = if kind == "albums" {
        sqlx::query_scalar(
            "SELECT DISTINCT a.name FROM main.artists a JOIN track_artists ta ON ta.artist_id=a.id JOIN tracks t ON t.id=ta.track_id JOIN name_group_members gm ON gm.entity_id=t.album_id WHERE gm.group_id=?"
        )
        .bind(id)
        .fetch_all(&state.read_db)
        .await?
    } else {
        vec![]
    };
    if kind == "albums" && artists.len() != 1 {
        return Ok(Err(()));
    }
    Ok(Ok((name, artists)))
}

async fn identity(
    state: &AppState,
    kind: &str,
    id: &str,
) -> Result<std::result::Result<Option<String>, ()>> {
    let singular = if kind == "artists" { "artist" } else { "album" };
    let known: Vec<String> = sqlx::query_scalar("SELECT DISTINCT external_id FROM entity_links WHERE namespace='musicbrainz' AND kind=? AND entity_id IN (SELECT entity_id FROM name_group_members WHERE group_id=?)").bind(singular).bind(id).fetch_all(&state.read_db).await?;
    if known.len() == 1 && mbid(&known[0]) {
        return Ok(Ok(Some(known[0].clone())));
    }
    if !known.is_empty() {
        return Ok(Err(()));
    }
    let (name, artists) = match entity_name_and_artists(state, kind, id).await? {
        Ok(res) => res,
        Err(()) => return Ok(Err(())),
    };
    let query = if kind == "artists" {
        format!("artist:{}", quoted(&name))
    } else {
        let clean_title = normalize_album_title(&name);
        format!(
            "releasegroup:{} AND artist:{}",
            quoted(&clean_title),
            quoted(&artists[0])
        )
    };
    let endpoint = if kind == "artists" {
        "artist"
    } else {
        "release-group"
    };
    let target = url(
        &format!("https://musicbrainz.org/ws/2/{endpoint}/"),
        &[("query", &query), ("limit", "100"), ("fmt", "json")],
    );
    let Some(data) = json(state, &target).await? else {
        return Ok(Ok(None));
    };
    let list = if kind == "artists" {
        "artists"
    } else {
        "release-groups"
    };
    if data[list].as_array().is_none() || data["count"].as_u64().is_none() {
        return Err(AppError::BadRequest(
            "MusicBrainz returned an invalid search response.".into(),
        ));
    }
    Ok(
        unique_match(&data, kind, &name, artists.first().map(String::as_str))
            .map(|v| v.map(str::to_owned)),
    )
}
pub(crate) fn caa_front_image_url(data: &Value) -> Result<Option<String>> {
    let images = data["images"].as_array().ok_or(AppError::Internal)?;
    let front: Vec<_> = images
        .iter()
        .filter(|v| v["front"] == true && v["approved"] == true)
        .collect();
    let Some(first_front) = front.first() else {
        return Ok(None);
    };
    let Some(image_url) = first_front["thumbnails"]["500"]
        .as_str()
        .or_else(|| first_front["image"].as_str())
        .and_then(secure_url)
    else {
        return Ok(None);
    };
    Ok(Some(image_url))
}
fn secure_url(value: &str) -> Option<String> {
    let url = library::archive_https(url::Url::parse(value).ok()?);
    (url.scheme() == "https" && url.username().is_empty() && url.password().is_none())
        .then(|| url.into())
}
fn commons_file(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    if url.host_str()? != "commons.wikimedia.org" {
        return None;
    }
    let title = url.path().strip_prefix("/wiki/")?;
    // Decode using the URL parser rather than letting an encoded title turn into a second query.
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
async fn commons(state: &AppState, file: &str, provider: ImageProvider) -> Result<Lookup> {
    let target = url(
        "https://commons.wikimedia.org/w/api.php",
        &[
            ("action", "query"),
            ("format", "json"),
            ("prop", "imageinfo"),
            ("titles", file),
            ("iiprop", "url|mime|extmetadata"),
            ("iiurlwidth", "500"),
            ("maxlag", "5"),
        ],
    );
    let Some(data) = json(state, &target).await? else {
        return Ok(Lookup::Missing);
    };
    let pages = data["query"]["pages"]
        .as_object()
        .ok_or(AppError::Internal)?;
    let info = pages
        .values()
        .find_map(|p| p["imageinfo"].as_array().and_then(|i| i.first()));
    let Some(info) = info else {
        return Ok(Lookup::Missing);
    };
    let meta = &info["extmetadata"];
    let Some(author) = text(meta, "Artist") else {
        return Ok(Lookup::Missing);
    };
    let Some(license) = text(meta, "LicenseShortName") else {
        return Ok(Lookup::Missing);
    };
    let Some(source_url) = info["descriptionurl"].as_str().and_then(secure_url) else {
        return Ok(Lookup::Missing);
    };
    let Some(image_url) = info["thumburl"]
        .as_str()
        .or_else(|| info["url"].as_str())
        .and_then(secure_url)
    else {
        return Ok(Lookup::Missing);
    };
    let credit = ImageCredit {
        provider: provider.key().into(),
        source_url,
        image_url: image_url.clone(),
        author: Some(author),
        license: Some(license),
        license_url: text(meta, "LicenseUrl").and_then(|v| secure_url(&v)),
        credit: text(meta, "Credit"),
    };
    Ok(Lookup::Found(FoundImage {
        bytes: library::download(&image_url, Some(state)).await?,
        credit,
    }))
}
pub(crate) fn album_title_matches(candidate_title: &str, target_title: &str) -> bool {
    let c = normalized(candidate_title);
    let t = normalized(target_title);
    c == t
        || normalized(&normalize_album_title(candidate_title))
            == normalized(&normalize_album_title(target_title))
}

pub(crate) fn artist_matches(candidate_artist: &str, target_artist: &str) -> bool {
    let c = normalized(candidate_artist);
    let t = normalized(target_artist);
    c == t
        || (c.contains(" & ") && c.split(" & ").any(|part| part.trim() == t))
        || (c.contains(", ") && c.split(", ").any(|part| part.trim() == t))
        || (t.contains(" & ") && t.split(" & ").any(|part| part.trim() == c))
        || (t.contains(", ") && t.split(", ").any(|part| part.trim() == c))
}

pub(crate) fn upgrade_itunes_artwork_url(url: &str) -> String {
    if url.contains("/100x100bb.") {
        url.replace("/100x100bb.", "/600x600bb.")
    } else if url.contains("/100x100.") {
        url.replace("/100x100.", "/600x600.")
    } else {
        url.to_string()
    }
}

pub(crate) struct ItunesAlbumMatch<'a> {
    pub artwork_url: &'a str,
    pub view_url: Option<&'a str>,
}

pub(crate) struct DeezerArtistMatch<'a> {
    pub picture_url: &'a str,
    pub link_url: Option<&'a str>,
}

pub(crate) fn find_itunes_album_candidate<'a>(
    data: &'a Value,
    target_title: &str,
    target_artist: &str,
) -> Option<ItunesAlbumMatch<'a>> {
    let results = data["results"].as_array()?;
    for item in results {
        let Some(c_title) = item["collectionName"].as_str() else {
            continue;
        };
        let Some(c_artist) = item["artistName"].as_str() else {
            continue;
        };
        if album_title_matches(c_title, target_title) && artist_matches(c_artist, target_artist) {
            let Some(artwork_url) = item["artworkUrl100"].as_str() else {
                continue;
            };
            let view_url = item["collectionViewUrl"]
                .as_str()
                .or_else(|| item["trackViewUrl"].as_str());
            return Some(ItunesAlbumMatch {
                artwork_url,
                view_url,
            });
        }
    }
    None
}

pub(crate) fn find_deezer_artist_candidate<'a>(
    data: &'a Value,
    target_artist: &str,
) -> Option<DeezerArtistMatch<'a>> {
    let results = data["data"].as_array()?;
    for item in results {
        let Some(c_name) = item["name"].as_str() else {
            continue;
        };
        if normalized(c_name) == normalized(target_artist) {
            let Some(picture_url) = item["picture_xl"]
                .as_str()
                .or_else(|| item["picture_big"].as_str())
                .or_else(|| item["picture_medium"].as_str())
                .or_else(|| item["picture"].as_str())
            else {
                continue;
            };
            let link_url = item["link"].as_str();
            return Some(DeezerArtistMatch {
                picture_url,
                link_url,
            });
        }
    }
    None
}

impl ImageLookup for PublicSources<'_> {
    async fn lookup(&self, provider: ImageProvider, kind: &str, id: &str) -> Result<Lookup> {
        let state = self.0;
        if provider == ImageProvider::Itunes {
            if kind == "albums" {
                let (name, artists) = match entity_name_and_artists(state, kind, id).await? {
                    Ok(res) => res,
                    Err(()) => return Ok(Lookup::Ambiguous),
                };
                if artists.is_empty() {
                    return Ok(Lookup::Ambiguous);
                }
                let artist = &artists[0];
                let clean_title = normalize_album_title(&name);
                let term = format!("{clean_title} {artist}");
                let target = url(
                    "https://itunes.apple.com/search",
                    &[("term", &term), ("entity", "album"), ("limit", "25")],
                );
                let Some(data) = json(state, &target).await? else {
                    return Ok(Lookup::Missing);
                };
                let Some(candidate) = find_itunes_album_candidate(&data, &name, artist) else {
                    return Ok(Lookup::Missing);
                };
                let upgraded = upgrade_itunes_artwork_url(candidate.artwork_url);
                let Some(image_url) = secure_url(&upgraded) else {
                    return Ok(Lookup::Missing);
                };
                let source_url = candidate
                    .view_url
                    .and_then(secure_url)
                    .unwrap_or_else(|| image_url.clone());
                return Ok(Lookup::Found(FoundImage {
                    bytes: library::download(&image_url, Some(state)).await?,
                    credit: ImageCredit {
                        provider: provider.key().into(),
                        source_url,
                        image_url,
                        author: None,
                        license: Some("Cover artwork may be copyrighted.".into()),
                        license_url: None,
                        credit: Some("Apple iTunes".into()),
                    },
                }));
            } else if kind == "artists" {
                let (name, _) = match entity_name_and_artists(state, kind, id).await? {
                    Ok(res) => res,
                    Err(()) => return Ok(Lookup::Ambiguous),
                };
                let target = url("https://api.deezer.com/search/artist", &[("q", &name)]);
                let Some(data) = json(state, &target).await? else {
                    return Ok(Lookup::Missing);
                };
                let Some(candidate) = find_deezer_artist_candidate(&data, &name) else {
                    return Ok(Lookup::Missing);
                };
                let Some(image_url) = secure_url(candidate.picture_url) else {
                    return Ok(Lookup::Missing);
                };
                let source_url = candidate
                    .link_url
                    .and_then(secure_url)
                    .unwrap_or_else(|| image_url.clone());
                return Ok(Lookup::Found(FoundImage {
                    bytes: library::download(&image_url, Some(state)).await?,
                    credit: ImageCredit {
                        provider: provider.key().into(),
                        source_url,
                        image_url,
                        author: None,
                        license: Some("Artist image may be copyrighted.".into()),
                        license_url: None,
                        credit: Some("Deezer".into()),
                    },
                }));
            } else {
                return Ok(Lookup::Missing);
            }
        }
        let identity = identity(state, kind, id).await?;
        let mbid = match identity {
            Ok(Some(id)) => id,
            Ok(None) => return Ok(Lookup::Missing),
            Err(()) => return Ok(Lookup::Ambiguous),
        };
        if provider == ImageProvider::CoverArtArchive {
            let Some(data) = json(
                state,
                &format!("https://coverartarchive.org/release-group/{mbid}"),
            )
            .await?
            else {
                return Ok(Lookup::Missing);
            };
            let Some(image_url) = caa_front_image_url(&data)? else {
                return Ok(Lookup::Missing);
            };
            return Ok(Lookup::Found(FoundImage {
                bytes: library::download(&image_url, Some(state)).await?,
                credit: ImageCredit {
                    provider: provider.key().into(),
                    source_url: format!("https://musicbrainz.org/release-group/{mbid}"),
                    image_url,
                    author: None,
                    license: Some(
                        "Cover artwork may be copyrighted; metadata licensing does not license the image."
                            .into(),
                    ),
                    license_url: Some(
                        "https://musicbrainz.org/doc/Copyright_and_DMCA_Compliance".into(),
                    ),
                    credit: Some("Cover Art Archive / MusicBrainz".into()),
                },
            }));
        }
        let target = url(
            &format!("https://musicbrainz.org/ws/2/artist/{mbid}"),
            &[("inc", "url-rels"), ("fmt", "json")],
        );
        let Some(data) = json(state, &target).await? else {
            return Ok(Lookup::Missing);
        };
        let relation_type = if provider == ImageProvider::MusicbrainzImage {
            "image"
        } else {
            "wikidata"
        };
        let relations = data["relations"].as_array().ok_or(AppError::Internal)?;
        let candidates: std::collections::BTreeSet<_> = relations
            .iter()
            .filter(|v| v["type"] == relation_type)
            .filter_map(|v| v["url"]["resource"].as_str())
            .collect();
        if candidates.len() > 1 {
            return Ok(Lookup::Ambiguous);
        }
        let Some(candidate) = candidates.first() else {
            return Ok(Lookup::Missing);
        };
        if provider == ImageProvider::MusicbrainzImage {
            let Some(file) = commons_file(candidate) else {
                return Ok(Lookup::Missing);
            };
            return commons(state, &file, provider).await;
        }
        let Ok(candidate) = url::Url::parse(candidate) else {
            return Ok(Lookup::Missing);
        };
        if candidate.host_str() != Some("www.wikidata.org") {
            return Ok(Lookup::Missing);
        }
        let Some(qid) = candidate.path().strip_prefix("/wiki/").filter(|v| {
            v.starts_with('Q') && v.len() > 1 && v[1..].bytes().all(|c| c.is_ascii_digit())
        }) else {
            return Ok(Lookup::Missing);
        };
        let Some(data) = json(
            state,
            &format!("https://www.wikidata.org/wiki/Special:EntityData/{qid}.json"),
        )
        .await?
        else {
            return Ok(Lookup::Missing);
        };
        let Some(claims) = data["entities"][qid]["claims"]["P18"].as_array() else {
            return Ok(Lookup::Missing);
        };
        let files: std::collections::BTreeSet<_> = claims
            .iter()
            .filter(|v| v["rank"] != "deprecated")
            .filter_map(|v| v["mainsnak"]["datavalue"]["value"].as_str())
            .collect();
        if files.len() != 1 {
            return Ok(if files.is_empty() {
                Lookup::Missing
            } else {
                Lookup::Ambiguous
            });
        }
        commons(state, &format!("File:{}", files.first().unwrap()), provider).await
    }
}
#[cfg(test)]
pub(crate) async fn fetch_with(
    state: &AppState,
    settings: &ArtworkSettings,
    kind: &str,
    id: &str,
    sources: &impl ImageLookup,
) -> Result<library::ArtworkFetch> {
    fetch_with_activity(state, settings, kind, id, sources, None).await
}

pub(crate) async fn fetch_with_activity(
    state: &AppState,
    settings: &ArtworkSettings,
    kind: &str,
    id: &str,
    sources: &impl ImageLookup,
    activity: Option<(&str, &str)>,
) -> Result<library::ArtworkFetch> {
    for provider in settings.enabled(kind) {
        // Configuration changes fence in-flight lookups as well as later queue entries.
        if artwork_settings::read(&mut *state.read_db.acquire().await?)
            .await?
            .version
            != settings.version
        {
            return Ok(library::ArtworkFetch::Busy);
        }
        if let Some((activity_id, member)) = activity {
            sqlx::query("UPDATE job_members SET current_provider=? WHERE job_id=? AND member=? AND state='pending'")
                .bind(provider.key())
                .bind(activity_id)
                .bind(member)
                .execute(&state.db)
                .await?;
        }
        let lookup = sources.lookup(provider, kind, id).await;
        if let Some((activity_id, member)) = activity {
            sqlx::query("UPDATE job_members SET current_provider=NULL WHERE job_id=? AND member=? AND current_provider=?")
                .bind(activity_id)
                .bind(member)
                .bind(provider.key())
                .execute(&state.db)
                .await?;
        }
        match lookup? {
            Lookup::Missing => continue,
            Lookup::Ambiguous => break,
            Lookup::Found(image) => {
                let mut tx = state.db.begin().await?;
                sqlx::query("UPDATE archive_state SET revision=revision WHERE singleton=1 AND maintenance=0 RETURNING revision").fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::Conflict("Archive is in maintenance.".into()))?;
                if artwork_settings::read(&mut tx).await?.version != settings.version {
                    return Ok(library::ArtworkFetch::Busy);
                }
                let exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM artwork WHERE kind=? AND entity_id=? AND bytes IS NOT NULL)",
                )
                .bind(kind)
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
                if exists {
                    return Ok(library::ArtworkFetch::Cached);
                }
                let now = auth::now_ms();
                sqlx::query("INSERT INTO artwork(kind,entity_id,source,bytes,mime,checked_at_ms,expires_at_ms,provenance_json) VALUES(?,?,?,?,?,?,NULL,?) ON CONFLICT(kind,entity_id,source) DO UPDATE SET bytes=excluded.bytes,mime=excluded.mime,checked_at_ms=excluded.checked_at_ms,expires_at_ms=NULL,provenance_json=excluded.provenance_json")
                    .bind(kind).bind(id).bind(provider.key()).bind(&image.bytes).bind(library::image_type(&image.bytes)?).bind(now).bind(serde_json::to_string(&image.credit).map_err(|_| AppError::Internal)?).execute(&mut *tx).await?;
                tx.commit().await?;
                return Ok(library::ArtworkFetch::Saved);
            }
        }
    }
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE archive_state SET revision=revision WHERE singleton=1 AND maintenance=0 RETURNING revision").fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::Conflict("Archive is in maintenance.".into()))?;
    if artwork_settings::read(&mut tx).await?.version != settings.version {
        return Ok(library::ArtworkFetch::Busy);
    }
    sqlx::query("INSERT INTO artwork_misses VALUES(?,?,?,?) ON CONFLICT(kind,entity_id) DO UPDATE SET settings_version=excluded.settings_version,expires_at_ms=excluded.expires_at_ms").bind(kind).bind(id).bind(settings.version).bind(auth::now_ms()+DAY).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(library::ArtworkFetch::Missing)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    async fn setup() -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().unwrap();
        let storage = crate::storage::open(dir.path()).await.unwrap();
        let config = crate::config::Config {
            bind_ip: "127.0.0.1".parse().unwrap(),
            external_origin: None,
            port: 3210,
            data_dir: dir.path().into(),
            sync_seconds: 120,
            now_playing_seconds: 30,
            import_limits: Default::default(),
            api_snapshot_limits: Default::default(),
            env_client_id: None,
            env_client_secret: None,
        };
        (dir, AppState::new(storage, config))
    }
    struct Fake<'a> {
        state: &'a AppState,
        calls: Mutex<Vec<ImageProvider>>,
        mode: &'static str,
    }
    impl ImageLookup for Fake<'_> {
        async fn lookup(&self, provider: ImageProvider, kind: &str, id: &str) -> Result<Lookup> {
            self.calls.lock().unwrap().push(provider);
            if self.mode == "error" {
                return Err(AppError::Internal);
            }
            if self.mode == "artwork_wait" {
                return Err(AppError::ArtworkWait { retry_at_ms: 99999999 });
            }
            if self.mode == "ambiguous" {
                return Ok(Lookup::Ambiguous);
            }
            if self.mode == "fallback" && provider != ImageProvider::Wikidata {
                return Ok(Lookup::Missing);
            }
            if self.mode == "fallback_itunes" && provider != ImageProvider::Itunes {
                return Ok(Lookup::Missing);
            }
            if self.mode == "custom" {
                sqlx::query("INSERT INTO artwork(kind,entity_id,source,bytes,mime,checked_at_ms) VALUES(?,?,'custom',?,'image/png',0)").bind(kind).bind(id).bind(b"\x89PNG\r\n\x1a\nowner".as_slice()).execute(&self.state.db).await?;
            }
            if self.mode == "settings_changed" {
                let settings = ArtworkSettings {
                    version: 1,
                    ..Default::default()
                };
                crate::db::set_setting(
                    &mut *self.state.db.acquire().await?,
                    "artwork_providers",
                    &serde_json::to_string(&settings).unwrap(),
                )
                .await?;
            }
            let credit_text = if provider == ImageProvider::Itunes {
                if kind == "artists" {
                    Some("Deezer".into())
                } else {
                    Some("Apple iTunes".into())
                }
            } else {
                None
            };
            Ok(Lookup::Found(FoundImage {
                bytes: b"\x89PNG\r\n\x1a\nautomatic".to_vec(),
                credit: ImageCredit {
                    provider: provider.key().into(),
                    source_url: "https://commons.wikimedia.org/wiki/File:Example.jpg".into(),
                    image_url: "https://upload.wikimedia.org/example.jpg".into(),
                    author: Some("Author".into()),
                    license: Some("CC BY 4.0".into()),
                    license_url: Some("https://creativecommons.org/licenses/by/4.0/".into()),
                    credit: credit_text,
                },
            }))
        }
    }
    #[tokio::test]
    async fn order_disabled_providers_fallback_and_credits_are_obeyed_without_spotify_login() {
        let (_dir, state) = setup().await;
        let mut settings = ArtworkSettings::default();
        settings.providers.swap(1, 2);
        let fake = Fake {
            state: &state,
            calls: Mutex::new(vec![]),
            mode: "found",
        };
        assert_eq!(
            fetch_with(&state, &settings, "artists", "artist", &fake)
                .await
                .unwrap(),
            library::ArtworkFetch::Saved
        );
        assert_eq!(*fake.calls.lock().unwrap(), vec![ImageProvider::Wikidata]);
        assert_eq!(
            credit(&state, "artists", "artist")
                .await
                .unwrap()
                .unwrap()
                .author
                .as_deref(),
            Some("Author")
        );
        let mut settings = ArtworkSettings::default();
        settings.providers[0].enabled = false;
        let fake = Fake {
            state: &state,
            calls: Mutex::new(vec![]),
            mode: "fallback",
        };
        assert_eq!(
            fetch_with(&state, &settings, "artists", "fallback", &fake)
                .await
                .unwrap(),
            library::ArtworkFetch::Saved
        );
        assert_eq!(
            *fake.calls.lock().unwrap(),
            vec![ImageProvider::MusicbrainzImage, ImageProvider::Wikidata]
        );
        let expiry: Option<i64> =
            sqlx::query_scalar("SELECT expires_at_ms FROM artwork WHERE entity_id='fallback'")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(expiry, None);
        state.storage.close().await;
    }
    #[tokio::test]
    async fn ambiguous_or_failed_lookup_never_falls_through_and_failure_does_not_poison_cache() {
        let (_dir, state) = setup().await;
        for mode in ["ambiguous", "error", "artwork_wait"] {
            let fake = Fake {
                state: &state,
                calls: Mutex::new(vec![]),
                mode,
            };
            let result =
                fetch_with(&state, &ArtworkSettings::default(), "artists", mode, &fake).await;
            assert_eq!(fake.calls.lock().unwrap().len(), 1);
            if mode == "error" || mode == "artwork_wait" {
                assert!(result.is_err());
            } else {
                assert_eq!(result.unwrap(), library::ArtworkFetch::Missing);
            }
        }
        let misses: Vec<String> = sqlx::query_scalar("SELECT entity_id FROM artwork_misses")
            .fetch_all(&state.db)
            .await
            .unwrap();
        assert_eq!(misses, vec!["ambiguous"]);
        state.storage.close().await;
    }
    #[tokio::test]
    async fn concurrent_upload_and_settings_change_fence_download_results() {
        let (_dir, state) = setup().await;
        for (mode, expected) in [
            ("custom", library::ArtworkFetch::Cached),
            ("settings_changed", library::ArtworkFetch::Busy),
        ] {
            let fake = Fake {
                state: &state,
                calls: Mutex::new(vec![]),
                mode,
            };
            assert_eq!(
                fetch_with(&state, &ArtworkSettings::default(), "artists", mode, &fake)
                    .await
                    .unwrap(),
                expected
            );
        }
        let images: Vec<(String, Vec<u8>)> = sqlx::query_as("SELECT source,bytes FROM artwork")
            .fetch_all(&state.db)
            .await
            .unwrap();
        assert_eq!(
            images,
            vec![("custom".into(), b"\x89PNG\r\n\x1a\nowner".to_vec())]
        );
        state.storage.close().await;
    }
    #[test]
    fn matching_relaxes_disambiguation_count_and_credits_while_rejecting_homonyms() {
        let id = "b10bbbfc-cf9e-42e0-be17-e2c3e1d2600d";
        // 1. Disambiguation comment with score 100 and exact name is accepted
        let data = json!({
            "count": 1,
            "artists": [{
                "name": "Architects",
                "id": id,
                "score": 100,
                "disambiguation": "UK alternative metal band"
            }]
        });
        assert_eq!(unique_match(&data, "artists", "Architects", None), Ok(Some(id)));

        // 2. Count > 100 or count > items.len() is accepted if unique exact match exists
        let data = json!({
            "count": 350,
            "artists": [
                { "name": "Coldplay", "id": id, "score": 100 },
                { "name": "Coldplay Tribute", "id": "other-id-0000-0000-0000-000000000000", "score": 90 }
            ]
        });
        assert_eq!(unique_match(&data, "artists", "Coldplay", None), Ok(Some(id)));

        // 3. Homonyms with same name are rejected as ambiguous
        let data = json!({
            "count": 2,
            "artists": [
                { "name": "John Smith", "id": id, "score": 100 },
                { "name": "John Smith", "id": "other-id-0000-0000-0000-000000000000", "score": 100 }
            ]
        });
        assert_eq!(unique_match(&data, "artists", "John Smith", None), Err(()));

        // 4. Multi-artist / collaboration credits are accepted if searched artist is present
        let data = json!({
            "count": 1,
            "release-groups": [{
                "title": "Under Pressure",
                "id": id,
                "score": 100,
                "artist-credit": [
                    { "artist": { "name": "Queen" }, "joinphrase": " & " },
                    { "artist": { "name": "David Bowie" } }
                ]
            }]
        });
        assert_eq!(unique_match(&data, "albums", "Under Pressure", Some("Queen")), Ok(Some(id)));
        assert_eq!(unique_match(&data, "albums", "Under Pressure", Some("David Bowie")), Ok(Some(id)));
        assert_eq!(unique_match(&data, "albums", "Under Pressure", Some("Different Artist")), Ok(None));

        // 5. Album titles with edition suffixes match release groups
        let data = json!({
            "count": 1,
            "release-groups": [{
                "title": "Abbey Road",
                "id": id,
                "score": 100,
                "artist-credit": [{ "artist": { "name": "The Beatles" } }]
            }]
        });
        assert_eq!(unique_match(&data, "albums", "Abbey Road (Deluxe Edition)", Some("The Beatles")), Ok(Some(id)));
        assert_eq!(unique_match(&data, "albums", "Abbey Road [Remastered]", Some("The Beatles")), Ok(Some(id)));
        assert_eq!(unique_match(&data, "albums", "Abbey Road - Single", Some("The Beatles")), Ok(Some(id)));
        assert_eq!(unique_match(&data, "albums", "Abbey Road - EP", Some("The Beatles")), Ok(Some(id)));
        assert_eq!(unique_match(&data, "albums", "Abbey Road (Expanded Edition)", Some("The Beatles")), Ok(Some(id)));

        // 6. Utility checks
        assert_eq!(
            commons_file("https://commons.wikimedia.org/wiki/File:A%26B.jpg"),
            Some("File:A&B.jpg".into())
        );
        assert!(commons_file("https://example.com/wiki/File:X.jpg").is_none());
        assert_eq!(
            secure_url("http://archive.org/download/cover.jpg"),
            Some("https://archive.org/download/cover.jpg".into())
        );
        assert!(secure_url("http://example.com/cover.jpg").is_none());
        assert_eq!(quoted("a\"b:c"), "\"a\\\"b\\:c\"");
    }
    #[test]
    fn caa_front_image_url_selects_first_approved_image() {
        let empty = json!({ "images": [] });
        assert_eq!(caa_front_image_url(&empty).unwrap(), None);

        let unapproved = json!({
            "images": [
                { "front": true, "approved": false, "image": "http://archive.org/front.jpg" },
                { "front": false, "approved": true, "image": "http://archive.org/back.jpg" }
            ]
        });
        assert_eq!(caa_front_image_url(&unapproved).unwrap(), None);

        let single = json!({
            "images": [
                {
                    "front": true,
                    "approved": true,
                    "thumbnails": { "500": "http://ia8000.us.archive.org/thumb1.jpg" },
                    "image": "http://ia8000.us.archive.org/front1.jpg"
                }
            ]
        });
        assert_eq!(
            caa_front_image_url(&single).unwrap(),
            Some("https://ia8000.us.archive.org/thumb1.jpg".into())
        );

        let multiple = json!({
            "images": [
                {
                    "front": true,
                    "approved": true,
                    "thumbnails": { "500": "http://ia8000.us.archive.org/thumb1.jpg" },
                    "image": "http://ia8000.us.archive.org/front1.jpg"
                },
                {
                    "front": true,
                    "approved": true,
                    "thumbnails": { "500": "http://ia8000.us.archive.org/thumb2.jpg" },
                    "image": "http://ia8000.us.archive.org/front2.jpg"
                }
            ]
        });
        assert_eq!(
            caa_front_image_url(&multiple).unwrap(),
            Some("https://ia8000.us.archive.org/thumb1.jpg".into())
        );
    }
    #[test]
    fn normalize_album_title_handles_common_edition_suffixes() {
        assert_eq!(normalize_album_title("Abbey Road (Deluxe Edition)"), "Abbey Road");
        assert_eq!(normalize_album_title("Nevermind [Remastered]"), "Nevermind");
        assert_eq!(normalize_album_title("Blinding Lights - Single"), "Blinding Lights");
        assert_eq!(normalize_album_title("Boyfriend - EP"), "Boyfriend");
        assert_eq!(normalize_album_title("Thriller (Expanded Edition)"), "Thriller");
        assert_eq!(normalize_album_title("The Dark Side of the Moon (50th Anniversary Remaster)"), "The Dark Side of the Moon");
        assert_eq!(normalize_album_title("OK Computer (Collector's Edition)"), "OK Computer");
        assert_eq!(normalize_album_title("Random Access Memories (10th Anniversary Edition)"), "Random Access Memories");
        assert_eq!(normalize_album_title("Song (Single Version)"), "Song");
        assert_eq!(normalize_album_title("Album (Bonus Track Edition)"), "Album");
        assert_eq!(normalize_album_title("Album [Deluxe]"), "Album");
        assert_eq!(normalize_album_title("Album - Deluxe Edition"), "Album");
        assert_eq!(normalize_album_title("Album - Remastered"), "Album");
        assert_eq!(normalize_album_title("Title (Deluxe Edition) [Remastered]"), "Title");
        assert_eq!(normalize_album_title("Single"), "Single");
        assert_eq!(normalize_album_title("EP"), "EP");
        assert_eq!(normalize_album_title("(Deluxe Edition)"), "(Deluxe Edition)");
    }

    #[test]
    fn itunes_artwork_url_upgrades_resolution_to_600x600() {
        assert_eq!(
            upgrade_itunes_artwork_url("https://is1-ssl.mzstatic.com/image/thumb/Music126/v4/24/e9/c2/24e9c269-bee0-62e0-6de6-257db163ee5a/21UM1IM54074.rgb.jpg/100x100bb.jpg"),
            "https://is1-ssl.mzstatic.com/image/thumb/Music126/v4/24/e9/c2/24e9c269-bee0-62e0-6de6-257db163ee5a/21UM1IM54074.rgb.jpg/600x600bb.jpg"
        );
        assert_eq!(
            upgrade_itunes_artwork_url("https://is1-ssl.mzstatic.com/image/thumb/Music/100x100bb.png"),
            "https://is1-ssl.mzstatic.com/image/thumb/Music/600x600bb.png"
        );
        assert_eq!(
            upgrade_itunes_artwork_url("https://is1-ssl.mzstatic.com/image/thumb/Music/100x100.jpg"),
            "https://is1-ssl.mzstatic.com/image/thumb/Music/600x600.jpg"
        );
        assert_eq!(
            upgrade_itunes_artwork_url("https://example.com/custom/500x500.jpg"),
            "https://example.com/custom/500x500.jpg"
        );
    }

    #[test]
    fn itunes_album_candidate_matching_romaji_and_editions() {
        // 1. Romaji album "Kyougen" by "Ado"
        let data = json!({
            "resultCount": 2,
            "results": [
                {
                    "collectionName": "Ado Kyougen Piano Collections",
                    "artistName": "daigoro789",
                    "artworkUrl100": "https://is1-ssl.mzstatic.com/piano/100x100bb.jpg",
                    "collectionViewUrl": "https://music.apple.com/piano"
                },
                {
                    "collectionName": "Kyougen",
                    "artistName": "ADO",
                    "artworkUrl100": "https://is1-ssl.mzstatic.com/ado/100x100bb.jpg",
                    "collectionViewUrl": "https://music.apple.com/ado-kyougen"
                }
            ]
        });
        let matched = find_itunes_album_candidate(&data, "Kyougen", "Ado").expect("matched Kyougen");
        assert_eq!(matched.artwork_url, "https://is1-ssl.mzstatic.com/ado/100x100bb.jpg");
        assert_eq!(matched.view_url, Some("https://music.apple.com/ado-kyougen"));

        // 2. Romaji album "Zanmu" by "Ado"
        let data = json!({
            "resultCount": 1,
            "results": [
                {
                    "collectionName": "Zanmu",
                    "artistName": "ADO",
                    "artworkUrl100": "https://is1-ssl.mzstatic.com/zanmu/100x100bb.jpg",
                    "collectionViewUrl": "https://music.apple.com/ado-zanmu"
                }
            ]
        });
        let matched = find_itunes_album_candidate(&data, "Zanmu", "Ado").expect("matched Zanmu");
        assert_eq!(matched.artwork_url, "https://is1-ssl.mzstatic.com/zanmu/100x100bb.jpg");

        // 3. Album with edition suffix: target "ALL OUT", candidate "ALL OUT - EP" by "K/DA"
        let data = json!({
            "resultCount": 1,
            "results": [
                {
                    "collectionName": "ALL OUT - EP",
                    "artistName": "K/DA",
                    "artworkUrl100": "https://is1-ssl.mzstatic.com/kda/100x100bb.jpg",
                    "collectionViewUrl": "https://music.apple.com/kda-all-out"
                }
            ]
        });
        let matched = find_itunes_album_candidate(&data, "ALL OUT", "K/DA").expect("matched ALL OUT");
        assert_eq!(matched.artwork_url, "https://is1-ssl.mzstatic.com/kda/100x100bb.jpg");

        // 4. Target with edition suffix: target "Abbey Road (Deluxe Edition)", candidate "Abbey Road"
        let data = json!({
            "resultCount": 1,
            "results": [
                {
                    "collectionName": "Abbey Road",
                    "artistName": "The Beatles",
                    "artworkUrl100": "https://is1-ssl.mzstatic.com/beatles/100x100bb.jpg",
                    "collectionViewUrl": "https://music.apple.com/beatles-abbey-road"
                }
            ]
        });
        let matched = find_itunes_album_candidate(&data, "Abbey Road (Deluxe Edition)", "The Beatles").expect("matched Abbey Road");
        assert_eq!(matched.artwork_url, "https://is1-ssl.mzstatic.com/beatles/100x100bb.jpg");

        // 5. Artist mismatch is rejected
        assert!(find_itunes_album_candidate(&data, "Abbey Road", "Coldplay").is_none());

        // 6. Title mismatch is rejected
        assert!(find_itunes_album_candidate(&data, "Let It Be", "The Beatles").is_none());
    }

    #[test]
    fn deezer_artist_candidate_matching_and_resolution() {
        let data = json!({
            "data": [
                {
                    "id": 1,
                    "name": "Ado (Col)",
                    "link": "https://www.deezer.com/artist/1",
                    "picture_big": "https://cdn-images.dzcdn.net/col/500.jpg",
                    "picture_xl": "https://cdn-images.dzcdn.net/col/1000.jpg"
                },
                {
                    "id": 2,
                    "name": "Ado & Montorsi",
                    "link": "https://www.deezer.com/artist/2",
                    "picture_big": "https://cdn-images.dzcdn.net/duo/500.jpg",
                    "picture_xl": "https://cdn-images.dzcdn.net/duo/1000.jpg"
                },
                {
                    "id": 3,
                    "name": "Ado",
                    "link": "https://www.deezer.com/artist/3",
                    "picture_big": "https://cdn-images.dzcdn.net/ado/500.jpg",
                    "picture_xl": "https://cdn-images.dzcdn.net/ado/1000.jpg"
                }
            ],
            "total": 3
        });
        let matched = find_deezer_artist_candidate(&data, "Ado").expect("matched Ado");
        assert_eq!(matched.picture_url, "https://cdn-images.dzcdn.net/ado/1000.jpg");
        assert_eq!(matched.link_url, Some("https://www.deezer.com/artist/3"));

        // Fallback to picture_big if picture_xl is missing
        let data_big_only = json!({
            "data": [
                {
                    "id": 4,
                    "name": "Daft Punk",
                    "link": "https://www.deezer.com/artist/4",
                    "picture_big": "https://cdn-images.dzcdn.net/daftpunk/500.jpg"
                }
            ]
        });
        let matched = find_deezer_artist_candidate(&data_big_only, "Daft Punk").expect("matched Daft Punk");
        assert_eq!(matched.picture_url, "https://cdn-images.dzcdn.net/daftpunk/500.jpg");

        // Non-matching artist
        assert!(find_deezer_artist_candidate(&data, "Taylor Swift").is_none());
    }

    #[tokio::test]
    async fn fallback_chain_albums_caa_to_itunes() {
        let (_dir, state) = setup().await;
        let settings = ArtworkSettings::default();
        let fake = Fake {
            state: &state,
            calls: Mutex::new(vec![]),
            mode: "fallback_itunes",
        };
        let result = fetch_with(&state, &settings, "albums", "album-1", &fake)
            .await
            .unwrap();
        assert_eq!(result, library::ArtworkFetch::Saved);
        // Fallback chain for albums: CAA (first) -> iTunes (second)
        assert_eq!(
            *fake.calls.lock().unwrap(),
            vec![ImageProvider::CoverArtArchive, ImageProvider::Itunes]
        );
        let cr = credit(&state, "albums", "album-1").await.unwrap().unwrap();
        assert_eq!(cr.provider, "itunes");
        assert_eq!(cr.credit.as_deref(), Some("Apple iTunes"));
        state.storage.close().await;
    }

    #[tokio::test]
    async fn fallback_chain_artists_mb_wikidata_to_itunes() {
        let (_dir, state) = setup().await;
        let settings = ArtworkSettings::default();
        let fake = Fake {
            state: &state,
            calls: Mutex::new(vec![]),
            mode: "fallback_itunes",
        };
        let result = fetch_with(&state, &settings, "artists", "artist-1", &fake)
            .await
            .unwrap();
        assert_eq!(result, library::ArtworkFetch::Saved);
        // Fallback chain for artists: MB (first) -> Wikidata (second) -> iTunes (third)
        assert_eq!(
            *fake.calls.lock().unwrap(),
            vec![
                ImageProvider::MusicbrainzImage,
                ImageProvider::Wikidata,
                ImageProvider::Itunes
            ]
        );
        let cr = credit(&state, "artists", "artist-1").await.unwrap().unwrap();
        assert_eq!(cr.provider, "itunes");
        assert_eq!(cr.credit.as_deref(), Some("Deezer"));
        state.storage.close().await;
    }

    #[tokio::test]
    async fn disabled_provider_skipped_in_fallback_chain() {
        let (_dir, state) = setup().await;
        let mut settings = ArtworkSettings::default();
        // Disable CAA
        settings.providers[0].enabled = false;
        let fake = Fake {
            state: &state,
            calls: Mutex::new(vec![]),
            mode: "fallback_itunes",
        };
        let result = fetch_with(&state, &settings, "albums", "album-disabled", &fake)
            .await
            .unwrap();
        assert_eq!(result, library::ArtworkFetch::Saved);
        // Only iTunes should be called since CAA was disabled
        assert_eq!(*fake.calls.lock().unwrap(), vec![ImageProvider::Itunes]);
        state.storage.close().await;
    }

    #[tokio::test]
    async fn provider_reordering_is_respected_in_lookup() {
        let (_dir, state) = setup().await;
        let mut settings = ArtworkSettings::default();
        // Move iTunes (index 3) to the very first position
        let itunes_pref = settings.providers.remove(3);
        settings.providers.insert(0, itunes_pref);

        let fake = Fake {
            state: &state,
            calls: Mutex::new(vec![]),
            mode: "found",
        };
        // For albums: iTunes is now 1st, so it should be called first and succeed immediately
        let result = fetch_with(&state, &settings, "albums", "album-reordered", &fake)
            .await
            .unwrap();
        assert_eq!(result, library::ArtworkFetch::Saved);
        assert_eq!(*fake.calls.lock().unwrap(), vec![ImageProvider::Itunes]);
        state.storage.close().await;
    }

    #[test]
    fn credit_serialization_and_roundtrip() {
        let credit = ImageCredit {
            provider: "itunes".into(),
            source_url: "https://music.apple.com/album/123".into(),
            image_url: "https://is1-ssl.mzstatic.com/image/600x600bb.jpg".into(),
            author: None,
            license: Some("Promotional artwork; may be copyrighted.".into()),
            license_url: None,
            credit: Some("Apple iTunes".into()),
        };
        let json_str = serde_json::to_string(&credit).unwrap();
        let restored: ImageCredit = serde_json::from_str(&json_str).unwrap();
        assert_eq!(restored.provider, "itunes");
        assert_eq!(restored.source_url, "https://music.apple.com/album/123");
        assert_eq!(restored.image_url, "https://is1-ssl.mzstatic.com/image/600x600bb.jpg");
        assert_eq!(restored.credit.as_deref(), Some("Apple iTunes"));
        assert_eq!(restored.license.as_deref(), Some("Promotional artwork; may be copyrighted."));
    }
}
