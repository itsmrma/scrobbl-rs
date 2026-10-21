use std::{
    env,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
};

#[derive(Clone)]
pub struct Config {
    pub port: u16,
    pub bind_ip: IpAddr,
    pub external_origin: Option<url::Url>,
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
        let bind_ip = env::var("SCROBBL_BIND")
            .unwrap_or_else(|_| "127.0.0.1".into())
            .parse::<IpAddr>()?;
        let external_origin = env::var("SCROBBL_EXTERNAL_ORIGIN")
            .ok()
            .map(|value| parse_origin(&value))
            .transpose()?;
        anyhow::ensure!(
            bind_ip.is_loopback() || external_origin.is_some(),
            "Set SCROBBL_EXTERNAL_ORIGIN when binding beyond loopback"
        );
        Ok(Self {
            port,
            bind_ip,
            external_origin,
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
    pub fn address(&self) -> SocketAddr {
        SocketAddr::new(self.bind_ip, self.port)
    }
    pub fn origin(&self) -> String {
        self.external_origin
            .as_ref()
            .map(|url| url.origin().ascii_serialization())
            .unwrap_or_else(|| format!("http://{}", self.address()))
    }
    pub fn host(&self) -> String {
        self.origin()
            .split_once("://")
            .expect("validated origin")
            .1
            .to_owned()
    }
    pub fn secure_cookies(&self) -> bool {
        self.external_origin
            .as_ref()
            .is_some_and(|url| url.scheme() == "https")
    }
    pub fn spotify_redirect_allowed(&self) -> bool {
        self.secure_cookies()
            || url::Url::parse(&self.origin())
                .ok()
                .and_then(|url| match url.host() {
                    Some(url::Host::Ipv4(ip)) => Some(ip.is_loopback()),
                    Some(url::Host::Ipv6(ip)) => Some(ip.is_loopback()),
                    _ => None,
                })
                .unwrap_or(false)
    }
    pub fn redirect_uri(&self) -> String {
        format!("{}/auth/spotify/callback", self.origin())
    }
}

pub fn parse_origin(value: &str) -> anyhow::Result<url::Url> {
    let url = url::Url::parse(value)?;
    anyhow::ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.host().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none(),
        "SCROBBL_EXTERNAL_ORIGIN must be an http(s) origin without credentials, path, query or fragment"
    );
    anyhow::ensure!(
        !matches!(url.host(), Some(url::Host::Ipv4(ip)) if ip.is_unspecified())
            && !matches!(url.host(), Some(url::Host::Ipv6(ip)) if ip.is_unspecified()),
        "SCROBBL_EXTERNAL_ORIGIN must use an address reachable by the browser, not an unspecified bind address"
    );
    Ok(url)
}
