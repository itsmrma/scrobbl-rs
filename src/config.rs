use std::{
    env,
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
};

#[derive(Clone)]
pub struct Config {
    pub port: u16,
    pub data_dir: PathBuf,
    pub sync_seconds: u64,
    pub now_playing_seconds: u64,
    pub import_limits: crate::imports::ImportLimits,
    pub env_client_id: Option<String>,
    pub env_client_secret: Option<String>,
}
impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        dotenvy::dotenv().ok();
        let port = env::var("SCROBBL_PORT")
            .unwrap_or_else(|_| "3210".into())
            .parse()?;
        anyhow::ensure!(port != 0, "SCROBBL_PORT must be between 1 and 65535");
        let sync_seconds = env::var("SCROBBL_SYNC_SECONDS")
            .unwrap_or_else(|_| "120".into())
            .parse::<u64>()?;
        let now_playing_seconds = env::var("SCROBBL_NOW_PLAYING_SECONDS")
            .unwrap_or_else(|_| "30".into())
            .parse::<u64>()?;
        anyhow::ensure!(
            sync_seconds >= 60,
            "SCROBBL_SYNC_SECONDS must be at least 60"
        );
        anyhow::ensure!(
            now_playing_seconds >= 15,
            "SCROBBL_NOW_PLAYING_SECONDS must be at least 15"
        );
        let env_client_id = env::var("SPOTIFY_CLIENT_ID")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let env_client_secret = env::var("SPOTIFY_CLIENT_SECRET")
            .ok()
            .filter(|v| !v.trim().is_empty());
        anyhow::ensure!(
            env_client_id.is_some() == env_client_secret.is_some(),
            "Set both SPOTIFY_CLIENT_ID and SPOTIFY_CLIENT_SECRET, or neither"
        );
        Ok(Self {
            port,
            data_dir: env::var_os("SCROBBL_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| "data".into()),
            sync_seconds,
            now_playing_seconds,
            import_limits: crate::imports::ImportLimits::from_env()?,
            env_client_id,
            env_client_secret,
        })
    }
    pub fn address(&self) -> SocketAddrV4 {
        SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.port)
    }
    pub fn origin(&self) -> String {
        format!("http://{}", self.address())
    }
    pub fn redirect_uri(&self) -> String {
        format!("{}/auth/spotify/callback", self.origin())
    }
}
