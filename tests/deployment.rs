use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use scrobbl_rs::{
    AppState,
    config::{Config, parse_origin},
    db, web,
};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn deployment(origin: &str) -> (tempfile::TempDir, AppState, Router) {
    let dir = tempfile::tempdir().unwrap();
    let store = db::open(dir.path()).await.unwrap();
    let config = Config {
        port: 3210,
        bind_ip: "0.0.0.0".parse().unwrap(),
        external_origin: Some(parse_origin(origin).unwrap()),
        data_dir: dir.path().into(),
        sync_seconds: 120,
        now_playing_seconds: 30,
        import_limits: Default::default(),
        api_snapshot_limits: Default::default(),
        env_client_id: Some("test-client".into()),
        env_client_secret: Some("test-secret".into()),
    };
    let state = AppState::new(store, config);
    let app = web::router(state.clone());
    (dir, state, app)
}
async fn call(
    app: &Router,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Value,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    for (key, value) in headers {
        request = request.header(*key, *value);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        headers,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
#[tokio::test]
async fn https_origin_survives_proxy_and_oauth_requires_the_same_secure_session() {
    let (_dir, state, app) = deployment("https://music.example.org:8443").await;
    let host = state.config.host();
    let origin = state.config.origin();
    let base = [("host", host.as_str()), ("origin", origin.as_str())];
    let (status, headers, setup) = call(
        &app,
        "POST",
        "/api/setup",
        &base,
        json!({"username":"owner","password":"test deployment password 7!"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = headers["set-cookie"].to_str().unwrap();
    for attribute in ["HttpOnly", "SameSite=Lax", "Path=/", "; Secure"] {
        assert!(cookie.contains(attribute));
    }
    let cookie = cookie.split(';').next().unwrap();
    let csrf = setup["csrf"].as_str().unwrap();
    let authenticated = [
        ("host", host.as_str()),
        ("origin", origin.as_str()),
        ("cookie", cookie),
        ("x-csrf-token", csrf),
    ];
    let (status, _, _) = call(&app, "POST", "/api/logout", &authenticated[..3], json!({})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, connect) = call(
        &app,
        "POST",
        "/api/spotify/connect",
        &authenticated,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let url = url::Url::parse(connect["url"].as_str().unwrap()).unwrap();
    let pairs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(
        pairs["redirect_uri"],
        "https://music.example.org:8443/auth/spotify/callback"
    );
    let callback = format!(
        "/auth/spotify/callback?state={}&error=access_denied",
        pairs["state"]
    );
    let (_, other_headers, _) = call(
        &app,
        "POST",
        "/api/login",
        &base,
        json!({"username":"owner","password":"test deployment password 7!"}),
    )
    .await;
    let other_cookie = other_headers["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    assert_eq!(
        call(
            &app,
            "GET",
            &callback,
            &[("host", &host), ("cookie", other_cookie)],
            json!({})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, headers, _) = call(&app, "GET", &callback, &authenticated, json!({})).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(
        headers["location"],
        format!(
            "/spotify/connect-result/{}",
            connect["attempt"].as_str().unwrap()
        )
    );
    let attempt_url = format!(
        "/api/spotify/connect/{}",
        connect["attempt"].as_str().unwrap()
    );
    let (status, _, result) = call(&app, "GET", &attempt_url, &authenticated, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["outcome"], "error");
    assert!(result["message"].as_str().unwrap().contains("declined"));
    assert_eq!(
        call(
            &app,
            "GET",
            &attempt_url,
            &[("host", &host), ("cookie", other_cookie)],
            json!({})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&app, "GET", &callback, &authenticated, json!({}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    // Changing a forwarded protocol or hostname cannot change the configured callback or the cookie policy.
    let spoof = [
        ("host", host.as_str()),
        ("origin", "http://music.example.org:8443"),
        ("x-forwarded-proto", "https"),
        ("forwarded", "host=music.example.org:8443;proto=https"),
    ];
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/login",
            &spoof,
            json!({"username":"owner","password":"test deployment password 7!"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            "GET",
            "/healthz",
            &[("host", "evil.example"), ("x-forwarded-host", &host)],
            json!({})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            "GET",
            "/healthz",
            &[
                ("host", &host),
                ("x-forwarded-host", "evil.example"),
                ("x-forwarded-proto", "http")
            ],
            json!({})
        )
        .await
        .0,
        StatusCode::OK
    );
    // Expired state is also rejected even with the correct session.
    let (_, _, connect) = call(
        &app,
        "POST",
        "/api/spotify/connect",
        &authenticated,
        json!({}),
    )
    .await;
    let url = url::Url::parse(connect["url"].as_str().unwrap()).unwrap();
    let secret = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    sqlx::query("UPDATE oauth_states SET expires_at_ms=0")
        .execute(&state.private_db)
        .await
        .unwrap();
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/auth/spotify/callback?state={secret}&error=access_denied"),
            &authenticated,
            json!({})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, headers, _) = call(&app, "POST", "/api/logout", &authenticated, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers["set-cookie"]
            .to_str()
            .unwrap()
            .contains("Max-Age=0; Secure")
    );
    state.storage.close().await;
}
#[tokio::test]
async fn private_http_browsing_works_but_cannot_start_an_invalid_spotify_redirect() {
    let (_dir, state, app) = deployment("http://192.0.2.50:4321").await;
    let base = [
        ("host", "192.0.2.50:4321"),
        ("origin", "http://192.0.2.50:4321"),
    ];
    let (status, headers, body) = call(
        &app,
        "POST",
        "/api/setup",
        &base,
        json!({"username":"owner","password":"test deployment password 7!"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = headers["set-cookie"].to_str().unwrap();
    assert!(!cookie.contains("Secure"));
    let headers = [
        base[0],
        base[1],
        ("cookie", cookie.split(';').next().unwrap()),
        ("x-csrf-token", body["csrf"].as_str().unwrap()),
        ("x-forwarded-proto", "https"),
    ];
    assert_eq!(
        call(&app, "GET", "/api/v1/preferences", &headers, json!({}))
            .await
            .0,
        StatusCode::OK
    );
    let (status, _, body) = call(&app, "POST", "/api/spotify/connect", &headers, json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("HTTPS"));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM oauth_states")
            .fetch_one(&state.private_db)
            .await
            .unwrap(),
        0
    );
    state.storage.close().await;
}
#[test]
fn external_origin_is_strict_and_keeps_nonstandard_ports_and_ipv6() {
    for value in [
        "ftp://example.org",
        "https://example.org/path",
        "https://user:secret@example.org",
        "https://example.org?x=1",
        "https://example.org#x",
        "https://0.0.0.0",
        "http://[::]",
        "example.org",
    ] {
        assert!(parse_origin(value).is_err(), "{value}");
    }
    for (input, expected) in [
        (
            "https://music.example.org:443/",
            "https://music.example.org",
        ),
        (
            "https://music.example.org:8443",
            "https://music.example.org:8443",
        ),
        ("http://[::1]:3210", "http://[::1]:3210"),
    ] {
        assert_eq!(
            parse_origin(input).unwrap().origin().ascii_serialization(),
            expected
        );
    }
}
