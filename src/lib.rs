pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod imports;
pub mod spotify;
pub mod web;

use config::Config;
use sqlx::SqlitePool;
use std::{collections::VecDeque, sync::Arc, time::Instant};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub config: Config,
    pub spotify: spotify::Spotify,
    pub login_attempts: Arc<Mutex<VecDeque<Instant>>>,
    pub imports: Arc<Mutex<()>>,
}
impl AppState {
    pub fn new(db: SqlitePool, config: Config) -> Self {
        Self {
            db,
            config,
            spotify: spotify::Spotify::new(),
            login_attempts: Default::default(),
            imports: Default::default(),
        }
    }
}
