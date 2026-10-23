use scrobbl_rs::{AppState, config::Config, db, spotify, web};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        // These SQLite functions return static, NUL-terminated strings.
        let (sqlite, source) = unsafe {
            (
                std::ffi::CStr::from_ptr(libsqlite3_sys::sqlite3_libversion()),
                std::ffi::CStr::from_ptr(libsqlite3_sys::sqlite3_sourceid()),
            )
        };
        println!(
            "scrobbl.rs {}\nSQLite {}\nSQLite source {}",
            env!("CARGO_PKG_VERSION"),
            sqlite.to_string_lossy(),
            source.to_string_lossy()
        );
        return Ok(());
    }
    let config = Config::from_env()?;
    if std::env::args().nth(1).as_deref() == Some("--recover-upgrade") {
        let args: Vec<String> = std::env::args().collect();
        anyhow::ensure!(
            args.len() == 5 && args[3] == "--to",
            "Usage: scrobbl-rs --recover-upgrade pre-upgrade-ID --to NEW_DIRECTORY"
        );
        scrobbl_rs::recovery::recover_upgrade(
            &config.data_dir,
            &args[2],
            std::path::Path::new(&args[4]),
        )
        .await?;
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("--healthcheck") {
        let ip = if config.bind_ip.is_unspecified() {
            if config.bind_ip.is_ipv4() {
                "127.0.0.1"
            } else {
                "[::1]"
            }
            .to_owned()
        } else if config.bind_ip.is_ipv6() {
            format!("[{}]", config.bind_ip)
        } else {
            config.bind_ip.to_string()
        };
        reqwest::Client::new()
            .get(format!("http://{ip}:{}/healthz", config.port))
            .header("Host", config.host())
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await?
            .error_for_status()?;
        return Ok(());
    }
    scrobbl_rs::listenbrainz::Limits::from_env()?;
    let pool = db::open(&config.data_dir).await?;
    let state = AppState::new(pool, config.clone());
    let listener = tokio::net::TcpListener::bind(config.address()).await?;
    let worker = tokio::spawn(spotify::worker(state.clone()));
    let calendar_worker = tokio::spawn(scrobbl_rs::calendar::worker(state.storage.clone()));
    let import_worker = tokio::spawn(scrobbl_rs::import_worker::worker(state.storage.clone()));
    let listenbrainz_worker = tokio::spawn(scrobbl_rs::listenbrainz::worker(state.storage.clone()));
    let recovery_worker = tokio::spawn(scrobbl_rs::recovery::worker(state.storage.clone()));
    let retention_worker = tokio::spawn(scrobbl_rs::retention::worker(state.storage.clone()));
    let artwork_worker = tokio::spawn(scrobbl_rs::artwork::worker(state.clone()));
    println!("scrobbl.rs is running at {}", config.origin());
    println!("Data: {}", config.data_dir.canonicalize()?.display());
    println!("Keep this process running to collect Spotify listens. Press Ctrl+C to stop.");
    let result = axum::serve(listener, web::router(state.clone()))
        .with_graceful_shutdown(shutdown())
        .await;
    worker.abort();
    let _ = worker.await;
    calendar_worker.abort();
    let _ = calendar_worker.await;
    import_worker.abort();
    let _ = import_worker.await;
    recovery_worker.abort();
    let _ = recovery_worker.await;
    retention_worker.abort();
    let _ = retention_worker.await;
    listenbrainz_worker.abort();
    let _ = listenbrainz_worker.await;
    artwork_worker.abort();
    let _ = artwork_worker.await;
    state.storage.close().await;
    result?;
    Ok(())
}

async fn shutdown() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
