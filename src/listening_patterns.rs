//! Per-listen listening habits. Missing source facts are never inferred.
use crate::{
    AppState, auth,
    consultation::{self, Filters},
    error::{AppError, Result},
};
use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Datelike, Timelike, Utc};
use serde_json::{Value, json};
use sqlx::Row;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn context(source: &str, raw: &Value) -> String {
    let mut result = serde_json::Map::new();
    if source == "spotify_import" {
        for key in ["platform", "conn_country", "reason_start", "reason_end"] {
            if let Some(value) = raw[key]
                .as_str()
                .map(str::trim)
                .filter(|v| !v.is_empty() && v.chars().count() <= 160)
            {
                result.insert(key.into(), json!(value));
            }
        }
        for key in ["shuffle", "skipped", "offline"] {
            if let Some(value) = raw[key].as_bool() {
                result.insert(key.into(), json!(value));
            }
        }
    }
    Value::Object(result).to_string()
}

fn fact(contexts: &[Value], key: &str) -> String {
    let values: BTreeSet<String> = contexts
        .iter()
        .filter_map(|v| match &v[key] {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Bool(b) => Some(if *b { "Yes" } else { "No" }.into()),
            _ => None,
        })
        .collect();
    match values.len() {
        0 => "Unknown".into(),
        1 => values.into_iter().next().unwrap_or_default(),
        _ => "Conflicting values".into(),
    }
}

pub async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filters): Query<Filters>,
) -> Result<Json<Value>> {
    auth::session(&state, &headers).await?;
    let mut view = state.storage.read_view().await?;
    let conn = &mut *view.transaction;
    let mut query = consultation::selection_at(conn, &filters, false, auth::now_ms()).await?;
    query.push("SELECT s.played_at_ms,c.timezone,(SELECT json_group_array(json(v.context_json)) FROM observations o JOIN observation_variants v ON v.observation_id=o.id WHERE o.listen_id=s.id AND o.source='spotify_import') AS contexts,s.duration_kind FROM selected s JOIN calendars c ON c.id=s.calendar_id WHERE s.counted=1");
    let rows = query.build().fetch_all(&mut *conn).await?;
    let total = rows.len();
    let mut series: BTreeMap<&str, BTreeMap<String, u64>> = BTreeMap::new();
    series.insert(
        "hours",
        (0..24).map(|n| (format!("{n:02}:00"), 0)).collect(),
    );
    series.insert(
        "weekdays",
        [
            "1 Monday",
            "2 Tuesday",
            "3 Wednesday",
            "4 Thursday",
            "5 Friday",
            "6 Saturday",
            "7 Sunday",
        ]
        .into_iter()
        .map(|s| (s.to_owned(), 0))
        .collect(),
    );
    let mut zones = BTreeMap::<String, chrono_tz::Tz>::new();
    for row in rows {
        let zone: String = row.get("timezone");
        let tz = if let Some(tz) = zones.get(&zone) {
            *tz
        } else {
            let tz = zone.parse().map_err(|_| AppError::Internal)?;
            zones.insert(zone, tz);
            tz
        };
        let local = DateTime::<Utc>::from_timestamp_millis(row.get("played_at_ms"))
            .ok_or(AppError::Internal)?
            .with_timezone(&tz);
        let contexts: Vec<Value> =
            serde_json::from_str(row.get("contexts")).map_err(|_| AppError::Internal)?;
        let weekday = format!(
            "{} {}",
            local.weekday().number_from_monday(),
            local.format("%A")
        );
        let mut values = vec![
            ("hours", format!("{:02}:00", local.hour())),
            ("weekdays", weekday),
            ("evidence", row.get::<String, _>("duration_kind")),
        ];
        for (metric, key) in [
            ("devices", "platform"),
            ("countries", "conn_country"),
            ("shuffle", "shuffle"),
            ("skipped", "skipped"),
            ("offline", "offline"),
            ("start", "reason_start"),
            ("end", "reason_end"),
        ] {
            values.push((metric, fact(&contexts, key)));
        }
        for (metric, label) in values {
            *series.entry(metric).or_default().entry(label).or_default() += 1;
        }
    }
    let mut coverage = BTreeMap::new();
    let series: BTreeMap<_,_> = series.into_iter().map(|(metric, bins)| {
        let unknown = bins.get("Unknown").copied().unwrap_or(0);
        let conflicting = bins.get("Conflicting values").copied().unwrap_or(0);
        coverage.insert(metric,json!({"known":total as u64 - unknown - conflicting,"unknown":unknown,"conflicting":conflicting}));
        let mut bins: Vec<_> = bins.into_iter().collect();
        if metric != "hours" && metric != "weekdays" { bins.sort_by(|a,b| b.1.cmp(&a.1).then_with(||a.0.cmp(&b.0))); }
        // Bound categorical output while preserving every listen in the total.
        if bins.len() > 30 {
            let others: u64 = bins.drain(29..).map(|(_,count)|count).sum();
            bins.push(("Other values".into(),others));
        }
        (metric,bins.into_iter().map(|(label,count)|json!({"label":label,"count":count})).collect::<Vec<_>>())
    }).collect();
    Ok(Json(
        json!({"total":total,"series":series,"coverage":coverage}),
    ))
}
