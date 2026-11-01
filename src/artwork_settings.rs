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
    CoverArtArchive,
    MusicbrainzImage,
    Wikidata,
    Itunes,
}
impl ImageProvider {
    pub fn key(self) -> &'static str {
        match self {
            Self::CoverArtArchive => "cover_art_archive",
            Self::MusicbrainzImage => "musicbrainz_image",
            Self::Wikidata => "wikidata",
            Self::Itunes => "itunes",
        }
    }
    pub fn supports(self, kind: &str) -> bool {
        match self {
            Self::CoverArtArchive => kind == "albums",
            Self::MusicbrainzImage | Self::Wikidata => kind == "artists",
            Self::Itunes => kind == "albums" || kind == "artists",
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
impl Default for ArtworkSettings {
    fn default() -> Self {
        Self {
            version: 0,
            providers: [
                ImageProvider::CoverArtArchive,
                ImageProvider::MusicbrainzImage,
                ImageProvider::Wikidata,
                ImageProvider::Itunes,
            ]
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
        let defaults = Self::default();
        if self.providers.len() != defaults.providers.len()
            || defaults.providers.iter().any(|p| {
                self.providers
                    .iter()
                    .filter(|v| v.provider == p.provider)
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
pub async fn read(conn: &mut SqliteConnection) -> Result<ArtworkSettings> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='artwork_providers'")
            .fetch_optional(conn)
            .await?;
    let settings = match value {
        Some(value) => {
            let mut s: ArtworkSettings =
                serde_json::from_str(&value).map_err(|_| AppError::Internal)?;
            let defaults = ArtworkSettings::default();
            for default_pref in defaults.providers {
                if !s
                    .providers
                    .iter()
                    .any(|p| p.provider == default_pref.provider)
                {
                    s.providers.push(default_pref);
                }
            }
            s
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

    #[test]
    fn itunes_provider_properties() {
        let provider = ImageProvider::Itunes;
        assert_eq!(provider.key(), "itunes");
        assert!(provider.supports("albums"));
        assert!(provider.supports("artists"));
        assert!(!provider.supports("tracks"));
    }

    #[test]
    fn default_artwork_settings_includes_itunes_fourth() {
        let settings = ArtworkSettings::default();
        assert_eq!(settings.providers.len(), 4);
        assert_eq!(
            settings.providers,
            vec![
                ProviderPreference {
                    provider: ImageProvider::CoverArtArchive,
                    enabled: true,
                },
                ProviderPreference {
                    provider: ImageProvider::MusicbrainzImage,
                    enabled: true,
                },
                ProviderPreference {
                    provider: ImageProvider::Wikidata,
                    enabled: true,
                },
                ProviderPreference {
                    provider: ImageProvider::Itunes,
                    enabled: true,
                },
            ]
        );
    }

    #[tokio::test]
    async fn read_appends_missing_itunes_provider_for_backward_compatibility() {
        use sqlx::Connection;
        let mut conn = sqlx::SqliteConnection::connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .execute(&mut conn)
            .await
            .unwrap();

        // Old settings JSON with only 3 providers (CAA disabled, MB enabled, Wikidata disabled)
        let legacy_json = r#"{"version":2,"providers":[{"provider":"cover_art_archive","enabled":false},{"provider":"musicbrainz_image","enabled":true},{"provider":"wikidata","enabled":false}]}"#;
        sqlx::query("INSERT INTO settings (key, value) VALUES ('artwork_providers', ?)")
            .bind(legacy_json)
            .execute(&mut conn)
            .await
            .unwrap();

        let loaded = read(&mut conn).await.unwrap();
        assert_eq!(loaded.version, 2);
        assert_eq!(loaded.providers.len(), 4);
        assert_eq!(loaded.providers[0].provider, ImageProvider::CoverArtArchive);
        assert!(!loaded.providers[0].enabled);
        assert_eq!(
            loaded.providers[1].provider,
            ImageProvider::MusicbrainzImage
        );
        assert!(loaded.providers[1].enabled);
        assert_eq!(loaded.providers[2].provider, ImageProvider::Wikidata);
        assert!(!loaded.providers[2].enabled);
        // Itunes appended at the end with enabled: true
        assert_eq!(loaded.providers[3].provider, ImageProvider::Itunes);
        assert!(loaded.providers[3].enabled);
    }

    #[test]
    fn validate_accepts_all_four_providers_and_rejects_duplicates_or_missing() {
        let valid = ArtworkSettings::default();
        assert!(valid.validate().is_ok());

        let missing = ArtworkSettings {
            version: 0,
            providers: vec![
                ProviderPreference {
                    provider: ImageProvider::CoverArtArchive,
                    enabled: true,
                },
                ProviderPreference {
                    provider: ImageProvider::MusicbrainzImage,
                    enabled: true,
                },
            ],
        };
        assert!(missing.validate().is_err());

        let duplicate = ArtworkSettings {
            version: 0,
            providers: vec![
                ProviderPreference {
                    provider: ImageProvider::CoverArtArchive,
                    enabled: true,
                },
                ProviderPreference {
                    provider: ImageProvider::CoverArtArchive,
                    enabled: false,
                },
                ProviderPreference {
                    provider: ImageProvider::MusicbrainzImage,
                    enabled: true,
                },
                ProviderPreference {
                    provider: ImageProvider::Wikidata,
                    enabled: true,
                },
            ],
        };
        assert!(duplicate.validate().is_err());
    }
}
