//! Portable projections of a frozen archive. The history selection is shared with the UI.
use anyhow::ensure;
use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Connection, Row, SqliteConnection, sqlite::SqliteConnectOptions};
use std::{
    io::{BufWriter, Write},
    path::Path,
};

pub const FORMAT_VERSION: i64 = 2;

#[derive(Default, Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
pub struct ExportSelection {
    pub from: Option<String>,
    pub to: Option<String>,
    pub q: Option<String>,
    pub status: Option<String>,
}
impl ExportSelection {
    pub fn filters(&self) -> crate::consultation::Filters {
        crate::consultation::Filters {
            from: self.from.clone(),
            to: self.to.clone(),
            q: self.q.clone(),
            status: self.status.clone(),
            ..Default::default()
        }
    }
}

fn csv(value: &str) -> String {
    // Spreadsheet readers must not interpret source-supplied metadata as formulas.
    let prefix = if value
        .trim_start()
        .starts_with(['=', '+', '-', '@', '\t', '\r'])
    {
        "'"
    } else {
        ""
    };
    format!("\"{prefix}{}\"", value.replace('"', "\"\""))
}

pub async fn write(
    snapshot: &Path,
    destination: &Path,
    format: &str,
    selection: &ExportSelection,
) -> anyhow::Result<i64> {
    ensure!(matches!(format, "json" | "csv"), "Choose JSON or CSV");
    let mut conn = SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(snapshot)
            .read_only(true),
    )
    .await?;
    let identity: (String, String, i64) =
        sqlx::query_as("SELECT instance_id,generation,revision FROM archive_state")
            .fetch_one(&mut conn)
            .await?;
    let mut query = crate::consultation::selection(&mut conn, &selection.filters()).await?;
    query.push("SELECT s.*,c.timezone,c.week_start,c.rules_version,t.duration_ms AS track_duration_ms, (SELECT json_group_array(json_object('id',id,'name',name)) FROM (SELECT a.id,a.name FROM track_artists ta JOIN artists a ON a.id=ta.artist_id WHERE ta.track_id=s.track_id ORDER BY ta.ordinal)) AS artists, (SELECT json_group_array(json_object('id',o.id,'source',o.source,'source_key',o.source_key,'identity_version',o.identity_version,'source_timestamp_ms',o.source_timestamp_ms,'raw',json('null'),'raw_available',json('false'),'raw_fingerprint',(SELECT fingerprint FROM original_payloads WHERE id=o.payload_id),'variants',json((SELECT json_group_array(json_object('id',v.id,'raw',json('null'),'raw_available',json('false'),'raw_fingerprint',v.fingerprint,'played_at_ms',v.played_at_ms,'listened_ms',v.listened_ms,'duration_kind',v.duration_kind,'counted',json(CASE v.counted WHEN 1 THEN 'true' ELSE 'false' END),'policy_version',v.policy_version,'occurrences',json((SELECT json_group_array(json_object('job_id',p.job_id,'file_id',p.file_id,'member',p.member,'row_number',p.row_number,'raw_fingerprint',(SELECT fingerprint FROM original_payloads WHERE id=p.payload_id),'source_retained',json(CASE WHEN EXISTS(SELECT 1 FROM source_files f WHERE f.id=p.file_id AND f.retained=1) THEN 'true' ELSE 'false' END))) FROM source_occurrences p WHERE p.variant_id=v.id)))) FROM observation_variants v WHERE v.observation_id=o.id)))) FROM observations o WHERE o.listen_id=s.id) AS observations FROM selected s JOIN calendars c ON c.id=s.calendar_id JOIN tracks t ON t.id=s.track_id ORDER BY s.played_at_ms,s.id");
    let mut file = BufWriter::new(
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?,
    );
    if format == "json" {
        let meta = json!({"format":"scrobbl-export","format_version":FORMAT_VERSION,"original_retention":"compact","app_version":env!("CARGO_PKG_VERSION"),"instance_id":identity.0,"generation":identity.1,"revision":identity.2,"selection":selection,"timestamp_unit":"UTC milliseconds since Unix epoch","duration_unit":"milliseconds"});
        let text = serde_json::to_string(&meta)?;
        write!(file, "{},\"listens\":[", &text[..text.len() - 1])?;
    } else {
        file.write_all(b"id,played_at_ms,title,album,artists_json,track_duration_ms,listened_ms,duration_kind,counted,calendar_id,timezone,week_start,local_date,week_date,first_acquired_at_ms,needs_review\r\n")?;
    }
    let mut count = 0;
    let mut rows = query.build().fetch(&mut conn);
    while let Some(row) = rows.try_next().await? {
        let artists: Value = serde_json::from_str(&row.get::<String, _>("artists"))?;
        let listened: Option<i64> = row.get("listened_ms");
        if format == "json" {
            if count > 0 {
                file.write_all(b",")?;
            }
            serde_json::to_writer(
                &mut file,
                &json!({"id":row.get::<String,_>("id"),"track_id":row.get::<String,_>("track_id"),"title":row.get::<String,_>("title"),"album_id":row.get::<Option<String>,_>("album_id"),"album":row.get::<Option<String>,_>("album"),"spotify_url":row.get::<Option<String>,_>("spotify_url"),"artists":artists,"track_duration_ms":row.get::<Option<i64>,_>("track_duration_ms"),"played_at_ms":row.get::<i64,_>("played_at_ms"),"first_acquired_at_ms":row.get::<i64,_>("first_acquired_at_ms"),"listened_ms":listened,"duration_kind":row.get::<String,_>("duration_kind"),"counted":row.get::<bool,_>("counted"),"needs_review":row.get::<bool,_>("needs_review"),"policy_version":row.get::<i64,_>("policy_version"),"calendar":{"id":row.get::<String,_>("calendar_id"),"timezone":row.get::<String,_>("timezone"),"week_start":row.get::<i64,_>("week_start"),"rules_version":row.get::<i64,_>("rules_version"),"local_date":row.get::<String,_>("local_date"),"week_date":row.get::<String,_>("week_date")},"observations":serde_json::from_str::<Value>(&row.get::<String,_>("observations"))?}),
            )?;
        } else {
            let fields = vec![
                row.get::<String, _>("id"),
                row.get::<i64, _>("played_at_ms").to_string(),
                csv(&row.get::<String, _>("title")),
                csv(&row.get::<Option<String>, _>("album").unwrap_or_default()),
                csv(&artists.to_string()),
                row.get::<Option<i64>, _>("track_duration_ms")
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                listened.map(|v| v.to_string()).unwrap_or_default(),
                row.get::<String, _>("duration_kind"),
                row.get::<bool, _>("counted").to_string(),
                csv(&row.get::<String, _>("calendar_id")),
                csv(&row.get::<String, _>("timezone")),
                row.get::<i64, _>("week_start").to_string(),
                row.get::<String, _>("local_date"),
                row.get::<String, _>("week_date"),
                row.get::<i64, _>("first_acquired_at_ms").to_string(),
                row.get::<bool, _>("needs_review").to_string(),
            ];
            write!(file, "{}\r\n", fields.join(","))?;
        }
        count += 1;
    }
    drop(rows);
    if format == "json" {
        file.write_all(b"]}")?;
    }
    file.flush()?;
    file.get_ref().sync_all()?;
    conn.close().await?;
    Ok(count)
}
