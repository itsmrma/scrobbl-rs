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
}
impl ImageProvider {
    pub fn key(self) -> &'static str {
        match self {
            Self::CoverArtArchive => "cover_art_archive",
            Self::MusicbrainzImage => "musicbrainz_image",
            Self::Wikidata => "wikidata",
        }
    }
    pub fn supports(self, kind: &str) -> bool {
        match self {
            Self::CoverArtArchive => kind == "albums",
            Self::MusicbrainzImage | Self::Wikidata => kind == "artists",
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
        Some(value) => serde_json::from_str(&value).map_err(|_| AppError::Internal)?,
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
