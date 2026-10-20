//! Assigned calendars. UTC instants never change when calendar preferences change.
use crate::{
    auth, db,
    error::{AppError, Result},
    jobs::{Job, validate_lease},
    storage::Storage,
};
use chrono::{DateTime, Datelike, Duration, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Row, SqliteConnection};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Calendar {
    pub id: String,
    pub timezone: String,
    pub week_start: i64,
    pub rules_version: i64,
}
impl Calendar {
    pub fn dates(&self, time: i64) -> Result<(String, String)> {
        let tz: Tz = self
            .timezone
            .parse()
            .map_err(|_| AppError::BadRequest("Unknown IANA timezone.".into()))?;
        let day = DateTime::<Utc>::from_timestamp_millis(time)
            .ok_or_else(|| AppError::BadRequest("Timestamp outside calendar range.".into()))?
            .with_timezone(&tz)
            .date_naive();
        let offset =
            (i64::from(day.weekday().num_days_from_monday()) - self.week_start).rem_euclid(7);
        let week = day
            .checked_sub_signed(Duration::days(offset))
            .ok_or_else(|| AppError::BadRequest("Timestamp outside calendar range.".into()))?;
        Ok((day.to_string(), week.to_string()))
    }
}
/// Shared selection for UI, exports and integrations. Bounds apply to each assigned calendar.
pub enum Period {
    All,
    Today,
    Week,
    Month,
    Year,
    Dates {
        from: Option<chrono::NaiveDate>,
        through: Option<chrono::NaiveDate>,
    },
    RollingDays(i64),
}
pub async fn filter(
    conn: &mut SqliteConnection,
    query: &mut sqlx::QueryBuilder<sqlx::Sqlite>,
    period: &Period,
    now_ms: i64,
) -> Result<()> {
    match period {
        Period::All => return Ok(()),
        Period::Dates { from, through } => {
            if from.zip(*through).is_some_and(|(f, t)| f > t) {
                return Err(AppError::BadRequest(
                    "The end date must be on or after the start date.".into(),
                ));
            }
            if let Some(date) = from {
                query
                    .push(" AND l.local_date>=")
                    .push_bind(date.to_string());
            }
            if let Some(date) = through {
                query
                    .push(" AND l.local_date<=")
                    .push_bind(date.to_string());
            }
            return Ok(());
        }
        Period::RollingDays(days) => {
            if *days <= 0 || *days > 36600 {
                return Err(AppError::BadRequest("Invalid rolling period.".into()));
            }
            query
                .push(" AND l.played_at_ms>=")
                .push_bind(now_ms.saturating_sub(days * 86_400_000))
                .push(" AND l.played_at_ms<=")
                .push_bind(now_ms);
            return Ok(());
        }
        _ => {}
    }
    let calendars: Vec<Calendar> = sqlx::query_as("SELECT * FROM calendars")
        .fetch_all(conn)
        .await?;
    query.push(" AND (");
    for (index, calendar) in calendars.iter().enumerate() {
        let tz: Tz = calendar.timezone.parse().map_err(|_| AppError::Internal)?;
        let today = DateTime::<Utc>::from_timestamp_millis(now_ms)
            .ok_or(AppError::Internal)?
            .with_timezone(&tz)
            .date_naive();
        let start = match period {
            Period::Today => today,
            Period::Week => {
                today
                    - Duration::days(
                        (i64::from(today.weekday().num_days_from_monday()) - calendar.week_start)
                            .rem_euclid(7),
                    )
            }
            Period::Month => today.with_day(1).ok_or(AppError::Internal)?,
            Period::Year => {
                chrono::NaiveDate::from_ymd_opt(today.year(), 1, 1).ok_or(AppError::Internal)?
            }
            _ => unreachable!(),
        };
        if index > 0 {
            query.push(" OR ");
        }
        query
            .push("(l.calendar_id=")
            .push_bind(calendar.id.clone())
            .push(" AND l.local_date>=")
            .push_bind(start.to_string())
            .push(" AND l.local_date<=")
            .push_bind(today.to_string())
            .push(")");
    }
    if calendars.is_empty() {
        query.push("0");
    }
    query.push(")");
    Ok(())
}
pub(crate) async fn get(conn: &mut SqliteConnection, id: &str) -> Result<Calendar> {
    Ok(sqlx::query_as("SELECT * FROM calendars WHERE id=?")
        .bind(id)
        .fetch_one(conn)
        .await?)
}
pub(crate) async fn current(conn: &mut SqliteConnection) -> Result<Calendar> {
    Ok(sqlx::query_as(
        "SELECT c.* FROM calendars c JOIN settings s ON s.value=c.id WHERE s.key='calendar_id'",
    )
    .fetch_one(conn)
    .await?)
}
impl Storage {
    /// None means settings published for future acquisitions. Some(id) is a persistent global job.
    pub async fn change_calendar(
        &self,
        timezone: &str,
        week_start: i64,
        whole_history: bool,
    ) -> Result<Option<String>> {
        if timezone.parse::<Tz>().is_err() || !(0..=6).contains(&week_start) {
            return Err(AppError::BadRequest(
                "Use an IANA timezone and week start 0 (Monday) through 6.".into(),
            ));
        }
        let mut tx = self.writer.begin().await?;
        // First statement reserves the sole writer before reading state.
        sqlx::query("UPDATE archive_state SET revision=revision WHERE singleton=1 AND maintenance=0 RETURNING revision").fetch_optional(&mut *tx).await?.ok_or_else(||AppError::Conflict("Archive is in maintenance.".into()))?;
        let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM jobs WHERE kind='calendar' AND state IN ('queued','running','waiting'))").fetch_one(&mut *tx).await?;
        if active {
            return Err(AppError::Conflict(
                "A calendar recalculation is already pending.".into(),
            ));
        }
        let calendar_id = db::new_id();
        sqlx::query("INSERT INTO calendars VALUES(?,?,?,1)")
            .bind(&calendar_id)
            .bind(timezone)
            .bind(week_start)
            .execute(&mut *tx)
            .await?;
        let job = if whole_history {
            let base: i64 =
                sqlx::query_scalar("SELECT revision FROM archive_state WHERE singleton=1")
                    .fetch_one(&mut *tx)
                    .await?;
            let job_id = db::new_id();
            sqlx::query("INSERT INTO jobs(id,kind,parameters_json,state,created_at_ms,updated_at_ms) VALUES(?,'calendar',?,'queued',?,?)")
                .bind(&job_id).bind(serde_json::json!({"calendar_id":calendar_id,"base_revision":base}).to_string()).bind(auth::now_ms()).bind(auth::now_ms()).execute(&mut *tx).await?;
            Some(job_id)
        } else {
            db::set_setting(&mut tx, "calendar_id", &calendar_id).await?;
            sqlx::query("UPDATE archive_state SET revision=revision+1 WHERE singleton=1")
                .execute(&mut *tx)
                .await?;
            None
        };
        tx.commit().await?;
        Ok(job)
    }

    /// One bounded preparation step; publication is a single set-based transaction.
    /// Concurrent acquisitions/reconciliation cause a durable rebase, never a partial view.
    pub async fn calendar_step(&self, lease: &Job) -> Result<bool> {
        if lease.kind != "calendar" {
            return Err(AppError::BadRequest("Expected calendar job.".into()));
        }
        let mut tx = self.writer.begin().await?;
        validate_lease(&mut tx, lease).await?;
        let params: serde_json::Value =
            serde_json::from_str(&lease.parameters_json).map_err(|_| AppError::Internal)?;
        let id = params["calendar_id"].as_str().ok_or(AppError::Internal)?;
        let calendar = get(&mut tx, id).await?;
        let rows=sqlx::query("SELECT id,played_at_ms FROM listens l WHERE NOT EXISTS(SELECT 1 FROM calendar_staging s WHERE s.job_id=? AND s.listen_id=l.id) ORDER BY id LIMIT 1000")
            .bind(&lease.id).fetch_all(&mut *tx).await?;
        for row in &rows {
            let (date, week) = calendar.dates(row.get("played_at_ms"))?;
            sqlx::query("INSERT INTO calendar_staging VALUES(?,?,?,?)")
                .bind(&lease.id)
                .bind(row.get::<String, _>("id"))
                .bind(date)
                .bind(week)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("UPDATE jobs SET checkpoint=checkpoint+?,processed=processed+? WHERE id=?")
            .bind(rows.len() as i64)
            .bind(rows.len() as i64)
            .bind(&lease.id)
            .execute(&mut *tx)
            .await?;
        if rows.len() == 1000 {
            tx.commit().await?;
            return Ok(false);
        }
        let revision: i64 =
            sqlx::query_scalar("SELECT revision FROM archive_state WHERE singleton=1")
                .fetch_one(&mut *tx)
                .await?;
        if params["base_revision"].as_i64() != Some(revision) {
            sqlx::query("DELETE FROM calendar_staging WHERE job_id=?")
                .bind(&lease.id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE jobs SET checkpoint=0,processed=0,parameters_json=? WHERE id=?")
                .bind(serde_json::json!({"calendar_id":id,"base_revision":revision}).to_string())
                .bind(&lease.id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            return Ok(false);
        }
        let revision:i64=sqlx::query_scalar("UPDATE archive_state SET revision=revision+1 WHERE singleton=1 AND maintenance=0 RETURNING revision")
            .fetch_optional(&mut *tx).await?.ok_or_else(||AppError::Conflict("Archive is in maintenance.".into()))?;
        sqlx::query("UPDATE listens SET calendar_id=?,local_date=s.local_date,week_date=s.week_date,revision=? FROM calendar_staging s WHERE s.job_id=? AND s.listen_id=listens.id")
            .bind(id).bind(revision).bind(&lease.id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO changes(revision,listen_id,operation) SELECT ?,listen_id,'upsert' FROM calendar_staging WHERE job_id=?").bind(revision).bind(&lease.id).execute(&mut *tx).await?;
        db::set_setting(&mut tx, "calendar_id", id).await?;
        sqlx::query("UPDATE jobs SET state='completed',updated_at_ms=? WHERE id=?")
            .bind(auth::now_ms())
            .bind(&lease.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM calendar_staging WHERE job_id=?")
            .bind(&lease.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }
}
pub async fn worker(storage: Storage) {
    loop {
        if let Ok(Some(mut lease)) = storage.claim("calendar").await {
            loop {
                match storage.calendar_step(&lease).await {
                    Ok(true) => break,
                    Ok(false) => match storage.job(&lease.id).await {
                        Ok(job) => lease = job,
                        Err(_) => break,
                    },
                    Err(_) => {
                        let _ = storage
                            .pause(&lease, None, "calendar_recalculation_failed")
                            .await;
                        break;
                    }
                }
                tokio::task::yield_now().await;
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}
