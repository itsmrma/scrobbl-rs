use crate::{
    AppState, auth, calendar, db,
    dto::*,
    error::{AppError, Result},
};
use axum::{Json, extract::State, http::HeaderMap};
use sqlx::SqliteConnection;

pub const PERIODS: &[&str] = &["all", "today", "week", "month", "year", "7d", "30d", "365d"];
pub async fn read(conn: &mut SqliteConnection) -> Result<Preferences> {
    let calendar = calendar::current(conn).await?;
    async fn value(conn: &mut SqliteConnection, key: &str, default: &str) -> Result<String> {
        Ok(
            sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key=?")
                .bind(key)
                .fetch_optional(conn)
                .await?
                .unwrap_or_else(|| default.into()),
        )
    }
    Ok(Preferences {
        name: value(conn, "instance_name", "scrobbl.rs").await?,
        initial_period: value(conn, "initial_period", "all").await?,
        hour_format: value(conn, "hour_format", "24").await?,
        timezone: calendar.timezone,
        week_start: calendar.week_start,
        version: value(conn, "preferences_version", "0")
            .await?
            .parse()
            .map_err(|_| AppError::Internal)?,
    })
}
pub async fn get(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Preferences>> {
    auth::session(&state, &headers).await?;
    let mut view = state.storage.read_view().await?;
    Ok(Json(read(&mut view.transaction).await?))
}
pub async fn save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(form): Json<SavePreferences>,
) -> Result<Json<SavedPreferences>> {
    auth::require_write(&state, &headers).await?;
    let mut p = form.preferences;
    p.name = p.name.trim().into();
    if p.name.is_empty()
        || p.name.chars().count() > 80
        || p.name.chars().any(char::is_control)
        || !PERIODS.contains(&p.initial_period.as_str())
        || !["12", "24"].contains(&p.hour_format.as_str())
        || p.timezone.parse::<chrono_tz::Tz>().is_err()
        || !(0..=6).contains(&p.week_start)
    {
        return Err(AppError::BadRequest("Use a name of 1–80 characters, a valid period, 12/24-hour format and an IANA calendar.".into()));
    }
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE archive_state SET revision=revision WHERE singleton=1 AND maintenance=0 RETURNING revision").fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::Conflict("Archive is in maintenance.".into()))?;
    let previous = read(&mut tx).await?;
    if previous.version != p.version {
        return Err(AppError::Conflict(
            "Settings changed in another session. Reload settings before saving.".into(),
        ));
    }
    let changed = previous.timezone != p.timezone || previous.week_start != p.week_start;
    let job_id = if changed {
        let scope = form.calendar_scope.ok_or_else(|| {
            AppError::BadRequest("Choose how the calendar change applies before saving.".into())
        })?;
        calendar::change_in_transaction(
            &mut tx,
            &p.timezone,
            p.week_start,
            matches!(scope, CalendarScope::All),
        )
        .await?
    } else {
        None
    };
    for (key, value) in [
        ("instance_name", p.name.as_str()),
        ("initial_period", p.initial_period.as_str()),
        ("hour_format", p.hour_format.as_str()),
    ] {
        db::set_setting(&mut tx, key, value).await?;
    }
    db::set_setting(
        &mut tx,
        "preferences_version",
        &(previous.version + 1).to_string(),
    )
    .await?;
    let preferences = read(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(SavedPreferences {
        preferences,
        job_id,
    }))
}
