//! Environment driven configuration, in the spirit of vaultwarden's `config.rs`.
//!
//! Static values come from the environment (or a `.env` file). A small set of
//! values can be changed at runtime from the admin panel; those live in the
//! `server_settings` table and are cached in [`SETTINGS`].

use std::{
    env,
    net::IpAddr,
    path::PathBuf,
    str::FromStr,
    sync::{LazyLock, RwLock},
};

use serde::{Deserialize, Serialize};

use crate::util::IpNet;

pub static CONFIG: LazyLock<Config> = LazyLock::new(Config::load);
pub static SETTINGS: LazyLock<RwLock<RuntimeSettings>> =
    LazyLock::new(|| RwLock::new(RuntimeSettings::from_config(&CONFIG)));

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub const KDF_PBKDF2: i32 = 0;
pub const KDF_ITERATIONS_MIN: i32 = 100_000;
pub const KDF_ITERATIONS_MAX: i32 = 2_000_000;

#[derive(Debug, Clone)]
pub struct Config {
    /// Public base URL, e.g. `https://cloud.example.com`. Used for share links and the `config` endpoint.
    pub domain: String,
    pub address: String,
    pub port: u16,
    pub data_folder: PathBuf,
    pub database_url: String,
    pub web_vault_folder: PathBuf,
    pub web_vault_enabled: bool,

    pub signups_allowed: bool,
    pub signups_domains_whitelist: Vec<String>,
    pub sends_allowed: bool,
    pub user_quota_mb: Option<i64>,
    pub attachment_max_mb: i64,
    pub json_limit_mb: usize,

    pub kdf_iterations_default: i32,
    pub access_token_ttl_secs: i64,
    /// A device that has not refreshed its token for this many days must log in again.
    pub refresh_token_days: i64,
    /// Header that carries the real client IP when running behind a reverse proxy. `none` disables it.
    pub ip_header: Option<String>,
    /// Peers allowed to set `ip_header`. Everyone else is identified by the socket address.
    pub trusted_proxies: TrustedProxies,
    pub login_ratelimit_per_minute: u32,

    pub event_retention_days: i64,
    pub tombstone_retention_days: i64,
    pub log_level: String,
}

impl Config {
    fn load() -> Self {
        let data_folder = PathBuf::from(get_env("DATA_FOLDER").unwrap_or_else(|| "data".into()));
        let database_url =
            get_env("DATABASE_URL").unwrap_or_else(|| data_folder.join("db.sqlite3").to_string_lossy().into_owned());
        let ip_header = get_env("IP_HEADER").unwrap_or_else(|| "X-Real-IP".into());

        Self {
            domain: get_env("DOMAIN").unwrap_or_default().trim_end_matches('/').to_string(),
            address: get_env("ADDRESS").unwrap_or_else(|| "0.0.0.0".into()),
            port: parse_env("PORT", 8000),
            data_folder,
            database_url,
            web_vault_folder: PathBuf::from(get_env("WEB_VAULT_FOLDER").unwrap_or_else(|| "web-vault".into())),
            web_vault_enabled: parse_env("WEB_VAULT_ENABLED", true),

            signups_allowed: parse_env("SIGNUPS_ALLOWED", true),
            signups_domains_whitelist: get_env("SIGNUPS_DOMAINS_WHITELIST")
                .map(|v| v.split(',').map(|d| d.trim().to_lowercase()).filter(|d| !d.is_empty()).collect())
                .unwrap_or_default(),
            sends_allowed: parse_env("SENDS_ALLOWED", true),
            user_quota_mb: get_env("USER_QUOTA_MB").and_then(|v| v.parse().ok()).filter(|v: &i64| *v > 0),
            attachment_max_mb: parse_env("ATTACHMENT_MAX_MB", 100),
            json_limit_mb: parse_env("JSON_LIMIT_MB", 32),

            kdf_iterations_default: parse_env("KDF_ITERATIONS_DEFAULT", 600_000)
                .clamp(KDF_ITERATIONS_MIN, KDF_ITERATIONS_MAX),
            access_token_ttl_secs: parse_env("ACCESS_TOKEN_TTL_SECS", 7200),
            refresh_token_days: parse_env("REFRESH_TOKEN_DAYS", 90),
            ip_header: (!ip_header.eq_ignore_ascii_case("none")).then_some(ip_header),
            trusted_proxies: TrustedProxies::parse(get_env("TRUSTED_PROXIES").as_deref()),
            login_ratelimit_per_minute: parse_env("LOGIN_RATELIMIT_PER_MINUTE", 10),

            event_retention_days: parse_env("EVENT_RETENTION_DAYS", 365),
            tombstone_retention_days: parse_env("TOMBSTONE_RETENTION_DAYS", 180),
            log_level: get_env("LOG_LEVEL").unwrap_or_else(|| "info".into()),
        }
    }

    pub fn attachment_max_bytes(&self) -> i64 {
        self.attachment_max_mb.saturating_mul(1024 * 1024)
    }

    pub fn blobs_folder(&self) -> PathBuf {
        self.data_folder.join("blobs")
    }

    pub fn tmp_folder(&self) -> PathBuf {
        self.data_folder.join("tmp")
    }
}

/// Reverse proxies whose client-IP header is believed (`TRUSTED_PROXIES`).
#[derive(Debug, Clone)]
pub enum TrustedProxies {
    /// `*`: every peer, the behaviour before this setting existed.
    Any,
    List(Vec<IpNet>),
}

impl TrustedProxies {
    /// Loopback and private networks: a proxy on the same host, in Docker or on the LAN.
    const DEFAULT: &'static [&'static str] =
        &["127.0.0.0/8", "::1/128", "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "fc00::/7"];

    pub fn parse(raw: Option<&str>) -> Self {
        let raw = raw.map(str::trim).unwrap_or_default();
        if raw == "*" || raw.eq_ignore_ascii_case("any") {
            return Self::Any;
        }
        let entries: Vec<&str> = if raw.is_empty() {
            Self::DEFAULT.to_vec()
        } else {
            raw.split(',').map(str::trim).filter(|s| !s.is_empty()).collect()
        };
        Self::List(
            entries
                .into_iter()
                .filter_map(|entry| {
                    let net = IpNet::parse(entry);
                    if net.is_none() {
                        eprintln!(
                            "[WARNING] TRUSTED_PROXIES entry {entry:?} is not an IP address or network, ignoring it"
                        );
                    }
                    net
                })
                .collect(),
        )
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        match self {
            Self::Any => true,
            Self::List(nets) => nets.iter().any(|net| net.contains(ip)),
        }
    }
}

/// Settings an administrator can change without restarting the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSettings {
    pub signups_allowed: bool,
    pub sends_allowed: bool,
    /// `None` means unlimited.
    pub default_quota_bytes: Option<i64>,
    pub event_retention_days: i64,
}

impl RuntimeSettings {
    fn from_config(config: &Config) -> Self {
        Self {
            signups_allowed: config.signups_allowed,
            sends_allowed: config.sends_allowed,
            default_quota_bytes: config.user_quota_mb.map(|mb| mb.saturating_mul(1024 * 1024)),
            event_retention_days: config.event_retention_days,
        }
    }

    pub fn current() -> Self {
        SETTINGS.read().expect("settings lock poisoned").clone()
    }

    pub fn replace(next: RuntimeSettings) {
        *SETTINGS.write().expect("settings lock poisoned") = next;
    }
}

fn get_env(key: &str) -> Option<String> {
    env::var(key).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn parse_env<T: FromStr>(key: &str, default: T) -> T {
    match get_env(key) {
        Some(raw) => match raw.parse() {
            Ok(value) => value,
            Err(_) => {
                eprintln!("[WARNING] {key}={raw:?} is invalid, using the default value");
                default
            }
        },
        None => default,
    }
}
