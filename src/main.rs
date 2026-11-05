use scrobbl_rs::{AppState, config::Config, db, spotify, web};

fn main() -> anyhow::Result<()> {
    loop {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        let result = runtime.block_on(start());
        // Wait for blocking parsers/file copies from the old instance before
        // opening the archive again. Aborting an async worker alone cannot do this.
        drop(runtime);
        if result? != scrobbl_rs::account::Action::Restart {
            return Ok(());
        }
    }
}

async fn start() -> anyhow::Result<scrobbl_rs::account::Action> {
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
        return Ok(scrobbl_rs::account::Action::Stop);
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
        return Ok(scrobbl_rs::account::Action::Stop);
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
        return Ok(scrobbl_rs::account::Action::Stop);
    }
    run(config).await
}

struct RunningState {
    state: AppState,
    workers: Vec<tokio::task::JoinHandle<()>>,
}

async fn run(config: Config) -> anyhow::Result<scrobbl_rs::account::Action> {
    scrobbl_rs::listenbrainz::Limits::from_env()?;
    let listener = tokio::net::TcpListener::bind(config.address()).await?;
    println!("scrobbl.rs is running at {}", config.origin());
    println!("Keep this process running to collect Spotify listens. Press Ctrl+C to stop.");

    let boot = scrobbl_rs::boot::BootManager::new();
    let app = scrobbl_rs::boot::router(boot.clone());

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
    let (action_tx, action_rx) = tokio::sync::watch::channel(scrobbl_rs::account::Action::Stop);
    let (boot_result_tx, mut boot_result_rx) =
        tokio::sync::oneshot::channel::<anyhow::Result<RunningState>>();

    let boot_task = {
        let boot = boot.clone();
        let config = config.clone();
        let shutdown_tx_action = shutdown_tx.clone();
        let action_tx_clone = action_tx.clone();
        tokio::spawn(async move {
            boot.set_stage("opening_database");
            let pool = match db::open(&config.data_dir).await {
                Ok(p) => p,
                Err(e) => {
                    boot.set_error(e.to_string());
                    let _ = boot_result_tx.send(Err(e));
                    let _ = shutdown_tx_action.send(true);
                    return;
                }
            };
            let state = AppState::new(pool, config.clone());
            boot.set_stage("starting_workers");
            let workers = vec![
                tokio::spawn(spotify::worker(state.clone())),
                tokio::spawn(scrobbl_rs::calendar::worker(state.storage.clone())),
                tokio::spawn(scrobbl_rs::import_worker::worker(state.storage.clone())),
                tokio::spawn(scrobbl_rs::listenbrainz::worker(state.storage.clone())),
                tokio::spawn(scrobbl_rs::recovery::worker(state.storage.clone())),
                tokio::spawn(scrobbl_rs::retention::worker(state.storage.clone())),
                tokio::spawn(scrobbl_rs::artwork::worker(state.clone())),
                tokio::spawn(scrobbl_rs::ai::worker(state.clone())),
            ];
            if let Ok(canon) = config.data_dir.canonicalize() {
                println!("Data: {}", canon.display());
            }
            let ready_router = web::router(state.clone());
            boot.set_ready(ready_router);

            let mut control = state.application.action.subscribe();
            tokio::spawn(async move {
                if control.changed().await.is_ok() {
                    let action = *control.borrow();
                    let _ = action_tx_clone.send(action);
                    let _ = shutdown_tx_action.send(true);
                }
            });

            let _ = boot_result_tx.send(Ok(RunningState { state, workers }));
        })
    };

    let signal_tx = shutdown_tx.clone();
    tokio::spawn(async move {
        shutdown().await;
        let _ = signal_tx.send(true);
    });

    let serve_result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.changed().await;
        })
        .await;

    boot_task.abort();
    serve_result?;

    match (&mut boot_result_rx).await {
        Ok(Ok(RunningState { state, workers })) => {
            for worker in workers {
                worker.abort();
                let _ = worker.await;
            }
            state.storage.close().await;
            Ok(*action_rx.borrow())
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Ok(*action_rx.borrow()),
    }
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
