pub mod account;
pub mod activities;
pub mod api_keys;
pub mod artwork;
pub mod artwork_settings;
pub mod artwork_sources;
pub mod auth;
pub mod browse;
pub mod calendar;
pub mod config;
pub mod consultation;
pub mod dashboard_layout;
pub mod db;
pub mod dto;
pub mod error;
pub mod exports;
pub mod import_worker;
pub mod imports;
mod ingest;
pub mod jobs;
pub mod library;
pub mod listenbrainz;
pub mod listening_patterns;
pub mod name_groups;
pub mod preferences;
pub mod read_api;
pub mod recovery;
pub mod recovery_http;
pub mod retention;
pub mod review;
pub mod spotify;
pub mod storage;
pub mod web;

use config::Config;
use sqlx::SqlitePool;
use std::{collections::VecDeque, sync::Arc, time::Instant};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct AppState {
    pub application: account::Application,
    pub db: SqlitePool,
    pub private_db: SqlitePool,
    pub read_db: SqlitePool,
    pub storage: storage::Storage,
    pub config: Config,
    pub spotify: spotify::Spotify,
    pub(crate) artwork_gate:
        Arc<Mutex<std::collections::HashMap<String, std::sync::Weak<Mutex<()>>>>>,
    pub(crate) artwork_fetches: Arc<tokio::sync::Semaphore>,
    pub login_attempts: Arc<Mutex<VecDeque<Instant>>>,
    pub(crate) dashboard_cache: Arc<Mutex<VecDeque<(String, serde_json::Value)>>>,
}
impl AppState {
    pub fn new(storage: storage::Storage, config: Config) -> Self {
        Self {
            application: account::Application::new(),
            db: storage.writer.clone(),
            private_db: storage.private.clone(),
            read_db: storage.readers.clone(),
            storage,
            config,
            spotify: spotify::Spotify::new(),
            login_attempts: Default::default(),
            artwork_gate: Default::default(),
            artwork_fetches: Arc::new(tokio::sync::Semaphore::new(1)),
            dashboard_cache: Default::default(),
        }
    }
}
