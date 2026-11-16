//! Bring your own AI: Ask interpreted by an OpenAI-compatible server.
//!
//! A local mock server stands in for Ollama, OpenAI and similar. Nothing here
//! touches the network beyond 127.0.0.1.
use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Request, StatusCode},
    routing::post as post_route,
};
use http_body_util::BodyExt;
use scrobbl_rs::{
    AppState,
    ask::{Ask, parser::ModelParser},
    auth,
    config::Config,
    db, web,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

type Seen = Arc<Mutex<Vec<(Option<String>, Value)>>>;

#[derive(Clone, Default)]
struct Mock {
    seen: Seen,
    /// Reject `json_schema` like a server that does not support it.
    reject_schema: bool,
    reply: Arc<Mutex<String>>,
    status: Arc<Mutex<u16>>,
}

async fn chat(
    State(mock): State<Mock>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let auth = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let schema = body["response_format"]["type"] == "json_schema";
    mock.seen.lock().unwrap().push((auth, body));
    let status = *mock.status.lock().unwrap();
    if status != 0 && status != 200 {
        return (
            StatusCode::from_u16(status).unwrap(),
            Json(json!({"error": {"message": "nope"}})),
        );
    }
    if mock.reject_schema && schema {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"message": "response_format json_schema is not supported"}})),
        );
    }
    let reply = mock.reply.lock().unwrap().clone();
    (
        StatusCode::OK,
        Json(json!({"choices": [{"message": {"role": "assistant", "content": reply}}]})),
    )
}

async fn serve(mock: Mock) -> String {
    let router = Router::new()
        .route("/v1/chat/completions", post_route(chat))
        .with_state(mock);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{address}/v1")
}

const PLAYS_OF_BJORK: &str = r#"{"artist":"","group_by":"none","limit":"none","metric":"plays","month":"none","name":"Björk","order":"desc","period":"all_time","target":"artist","weekday":"any","year":0}"#;

fn settings(url: &str, extra: Value) -> Value {
    let mut body = json!({
        "enabled": true,
        "provider": "remote",
        "answer_mode": "guided",
        "remote_url": url,
        "remote_model": "my-model",
        "active_model": null,
        "threads": 4,
        "unload_minutes": 15,
        "model_dir": null,
    });
    for (key, value) in extra.as_object().unwrap() {
        body[key] = value.clone();
    }
    body
}

async fn setup_with(url: &str, extra: Value) -> (tempfile::TempDir, AppState, String) {
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let config = Config {
        bind_ip: "127.0.0.1".parse().unwrap(),
        external_origin: None,
        port: 3210,
        data_dir: dir.path().into(),
        sync_seconds: 120,
        now_playing_seconds: 30,
        import_limits: Default::default(),
        api_snapshot_limits: Default::default(),
        env_client_id: None,
        env_client_secret: None,
    };
    let token = auth::random_secret();
    sqlx::query("INSERT INTO sessions VALUES(?,'test-csrf',9999999999999)")
        .bind(auth::digest(&token))
        .execute(&storage.private)
        .await
        .unwrap();
    let mut state = AppState::new(storage, config);
    state.ask = Ask::with_parser(
        Arc::new(ModelParser::local(
            state.db.clone(),
            dir.path().to_path_buf(),
        )),
        false,
        true,
        true,
    );
    let (status, body) = request(
        &state,
        &token,
        "POST",
        "/api/v1/ai/settings",
        Some(settings(url, extra)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    (dir, state, token)
}

async fn setup(url: &str) -> (tempfile::TempDir, AppState, String) {
    setup_with(url, json!({"remote_key": "sk-secret-key"})).await
}

async fn request(
    state: &AppState,
    token: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let response = web::router(state.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("host", "127.0.0.1:3210")
                .header("origin", "http://127.0.0.1:3210")
                .header("content-type", "application/json")
                .header("cookie", format!("scrobbl_session={token}"))
                .header("x-csrf-token", "test-csrf")
                .body(Body::from(
                    body.map(|b| serde_json::to_vec(&b).unwrap())
                        .unwrap_or_default(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn ask(state: &AppState, token: &str) -> (StatusCode, Value) {
    request(
        state,
        token,
        "POST",
        "/api/v1/ask",
        Some(json!({"question": "How many times did I listen to Björk?"})),
    )
    .await
}

#[tokio::test]
async fn a_remote_server_interprets_the_question_and_rust_still_computes() {
    let mock = Mock::default();
    *mock.reply.lock().unwrap() = PLAYS_OF_BJORK.into();
    let url = serve(mock.clone()).await;
    let (_dir, state, token) = setup(&url).await;

    let (status, body) = ask(&state, &token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // The artist is not in this empty archive: Rust, not the model, decides.
    assert_eq!(body["status"], "failed", "{body}");
    assert_eq!(body["query"]["metric"], "plays");
    assert_eq!(body["query"]["entity"]["name"], "Björk");

    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let (auth, sent) = &seen[0];
    assert_eq!(auth.as_deref(), Some("Bearer sk-secret-key"));
    assert_eq!(sent["model"], "my-model");
    assert_eq!(sent["temperature"], 0);
    assert_eq!(sent["response_format"]["type"], "json_schema");
    assert_eq!(
        sent["messages"][1]["content"],
        "How many times did I listen to Björk?"
    );
    // Only the vocabulary prompt and the question are sent.
    assert_eq!(sent["messages"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn it_falls_back_when_the_server_rejects_json_schema() {
    let mock = Mock {
        reject_schema: true,
        ..Mock::default()
    };
    *mock.reply.lock().unwrap() = format!("```json\n{PLAYS_OF_BJORK}\n```");
    let url = serve(mock.clone()).await;
    let (_dir, state, token) = setup(&url).await;

    let (_, body) = ask(&state, &token).await;
    assert_eq!(body["query"]["metric"], "plays", "{body}");
    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].1["response_format"]["type"], "json_schema");
    assert_eq!(seen[1].1["response_format"]["type"], "json_object");
}

#[tokio::test]
async fn server_errors_become_clear_messages_and_the_key_stays_secret() {
    let mock = Mock::default();
    *mock.status.lock().unwrap() = 401;
    let url = serve(mock).await;
    let (_dir, state, token) = setup(&url).await;

    let (status, body) = ask(&state, &token).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.to_string().contains("refused the API key"), "{body}");
    assert!(!body.to_string().contains("sk-secret-key"));

    let (_, stored) = request(&state, &token, "GET", "/api/v1/ai/settings", None).await;
    assert_eq!(stored["provider"], "remote");
    assert_eq!(stored["remote_key_set"], true);
    assert_eq!(stored["model_ready"], true);
    assert!(!stored.to_string().contains("sk-secret-key"));

    let (_, ready) = request(&state, &token, "GET", "/api/v1/ask/status", None).await;
    assert_eq!(ready["provider"], "remote");
    assert_eq!(ready["model_ready"], true);
    assert_eq!(ready["remote_model"], "my-model");

    // Saving without a key keeps the stored one; an empty key clears it.
    let (status, kept) = request(
        &state,
        &token,
        "POST",
        "/api/v1/ai/settings",
        Some(settings(&url, json!({}))),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{kept}");
    assert_eq!(kept["remote_key_set"], true);
    let (_, cleared) = request(
        &state,
        &token,
        "POST",
        "/api/v1/ai/settings",
        Some(settings(&url, json!({"remote_key": ""}))),
    )
    .await;
    assert_eq!(cleared["remote_key_set"], false);
}

#[tokio::test]
async fn the_connection_test_checks_the_server_and_settings_are_validated() {
    let mock = Mock::default();
    *mock.reply.lock().unwrap() = PLAYS_OF_BJORK.into();
    let url = serve(mock).await;
    let (_dir, state, token) = setup(&url).await;

    let (status, test) = request(&state, &token, "POST", "/api/v1/ai/test", None).await;
    assert_eq!(status, StatusCode::OK, "{test}");
    assert_eq!(test["model_ready"], true, "{test}");
    assert!(test["message"].as_str().unwrap().contains("Connected"));

    for bad in ["ftp://host/v1", "http://user:pw@host/v1", "nonsense"] {
        let (status, _) = request(
            &state,
            &token,
            "POST",
            "/api/v1/ai/settings",
            Some(settings(bad, json!({}))),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
    let (status, _) = request(
        &state,
        &token,
        "POST",
        "/api/v1/ai/settings",
        Some(settings(
            "",
            json!({"remote_url": null, "remote_model": null}),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---- Chained steps, follow-ups and explicit ranges -------------------------

fn at(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s)
        .unwrap()
        .timestamp_millis()
}

fn play(
    index: usize,
    when: &str,
    artist_id: &str,
    artist: &str,
    title: &str,
) -> scrobbl_rs::db::Event {
    let raw = json!({
        "uri": format!("spotify:track:{index:022}"),
        "type": "track",
        "name": title,
        "duration_ms": 240000,
        "album": {"id": format!("album-{artist_id}"), "name": format!("{artist} album"), "images": []},
        "artists": [{"id": artist_id, "name": artist}],
    });
    let when = at(when);
    scrobbl_rs::db::Event {
        source: "spotify_api",
        source_key: format!("{index}:{when}"),
        source_timestamp_ms: when,
        played_at_ms: when,
        listened_ms: Some(200_000),
        duration_kind: scrobbl_rs::db::DurationKind::Actual,
        counted: true,
        track: scrobbl_rs::spotify::parse_track(&raw).unwrap(),
        raw,
    }
}

async fn seed(state: &AppState) {
    // Björk: three plays in January, one in February 2024. Radiohead: one.
    let events = vec![
        play(1, "2024-01-10T10:00:00Z", "bjork", "Björk", "Jóga"),
        play(1, "2024-01-11T10:00:00Z", "bjork", "Björk", "Jóga"),
        play(2, "2024-01-12T10:00:00Z", "bjork", "Björk", "Hunter"),
        play(2, "2024-02-12T10:00:00Z", "bjork", "Björk", "Hunter"),
        play(3, "2024-02-13T10:00:00Z", "radiohead", "Radiohead", "Creep"),
    ];
    let mut tx = state.storage.writer.begin().await.unwrap();
    scrobbl_rs::db::ingest(&mut tx, &events).await.unwrap();
    tx.commit().await.unwrap();
}

fn step(group_by: &str, limit: &str, name: &str, target: &str, extra: Value) -> Value {
    let mut step = json!({
        "artist": "", "group_by": group_by, "limit": limit, "metric": "plays",
        "month": "none", "name": name, "order": "desc", "period": "all_time",
        "target": target, "weekday": "any", "year": 0, "from": "", "to": "",
        "hour": -1, "source": "any", "ref_step": -1, "ref_row": 0,
    });
    for (key, value) in extra.as_object().unwrap() {
        step[key] = value.clone();
    }
    step
}

#[tokio::test]
async fn a_chained_question_runs_each_step_and_binds_rows_in_rust() {
    let mock = Mock::default();
    // "In the month I listened to my top artist the most, which track?" in
    // three steps: top artist, its top month, the top track of that month.
    *mock.reply.lock().unwrap() = json!({"steps": [
        step("artist", "one", "", "none", json!({})),
        step("month", "one", "", "none", json!({"ref_step": 0})),
        step("track", "one", "", "none", json!({"ref_step": 1})),
    ]})
    .to_string();
    let url = serve(mock.clone()).await;
    let (_dir, state, token) = setup(&url).await;
    seed(&state).await;

    let (status, body) = ask(&state, &token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ok", "{body}");
    // The last step answers; the two before it are kept, oldest first.
    let earlier = body["earlier"].as_array().unwrap();
    assert_eq!(earlier.len(), 2);
    assert_eq!(earlier[0]["result"]["rows"][0]["label"], "Björk");
    assert_eq!(earlier[1]["result"]["rows"][0]["label"], "2024-01");
    assert_eq!(earlier[1]["query"]["entity"]["name"], "Björk");
    assert_eq!(body["result"]["rows"][0]["label"], "Jóga");
    assert_eq!(body["result"]["rows"][0]["plays"], 2);
    assert_eq!(body["query"]["from"], "2024-01-01");
    assert_eq!(body["query"]["to"], "2024-01-31");
    assert_eq!(body["query"]["entity"]["name"], "Björk");

    // One request, and the model never saw any result.
    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let sent = seen[0].1.to_string();
    assert!(!sent.contains("Jóga") && !sent.contains("Hunter"), "{sent}");
}

#[tokio::test]
async fn a_follow_up_sends_the_previous_interpretation_but_never_results() {
    let mock = Mock::default();
    *mock.reply.lock().unwrap() =
        json!({"steps": [step("none", "none", "Björk", "artist", json!({"year": 2024}))]})
            .to_string();
    let url = serve(mock.clone()).await;
    let (_dir, state, token) = setup(&url).await;
    seed(&state).await;

    let (status, body) = request(
        &state,
        &token,
        "POST",
        "/api/v1/ask",
        Some(json!({
            "question": "and in 2024?",
            "previous": {
                "question": "How many times did I listen to Björk?",
                "chips": [
                    {"key": "metric", "label": "Metric", "value": "Plays"},
                    {"key": "entity", "label": "Artist", "value": "Björk"},
                ],
            },
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ok", "{body}");
    assert_eq!(body["result"]["scalar"], 4);
    assert_eq!(body["earlier"].as_array().unwrap().len(), 0);

    let seen = mock.seen.lock().unwrap();
    let user = seen[0].1["messages"][1]["content"].as_str().unwrap();
    assert!(user.contains("Previous question: How many times did I listen to Björk?"));
    assert!(user.contains("Previous interpretation: Metric: Plays; Artist: Björk"));
    assert!(user.ends_with("New question: and in 2024?"), "{user}");
}

#[tokio::test]
async fn an_explicit_range_an_hour_and_a_source_narrow_one_step() {
    let mock = Mock::default();
    *mock.reply.lock().unwrap() = json!({"steps": [step(
        "track", "five", "", "none",
        json!({"from": "2024-02-01", "to": "2024-02-29", "hour": 11, "source": "spotify_api"}),
    )]})
    .to_string();
    let url = serve(mock).await;
    let (_dir, state, token) = setup(&url).await;
    seed(&state).await;

    let (_, body) = ask(&state, &token).await;
    assert_eq!(body["status"], "ok", "{body}");
    assert_eq!(body["query"]["from"], "2024-02-01");
    assert_eq!(body["query"]["hour"], 11);
    assert_eq!(body["query"]["source"], "spotify_api");
    let labels: Vec<_> = body["result"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["label"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(labels, ["Creep", "Hunter"]);
    // The range is a chip, so it can be edited like any other part.
    assert!(
        body["chips"]
            .as_array()
            .unwrap()
            .iter()
            .any(|chip| chip["key"] == "from" && chip["value"] == "2024-02-01")
    );
}

#[tokio::test]
async fn impossible_plans_are_refused_or_fail_clearly() {
    let mock = Mock::default();
    let url = serve(mock.clone()).await;
    let (_dir, state, token) = setup(&url).await;
    seed(&state).await;

    // A step cannot reference itself or a later step.
    *mock.reply.lock().unwrap() =
        json!({"steps": [step("none", "none", "Björk", "artist", json!({"ref_step": 0}))]})
            .to_string();
    let (_, body) = ask(&state, &token).await;
    assert_eq!(body["status"], "unsupported", "{body}");

    // A bad date is refused, not guessed.
    *mock.reply.lock().unwrap() =
        json!({"steps": [step("none", "none", "Björk", "artist", json!({"from": "last week"}))]})
            .to_string();
    let (_, body) = ask(&state, &token).await;
    assert_eq!(body["status"], "unsupported", "{body}");

    // Too many steps.
    let many: Vec<Value> = (0..5)
        .map(|_| step("none", "none", "Björk", "artist", json!({})))
        .collect();
    *mock.reply.lock().unwrap() = json!({ "steps": many }).to_string();
    let (_, body) = ask(&state, &token).await;
    assert_eq!(body["status"], "unsupported", "{body}");

    // Building on a step that gave no rows fails with an explanation.
    *mock.reply.lock().unwrap() = json!({"steps": [
        step("none", "none", "Björk", "artist", json!({})),
        step("track", "one", "", "none", json!({"ref_step": 0})),
    ]})
    .to_string();
    let (_, body) = ask(&state, &token).await;
    assert_eq!(body["status"], "failed", "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("did not group its answer"),
        "{body}"
    );
    assert_eq!(body["earlier"].as_array().unwrap().len(), 1);
}

// ---- Free questions (SQL mode) tracer bullet ------------------------------

async fn setup_sql_default(url: &str) -> (tempfile::TempDir, AppState, String) {
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let config = Config {
        bind_ip: "127.0.0.1".parse().unwrap(),
        external_origin: None,
        port: 3210,
        data_dir: dir.path().into(),
        sync_seconds: 120,
        now_playing_seconds: 30,
        import_limits: Default::default(),
        api_snapshot_limits: Default::default(),
        env_client_id: None,
        env_client_secret: None,
    };
    let token = auth::random_secret();
    sqlx::query("INSERT INTO sessions VALUES(?,'test-csrf',9999999999999)")
        .bind(auth::digest(&token))
        .execute(&storage.private)
        .await
        .unwrap();
    let mut state = AppState::new(storage, config);
    state.ask = Ask::with_parser(
        Arc::new(ModelParser::local(
            state.db.clone(),
            dir.path().to_path_buf(),
        )),
        false,
        true,
        true,
    );
    // Omit `answer_mode` so remote defaults to `sql`.
    let (status, body) = request(
        &state,
        &token,
        "POST",
        "/api/v1/ai/settings",
        Some(json!({
            "enabled": true,
            "provider": "remote",
            "remote_url": url,
            "remote_model": "my-model",
            "remote_key": "sk-secret-key",
            "active_model": null,
            "threads": 4,
            "unload_minutes": 15,
            "model_dir": null,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    (dir, state, token)
}

#[tokio::test]
async fn remote_defaults_to_sql_mode_while_local_stays_guided() {
    let mock = Mock::default();
    let url = serve(mock).await;
    let (_dir, state, token) = setup_sql_default(&url).await;

    let (_, ai) = request(&state, &token, "GET", "/api/v1/ai/settings", None).await;
    assert_eq!(ai["answer_mode"], "sql", "{ai}");

    let (_, ask_status) = request(&state, &token, "GET", "/api/v1/ask/status", None).await;
    assert_eq!(ask_status["answer_mode"], "sql", "{ask_status}");

    // Switching to local keeps local on guided; switching back preserves remote's choice.
    let (status, local_ai) = request(
        &state,
        &token,
        "POST",
        "/api/v1/ai/settings",
        Some(json!({
            "enabled": true,
            "provider": "local",
            "active_model": null,
            "threads": 4,
            "unload_minutes": 15,
            "model_dir": null,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{local_ai}");
    assert_eq!(local_ai["answer_mode"], "guided", "{local_ai}");
    let (_, local_status) = request(&state, &token, "GET", "/api/v1/ask/status", None).await;
    assert_eq!(local_status["answer_mode"], "guided", "{local_status}");
}

#[tokio::test]
async fn remote_sql_mode_executes_valid_query_against_projection_and_hides_archive_values() {
    let mock = Mock::default();
    *mock.reply.lock().unwrap() = json!({
        "sql": "SELECT t.title AS track, COUNT(*) AS plays FROM listens l JOIN tracks t ON t.id = l.track_id WHERE l.counted = 1 GROUP BY t.id ORDER BY plays DESC, t.title ASC",
        "title": "Top tracks",
        "computed": "Counted plays per track across all time.",
        "answer_kind": "table",
        "columns": [
            {"name": "track", "format": "track"},
            {"name": "plays", "format": "count"}
        ]
    })
    .to_string();
    let url = serve(mock.clone()).await;
    let (_dir, state, token) = setup_sql_default(&url).await;
    seed(&state).await;

    // Seed an extra track/artist with a distinct secret-like fixture name to verify privacy.
    let extra = play(
        99,
        "2024-03-01T10:00:00Z",
        "fixture-ar99",
        "PrivateFixtureArtist99",
        "PrivateFixtureTrack99",
    );
    let mut tx = state.storage.writer.begin().await.unwrap();
    scrobbl_rs::db::ingest(&mut tx, &[extra]).await.unwrap();
    tx.commit().await.unwrap();

    let (status, body) = request(
        &state,
        &token,
        "POST",
        "/api/v1/ask",
        Some(json!({"question": "What are my top tracks?"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ok", "{body}");
    assert_eq!(body["sql"]["title"], "Top tracks");
    assert_eq!(
        body["sql"]["computed"],
        "Counted plays per track across all time."
    );
    assert_eq!(body["sql"]["answer_kind"], "table");
    assert_eq!(
        body["sql"]["columns"],
        json!([
            {"name": "track", "format": "track"},
            {"name": "plays", "format": "count"}
        ])
    );
    assert_eq!(
        body["sql"]["rows"],
        json!([
            ["Hunter", 2],
            ["Jóga", 2],
            ["Creep", 1],
            ["PrivateFixtureTrack99", 1]
        ])
    );
    assert_eq!(body["sql"]["truncated"], false);

    // Privacy: prompt carries projection schema, counting rules, today's date, and NO archive values.
    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let sent = seen[0].1.to_string();
    assert!(sent.contains("listens("), "{sent}");
    assert!(sent.contains("counted = 1"), "{sent}");
    for secret in [
        "PrivateFixtureArtist99",
        "PrivateFixtureTrack99",
        "Radiohead",
        "Jóga",
        "Hunter",
        "Creep",
        "sk-secret-key",
    ] {
        assert!(!sent.contains(secret), "prompt leaked `{secret}`: {sent}");
    }
}

#[tokio::test]
async fn remote_sql_mode_rejects_unsafe_queries_with_rule_code_and_no_data() {
    let mock = Mock::default();
    let url = serve(mock.clone()).await;
    let (_dir, state, token) = setup_sql_default(&url).await;
    seed(&state).await;

    for (sql, expected_rule) in [
        ("DELETE FROM listens", "not_select"),
        ("SELECT 1; DROP TABLE listens", "multiple_statements"),
        ("SELECT value FROM settings", "unauthorized_table"),
    ] {
        *mock.reply.lock().unwrap() = json!({
            "sql": sql,
            "title": "Unsafe",
            "computed": "Should be rejected.",
            "answer_kind": "table",
            "columns": []
        })
        .to_string();

        let (status, body) = request(
            &state,
            &token,
            "POST",
            "/api/v1/ask",
            Some(json!({"question": "Give me everything"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["status"], "failed", "{sql}: {body}");
        assert_eq!(body["sql"]["rule_code"], expected_rule, "{sql}: {body}");
        assert_eq!(body["sql"]["rows"], json!([]), "{sql}: {body}");
    }
}

#[tokio::test]
async fn remote_sql_mode_handles_unsupported_question_without_running_sql() {
    let mock = Mock::default();
    *mock.reply.lock().unwrap() = json!({
        "sql": "",
        "title": "",
        "computed": "I can only answer questions about your listening archive.",
        "answer_kind": "unsupported",
        "columns": []
    })
    .to_string();
    let url = serve(mock).await;
    let (_dir, state, token) = setup_sql_default(&url).await;

    let (status, body) = request(
        &state,
        &token,
        "POST",
        "/api/v1/ask",
        Some(json!({"question": "What is the weather?"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "unsupported", "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("listening archive"),
        "{body}"
    );
}

#[tokio::test]
async fn remote_connection_test_accepts_sql_mode_response() {
    let mock = Mock::default();
    *mock.reply.lock().unwrap() = json!({
        "sql": "SELECT COUNT(*) AS plays FROM listens WHERE counted = 1",
        "title": "Plays",
        "computed": "Total counted plays.",
        "answer_kind": "value",
        "columns": [{"name": "plays", "format": "count"}]
    })
    .to_string();
    let url = serve(mock).await;
    let (_dir, state, token) = setup_sql_default(&url).await;

    let (status, test) = request(&state, &token, "POST", "/api/v1/ai/test", None).await;
    assert_eq!(status, StatusCode::OK, "{test}");
    assert_eq!(test["model_ready"], true, "{test}");
    assert!(test["message"].as_str().unwrap().contains("Connected"));
}
