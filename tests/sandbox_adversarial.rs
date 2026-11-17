//! Adversarial SQL sandbox corpus, projection secrecy scan, and server
//! concurrency tests (#210).
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post as post_route,
};
use rusqlite::Connection;
use scrobbl_rs::{
    AppState,
    ask::{
        Ask,
        parser::ModelParser,
        projection::{self, ProjectionGate},
        sandbox::{self, SandboxLimits, SandboxRule, SqlValue},
    },
    auth,
    config::Config,
    db::{self, DurationKind, Event},
    spotify::parse_track,
    web,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;

#[derive(Deserialize)]
struct CorpusCase {
    id: String,
    category: String,
    sql: String,
    expected: String,
}

fn adversarial_projection() -> NamedTempFile {
    let file = NamedTempFile::new().unwrap();
    let conn = Connection::open(file.path()).unwrap();
    conn.execute_batch(&projection::ddl()).unwrap();
    conn.execute(
        "INSERT INTO artists(id, name) VALUES ('a1', 'CanaryArtistSecret'), ('a2', 'Ado')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO albums(id, name) VALUES ('al1', 'CanaryAlbumSecret')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tracks(id, title, album_id, duration_ms, edition_key) VALUES ('t1', 'CanaryTrackSecret', 'al1', 210000, 'canary:a1')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO track_artists(track_id, artist_id, ordinal) VALUES ('t1', 'a1', 0)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO album_artists(album_id, artist_id, ordinal) VALUES ('al1', 'a1', 0)",
        [],
    )
    .unwrap();
    // Insert 60 listens so multi-way cartesian joins quickly blow through budgets.
    let mut stmt = conn
        .prepare(
            "INSERT INTO listens(id, track_id, played_at_ms, played_at_local, local_date, week_date, local_year, local_month, weekday, week_start, local_hour, listened_ms, duration_kind, counted, needs_review, sources) \
             VALUES (?1, 't1', ?2, '2024-01-10 11:00:00', '2024-01-10', '2024-01-08', 2024, 1, 3, 0, 11, 210000, 'actual', 1, 0, '[\"spotify_api\"]')",
        )
        .unwrap();
    for i in 0..60_i64 {
        stmt.execute(rusqlite::params![
            format!("l{i}"),
            1704880800000_i64 + i * 60_000
        ])
        .unwrap();
    }
    drop(stmt);
    file
}

#[test]
fn adversarial_corpus_runs_and_enforces_every_named_rule() {
    let raw = include_str!("fixtures/sandbox_adversarial.jsonl");
    let cases: Vec<CorpusCase> = raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("valid JSONL corpus line"))
        .collect();
    assert!(
        cases.len() >= 90,
        "corpus should remain comprehensive, found {}",
        cases.len()
    );

    let categories: BTreeSet<&str> = cases.iter().map(|c| c.category.as_str()).collect();
    for required in [
        "write",
        "attach_detach",
        "pragma",
        "vacuum",
        "load_extension",
        "schema_tables",
        "disallowed_functions",
        "multi_statement_comments",
        "explain",
        "recursive_cte",
        "cartesian_join",
        "blob_and_width",
        "deep_nesting",
        "long_pattern",
        "large_json",
        "sqlite_limits",
        "unicode_nul",
        "placeholders",
        "archive_only_tables",
        "valid_control",
    ] {
        assert!(
            categories.contains(required),
            "corpus missing required attack category `{required}`"
        );
    }

    let proj = adversarial_projection();
    let limits = SandboxLimits {
        timeout: Duration::from_millis(500),
        max_instructions: 200_000,
        ..SandboxLimits::default()
    };
    let canaries = [
        "CanaryArtistSecret",
        "CanaryAlbumSecret",
        "CanaryTrackSecret",
    ];

    let mut failures = Vec::new();
    for case in &cases {
        let started = Instant::now();
        let outcome = sandbox::execute(proj.path(), &case.sql, limits);
        let elapsed = started.elapsed();
        if elapsed >= Duration::from_millis(1500) {
            failures.push(format!(
                "case `{}` exceeded time budget ({elapsed:?})",
                case.id
            ));
        }

        match case.expected.as_str() {
            "ok" => match outcome {
                Ok(out) => {
                    if out.rows.is_empty() {
                        failures.push(format!("valid control `{}` returned no rows", case.id));
                    }
                }
                Err(err) => failures.push(format!("valid control `{}` failed: {err:?}", case.id)),
            },
            "budget_stopped" => match outcome {
                Ok(out) => failures.push(format!(
                    "case `{}` expected budget stop, got Ok({out:?})",
                    case.id
                )),
                Err(err) => {
                    if !matches!(
                        err.rule,
                        SandboxRule::InstructionLimit | SandboxRule::Timeout
                    ) {
                        failures.push(format!(
                            "case `{}` expected budget stop, got {err:?}",
                            case.id
                        ));
                    }
                    if !err.message.contains(err.rule_name()) {
                        failures.push(format!(
                            "case `{}` message missing rule name: {err:?}",
                            case.id
                        ));
                    }
                }
            },
            "budget_stopped_or_too_large" => match outcome {
                Ok(out) => failures.push(format!(
                    "case `{}` expected budget stop or output_too_large, got Ok({out:?})",
                    case.id
                )),
                Err(err) => {
                    if !matches!(
                        err.rule,
                        SandboxRule::InstructionLimit
                            | SandboxRule::Timeout
                            | SandboxRule::OutputTooLarge
                    ) {
                        failures.push(format!(
                            "case `{}` expected budget stop or output_too_large, got {err:?}",
                            case.id
                        ));
                    }
                    if !err.message.contains(err.rule_name()) {
                        failures.push(format!(
                            "case `{}` message missing rule name: {err:?}",
                            case.id
                        ));
                    }
                }
            },
            "null_or_output_too_large" => match outcome {
                Err(err) => {
                    if err.rule != SandboxRule::OutputTooLarge {
                        failures.push(format!(
                            "case `{}` expected output_too_large, got {err:?}",
                            case.id
                        ));
                    }
                    if !err.message.contains(err.rule_name()) {
                        failures.push(format!(
                            "case `{}` message missing rule name: {err:?}",
                            case.id
                        ));
                    }
                }
                Ok(out) => {
                    if out.rows != vec![vec![SqlValue::Null]] {
                        failures.push(format!(
                            "case `{}` expected [[Null]] when SQLITE_LIMIT_LENGTH caps printf, got {out:?}",
                            case.id
                        ));
                    }
                }
            },
            expected_rule => match outcome {
                Ok(out) => failures.push(format!(
                    "case `{}` should be rejected with `{expected_rule}`, got Ok({out:?})",
                    case.id
                )),
                Err(err) => {
                    if err.rule_name() != expected_rule {
                        failures.push(format!(
                            "case `{}` expected `{expected_rule}`, got `{}` ({err:?})",
                            case.id,
                            err.rule_name()
                        ));
                    }
                    if !err.message.contains(expected_rule) {
                        failures.push(format!(
                            "case `{}` owner message must name rule `{expected_rule}`: {}",
                            case.id, err.message
                        ));
                    }
                    for canary in canaries {
                        if err.model_error.contains(canary)
                            || err.message.contains(canary)
                            || err.detail.contains(canary)
                        {
                            failures.push(format!(
                                "case `{}` leaked canary `{canary}`: {err:?}",
                                case.id
                            ));
                        }
                    }
                }
            },
        }
    }
    assert!(
        failures.is_empty(),
        "adversarial corpus failures:\n{}",
        failures.join("\n")
    );

    // Also verify default SandboxLimits stops an endless recursive CTE well within budget.
    let started = Instant::now();
    let err = sandbox::execute(
        proj.path(),
        "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt) SELECT COUNT(*) FROM cnt",
        SandboxLimits::default(),
    )
    .unwrap_err();
    assert!(
        matches!(
            err.rule,
            SandboxRule::InstructionLimit | SandboxRule::Timeout
        ),
        "{err:?}"
    );
    assert!(started.elapsed() <= SandboxLimits::default().timeout + Duration::from_secs(1));
}

#[tokio::test]
async fn projection_file_contains_no_secrets_or_archive_only_tables() {
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let t1 = chrono::DateTime::parse_from_rfc3339("2024-02-01T12:00:00Z")
        .unwrap()
        .timestamp_millis();
    let track = parse_track(&json!({
        "uri": "spotify:track:0000000000000000000009",
        "type": "track",
        "name": "Starlight",
        "duration_ms": 240_000,
        "album": {"id": "al9", "name": "Black Holes", "images": []},
        "artists": [{"id": "ar9", "name": "Muse"}],
    }))
    .unwrap();
    let event = Event {
        source: "spotify_api",
        source_key: "scan-k1".into(),
        source_timestamp_ms: t1,
        played_at_ms: t1,
        listened_ms: Some(240_000),
        duration_kind: DurationKind::Actual,
        counted: true,
        track,
        raw: json!({}),
    };

    let mut tx = storage.writer.begin().await.unwrap();
    db::ingest(&mut tx, &[event]).await.unwrap();
    db::set_setting(&mut tx, "ai_remote_key", "sk-ai-key-canary-54321")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Seed private DB secrets (Spotify client secret, sessions, API keys).
    sqlx::query("INSERT INTO settings(key, value) VALUES ('spotify_client_secret', 'sp-client-secret-canary-98765')")
        .execute(&storage.private)
        .await
        .unwrap();
    sqlx::query("INSERT INTO sessions(digest, csrf, expires_at_ms) VALUES ('session-canary-digest', 'csrf-canary', 9999999999999)")
        .execute(&storage.private)
        .await
        .unwrap();
    sqlx::query("INSERT INTO api_keys(digest, name, expires_at_ms, revoked_at_ms) VALUES ('apikey-canary-digest', 'canary-key', NULL, NULL)")
        .execute(&storage.private)
        .await
        .unwrap();

    let gate = ProjectionGate::default();
    let proj_path = projection::ensure(&storage, &gate).await.unwrap();
    let raw_bytes = std::fs::read(&proj_path).unwrap();
    let raw_lossy = String::from_utf8_lossy(&raw_bytes);

    for secret in [
        "sp-client-secret-canary-98765",
        "sk-ai-key-canary-54321",
        "session-canary-digest",
        "apikey-canary-digest",
    ] {
        assert!(
            !raw_lossy.contains(secret),
            "projection file leaked a secret"
        );
        let utf16le: Vec<u8> = secret
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        assert!(
            !raw_bytes
                .windows(utf16le.len())
                .any(|w| w == utf16le.as_slice()),
            "projection file leaked a UTF-16LE secret"
        );
    }

    let conn = Connection::open(&proj_path).unwrap();
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type IN ('table', 'view')")
        .unwrap();
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();

    for forbidden_table in [
        "settings",
        "sessions",
        "api_keys",
        "owner",
        "spotify_tokens",
        "oauth_states",
        "original_payloads",
        "local_corrections",
        "observations",
        "source_files",
    ] {
        assert!(
            !tables
                .iter()
                .any(|t| t.eq_ignore_ascii_case(forbidden_table)),
            "forbidden table `{forbidden_table}` found in projection sqlite_master: {tables:?}"
        );
        assert!(
            !raw_lossy.contains(forbidden_table),
            "forbidden table name `{forbidden_table}` found in projection bytes"
        );
    }
}

#[derive(Clone, Default)]
struct MockAi {
    reply: Arc<Mutex<String>>,
}

async fn mock_chat(
    State(mock): State<MockAi>,
    _headers: HeaderMap,
    Json(_body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let reply = mock.reply.lock().unwrap().clone();
    (
        StatusCode::OK,
        Json(json!({"choices": [{"message": {"role": "assistant", "content": reply}}]})),
    )
}

#[tokio::test]
async fn server_keeps_serving_requests_while_sandbox_stops_runaway_sql() {
    let mock = MockAi::default();
    *mock.reply.lock().unwrap() = json!({
        "sql": "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt) SELECT COUNT(*) FROM cnt",
        "title": "Endless",
        "computed": "Endless loop test.",
        "answer_kind": "value",
        "columns": [{"name": "c", "format": "count"}]
    })
    .to_string();

    let mock_router = Router::new()
        .route("/v1/chat/completions", post_route(mock_chat))
        .with_state(mock);
    let mock_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock_addr = mock_listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(mock_listener, mock_router).await.unwrap() });
    let mock_url = format!("http://{mock_addr}/v1");

    let app_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let app_addr = app_listener.local_addr().unwrap();

    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let token = auth::random_secret();
    sqlx::query("INSERT INTO sessions VALUES(?,'test-csrf',9999999999999)")
        .bind(auth::digest(&token))
        .execute(&storage.private)
        .await
        .unwrap();

    let t1 = chrono::DateTime::parse_from_rfc3339("2024-02-01T12:00:00Z")
        .unwrap()
        .timestamp_millis();
    let track = parse_track(&json!({
        "uri": "spotify:track:0000000000000000000008",
        "type": "track",
        "name": "Hysteria",
        "duration_ms": 227_000,
        "album": {"id": "al8", "name": "Absolution", "images": []},
        "artists": [{"id": "ar8", "name": "Muse"}],
    }))
    .unwrap();
    let mut tx = storage.writer.begin().await.unwrap();
    db::ingest(
        &mut tx,
        &[Event {
            source: "spotify_api",
            source_key: "conc-k1".into(),
            source_timestamp_ms: t1,
            played_at_ms: t1,
            listened_ms: Some(227_000),
            duration_kind: DurationKind::Actual,
            counted: true,
            track,
            raw: json!({}),
        }],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let config = Config {
        bind_ip: "127.0.0.1".parse().unwrap(),
        external_origin: None,
        port: app_addr.port(),
        data_dir: dir.path().into(),
        sync_seconds: 120,
        now_playing_seconds: 30,
        import_limits: Default::default(),
        api_snapshot_limits: Default::default(),
        env_client_id: None,
        env_client_secret: None,
    };
    let mut state = AppState::new(storage, config);
    state.ask = Ask::with_parser(
        Arc::new(ModelParser::local(
            state.db.clone(),
            dir.path().join("models"),
        )),
        false,
        false,
        true,
    );
    let router = web::router(state);
    tokio::spawn(async move { axum::serve(app_listener, router).await.unwrap() });

    let base = format!("http://{app_addr}");
    let client = reqwest::Client::new();

    // Configure remote AI in SQL mode.
    let res = client
        .post(format!("{base}/api/v1/ai/settings"))
        .header("cookie", format!("scrobbl_session={token}"))
        .header("x-csrf-token", "test-csrf")
        .header("origin", &base)
        .json(&json!({
            "enabled": true,
            "provider": "remote",
            "answer_mode": "sql",
            "remote_url": mock_url,
            "remote_model": "test-model",
            "active_model": null,
            "threads": 4,
            "unload_minutes": 15,
            "model_dir": null,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Fire the runaway SQL Ask request in the background.
    let ask_client = client.clone();
    let ask_base = base.clone();
    let ask_token = token.clone();
    let ask_task = tokio::spawn(async move {
        let started = Instant::now();
        let res = ask_client
            .post(format!("{ask_base}/api/v1/ask"))
            .header("cookie", format!("scrobbl_session={ask_token}"))
            .header("x-csrf-token", "test-csrf")
            .header("origin", &ask_base)
            .json(&json!({"question": "Run an endless loop"}))
            .send()
            .await
            .unwrap();
        let status = res.status();
        let body: Value = res.json().await.unwrap();
        (status, body, started.elapsed())
    });

    // Give the Ask handler a moment to enter sandbox execution, then confirm
    // the HTTP server continues serving other requests promptly.
    tokio::time::sleep(Duration::from_millis(30)).await;
    for _ in 0..5 {
        let started = Instant::now();
        let health = client.get(format!("{base}/healthz")).send().await.unwrap();
        assert_eq!(health.status(), StatusCode::OK);

        let ask_status = client
            .get(format!("{base}/api/v1/ask/status"))
            .header("cookie", format!("scrobbl_session={token}"))
            .send()
            .await
            .unwrap();
        assert_eq!(ask_status.status(), StatusCode::OK);
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "concurrent HTTP requests stalled for {:?}",
            started.elapsed()
        );
    }

    let (status, body, elapsed) = ask_task.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "failed", "{body}");
    let rule = body["sql"]["rule_code"].as_str().unwrap_or("");
    assert!(
        matches!(rule, "instruction_limit" | "timeout"),
        "expected instruction_limit or timeout, got {body}"
    );
    assert!(
        elapsed <= SandboxLimits::default().timeout + Duration::from_secs(2),
        "runaway SQL took too long: {elapsed:?}"
    );
}
