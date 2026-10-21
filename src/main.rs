use scrobbl_rs::{AppState, config::Config, db, spotify, web};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_env()?;
    let pool = db::open(&config.data_dir).await?;
    let state = AppState::new(pool, config.clone());
    let listener = tokio::net::TcpListener::bind(config.address()).await?;
    let worker = tokio::spawn(spotify::worker(state.clone()));
    let calendar_worker = tokio::spawn(scrobbl_rs::calendar::worker(state.storage.clone()));
    let import_worker = tokio::spawn(scrobbl_rs::import_worker::worker(state.storage.clone()));
    println!("scrobbl.rs is running at {}", config.origin());
    println!("Data: {}", config.data_dir.canonicalize()?.display());
    println!("Keep this process running to collect Spotify listens. Press Ctrl+C to stop.");
    let result = axum::serve(listener, web::router(state.clone()))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    worker.abort();
    let _ = worker.await;
    calendar_worker.abort();
    let _ = calendar_worker.await;
    import_worker.abort();
    let _ = import_worker.await;
    state.storage.close().await;
    result?;
    Ok(())
}
