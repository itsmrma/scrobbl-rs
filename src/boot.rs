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
        "Si è verificato un errore critico durante l'avvio."
    } else if status.stage == "migrating" || status.stage == "opening_database" {
        "Aggiornamento del database in corso..."
    } else if status.stage == "starting_workers" {
        "Avvio dei servizi in corso..."
    } else {
        "Avvio di scrobbl.rs in corso..."
    };
    let escaped_error = html_escape(status.error.as_deref().unwrap_or(""));

    format!(
        r#"<!doctype html>
<html lang="it">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <meta name="color-scheme" content="dark light" />
    <link rel="icon" href="/favicon.svg" type="image/svg+xml" />
    <title>scrobbl.rs - Avvio</title>
    <style>
      :root {{
        color-scheme: dark light;
        --splash-bg: #13151b;
        --splash-text: #f2f3f7;
        --splash-muted: #bac0cc;
        --splash-accent: #c9ee8b;
        --splash-surface: #1c1f27;
        --splash-surface-raised: #292d38;
        --splash-border: #454a58;
        --splash-error: #ffaaa0;
        --splash-error-bg: rgba(255, 170, 160, 0.1);
      }}
      @media (prefers-color-scheme: light) {{
        :root {{
          --splash-bg: #f5f6fa;
          --splash-text: #18212d;
          --splash-muted: #425065;
          --splash-surface: #ffffff;
          --splash-surface-raised: #e9ecf3;
          --splash-border: #9da7b6;
          --splash-error: #a22620;
          --splash-error-bg: rgba(162, 38, 32, 0.08);
        }}
      }}
      * {{
        box-sizing: border-box;
      }}
      body, html {{
        margin: 0;
        padding: 0;
        width: 100%;
        height: 100%;
        background-color: var(--splash-bg);
        color: var(--splash-text);
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      }}
      .splash-container {{
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        min-height: 100vh;
        padding: 24px;
        text-align: center;
      }}
      .splash-brand {{
        font-size: 32px;
        font-weight: 700;
        letter-spacing: -0.04em;
        color: var(--splash-text);
        margin-bottom: 24px;
        user-select: none;
      }}
      .splash-brand .accent {{
        color: var(--splash-accent);
      }}
      .splash-loader {{
        display: {loader_display};
        align-items: center;
        justify-content: center;
        margin-bottom: 16px;
        height: 24px;
      }}
      .splash-dot {{
        width: 10px;
        height: 10px;
        border-radius: 50%;
        background-color: var(--splash-accent);
        box-shadow: 0 0 12px var(--splash-accent);
        animation: splash-pulse 1.4s ease-in-out infinite;
      }}
      @keyframes splash-pulse {{
        0%, 100% {{
          transform: scale(0.75);
          opacity: 0.4;
        }}
        50% {{
          transform: scale(1.25);
          opacity: 1;
        }}
      }}
      @media (prefers-reduced-motion: reduce) {{
        .splash-dot {{
          animation: none;
          opacity: 0.9;
        }}
      }}
      .splash-status {{
        font-size: 14px;
        color: var(--splash-muted);
        margin: 0;
        max-width: 360px;
        line-height: 1.5;
      }}
      .splash-error-panel {{
        display: {error_display};
        margin-top: 24px;
        max-width: 480px;
        width: 100%;
        padding: 16px 20px;
        border-radius: 12px;
        background: var(--splash-error-bg);
        border: 1px solid var(--splash-error);
        text-align: left;
      }}
      .splash-error-title {{
        font-size: 15px;
        font-weight: 600;
        color: var(--splash-error);
        margin: 0 0 8px;
      }}
      .splash-error-message {{
        font-size: 13px;
        color: var(--splash-text);
        margin: 0;
        line-height: 1.5;
        word-break: break-word;
        font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace;
        background: var(--splash-surface-raised);
        padding: 10px 12px;
        border-radius: 6px;
        border: 1px solid var(--splash-border);
      }}
      .splash-retry-btn {{
        margin-top: 14px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        padding: 8px 16px;
        min-height: 38px;
        font-size: 13px;
        font-weight: 600;
        border-radius: 8px;
        background: var(--splash-surface-raised);
        color: var(--splash-text);
        border: 1px solid var(--splash-border);
        cursor: pointer;
      }}
      .splash-retry-btn:hover {{
        filter: brightness(1.1);
      }}
    </style>
  </head>
  <body>
    <div class="splash-container">
      <div class="splash-brand">scrobbl<span class="accent">.</span>rs</div>
      <div class="splash-loader" id="splash-loader">
        <div class="splash-dot"></div>
      </div>
      <p class="splash-status" id="splash-status">{initial_status}</p>
      <div class="splash-error-panel" id="splash-error-panel">
        <div class="splash-error-title">Errore durante l'avvio</div>
        <pre class="splash-error-message" id="splash-error-message">{escaped_error}</pre>
        <button type="button" class="splash-retry-btn" onclick="location.reload()">Ricarica pagina</button>
      </div>
    </div>
    <script>
      (function() {{
        var timer = setInterval(function() {{
          fetch('/api/boot-status')
            .then(function(r) {{ return r.json(); }})
            .then(function(s) {{
              if (s.ready) {{
                clearInterval(timer);
                window.location.reload();
              }} else if (s.error) {{
                clearInterval(timer);
                var loader = document.getElementById('splash-loader');
                var statusEl = document.getElementById('splash-status');
                var errPanel = document.getElementById('splash-error-panel');
                var errMsg = document.getElementById('splash-error-message');
                if (loader) loader.style.display = 'none';
                if (statusEl) statusEl.textContent = 'Si è verificato un errore critico durante l\'avvio.';
                if (errMsg) errMsg.textContent = s.error;
                if (errPanel) errPanel.style.display = 'block';
              }} else if (s.stage) {{
                var statusEl = document.getElementById('splash-status');
                if (statusEl) {{
                  if (s.stage === 'migrating' || s.stage === 'opening_database') {{
                    statusEl.textContent = 'Aggiornamento del database in corso...';
                  }} else if (s.stage === 'starting_workers') {{
                    statusEl.textContent = 'Avvio dei servizi in corso...';
                  }} else {{
                    statusEl.textContent = 'Avvio di scrobbl.rs in corso...';
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
