//! Assigned calendars. UTC instants never change when calendar preferences change.
use crate::{
    auth, db,
    error::{AppError, Result},
    jobs::{Job, validate_lease},
    storage::Storage,
};
use chrono::{DateTime, Datelike, Duration, Months, NaiveDate, Utc};
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
    pub fn parse_tz(&self) -> Result<Tz> {
        self.timezone
            .parse()
            .map_err(|_| AppError::BadRequest("Unknown IANA timezone.".into()))
    }
    pub fn dates_with_tz(&self, tz: Tz, time: i64) -> Result<(String, String)> {
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
    pub fn dates(&self, time: i64) -> Result<(String, String)> {
        self.dates_with_tz(self.parse_tz()?, time)
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
    query.push(") AND l.played_at_ms<=").push_bind(now_ms);
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
        let mut tx = self.writer.begin().await?;
        let job = change_in_transaction(&mut tx, timezone, week_start, whole_history).await?;
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
        let tz = calendar.parse_tz()?;
        let rows = sqlx::query(
            "SELECT id,played_at_ms FROM listens l WHERE NOT EXISTS(SELECT 1 FROM calendar_staging s WHERE s.job_id=? AND s.listen_id=l.id) ORDER BY id LIMIT 1000",
        )
        .bind(&lease.id)
        .fetch_all(&mut *tx)
        .await?;
        let mut staged = Vec::with_capacity(rows.len());
        for row in &rows {
            let (date, week) = calendar.dates_with_tz(tz, row.get("played_at_ms"))?;
            staged.push((row.get::<String, _>("id"), date, week));
        }
        for chunk in staged.chunks(200) {
            let mut builder = sqlx::QueryBuilder::new(
                "INSERT INTO calendar_staging(job_id,listen_id,local_date,week_date) ",
            );
            builder.push_values(chunk, |mut b, (listen_id, date, week)| {
                b.push_bind(&lease.id)
                    .push_bind(listen_id)
                    .push_bind(date)
                    .push_bind(week);
            });
            builder.build().execute(&mut *tx).await?;
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
        sqlx::query(
            "INSERT INTO changes(revision,listen_id,operation)
             SELECT ?,s.listen_id,CASE WHEN l.trashed_at_ms IS NULL THEN 'upsert' ELSE 'remove' END
             FROM calendar_staging s
             JOIN listens l ON l.id=s.listen_id
             WHERE s.job_id=?",
        )
        .bind(revision)
        .bind(&lease.id)
        .execute(&mut *tx)
        .await?;
        db::prune_revision_log(&mut tx, revision).await?;
        db::set_setting(&mut tx, "calendar_id", id).await?;
        sqlx::query("INSERT INTO settings(key,value) VALUES('preferences_version','1') ON CONFLICT(key) DO UPDATE SET value=CAST(value AS INTEGER)+1").execute(&mut *tx).await?;
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

/// Caller holds the archive writer; preferences and future/calendar work commit together.
pub(crate) async fn change_in_transaction(
    conn: &mut SqliteConnection,
    timezone: &str,
    week_start: i64,
    whole_history: bool,
) -> Result<Option<String>> {
    if timezone.parse::<Tz>().is_err() || !(0..=6).contains(&week_start) {
        return Err(AppError::BadRequest(
            "Use an IANA timezone and week start 0 (Monday) through 6.".into(),
        ));
    }
    // First statement reserves the sole writer before reading state.
    sqlx::query("UPDATE archive_state SET revision=revision WHERE singleton=1 AND maintenance=0 RETURNING revision").fetch_optional(&mut *conn).await?.ok_or_else(||AppError::Conflict("Archive is in maintenance.".into()))?;
    let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM jobs WHERE kind='calendar' AND state IN ('queued','running','waiting'))").fetch_one(&mut *conn).await?;
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
        .execute(&mut *conn)
        .await?;
    let job = if whole_history {
        let base: i64 = sqlx::query_scalar("SELECT revision FROM archive_state WHERE singleton=1")
            .fetch_one(&mut *conn)
            .await?;
        let job_id = db::new_id();
        sqlx::query("INSERT INTO jobs(id,kind,parameters_json,state,created_at_ms,updated_at_ms) VALUES(?,'calendar',?,'queued',?,?)")
                .bind(&job_id).bind(serde_json::json!({"calendar_id":calendar_id,"base_revision":base}).to_string()).bind(auth::now_ms()).bind(auth::now_ms()).execute(&mut *conn).await?;
        Some(job_id)
    } else {
        db::set_setting(conn, "calendar_id", &calendar_id).await?;
        sqlx::query("UPDATE archive_state SET revision=revision+1 WHERE singleton=1")
            .execute(&mut *conn)
            .await?;
        None
    };
    Ok(job)
}

pub fn bucket_bounds(
    day: NaiveDate,
    grain: &str,
    week_start: i64,
) -> Result<(NaiveDate, NaiveDate)> {
    let invalid = || AppError::BadRequest("Activity date outside supported range.".into());
    let start = match grain {
        "day" => day,
        "week" => day
            .checked_sub_signed(Duration::days(
                (i64::from(day.weekday().num_days_from_monday()) - week_start).rem_euclid(7),
            ))
            .ok_or_else(invalid)?,
        "month" => day.with_day(1).ok_or_else(invalid)?,
        "year" => NaiveDate::from_ymd_opt(day.year(), 1, 1).ok_or_else(invalid)?,
        _ => {
            return Err(AppError::BadRequest(
                "Choose day, week, month or year.".into(),
            ));
        }
    };
    let end = match grain {
        "day" => start,
        "week" => start
            .checked_add_signed(Duration::days(6))
            .ok_or_else(invalid)?,
        "month" => start
            .checked_add_months(Months::new(1))
            .and_then(|d| d.pred_opt())
            .ok_or_else(invalid)?,
        "year" => NaiveDate::from_ymd_opt(start.year() + 1, 1, 1)
            .and_then(|d| d.pred_opt())
            .ok_or_else(invalid)?,
        _ => unreachable!(),
    };
    Ok((start, end))
}
