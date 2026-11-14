//! Instance-wide ordered public image providers, independent of Spotify collection.
use crate::{
    AppState, auth, db,
    error::{AppError, Result},
};
use axum::{Json, extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum ImageProvider {
    /// Deezer public search: album covers and artist pictures, fast and wide.
    Deezer,
    /// Apple iTunes Search API: album covers.
    Itunes,
    /// MusicBrainz release group → Cover Art Archive: album covers.
    CoverArtArchive,
    /// MusicBrainz artist → Commons file or Wikidata P18: attributed portraits.
    Wikimedia,
}
impl ImageProvider {
    pub fn key(self) -> &'static str {
        match self {
            Self::Deezer => "deezer",
            Self::Itunes => "itunes",
            Self::CoverArtArchive => "cover_art_archive",
            Self::Wikimedia => "wikimedia",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Deezer => "Deezer",
            Self::Itunes => "Apple iTunes",
            Self::CoverArtArchive => "Cover Art Archive",
            Self::Wikimedia => "Wikimedia Commons",
        }
    }
    pub fn supports(self, kind: &str) -> bool {
        match self {
            Self::Deezer => kind == "albums" || kind == "artists",
            Self::Itunes | Self::CoverArtArchive => kind == "albums",
            Self::Wikimedia => kind == "artists",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct ProviderPreference {
    pub provider: ImageProvider,
    pub enabled: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct ArtworkSettings {
    pub providers: Vec<ProviderPreference>,
    pub version: i64,
}
const DEFAULT_ORDER: [ImageProvider; 4] = [
    ImageProvider::Deezer,
    ImageProvider::Itunes,
    ImageProvider::CoverArtArchive,
    ImageProvider::Wikimedia,
];
impl Default for ArtworkSettings {
    fn default() -> Self {
        Self {
            version: 0,
            providers: DEFAULT_ORDER
                .into_iter()
                .map(|provider| ProviderPreference {
                    provider,
                    enabled: true,
                })
                .collect(),
        }
    }
}
impl ArtworkSettings {
    pub fn enabled<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = ImageProvider> + 'a {
        self.providers
            .iter()
            .filter(move |p| p.enabled && p.provider.supports(kind))
            .map(|p| p.provider)
    }
    fn validate(&self) -> Result<()> {
        if self.providers.len() != DEFAULT_ORDER.len()
            || DEFAULT_ORDER.iter().any(|provider| {
                self.providers
                    .iter()
                    .filter(|v| v.provider == *provider)
                    .count()
                    != 1
            })
        {
            return Err(AppError::BadRequest(
                "Include each supported image provider exactly once.".into(),
            ));
        }
        Ok(())
    }
}

/// Settings saved before the image engine rewrite used `cover_art_archive`,
/// `musicbrainz_image`, `wikidata` and `itunes` (which also searched Deezer
/// for artists). They move to the new fast-first order, keeping the owner's
/// choices: a disabled legacy source disables its successor.
fn upgrade_legacy(value: &serde_json::Value) -> Option<ArtworkSettings> {
    let providers = value["providers"].as_array()?;
    let enabled = |key: &str| {
        providers
            .iter()
            .find(|p| p["provider"] == key)
            .map(|p| p["enabled"] == true)
    };
    let legacy = ["musicbrainz_image", "wikidata"]
        .iter()
        .any(|key| enabled(key).is_some());
    if !legacy {
        return None;
    }
    let itunes = enabled("itunes").unwrap_or(true);
    let flags = [
        itunes,
        itunes,
        enabled("cover_art_archive").unwrap_or(true),
        enabled("musicbrainz_image").unwrap_or(true) || enabled("wikidata").unwrap_or(true),
    ];
    Some(ArtworkSettings {
        version: value["version"].as_i64().unwrap_or(0),
        providers: DEFAULT_ORDER
            .into_iter()
            .zip(flags)
            .map(|(provider, enabled)| ProviderPreference { provider, enabled })
            .collect(),
    })
}

pub async fn read(conn: &mut SqliteConnection) -> Result<ArtworkSettings> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='artwork_providers'")
            .fetch_optional(conn)
            .await?;
    let settings = match value {
        Some(value) => {
            let raw: serde_json::Value =
                serde_json::from_str(&value).map_err(|_| AppError::Internal)?;
            if let Some(upgraded) = upgrade_legacy(&raw) {
                upgraded
            } else {
                let mut s: ArtworkSettings =
                    serde_json::from_value(raw).map_err(|_| AppError::Internal)?;
                for provider in DEFAULT_ORDER {
                    if !s.providers.iter().any(|p| p.provider == provider) {
                        s.providers.push(ProviderPreference {
                            provider,
                            enabled: true,
                        });
                    }
                }
                s
            }
        }
        None => ArtworkSettings::default(),
    };
    settings.validate()?;
    Ok(settings)
}
pub async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ArtworkSettings>> {
    auth::session(&state, &headers).await?;
    Ok(Json(read(&mut *state.read_db.acquire().await?).await?))
}
pub async fn save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut form): Json<ArtworkSettings>,
) -> Result<Json<ArtworkSettings>> {
    auth::require_write(&state, &headers).await?;
    form.validate()?;
    crate::library::writable(&state).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE archive_state SET revision=revision WHERE singleton=1 AND maintenance=0 RETURNING revision").fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::Conflict("Archive is in maintenance.".into()))?;
    let current = read(&mut tx).await?;
    if current.version != form.version {
        return Err(AppError::Conflict(
            "Image providers changed in another session. Reload before saving.".into(),
        ));
    }
    form.version += 1;
    db::set_setting(
        &mut tx,
        "artwork_providers",
        &serde_json::to_string(&form).map_err(|_| AppError::Internal)?,
    )
    .await?;
    // Retry absences under the new policy; saved bytes, including legacy Spotify, stay intact.
    sqlx::query("DELETE FROM artwork_misses")
        .execute(&mut *tx)
        .await?;
    // New settings can make previously unavailable work takeable, retaining user pauses.
    sqlx::query("UPDATE job_members SET artwork_retry_at_ms=0,artwork_attempts=0,error_code=NULL WHERE state='pending' AND job_id IN (SELECT id FROM jobs WHERE kind='artwork')").execute(&mut *tx).await?;
    crate::artwork::enqueue_missing(&mut tx).await?;
    sqlx::query("UPDATE jobs SET retry_at_ms=0 WHERE kind='artwork' AND state='waiting' AND error_code='artwork_provider_wait'").execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(form))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Connection;

    async fn with_saved(json: &str) -> ArtworkSettings {
        let mut conn = sqlx::SqliteConnection::connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .execute(&mut conn)
            .await
            .unwrap();
        sqlx::query("INSERT INTO settings (key, value) VALUES ('artwork_providers', ?)")
            .bind(json)
            .execute(&mut conn)
            .await
            .unwrap();
        read(&mut conn).await.unwrap()
    }

    #[test]
    fn default_order_is_fast_first_and_every_kind_has_a_source() {
        let settings = ArtworkSettings::default();
        assert_eq!(
            settings
                .providers
                .iter()
                .map(|p| p.provider)
                .collect::<Vec<_>>(),
            DEFAULT_ORDER
        );
        assert_eq!(
            settings.enabled("albums").next(),
            Some(ImageProvider::Deezer)
        );
        assert_eq!(
            settings.enabled("artists").next(),
            Some(ImageProvider::Deezer)
        );
        assert_eq!(settings.enabled("tracks").count(), 0);
    }

    #[tokio::test]
    async fn legacy_settings_move_to_the_new_order_keeping_disabled_sources() {
        let loaded = with_saved(r#"{"version":7,"providers":[{"provider":"cover_art_archive","enabled":false},{"provider":"musicbrainz_image","enabled":false},{"provider":"wikidata","enabled":false},{"provider":"itunes","enabled":true}]}"#).await;
        assert_eq!(loaded.version, 7);
        assert_eq!(
            loaded
                .providers
                .iter()
                .map(|p| (p.provider.key(), p.enabled))
                .collect::<Vec<_>>(),
            vec![
                ("deezer", true),
                ("itunes", true),
                ("cover_art_archive", false),
                ("wikimedia", false),
            ]
        );
    }

    #[tokio::test]
    async fn current_settings_round_trip_and_gain_new_providers() {
        let loaded = with_saved(r#"{"version":3,"providers":[{"provider":"wikimedia","enabled":true},{"provider":"deezer","enabled":false}]}"#).await;
        assert_eq!(loaded.providers.len(), 4);
        assert_eq!(loaded.providers[0].provider, ImageProvider::Wikimedia);
        assert!(!loaded.providers[1].enabled);
    }

    #[test]
    fn validate_rejects_duplicates_or_missing() {
        assert!(ArtworkSettings::default().validate().is_ok());
        let mut missing = ArtworkSettings::default();
        missing.providers.pop();
        assert!(missing.validate().is_err());
        let mut duplicate = ArtworkSettings::default();
        duplicate.providers[1].provider = ImageProvider::Deezer;
        assert!(duplicate.validate().is_err());
    }
}
