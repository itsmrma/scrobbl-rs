use axum::{
    Router,
    body::Body,
    extract::State,
    http::{HeaderValue, Request, StatusCode, header},
    response::{IntoResponse, Json, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use tower::ServiceExt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BootStatus {
    pub ready: bool,
    pub stage: String,
    pub error: Option<String>,
}

impl Default for BootStatus {
    fn default() -> Self {
        Self {
            ready: false,
            stage: "starting".to_string(),
            error: None,
        }
    }
}

#[derive(Clone)]
pub struct BootManager {
    status: Arc<RwLock<BootStatus>>,
    ready_router: Arc<RwLock<Option<Router>>>,
}

impl BootManager {
    pub fn new() -> Self {
        Self {
            status: Arc::new(RwLock::new(BootStatus::default())),
            ready_router: Arc::new(RwLock::new(None)),
        }
    }

    pub fn set_stage(&self, stage: impl Into<String>) {
        if let Ok(mut status) = self.status.write() {
            status.stage = stage.into();
        }
    }

    pub fn set_ready(&self, router: Router) {
        if let Ok(mut status) = self.status.write() {
            status.ready = true;
            status.stage = "ready".to_string();
            status.error = None;
        }
        if let Ok(mut ready) = self.ready_router.write() {
            *ready = Some(router);
        }
    }

    pub fn set_error(&self, error: impl Into<String>) {
        let err_str = error.into();
        if let Ok(mut status) = self.status.write() {
            status.ready = false;
            status.stage = "error".to_string();
            status.error = Some(err_str);
        }
    }

    pub fn status(&self) -> BootStatus {
        self.status
            .read()
            .map(|s| s.clone())
            .unwrap_or_else(|_| BootStatus {
                ready: false,
                stage: "error".to_string(),
                error: Some("Lock poisoned".to_string()),
            })
    }

    pub fn is_ready(&self) -> bool {
        self.status.read().map(|s| s.ready).unwrap_or(false)
    }

    pub fn ready_router(&self) -> Option<Router> {
        self.ready_router.read().ok().and_then(|r| r.clone())
    }
}

impl Default for BootManager {
    fn default() -> Self {
        Self::new()
    }
}

pub fn router(boot: BootManager) -> Router {
    Router::new()
        .route("/api/boot-status", get(boot_status_handler))
        .route("/favicon.svg", get(favicon_handler))
        .fallback(dispatch_handler)
        .with_state(boot)
}

async fn boot_status_handler(State(boot): State<BootManager>) -> Json<BootStatus> {
    Json(boot.status())
}

async fn favicon_handler(State(boot): State<BootManager>, req: Request<Body>) -> Response {
    if let Some(router) = boot.ready_router() {
        router
            .oneshot(req)
            .await
            .unwrap_or_else(|infallible| match infallible {})
    } else {
        (
            StatusCode::OK,
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static("image/svg+xml"),
            )],
            include_str!("../static/favicon.svg"),
        )
            .into_response()
    }
}

async fn dispatch_handler(State(boot): State<BootManager>, req: Request<Body>) -> Response {
    if let Some(router) = boot.ready_router() {
        router
            .oneshot(req)
            .await
            .unwrap_or_else(|infallible| match infallible {})
    } else {
        let path = req.uri().path();
        if path == "/healthz" || path.starts_with("/api/") {
            let status = boot.status();
            let mut response = (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": "Server is starting up",
                    "ready": false,
                    "stage": status.stage,
                    "details": status.error,
                })),
            )
                .into_response();
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
            response
        } else {
            let status = boot.status();
            let html = standalone_splash_html(&status);
            (
                StatusCode::OK,
                [(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/html; charset=utf-8"),
                )],
                html,
            )
                .into_response()
        }
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn standalone_splash_html(status: &BootStatus) -> String {
    let error_display = if status.error.is_some() {
        "block"
    } else {
        "none"
    };
    let loader_display = if status.error.is_some() {
        "none"
    } else {
        "flex"
    };
    let initial_status = if status.error.is_some() {
        "A critical error occurred while starting the archive."
    } else if status.stage == "migrating" || status.stage == "opening_database" {
        "Opening and verifying SQLite WAL archive..."
    } else if status.stage == "starting_workers" {
        "Starting background collection and artwork workers..."
    } else {
        "Starting scrobbl.rs..."
    };
    let escaped_error = html_escape(status.error.as_deref().unwrap_or(""));

    format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <meta name="color-scheme" content="dark light" />
    <link rel="icon" href="/favicon.svg" type="image/svg+xml" />
    <title>scrobbl.rs — Starting</title>
    <style>
      :root {{
        color-scheme: dark light;
        --splash-bg: #0d0f13;
        --splash-surface: #151921;
        --splash-surface-raised: #1e2430;
        --splash-border: rgba(255, 255, 255, 0.1);
        --splash-text: #f3f5f8;
        --splash-muted: #9aa3b2;
        --splash-accent: #c9ee8b;
        --splash-error: #ff8e80;
        --splash-error-bg: rgba(255, 142, 128, 0.1);
      }}
      @media (prefers-color-scheme: light) {{
        :root {{
          --splash-bg: #f4f6f9;
          --splash-surface: #ffffff;
          --splash-surface-raised: #eef1f6;
          --splash-border: rgba(20, 28, 40, 0.14);
          --splash-text: #141b24;
          --splash-muted: #4b5668;
          --splash-error: #b3261e;
          --splash-error-bg: rgba(179, 38, 30, 0.08);
        }}
      }}
      * {{ box-sizing: border-box; }}
      html, body {{
        margin: 0;
        padding: 0;
        width: 100%;
        min-height: 100vh;
        background:
          radial-gradient(circle at 50% 0%, rgba(201, 238, 139, 0.09), transparent 55%),
          var(--splash-bg);
        color: var(--splash-text);
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      }}
      .splash-container {{
        display: grid;
        place-items: center;
        min-height: 100vh;
        padding: 24px;
      }}
      .splash-card {{
        width: min(560px, 100%);
        background: var(--splash-surface);
        border: 1px solid var(--splash-border);
        border-radius: 16px;
        padding: 36px 32px;
        box-shadow: 0 24px 64px rgba(0, 0, 0, 0.35);
      }}
      .splash-header {{
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 16px;
        margin-bottom: 24px;
      }}
      .splash-brand {{
        font-size: 30px;
        font-weight: 800;
        letter-spacing: -0.04em;
        line-height: 1;
      }}
      .splash-brand .accent {{ color: var(--splash-accent); }}
      .splash-pill {{
        display: inline-flex;
        align-items: center;
        gap: 8px;
        padding: 6px 12px;
        border-radius: 999px;
        font-size: 12px;
        font-weight: 600;
        letter-spacing: 0.03em;
        text-transform: uppercase;
        background: var(--splash-surface-raised);
        border: 1px solid var(--splash-border);
        color: var(--splash-muted);
      }}
      .splash-loader {{
        display: {loader_display};
        align-items: center;
        gap: 10px;
      }}
      .splash-dot {{
        width: 9px;
        height: 9px;
        border-radius: 50%;
        background: var(--splash-accent);
        box-shadow: 0 0 12px var(--splash-accent);
        animation: splash-pulse 1.3s ease-in-out infinite;
      }}
      @keyframes splash-pulse {{
        0%, 100% {{ transform: scale(0.8); opacity: 0.45; }}
        50% {{ transform: scale(1.2); opacity: 1; }}
      }}
      @media (prefers-reduced-motion: reduce) {{
        .splash-dot {{ animation: none; opacity: 1; }}
      }}
      .splash-status {{
        font-size: 15px;
        font-weight: 500;
        color: var(--splash-text);
        margin: 0 0 20px;
        line-height: 1.5;
      }}
      .splash-stages {{
        display: grid;
        grid-template-columns: repeat(3, minmax(0, 1fr));
        gap: 10px;
      }}
      .splash-stage {{
        padding: 12px;
        border-radius: 10px;
        background: var(--splash-surface-raised);
        border: 1px solid var(--splash-border);
      }}
      .splash-stage small {{
        display: block;
        font-size: 11px;
        font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
        color: var(--splash-muted);
        margin-bottom: 4px;
      }}
      .splash-stage strong {{
        display: block;
        font-size: 13px;
        font-weight: 600;
      }}
      @media (max-width: 560px) {{
        .splash-card {{ padding: 24px 20px; }}
        .splash-stages {{ grid-template-columns: 1fr; }}
      }}
      .splash-error-panel {{
        display: {error_display};
        margin-top: 20px;
        padding: 16px 18px;
        border-radius: 12px;
        background: var(--splash-error-bg);
        border: 1px solid var(--splash-error);
      }}
      .splash-error-title {{
        font-size: 14px;
        font-weight: 700;
        color: var(--splash-error);
        margin: 0 0 8px;
      }}
      .splash-error-message {{
        font-size: 12.5px;
        color: var(--splash-text);
        margin: 0;
        line-height: 1.5;
        white-space: pre-wrap;
        word-break: break-word;
        font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
        background: var(--splash-surface-raised);
        padding: 10px 12px;
        border-radius: 8px;
        border: 1px solid var(--splash-border);
      }}
      .splash-retry-btn {{
        margin-top: 14px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        padding: 10px 16px;
        min-height: 44px;
        font-size: 13px;
        font-weight: 600;
        border-radius: 8px;
        background: var(--splash-surface-raised);
        color: var(--splash-text);
        border: 1px solid var(--splash-border);
        cursor: pointer;
      }}
    </style>
  </head>
  <body>
    <div class="splash-container">
      <div class="splash-card">
        <div class="splash-header">
          <div class="splash-brand">scrobbl<span class="accent">.</span>rs</div>
          <div class="splash-pill">
            <span class="splash-loader" id="splash-loader"><span class="splash-dot"></span></span>
            <span id="splash-badge">Booting</span>
          </div>
        </div>
        <p class="splash-status" id="splash-status">{initial_status}</p>
        <div class="splash-stages">
          <div class="splash-stage"><small>01 · LISTENER</small><strong>HTTP Bound</strong></div>
          <div class="splash-stage"><small>02 · ARCHIVE</small><strong id="stage-db">SQLite WAL</strong></div>
          <div class="splash-stage"><small>03 · WORKERS</small><strong id="stage-workers">Sync &amp; Art</strong></div>
        </div>
        <div class="splash-error-panel" id="splash-error-panel">
          <div class="splash-error-title">Startup error</div>
          <pre class="splash-error-message" id="splash-error-message">{escaped_error}</pre>
          <button type="button" class="splash-retry-btn" onclick="location.reload()">Reload page</button>
        </div>
      </div>
    </div>
    <script>
      (function() {{
        var timer = setInterval(function() {{
          fetch('/api/boot-status', {{ cache: 'no-store' }})
            .then(function(r) {{ return r.json(); }})
            .then(function(s) {{
              if (s.ready) {{
                clearInterval(timer);
                window.location.reload();
              }} else if (s.error) {{
                clearInterval(timer);
                var loader = document.getElementById('splash-loader');
                var badge = document.getElementById('splash-badge');
                var statusEl = document.getElementById('splash-status');
                var errPanel = document.getElementById('splash-error-panel');
                var errMsg = document.getElementById('splash-error-message');
                if (loader) loader.style.display = 'none';
                if (badge) badge.textContent = 'Error';
                if (statusEl) statusEl.textContent = 'A critical error occurred while starting the archive.';
                if (errMsg) errMsg.textContent = s.error;
                if (errPanel) errPanel.style.display = 'block';
              }} else if (s.stage) {{
                var statusEl = document.getElementById('splash-status');
                if (statusEl) {{
                  if (s.stage === 'migrating' || s.stage === 'opening_database') {{
                    statusEl.textContent = 'Opening and verifying SQLite WAL archive...';
                  }} else if (s.stage === 'starting_workers') {{
                    statusEl.textContent = 'Starting background collection and artwork workers...';
                  }} else {{
                    statusEl.textContent = 'Starting scrobbl.rs...';
                  }}
                }}
              }}
            }})
            .catch(function() {{}});
        }}, 500);
      }})();
    </script>
  </body>
</html>
"#
    )
}
