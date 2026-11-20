//! Fixed, versioned synthetic archive used by the Ask SQL evaluation.
//!
//! The fixture is deterministic: local dates are written in `Europe/Rome`, the
//! assigned calendar of a fresh archive, and nothing depends on today. It holds
//! no personal data. Both `tests/` and the `ask-eval` binary build it with
//! [`build`], so the expected result values in `evals/ask/sql_corpus.jsonl` are
//! scored against exactly the same rows the model's SQL runs on.
//!
//! Bump [`FIXTURE_VERSION`] whenever a row changes; the corpus expects a fixed
//! version and the model-free test re-checks every expected value.
use crate::{
    ask::projection::{self, ProjectionGate},
    db::{self, DurationKind, Event, Track},
    spotify::parse_track,
    storage::Storage,
};
use chrono::{NaiveDateTime, TimeZone};
use chrono_tz::Europe::Rome;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Version of the synthetic archive and of the expected values that depend on it.
pub const FIXTURE_VERSION: u32 = 2;

/// The date shown to the model as "today" while evaluating the fixed corpus.
/// Every corpus question names a fixed year or date, so this value never
/// changes an expected result; it only keeps relative wording unambiguous.
pub const TODAY: &str = "2025-02-01";

/// A built fixture: the archive plus its complete Ask projection.
pub struct Fixture {
    pub storage: Storage,
    pub projection: PathBuf,
}

impl Fixture {
    pub fn projection_path(&self) -> &Path {
        &self.projection
    }
}

/// Build the fixture inside `data_dir`, which the caller owns.
pub async fn build(data_dir: &Path) -> anyhow::Result<Fixture> {
    let storage = db::open(data_dir).await?;

    let joga = track(
        "spotify:track:0000000000000000000001",
        "Joga",
        Some(("al-homogenic", "Homogenic")),
        &[("ar-bjork", "Bjork")],
    );
    let show_single = track(
        "spotify:track:0000000000000000000002",
        "Show",
        Some(("al-show-single", "Show - Single")),
        &[("ar-ado-1", "Ado")],
    );
    let show_zanmu = track(
        "spotify:track:0000000000000000000003",
        "Show",
        Some(("al-zanmu", "Zanmu")),
        &[("ar-ado-2", "Ado")],
    );
    let odo = track(
        "spotify:track:0000000000000000000004",
        "Odo",
        Some(("al-zanmu", "Zanmu")),
        &[("ar-ado-2", "Ado")],
    );
    let boy_with_luv = track(
        "spotify:track:0000000000000000000006",
        "Boy With Luv",
        Some(("al-persona", "Persona")),
        &[("ar-bts", "BTS"), ("ar-halsey", "Halsey")],
    );
    let mikrokosmos = track(
        "spotify:track:0000000000000000000007",
        "Mikrokosmos",
        Some(("al-persona", "Persona")),
        &[("ar-bts", "BTS")],
    );
    let weird_fishes = track(
        "spotify:track:0000000000000000000008",
        "Weird Fishes",
        Some(("al-in-rainbows", "In Rainbows")),
        &[("ar-radiohead", "Radiohead")],
    );
    let hoppipolla = track(
        "spotify:track:0000000000000000000009",
        "Hoppipolla",
        Some(("al-takk", "Takk...")),
        &[("ar-sigur-ros", "Sigur Ros")],
    );
    let trashed = track(
        "spotify:track:0000000000000000000099",
        "Trashed Song",
        Some(("al-trash", "Trashed Album")),
        &[("ar-trash", "Trashed Artist")],
    );

    let spotify_import = |device: &str,
                          shuffle: bool,
                          skipped: bool,
                          offline: bool,
                          reason_start: Option<&str>,
                          reason_end: Option<&str>| {
        let mut context = json!({
            "platform": device,
            "conn_country": "IT",
            "shuffle": shuffle,
            "skipped": skipped,
            "offline": offline,
        });
        if let (Some(object), Some(reason)) = (context.as_object_mut(), reason_start) {
            object.insert("reason_start".into(), json!(reason));
        }
        if let (Some(object), Some(reason)) = (context.as_object_mut(), reason_end) {
            object.insert("reason_end".into(), json!(reason));
        }
        context
    };

    let events = vec![
        // Bjork's first play: an import and a reconciled API observation.
        listen(
            "spotify_import",
            "bjork-1",
            "2024-01-01 00:30",
            Some(200_000),
            DurationKind::Actual,
            true,
            joga.clone(),
            spotify_import(
                "android",
                false,
                false,
                false,
                Some("trackdone"),
                Some("endplay"),
            ),
        ),
        listen(
            "spotify_api",
            "bjork-1-api",
            "2024-01-01 00:30",
            Some(200_000),
            DurationKind::Actual,
            true,
            joga.clone(),
            json!({}),
        ),
        listen(
            "spotify_import",
            "ado-show-single",
            "2024-01-02 15:00",
            Some(190_000),
            DurationKind::Estimated,
            true,
            show_single.clone(),
            spotify_import("android", true, false, false, None, None),
        ),
        listen(
            "listenbrainz",
            "ado-show-zanmu",
            "2024-01-03 16:00",
            None,
            DurationKind::Unknown,
            true,
            show_zanmu,
            json!({}),
        ),
        // A short actual play: present in the projection, not counted.
        listen(
            "spotify_import",
            "ado-odo-short",
            "2024-01-03 17:00",
            Some(12_000),
            DurationKind::Actual,
            false,
            odo.clone(),
            spotify_import("android", true, false, false, None, None),
        ),
        listen(
            "spotify_api",
            "trashed",
            "2024-01-03 18:00",
            Some(220_000),
            DurationKind::Actual,
            true,
            trashed,
            json!({}),
        ),
        listen(
            "spotify_api",
            "bts-boy-with-luv",
            "2024-02-14 20:00",
            Some(230_000),
            DurationKind::Actual,
            true,
            boy_with_luv,
            json!({}),
        ),
        listen(
            "spotify_import",
            "radiohead-skipped-offline",
            "2024-02-15 21:00",
            Some(240_000),
            DurationKind::Actual,
            true,
            weird_fishes.clone(),
            spotify_import("windows", false, true, true, None, Some("fwdbtn")),
        ),
        // After midnight in the assigned calendar (00:50 local).
        listen(
            "spotify_import",
            "ado-odo-midnight",
            "2024-03-01 00:50",
            Some(180_000),
            DurationKind::Actual,
            true,
            odo.clone(),
            spotify_import("android", false, false, false, None, None),
        ),
        listen(
            "spotify_api",
            "sigur-hoppipolla",
            "2024-03-02 10:00",
            Some(250_000),
            DurationKind::Actual,
            true,
            hoppipolla,
            json!({}),
        ),
        // Two plays of the same song on one day.
        listen(
            "spotify_api",
            "bjork-repeat-1",
            "2024-03-02 11:00",
            Some(200_000),
            DurationKind::Estimated,
            true,
            joga.clone(),
            json!({}),
        ),
        listen(
            "spotify_api",
            "bjork-repeat-2",
            "2024-03-02 12:00",
            Some(120_000),
            DurationKind::Actual,
            true,
            joga.clone(),
            json!({}),
        ),
        // Possible duplicate still awaiting review: counted like the rest.
        listen(
            "spotify_import",
            "ado-odo-review",
            "2024-04-10 12:00",
            Some(180_000),
            DurationKind::Actual,
            true,
            odo,
            spotify_import("android", false, false, false, None, None),
        ),
        listen(
            "spotify_api",
            "bts-mikrokosmos",
            "2024-05-05 09:00",
            Some(220_000),
            DurationKind::Actual,
            true,
            mikrokosmos,
            json!({}),
        ),
        listen(
            "spotify_api",
            "radiohead-unknown",
            "2024-06-01 22:00",
            None,
            DurationKind::Unknown,
            true,
            weird_fishes,
            json!({}),
        ),
        // A four-day Bjork streak, then a two-day one after a gap.
        listen(
            "spotify_api",
            "bjork-streak-1",
            "2024-06-10 12:00",
            Some(180_000),
            DurationKind::Actual,
            true,
            joga.clone(),
            json!({}),
        ),
        listen(
            "spotify_api",
            "bjork-streak-2",
            "2024-06-11 12:00",
            Some(180_000),
            DurationKind::Actual,
            true,
            joga.clone(),
            json!({}),
        ),
        listen(
            "spotify_api",
            "bjork-streak-3",
            "2024-06-12 12:00",
            Some(180_000),
            DurationKind::Actual,
            true,
            joga.clone(),
            json!({}),
        ),
        listen(
            "spotify_api",
            "bjork-streak-4",
            "2024-06-13 12:00",
            Some(180_000),
            DurationKind::Actual,
            true,
            joga.clone(),
            json!({}),
        ),
        listen(
            "spotify_api",
            "bjork-streak-5",
            "2024-06-20 12:00",
            Some(180_000),
            DurationKind::Actual,
            true,
            joga.clone(),
            json!({}),
        ),
        listen(
            "spotify_api",
            "bjork-streak-6",
            "2024-06-21 12:00",
            Some(180_000),
            DurationKind::Actual,
            true,
            joga.clone(),
            json!({}),
        ),
        listen(
            "spotify_api",
            "bjork-july",
            "2024-07-20 08:00",
            Some(100_000),
            DurationKind::Actual,
            true,
            joga,
            json!({}),
        ),
        // A return after a year: Ado's last play in the fixture.
        listen(
            "spotify_api",
            "ado-return",
            "2025-01-15 12:00",
            Some(190_000),
            DurationKind::Actual,
            true,
            show_single,
            json!({}),
        ),
    ];

    let mut tx = storage.writer.begin().await?;
    db::ingest(&mut tx, &events).await?;
    sqlx::query("UPDATE listens SET trashed_at_ms = 1 WHERE id IN (SELECT listen_id FROM observations WHERE source_key = 'trashed')")
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE listens SET needs_review = 1 WHERE id IN (SELECT listen_id FROM observations WHERE source_key = 'ado-odo-review')")
        .execute(&mut *tx)
        .await?;

    // Local corrections on the artist, album and track names the corpus asks about.
    let bjork_group: String = sqlx::query_scalar(
        "SELECT m.group_id FROM name_group_members m JOIN artists a ON a.id = m.entity_id WHERE m.kind = 'artists' AND a.name = 'Bjork' LIMIT 1",
    )
    .fetch_one(&mut *tx)
    .await?;
    let homogenic_group: String = sqlx::query_scalar(
        "SELECT m.group_id FROM name_group_members m JOIN albums a ON a.id = m.entity_id WHERE m.kind = 'albums' AND a.name = 'Homogenic' LIMIT 1",
    )
    .fetch_one(&mut *tx)
    .await?;
    let joga_id: String = sqlx::query_scalar("SELECT id FROM tracks WHERE title = 'Joga' LIMIT 1")
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO local_corrections(entity_kind, entity_id, field, value_json) VALUES ('artist', ?, 'name', '\"Björk\"')")
        .bind(&bjork_group)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO local_corrections(entity_kind, entity_id, field, value_json) VALUES ('album', ?, 'name', '\"Homogenic (Corrected)\"')")
        .bind(&homogenic_group)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO local_corrections(entity_kind, entity_id, field, value_json) VALUES ('track', ?, 'name', '\"Jóga\"')")
        .bind(&joga_id)
        .execute(&mut *tx)
        .await?;
    db::publish(&mut tx, &std::collections::BTreeSet::new()).await?;
    tx.commit().await?;

    let gate = ProjectionGate::default();
    let projection = projection::ensure(&storage, &gate).await?;
    Ok(Fixture {
        storage,
        projection,
    })
}

fn track(uri: &str, title: &str, album: Option<(&str, &str)>, artists: &[(&str, &str)]) -> Track {
    parse_track(&json!({
        "uri": uri,
        "type": "track",
        "name": title,
        "duration_ms": 240_000,
        "album": album
            .map(|(id, name)| json!({"id": id, "name": name, "images": []}))
            .unwrap_or_else(|| json!({"id": "none", "name": "", "images": []})),
        "artists": artists
            .iter()
            .map(|(id, name)| json!({"id": id, "name": name}))
            .collect::<Vec<_>>(),
    }))
    .expect("the fixture track is valid Spotify metadata")
}

#[allow(clippy::too_many_arguments)]
fn listen(
    source: &'static str,
    key: &'static str,
    local: &str,
    listened_ms: Option<i64>,
    duration_kind: DurationKind,
    counted: bool,
    track: Track,
    context: Value,
) -> Event {
    let when = local_millis(local);
    Event {
        source,
        source_key: key.to_owned(),
        source_timestamp_ms: when,
        played_at_ms: when,
        listened_ms,
        duration_kind,
        counted,
        track,
        raw: context,
    }
}

/// Local `Europe/Rome` wall-clock time as UTC epoch milliseconds.
fn local_millis(local: &str) -> i64 {
    let naive = NaiveDateTime::parse_from_str(local, "%Y-%m-%d %H:%M")
        .expect("the fixture local time is valid");
    Rome.from_local_datetime(&naive)
        .single()
        .expect("the fixture local time exists exactly once")
        .timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ask::sandbox::{self, SandboxLimits, SqlValue};

    #[tokio::test]
    async fn the_fixed_fixture_builds_a_projection_with_the_documented_rows() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = build(dir.path()).await.unwrap();

        let count = |sql: &str| {
            let out =
                sandbox::execute(fixture.projection_path(), sql, SandboxLimits::default()).unwrap();
            out.rows
                .into_iter()
                .next()
                .unwrap()
                .into_iter()
                .next()
                .unwrap()
        };

        // 23 ingested observations (one reconciled pair), 21 non-trashed listens,
        // 20 counted plays.
        assert_eq!(count("SELECT COUNT(*) FROM listens"), SqlValue::Integer(21));
        assert_eq!(
            count("SELECT SUM(counted) FROM listens"),
            SqlValue::Integer(20)
        );
        // The reconciled listen carries both sources and the corrected artist name.
        assert_eq!(
            sandbox::execute(
                fixture.projection_path(),
                "SELECT a.name FROM listens l JOIN track_artists ta ON ta.track_id = l.track_id JOIN artists a ON a.id = ta.artist_id WHERE l.local_date = '2024-01-01'",
                SandboxLimits::default(),
            )
            .unwrap()
            .rows,
            vec![vec![SqlValue::Text("Björk".into())]]
        );
        // The trashed artist never appears; the review candidate does.
        assert_eq!(
            count("SELECT COUNT(*) FROM artists WHERE name = 'Trashed Artist'"),
            SqlValue::Integer(0)
        );
        assert_eq!(
            count("SELECT SUM(needs_review) FROM listens"),
            SqlValue::Integer(1)
        );
        assert_eq!(count("SELECT COUNT(*) FROM meta"), SqlValue::Integer(1));
    }
}
