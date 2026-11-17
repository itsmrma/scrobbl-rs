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
    ask::{
        Ask,
        parser::ModelParser,
        runtime::{self, Runtime},
    },
    auth,
    config::Config,
    db, web,
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tower::ServiceExt;

type Seen = Arc<Mutex<Vec<(Option<String>, Value)>>>;

#[derive(Clone, Default)]
struct Mock {
    seen: Seen,
    /// Reject `json_schema` like a server that does not support it.
    reject_schema: bool,
    reply: Arc<Mutex<String>>,
    replies: Arc<Mutex<VecDeque<String>>>,
    status: Arc<Mutex<u16>>,
    delay: Arc<Mutex<Duration>>,
    request_count: Arc<AtomicUsize>,
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
    mock.request_count.fetch_add(1, Ordering::SeqCst);
    let delay = *mock.delay.lock().unwrap();
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
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
    let reply = mock
        .replies
        .lock()
        .unwrap()
        .pop_front()
        .unwrap_or_else(|| mock.reply.lock().unwrap().clone());
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

// ---- Repair rounds, follow-ups, cancellation and honest failures (#212) ----

fn sql_reply(sql: &str, title: &str, computed: &str, answer_kind: &str, columns: Value) -> String {
    json!({
        "sql": sql,
        "title": title,
        "computed": computed,
        "answer_kind": answer_kind,
        "columns": columns,
    })
    .to_string()
}

#[tokio::test]
async fn remote_sql_mode_repairs_a_rejected_query_and_never_leaks_archive_values() {
    let mock = Mock::default();
    {
        let mut replies = mock.replies.lock().unwrap();
        replies.push_back(sql_reply(
            "SELECT missing_col FROM listens",
            "Total plays",
            "Counted plays across all time.",
            "value",
            json!([{"name": "plays", "format": "count"}]),
        ));
        replies.push_back(sql_reply(
            "SELECT COUNT(*) AS plays FROM listens WHERE counted = 1",
            "Total plays",
            "Counted plays across all time.",
            "value",
            json!([{"name": "plays", "format": "count"}]),
        ));
    }
    let url = serve(mock.clone()).await;
    let (_dir, state, token) = setup_sql_default(&url).await;
    seed(&state).await;

    let extra = play(
        99,
        "2024-03-01T10:00:00Z",
        "fixture-ar99",
        "PrivateRepairArtist",
        "PrivateRepairTrack",
    );
    let mut tx = state.storage.writer.begin().await.unwrap();
    scrobbl_rs::db::ingest(&mut tx, &[extra]).await.unwrap();
    tx.commit().await.unwrap();

    let (status, body) = request(
        &state,
        &token,
        "POST",
        "/api/v1/ask",
        Some(json!({"question": "How many plays do I have?"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ok", "{body}");
    assert_eq!(body["sql"]["rows"], json!([[6]]), "{body}");

    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "expected initial + 1 repair call");
    let repair_sent = seen[1].1.to_string();
    assert!(
        repair_sent.contains("SELECT missing_col FROM listens"),
        "repair prompt must include failed SQL: {repair_sent}"
    );
    assert!(
        repair_sent.contains("no_such_column") && repair_sent.contains("missing_col"),
        "repair prompt must include sanitized error: {repair_sent}"
    );
    for (_, req) in seen.iter() {
        let text = req.to_string();
        for secret in [
            "PrivateRepairArtist",
            "PrivateRepairTrack",
            "Jóga",
            "Hunter",
            "Creep",
            "Radiohead",
            "sk-secret-key",
        ] {
            assert!(!text.contains(secret), "request leaked `{secret}`: {text}");
        }
        let user_msg = req["messages"][1]["content"].as_str().unwrap();
        assert!(
            !user_msg.contains("Björk"),
            "user message leaked archive value: {user_msg}"
        );
    }
}

#[tokio::test]
async fn remote_sql_mode_exhausts_two_repair_rounds_and_returns_honest_failure() {
    let mock = Mock::default();
    {
        let mut replies = mock.replies.lock().unwrap();
        replies.push_back(sql_reply(
            "SELECT value FROM settings",
            "Secrets",
            "Read settings.",
            "table",
            json!([]),
        ));
        replies.push_back(sql_reply(
            "DELETE FROM listens",
            "Delete",
            "Delete listens.",
            "table",
            json!([]),
        ));
        replies.push_back(sql_reply(
            "SELECT bad_col FROM listens",
            "Bad col",
            "Read bad_col.",
            "table",
            json!([]),
        ));
    }
    let url = serve(mock.clone()).await;
    let (_dir, state, token) = setup_sql_default(&url).await;
    seed(&state).await;

    // Even for a question that guided mode could answer ("How many times did I listen to Björk?"),
    // SQL mode must fail honestly and never silently switch to guided questions.
    let (status, body) = request(
        &state,
        &token,
        "POST",
        "/api/v1/ask",
        Some(json!({"question": "How many times did I listen to Björk?"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "failed", "{body}");
    assert!(
        body["query"].is_null(),
        "must never switch to guided: {body}"
    );
    assert!(body["result"].is_null(), "{body}");
    assert_eq!(body["sql"]["rule_code"], "sql_error", "{body}");
    assert_eq!(body["sql"]["rows"], json!([]), "{body}");
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains("2 repair attempts"),
        "expected repair exhaustion message: {message}"
    );
    assert!(
        message.contains("sql_error") && message.contains("bad_col"),
        "expected rule name and identifier in message: {message}"
    );

    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen.len(), 3, "expected 1 initial + 2 repair calls");
    let third_user = seen[2].1["messages"][1]["content"].as_str().unwrap();
    assert!(
        third_user.contains("SELECT value FROM settings"),
        "{third_user}"
    );
    assert!(third_user.contains("unauthorized_table"), "{third_user}");
    assert!(third_user.contains("DELETE FROM listens"), "{third_user}");
    assert!(third_user.contains("not_select"), "{third_user}");
}

#[tokio::test]
async fn remote_sql_mode_reports_timeout_honestly_without_retrying() {
    let mock = Mock::default();
    *mock.reply.lock().unwrap() = sql_reply(
        "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt) SELECT COUNT(*) FROM cnt",
        "Endless",
        "Endless loop.",
        "value",
        json!([{"name": "c", "format": "count"}]),
    );
    let url = serve(mock.clone()).await;
    let (_dir, mut state, token) = setup_sql_default(&url).await;
    state.ask.sql_limits.timeout = Duration::from_millis(5);
    state.ask.sql_limits.max_instructions = 10_000_000_000;
    seed(&state).await;

    let (status, body) = request(
        &state,
        &token,
        "POST",
        "/api/v1/ask",
        Some(json!({"question": "Count forever"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "failed", "{body}");
    assert_eq!(body["sql"]["rule_code"], "timeout", "{body}");
    let message = body["message"].as_str().unwrap();
    assert!(message.contains("timeout"), "{message}");
    assert_eq!(
        mock.seen.lock().unwrap().len(),
        1,
        "timeout must stop immediately without wasting repair rounds"
    );
}

#[tokio::test]
async fn remote_sql_mode_repair_can_conclude_unsupported_or_handle_bad_repair_json() {
    let mock = Mock::default();
    {
        let mut replies = mock.replies.lock().unwrap();
        replies.push_back(sql_reply(
            "SELECT mood FROM listens",
            "Mood",
            "Plays by mood.",
            "table",
            json!([]),
        ));
        replies.push_back(sql_reply(
            "",
            "",
            "The archive does not record song moods.",
            "unsupported",
            json!([]),
        ));
    }
    let url = serve(mock.clone()).await;
    let (_dir, state, token) = setup_sql_default(&url).await;
    seed(&state).await;

    let (status, body) = request(
        &state,
        &token,
        "POST",
        "/api/v1/ask",
        Some(json!({"question": "What mood do I listen to most?"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "unsupported", "{body}");
    assert!(
        body["message"].as_str().unwrap().contains("moods"),
        "{body}"
    );

    // If a repair round emits unparseable output instead of valid JSON, return an honest failure
    // keeping the last rule code and SQL rather than dropping into a generic HTTP 400.
    {
        let mut replies = mock.replies.lock().unwrap();
        replies.push_back(sql_reply(
            "SELECT value FROM settings",
            "Secrets",
            "Read settings.",
            "table",
            json!([]),
        ));
        replies.push_back("not valid json at all".into());
    }
    let (status, body) = request(
        &state,
        &token,
        "POST",
        "/api/v1/ask",
        Some(json!({"question": "Show secrets"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "failed", "{body}");
    assert_eq!(body["sql"]["rule_code"], "unauthorized_table", "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("unauthorized_table"),
        "{body}"
    );
}

#[tokio::test]
async fn remote_sql_mode_follow_up_sends_previous_sql_and_no_results() {
    let mock = Mock::default();
    let prev_sql = "SELECT a.name AS artist, COUNT(DISTINCT l.id) AS plays FROM listens l JOIN track_artists ta ON ta.track_id = l.track_id JOIN artists a ON a.id = ta.artist_id WHERE l.counted = 1 GROUP BY a.id ORDER BY plays DESC";
    {
        let mut replies = mock.replies.lock().unwrap();
        // First reply triggers a repair to confirm repair also keeps follow-up context without archive values.
        replies.push_back(sql_reply(
            "SELECT a.name AS artist, COUNT(DISTINCT l.id) AS plays FROM listens l JOIN track_artists ta ON ta.track_id = l.track_id JOIN artists a ON a.id = ta.artist_id WHERE l.counted = 1 AND l.bad_month = 2 GROUP BY a.id",
            "February plays per artist",
            "Counted plays per artist in February 2024.",
            "table",
            json!([{"name": "artist", "format": "artist"}, {"name": "plays", "format": "count"}]),
        ));
        replies.push_back(sql_reply(
            "SELECT a.name AS artist, COUNT(DISTINCT l.id) AS plays FROM listens l JOIN track_artists ta ON ta.track_id = l.track_id JOIN artists a ON a.id = ta.artist_id WHERE l.counted = 1 AND l.local_year = 2024 AND l.local_month = 2 GROUP BY a.id ORDER BY plays DESC, a.name ASC",
            "February 2024 plays per artist",
            "Counted plays per artist in February 2024.",
            "table",
            json!([{"name": "artist", "format": "artist"}, {"name": "plays", "format": "count"}]),
        ));
    }
    let url = serve(mock.clone()).await;
    let (_dir, state, token) = setup_sql_default(&url).await;
    seed(&state).await;

    let extra = play(
        99,
        "2024-03-01T10:00:00Z",
        "fixture-ar99",
        "PrivateFollowUpArtist",
        "PrivateFollowUpTrack",
    );
    let mut tx = state.storage.writer.begin().await.unwrap();
    scrobbl_rs::db::ingest(&mut tx, &[extra]).await.unwrap();
    tx.commit().await.unwrap();

    let (status, body) = request(
        &state,
        &token,
        "POST",
        "/api/v1/ask",
        Some(json!({
            "question": "and only in February 2024?",
            "previous": {
                "question": "How many times did I listen to each artist?",
                "chips": [],
                "sql": {
                    "title": "Plays per artist",
                    "computed": "Counted plays grouped by artist across all time.",
                    "sql": prev_sql,
                }
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ok", "{body}");
    assert_eq!(
        body["sql"]["rows"],
        json!([["Björk", 1], ["Radiohead", 1]]),
        "{body}"
    );

    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    for (idx, (_, req)) in seen.iter().enumerate() {
        let user = req["messages"][1]["content"].as_str().unwrap();
        assert!(
            user.contains("Previous question: How many times did I listen to each artist?"),
            "call {idx}: {user}"
        );
        assert!(
            user.contains("Previous title: Plays per artist"),
            "call {idx}: {user}"
        );
        assert!(
            user.contains("Previous computed: Counted plays grouped by artist across all time."),
            "call {idx}: {user}"
        );
        assert!(
            user.contains(&format!("Previous SQL: {prev_sql}")),
            "call {idx}: {user}"
        );
        assert!(
            user.contains("New question: and only in February 2024?"),
            "call {idx}: {user}"
        );
        let raw = req.to_string();
        for secret in [
            "PrivateFollowUpArtist",
            "PrivateFollowUpTrack",
            "Jóga",
            "Hunter",
            "Creep",
            "Radiohead",
            "sk-secret-key",
        ] {
            assert!(!raw.contains(secret), "call {idx} leaked `{secret}`: {raw}");
        }
    }
}

async fn serve_app(mut state: AppState) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    state.config.port = addr.port();
    let router = web::router(state);
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{addr}")
}

async fn wait_for_request(mock: &Mock, expected: usize) {
    for _ in 0..500 {
        if mock.request_count.load(Ordering::SeqCst) >= expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("timed out waiting for mock AI request #{expected}");
}

#[tokio::test]
async fn cancelling_during_long_sql_query_interrupts_sqlite_and_keeps_server_responsive() {
    let mock = Mock::default();
    let slow_sql = "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt) SELECT COUNT(*) FROM cnt";
    *mock.reply.lock().unwrap() = sql_reply(
        slow_sql,
        "Endless",
        "Endless loop.",
        "value",
        json!([{"name": "c", "format": "count"}]),
    );
    let url = serve(mock.clone()).await;
    let (_dir, mut state, token) = setup_sql_default(&url).await;
    state.ask.sql_limits.timeout = Duration::from_secs(10);
    state.ask.sql_limits.max_instructions = 10_000_000_000;
    seed(&state).await;

    let base = serve_app(state.clone()).await;
    let client = reqwest::Client::new();
    let started = std::time::Instant::now();

    let ask_task = tokio::spawn({
        let client = client.clone();
        let base = base.clone();
        let token = token.clone();
        async move {
            client
                .post(format!("{base}/api/v1/ask"))
                .header("cookie", format!("scrobbl_session={token}"))
                .header("x-csrf-token", "test-csrf")
                .header("origin", &base)
                .json(&json!({"question": "Count forever", "request_id": "sql-cancel-1"}))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    });

    wait_for_request(&mock, 1).await;
    tokio::time::sleep(Duration::from_millis(40)).await;

    let cancel_res = client
        .post(format!("{base}/api/v1/ask/cancel"))
        .header("cookie", format!("scrobbl_session={token}"))
        .header("x-csrf-token", "test-csrf")
        .header("origin", &base)
        .json(&json!({"request_id": "sql-cancel-1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(cancel_res.status(), StatusCode::OK);
    let cancel_body: Value = cancel_res.json().await.unwrap();
    assert_eq!(cancel_body["cancelled"], true, "{cancel_body}");

    let body = ask_task.await.unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "cancellation took too long: {:?}",
        started.elapsed()
    );
    assert_eq!(body["status"], "failed", "{body}");
    assert_eq!(body["sql"]["rule_code"], "cancelled", "{body}");
    assert!(
        body["message"].as_str().unwrap().contains("cancelled"),
        "{body}"
    );

    // Server remains responsive and a subsequent valid query succeeds immediately.
    *mock.reply.lock().unwrap() = sql_reply(
        "SELECT COUNT(*) AS plays FROM listens WHERE counted = 1",
        "Total plays",
        "Counted plays.",
        "value",
        json!([{"name": "plays", "format": "count"}]),
    );
    let next: Value = client
        .post(format!("{base}/api/v1/ask"))
        .header("cookie", format!("scrobbl_session={token}"))
        .header("x-csrf-token", "test-csrf")
        .header("origin", &base)
        .json(&json!({"question": "How many plays?"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(next["status"], "ok", "{next}");
    assert_eq!(next["sql"]["rows"], json!([[5]]), "{next}");
}

#[tokio::test]
async fn cancelling_during_remote_ai_inference_aborts_and_returns_honestly() {
    let mock = Mock::default();
    *mock.delay.lock().unwrap() = Duration::from_secs(10);
    *mock.reply.lock().unwrap() = sql_reply(
        "SELECT COUNT(*) AS plays FROM listens",
        "Plays",
        "Plays.",
        "value",
        json!([{"name": "plays", "format": "count"}]),
    );
    let url = serve(mock.clone()).await;
    let (_dir, state, token) = setup_sql_default(&url).await;
    seed(&state).await;

    let base = serve_app(state).await;
    let client = reqwest::Client::new();
    let started = std::time::Instant::now();

    let ask_task = tokio::spawn({
        let client = client.clone();
        let base = base.clone();
        let token = token.clone();
        async move {
            client
                .post(format!("{base}/api/v1/ask"))
                .header("cookie", format!("scrobbl_session={token}"))
                .header("x-csrf-token", "test-csrf")
                .header("origin", &base)
                .json(&json!({"question": "Slow LLM question", "request_id": "llm-cancel-1"}))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    });

    wait_for_request(&mock, 1).await;
    let cancel_body: Value = client
        .post(format!("{base}/api/v1/ask/cancel"))
        .header("cookie", format!("scrobbl_session={token}"))
        .header("x-csrf-token", "test-csrf")
        .header("origin", &base)
        .json(&json!({"request_id": "llm-cancel-1"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cancel_body["cancelled"], true, "{cancel_body}");

    let body = ask_task.await.unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "remote inference cancellation took too long: {:?}",
        started.elapsed()
    );
    assert_eq!(body["status"], "failed", "{body}");
    assert!(
        body["message"].as_str().unwrap().contains("cancelled"),
        "{body}"
    );
}

#[tokio::test]
async fn dropping_http_request_interrupts_in_flight_sql_query() {
    let mock = Mock::default();
    let slow_sql = "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt) SELECT COUNT(*) FROM cnt";
    *mock.reply.lock().unwrap() = sql_reply(
        slow_sql,
        "Endless",
        "Endless loop.",
        "value",
        json!([{"name": "c", "format": "count"}]),
    );
    let url = serve(mock.clone()).await;
    let (_dir, mut state, token) = setup_sql_default(&url).await;
    state.ask.sql_limits.timeout = Duration::from_secs(10);
    state.ask.sql_limits.max_instructions = 10_000_000_000;
    seed(&state).await;

    let base = serve_app(state.clone()).await;
    let client = reqwest::Client::new();

    let ask_task = tokio::spawn({
        let client = client.clone();
        let base = base.clone();
        let token = token.clone();
        async move {
            let _ = client
                .post(format!("{base}/api/v1/ask"))
                .header("cookie", format!("scrobbl_session={token}"))
                .header("x-csrf-token", "test-csrf")
                .header("origin", &base)
                .json(&json!({"question": "Count forever"}))
                .send()
                .await;
        }
    });

    wait_for_request(&mock, 1).await;
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(state.ask.active_count(), 1);
    ask_task.abort();
    let _ = ask_task.await;

    for _ in 0..100 {
        if state.ask.active_count() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        state.ask.active_count(),
        0,
        "aborting HTTP request must cancel in-flight SQL"
    );

    // Server stays responsive right away.
    *mock.reply.lock().unwrap() = sql_reply(
        "SELECT COUNT(*) AS plays FROM listens WHERE counted = 1",
        "Total plays",
        "Counted plays.",
        "value",
        json!([{"name": "plays", "format": "count"}]),
    );
    let next: Value = client
        .post(format!("{base}/api/v1/ask"))
        .header("cookie", format!("scrobbl_session={token}"))
        .header("x-csrf-token", "test-csrf")
        .header("origin", &base)
        .json(&json!({"question": "How many plays?"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(next["status"], "ok", "{next}");
}

#[tokio::test]
async fn ask_cancel_requires_owner_session_and_csrf_and_rejects_api_keys() {
    let mock = Mock::default();
    let url = serve(mock).await;
    let (_dir, state, token) = setup_sql_default(&url).await;
    let router = web::router(state);

    // Missing session -> 401
    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/ask/cancel")
                .header("host", "127.0.0.1:3210")
                .header("origin", "http://127.0.0.1:3210")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Missing CSRF -> 403
    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/ask/cancel")
                .header("host", "127.0.0.1:3210")
                .header("origin", "http://127.0.0.1:3210")
                .header("cookie", format!("scrobbl_session={token}"))
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // Read-only API key -> 403
    let res = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/ask/cancel")
                .header("host", "127.0.0.1:3210")
                .header("origin", "http://127.0.0.1:3210")
                .header("authorization", "Bearer deadbeef")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

// ---- Per-request answer mode (#208) ---------------------------------------

#[tokio::test]
async fn ask_accepts_a_per_request_mode_without_persisting_it() {
    let mock = Mock::default();
    let sql_answer = json!({
        "sql": "SELECT COUNT(*) AS plays FROM listens WHERE counted = 1",
        "title": "Total plays",
        "computed": "Total counted plays across all time.",
        "answer_kind": "value",
        "columns": [{"name": "plays", "format": "count"}],
    })
    .to_string();
    let url = serve(mock.clone()).await;
    let (_dir, state, token) = setup(&url).await;
    seed(&state).await;

    // Stored mode is guided, but one request may ask for SQL.
    {
        let mut replies = mock.replies.lock().unwrap();
        replies.push_back(sql_answer.clone());
        replies.push_back(PLAYS_OF_BJORK.to_string());
        replies.push_back(PLAYS_OF_BJORK.to_string());
    }
    let (status, sql_body) = request(
        &state,
        &token,
        "POST",
        "/api/v1/ask",
        Some(json!({"question": "How many plays do I have?", "mode": "sql"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sql_body}");
    assert_eq!(sql_body["status"], "ok", "{sql_body}");
    assert_eq!(sql_body["sql"]["rows"], json!([[5]]), "{sql_body}");
    assert!(sql_body["result"].is_null(), "{sql_body}");

    // The stored mode did not change: guided is still the default here.
    let (_, ai) = request(&state, &token, "GET", "/api/v1/ai/settings", None).await;
    assert_eq!(ai["answer_mode"], "guided", "{ai}");
    let (_, ask_status) = request(&state, &token, "GET", "/api/v1/ask/status", None).await;
    assert_eq!(ask_status["answer_mode"], "guided", "{ask_status}");

    // An explicit guided override answers exactly like the stored guided mode.
    let question = "How many times did I listen to Björk?";
    let (status, stored_guided) = request(
        &state,
        &token,
        "POST",
        "/api/v1/ask",
        Some(json!({"question": question})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stored_guided}");
    let (status, explicit_guided) = request(
        &state,
        &token,
        "POST",
        "/api/v1/ask",
        Some(json!({"question": question, "mode": "guided"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{explicit_guided}");
    assert_eq!(explicit_guided, stored_guided);
    assert_eq!(stored_guided["result"]["scalar"], 4, "{stored_guided}");

    // An unknown mode is a bad request naming both allowed values.
    let (status, body) = request(
        &state,
        &token,
        "POST",
        "/api/v1/ask",
        Some(json!({"question": question, "mode": "free"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let message = body["error"].as_str().unwrap_or_default();
    assert!(message.contains("free"), "{message}");
    assert!(
        message.contains("guided") && message.contains("sql"),
        "{message}"
    );
    assert_eq!(
        mock.seen.lock().unwrap().len(),
        3,
        "a rejected mode never reaches the model"
    );
}

// ---- Local engine SQL repair rounds (#208) --------------------------------

type SeenPrompts = Arc<Mutex<Vec<(String, runtime::GrammarKind)>>>;

/// Replays queued SQL answers and records every prompt and grammar.
struct ScriptedSqlEngine {
    seen: SeenPrompts,
    replies: Arc<Mutex<VecDeque<String>>>,
}

impl runtime::Engine for ScriptedSqlEngine {
    fn load(&mut self, _spec: &runtime::ModelSpec) -> Result<(), String> {
        Ok(())
    }

    fn unload(&mut self) {}

    fn generate(
        &mut self,
        prompt: &str,
        _cancel: &runtime::CancelToken,
    ) -> Result<String, runtime::InferenceError> {
        self.seen
            .lock()
            .unwrap()
            .push((prompt.to_string(), runtime::GrammarKind::Guided));
        Ok(String::new())
    }

    fn generate_with_grammar(
        &mut self,
        prompt: &str,
        grammar: runtime::GrammarKind,
        cancel: &runtime::CancelToken,
    ) -> Result<String, runtime::InferenceError> {
        if cancel.should_stop() {
            return Err(runtime::InferenceError::Cancelled);
        }
        self.seen
            .lock()
            .unwrap()
            .push((prompt.to_string(), grammar));
        Ok(self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| "not json".to_string()))
    }
}

fn local_metadata(id: &str, file: &str) -> Value {
    json!({
        "catalog_id": id,
        "file": file,
        "sha256": "0".repeat(64),
        "size_bytes": 16,
        "installed_at_ms": 1_760_000_000_000_i64,
        "license": "Apache-2.0",
        "checksum_verified": true,
    })
}

/// A local SQL-mode Ask backed by a scripted engine instead of llama.cpp.
async fn setup_scripted_local_sql(
    replies: Vec<String>,
) -> (tempfile::TempDir, AppState, String, SeenPrompts) {
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

    let id = "qwen3-1.7b-q4_k_m";
    let catalog = scrobbl_rs::ai::catalog::find(id).unwrap();
    let model_dir = dir.path().join("models").join(id);
    std::fs::create_dir_all(&model_dir).unwrap();
    std::fs::write(model_dir.join(&catalog.download.file), b"dummy gguf bytes").unwrap();
    std::fs::write(
        model_dir.join("metadata.json"),
        serde_json::to_vec(&local_metadata(id, &catalog.download.file)).unwrap(),
    )
    .unwrap();
    for (key, value) in [
        ("ai_enabled", "1"),
        ("ai_provider", "local"),
        ("ai_answer_mode_local", "sql"),
        ("ai_active_model", id),
        ("ai_threads", "4"),
        ("ai_unload_minutes", "15"),
    ] {
        sqlx::query("INSERT OR REPLACE INTO settings(key,value) VALUES(?,?)")
            .bind(key)
            .bind(value)
            .execute(&state.db)
            .await
            .unwrap();
    }

    let seen: SeenPrompts = Arc::new(Mutex::new(Vec::new()));
    let engine = ScriptedSqlEngine {
        seen: Arc::clone(&seen),
        replies: Arc::new(Mutex::new(replies.into())),
    };
    let parser = ModelParser::new(
        state.db.clone(),
        dir.path().to_path_buf(),
        Runtime::spawn(Box::new(engine), 4),
        Duration::from_secs(5),
    );
    state.ask = Ask::with_parser(Arc::new(parser), true, true, true);
    state.ask.sql_limits.timeout = Duration::from_secs(10);
    (dir, state, token, seen)
}

fn sql_reply_with(computed: &str, sql: &str) -> String {
    sql_reply(
        sql,
        "Plays",
        computed,
        "value",
        json!([{"name": "plays", "format": "count"}]),
    )
}

#[tokio::test]
async fn local_sql_mode_repairs_a_rejected_query() {
    let (_dir, state, token, seen) = setup_scripted_local_sql(vec![
        sql_reply_with("Attempted a bad column.", "SELECT missing_col FROM listens"),
        sql_reply_with(
            "Counted plays across all time.",
            "SELECT COUNT(*) AS plays FROM listens WHERE counted = 1",
        ),
    ])
    .await;
    seed(&state).await;

    let (status, body) = request(
        &state,
        &token,
        "POST",
        "/api/v1/ask",
        Some(json!({"question": "How many plays do I have?"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ok", "{body}");
    assert_eq!(body["sql"]["rows"], json!([[5]]), "{body}");

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "expected initial + one repair round");
    assert!(
        seen.iter()
            .all(|(_, grammar)| *grammar == runtime::GrammarKind::Sql)
    );
    let repair_prompt = &seen[1].0;
    assert!(
        repair_prompt.contains("SELECT missing_col FROM listens"),
        "repair prompt must include the failed SQL: {repair_prompt}"
    );
    assert!(
        repair_prompt.contains("How many plays do I have?"),
        "repair prompt must include the question: {repair_prompt}"
    );
    assert!(
        repair_prompt.contains("no_such_column") && repair_prompt.contains("missing_col"),
        "repair prompt must include the sanitized error: {repair_prompt}"
    );
    for secret in ["Jóga", "Hunter", "Creep"] {
        assert!(
            !repair_prompt.contains(secret),
            "repair prompt leaked `{secret}`: {repair_prompt}"
        );
    }
}

#[tokio::test]
async fn local_sql_mode_stops_after_two_repair_rounds() {
    let (_dir, state, token, seen) = setup_scripted_local_sql(vec![
        sql_reply_with("Read settings.", "SELECT value FROM settings"),
        sql_reply_with("Delete listens.", "DELETE FROM listens"),
        sql_reply_with("Read bad_col.", "SELECT bad_col FROM listens"),
    ])
    .await;
    seed(&state).await;

    let (status, body) = request(
        &state,
        &token,
        "POST",
        "/api/v1/ask",
        Some(json!({"question": "How many plays do I have?"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "failed", "{body}");
    assert_eq!(body["sql"]["rule_code"], "sql_error", "{body}");
    let message = body["message"].as_str().unwrap();
    assert!(message.contains("2 repair attempts"), "{message}");
    assert!(
        message.contains("Guided questions") && message.contains("larger model"),
        "local exhaustion keeps the guided hint: {message}"
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        3,
        "expected 1 initial + 2 repair calls"
    );
}
