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
pub const SPOTIFY_TTL_MS: i64 = 14 * DAY;
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
// The single image slot serializes callers; reservations and cooldowns survive restarts.
pub(crate) async fn request_slot(state: &AppState, host: &str) -> Result<()> {
    let mut tx = state.db.begin().await?;
    // Older releases stored one cooldown without the originating host. Honor
    // that finite reservation until expiry; all new waits are scoped per host.
    let retries: Vec<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key IN (?,'artwork_retry_at_ms')")
            .bind(format!("artwork_host_retry:{host}"))
            .fetch_all(&mut *tx)
            .await?;
    let now = auth::now_ms();
    if let Some(at) = retries
        .into_iter()
        .filter_map(|v| v.parse::<i64>().ok())
        .max()
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
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0)
        .min(7);
    let retry = at.unwrap_or_else(|| auth::now_ms() + (30_000i64 * (1 << attempts)).min(3_600_000));
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
    let count = data["count"].as_u64().ok_or(())?;
    // Search scores do not prove identity. Only one exact candidate in a complete result set.
    if count > items.len() as u64 {
        return Err(());
    }
    let matching: Vec<_> = items
        .iter()
        .filter(|item| {
            item[if kind == "artists" { "name" } else { "title" }]
                .as_str()
                .is_some_and(|v| normalized(v) == normalized(name))
                && artist.is_none_or(|artist| {
                    item["artist-credit"].as_array().is_some_and(|credits| {
                        credits.len() == 1
                            && credits[0]["artist"]["name"]
                                .as_str()
                                .is_some_and(|v| normalized(v) == normalized(artist))
                    })
                })
        })
        .collect();
    if matching.is_empty() {
        return Ok(None);
    }
    if matching.len() != 1
        || matching[0]["score"].as_u64() != Some(100)
        || matching[0]["disambiguation"]
            .as_str()
            .is_some_and(|v| !v.is_empty())
    {
        return Err(());
    }
    matching[0]["id"]
        .as_str()
        .filter(|id| mbid(id))
        .map(Some)
        .ok_or(())
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
    let table = if kind == "artists" {
        "artists"
    } else {
        "albums"
    };
    // A group deliberately separated from an identically named entity is still ambiguous externally.
    let names: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT DISTINCT e.name FROM main.{table} e JOIN name_group_members gm ON gm.entity_id=e.id WHERE gm.group_id=?"))).bind(id).fetch_all(&state.read_db).await?;
    if names.len() != 1 {
        return Ok(Err(()));
    }
    let name = &names[0];
    let artists: Vec<String> = if kind == "albums" {
        sqlx::query_scalar("SELECT DISTINCT a.name FROM main.artists a JOIN track_artists ta ON ta.artist_id=a.id JOIN tracks t ON t.id=ta.track_id JOIN name_group_members gm ON gm.entity_id=t.album_id WHERE gm.group_id=?").bind(id).fetch_all(&state.read_db).await?
    } else {
        vec![]
    };
    if kind == "albums" && artists.len() != 1 {
        return Ok(Err(()));
    }
    let query = if kind == "artists" {
        format!("artist:{}", quoted(name))
    } else {
        format!(
            "releasegroup:{} AND artist:{}",
            quoted(name),
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
        unique_match(&data, kind, name, artists.first().map(String::as_str))
            .map(|v| v.map(str::to_owned)),
    )
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
impl ImageLookup for PublicSources<'_> {
    async fn lookup(&self, provider: ImageProvider, kind: &str, id: &str) -> Result<Lookup> {
        let state = self.0;
        if provider == ImageProvider::Spotify {
            let connected: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM spotify_tokens)")
                .fetch_one(&state.private_db)
                .await?;
            if !connected {
                return Err(AppError::Unauthorized);
            }
            let result = library::spotify_image(state, kind, id).await;
            let result = match result {
                Err(AppError::Spotify(_)) => {
                    let retry = crate::db::setting(&state.db, "retry_at_ms")
                        .await?
                        .and_then(|v| v.parse::<i64>().ok())
                        .filter(|at| *at > auth::now_ms());
                    if let Some(retry_at_ms) = retry {
                        return Err(AppError::ArtworkWait { retry_at_ms });
                    }
                    return Err(AppError::Spotify(
                        "Spotify artwork is temporarily unavailable.".into(),
                    ));
                }
                result => result?,
            };
            let Some((image_url, source_url)) = result else {
                return Ok(Lookup::Missing);
            };
            return Ok(Lookup::Found(FoundImage {
                bytes: library::download(&image_url, Some(state)).await?,
                credit: ImageCredit {
                    provider: "spotify".into(),
                    source_url,
                    image_url,
                    author: None,
                    license: Some("Spotify Content — temporary cache".into()),
                    license_url: Some("https://developer.spotify.com/terms".into()),
                    credit: Some("Spotify".into()),
                },
            }));
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
            let images = data["images"].as_array().ok_or(AppError::Internal)?;
            let front: Vec<_> = images
                .iter()
                .filter(|v| v["front"] == true && v["approved"] == true)
                .collect();
            if front.len() != 1 {
                return Ok(if front.is_empty() {
                    Lookup::Missing
                } else {
                    Lookup::Ambiguous
                });
            }
            let Some(image_url) = front[0]["thumbnails"]["500"]
                .as_str()
                .or_else(|| front[0]["image"].as_str())
                .and_then(secure_url)
            else {
                return Ok(Lookup::Missing);
            };
            return Ok(Lookup::Found(FoundImage { bytes: library::download(&image_url, Some(state)).await?, credit: ImageCredit { provider: provider.key().into(), source_url: format!("https://musicbrainz.org/release-group/{mbid}"), image_url, author: None, license: Some("Cover artwork may be copyrighted; metadata licensing does not license the image.".into()), license_url: Some("https://musicbrainz.org/doc/Copyright_and_DMCA_Compliance".into()), credit: Some("Cover Art Archive / MusicBrainz".into()) } }));
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
pub(crate) async fn fetch_with(
    state: &AppState,
    settings: &ArtworkSettings,
    kind: &str,
    id: &str,
    sources: &impl ImageLookup,
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
        match sources.lookup(provider, kind, id).await? {
            Lookup::Missing => continue,
            Lookup::Ambiguous => break,
            Lookup::Found(image) => {
                // Disconnect and publication share the Spotify gate. New temporary
                // bytes cannot reappear after the owner disconnects the integration.
                let _spotify = if provider == ImageProvider::Spotify {
                    let guard = state.spotify.gate.lock().await;
                    let connected: bool =
                        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM spotify_tokens)")
                            .fetch_one(&state.private_db)
                            .await?;
                    if !connected {
                        return Err(AppError::Unauthorized);
                    }
                    Some(guard)
                } else {
                    None
                };
                let mut tx = state.db.begin().await?;
                sqlx::query("UPDATE archive_state SET revision=revision WHERE singleton=1 AND maintenance=0 RETURNING revision").fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::Conflict("Archive is in maintenance.".into()))?;
                if artwork_settings::read(&mut tx).await?.version != settings.version {
                    return Ok(library::ArtworkFetch::Busy);
                }
                let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM artwork WHERE kind=? AND entity_id=? AND bytes IS NOT NULL AND (source='custom' OR expires_at_ms IS NULL OR expires_at_ms>?))").bind(kind).bind(id).bind(auth::now_ms()).fetch_one(&mut *tx).await?;
                if exists {
                    return Ok(library::ArtworkFetch::Cached);
                }
                let now = auth::now_ms();
                sqlx::query("INSERT INTO artwork(kind,entity_id,source,bytes,mime,checked_at_ms,expires_at_ms,provenance_json) VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(kind,entity_id,source) DO UPDATE SET bytes=excluded.bytes,mime=excluded.mime,checked_at_ms=excluded.checked_at_ms,expires_at_ms=excluded.expires_at_ms,provenance_json=excluded.provenance_json")
                    .bind(kind).bind(id).bind(provider.key()).bind(&image.bytes).bind(library::image_type(&image.bytes)?).bind(now).bind((provider == ImageProvider::Spotify).then_some(now+SPOTIFY_TTL_MS)).bind(serde_json::to_string(&image.credit).map_err(|_| AppError::Internal)?).execute(&mut *tx).await?;
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
    let row = sqlx::query("SELECT source,provenance_json FROM artwork WHERE kind=? AND entity_id=? AND bytes IS NOT NULL AND (source='custom' OR expires_at_ms IS NULL OR expires_at_ms>?) ORDER BY CASE source WHEN 'custom' THEN 0 ELSE 1 END LIMIT 1").bind(kind).bind(id).bind(auth::now_ms()).fetch_optional(&state.read_db).await?;
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
            if self.mode == "ambiguous" {
                return Ok(Lookup::Ambiguous);
            }
            if self.mode == "spotify" && provider != ImageProvider::Spotify {
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
            Ok(Lookup::Found(FoundImage {
                bytes: b"\x89PNG\r\n\x1a\nautomatic".to_vec(),
                credit: ImageCredit {
                    provider: provider.key().into(),
                    source_url: "https://commons.wikimedia.org/wiki/File:Example.jpg".into(),
                    image_url: "https://upload.wikimedia.org/example.jpg".into(),
                    author: Some("Author".into()),
                    license: Some("CC BY 4.0".into()),
                    license_url: Some("https://creativecommons.org/licenses/by/4.0/".into()),
                    credit: None,
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
        sqlx::query("INSERT INTO spotify_tokens VALUES(1,'test','test',9999999999999)")
            .execute(&state.private_db)
            .await
            .unwrap();
        let mut settings = ArtworkSettings::default();
        settings.providers[1].enabled = false;
        let fake = Fake {
            state: &state,
            calls: Mutex::new(vec![]),
            mode: "spotify",
        };
        assert_eq!(
            fetch_with(&state, &settings, "artists", "fallback", &fake)
                .await
                .unwrap(),
            library::ArtworkFetch::Saved
        );
        assert_eq!(
            *fake.calls.lock().unwrap(),
            vec![ImageProvider::Wikidata, ImageProvider::Spotify]
        );
        let expiry: i64 =
            sqlx::query_scalar("SELECT expires_at_ms FROM artwork WHERE entity_id='fallback'")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert!((expiry - auth::now_ms() - SPOTIFY_TTL_MS).abs() < 1000);
        state.storage.close().await;
    }
    #[tokio::test]
    async fn ambiguous_or_failed_lookup_never_falls_through_and_failure_does_not_poison_cache() {
        let (_dir, state) = setup().await;
        for mode in ["ambiguous", "error"] {
            let fake = Fake {
                state: &state,
                calls: Mutex::new(vec![]),
                mode,
            };
            let result =
                fetch_with(&state, &ArtworkSettings::default(), "artists", mode, &fake).await;
            assert_eq!(fake.calls.lock().unwrap().len(), 1);
            if mode == "error" {
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
    fn matching_rejects_homonyms_incomplete_results_disambiguation_and_different_album_artists() {
        let id = "b10bbbfc-cf9e-42e0-be17-e2c3e1d2600d";
        let mut data = json!({"count":1,"artists":[{"name":"The Beatles","id":id,"score":100}]});
        assert_eq!(
            unique_match(&data, "artists", "The Beatles", None),
            Ok(Some(id))
        );
        data["count"] = json!(2);
        assert_eq!(unique_match(&data, "artists", "The Beatles", None), Err(()));
        let duplicate = data["artists"][0].clone();
        data["artists"].as_array_mut().unwrap().push(duplicate);
        assert_eq!(unique_match(&data, "artists", "The Beatles", None), Err(()));
        let data = json!({"count":1,"release-groups":[{"title":"Album","id":id,"score":100,"artist-credit":[{"artist":{"name":"Someone else"}}]}]});
        assert_eq!(
            unique_match(&data, "albums", "Album", Some("Artist")),
            Ok(None)
        );
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
}
