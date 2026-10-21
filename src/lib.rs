pub mod activities;
pub mod auth;
pub mod calendar;
pub mod config;
pub mod consultation;
pub mod db;
pub mod dto;
pub mod error;
pub mod import_worker;
pub mod imports;
pub mod jobs;
pub mod listenbrainz;
pub mod preferences;
pub mod spotify;
pub mod storage;
pub mod web;

use config::Config;
use sqlx::SqlitePool;
use std::{collections::VecDeque, sync::Arc, time::Instant};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub private_db: SqlitePool,
    pub read_db: SqlitePool,
    pub storage: storage::Storage,
    pub config: Config,
    pub spotify: spotify::Spotify,
    pub login_attempts: Arc<Mutex<VecDeque<Instant>>>,
}
impl AppState {
    pub fn new(storage: storage::Storage, config: Config) -> Self {
        Self {
            db: storage.writer.clone(),
            private_db: storage.private.clone(),
            read_db: storage.readers.clone(),
            storage,
            config,
            spotify: spotify::Spotify::new(),
            login_attempts: Default::default(),
        }
    }
}
