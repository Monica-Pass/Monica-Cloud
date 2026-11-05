//! Token issuing and request guards (vaultwarden's `auth.rs` counterpart).

use std::{
    net::{IpAddr, SocketAddr},
    sync::OnceLock,
};

use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::{HeaderMap, header, request::Parts},
};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

use crate::{
    CONFIG,
    api::AppState,
    config::TrustedProxies,
    db::models::{Device, User},
    err,
    error::{ApiResult, Error},
    util::parse_forwarded_ip,
};

const ISSUER: &str = "monica-cloud";
const TYP_ACCESS: &str = "access";
const TYP_SEND_FILE: &str = "send-file";
const SEND_FILE_TOKEN_SECS: i64 = 300;

static KEYS: OnceLock<(EncodingKey, DecodingKey)> = OnceLock::new();

/// Load the HMAC signing key from `DATA_FOLDER/jwt.key`, creating it on first start.
pub fn init_keys() -> std::io::Result<()> {
    let path = CONFIG.data_folder.join("jwt.key");
    let secret = match std::fs::read(&path) {
        Ok(bytes) if bytes.len() >= 32 => bytes,
        _ => {
            let bytes = data_encoding::HEXLOWER.encode(&crate::crypto::random_bytes::<64>()).into_bytes();
            write_private(&path, &bytes)?;
            bytes
        }
    };
    let _ = KEYS.set((EncodingKey::from_secret(&secret), DecodingKey::from_secret(&secret)));
    Ok(())
}

fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)
}

fn keys() -> &'static (EncodingKey, DecodingKey) {
    KEYS.get().expect("auth::init_keys must run before serving requests")
}

fn validation() -> Validation {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_issuer(&[ISSUER]);
    validation.leeway = 30;
    validation
}

#[derive(Debug, Serialize, Deserialize)]
struct AccessClaims {
    iss: String,
    typ: String,
    sub: String,
    dev: String,
    stamp: String,
    iat: i64,
    exp: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct SendFileClaims {
    iss: String,
    typ: String,
    sub: String,
    iat: i64,
    exp: i64,
}

pub fn issue_access_token(user: &User, device: &Device) -> String {
    let now = chrono::Utc::now().timestamp();
    let claims = AccessClaims {
        iss: ISSUER.into(),
        typ: TYP_ACCESS.into(),
        sub: user.uuid.clone(),
        dev: device.uuid.clone(),
        stamp: user.security_stamp.clone(),
        iat: now,
        exp: now + CONFIG.access_token_ttl_secs,
    };
    encode(&Header::new(Algorithm::HS256), &claims, &keys().0).expect("HS256 signing cannot fail")
}

pub fn issue_send_file_token(send_uuid: &str) -> String {
    let now = chrono::Utc::now().timestamp();
    let claims = SendFileClaims {
        iss: ISSUER.into(),
        typ: TYP_SEND_FILE.into(),
        sub: send_uuid.into(),
        iat: now,
        exp: now + SEND_FILE_TOKEN_SECS,
    };
    encode(&Header::new(Algorithm::HS256), &claims, &keys().0).expect("HS256 signing cannot fail")
}

/// Returns the Send id the download token was issued for.
pub fn verify_send_file_token(token: &str) -> ApiResult<String> {
    let data = decode::<SendFileClaims>(token, &keys().1, &validation())
        .map_err(|_| Error::Unauthorized("download link expired".into()))?;
    if data.claims.typ != TYP_SEND_FILE {
        err!(Unauthorized, "invalid download token");
    }
    Ok(data.claims.sub)
}

/// The caller's IP (see [`client_ip`]); empty when unknown.
pub struct ClientIp(pub String);

impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|ConnectInfo(addr)| addr.ip());
        let ip = client_ip(peer, &parts.headers, CONFIG.ip_header.as_deref(), &CONFIG.trusted_proxies);
        Ok(ClientIp(ip.map(|ip| ip.to_string()).unwrap_or_default()))
    }
}

/// The address a request came from. Only a trusted reverse proxy may report the client in
/// `header`; for anyone else the socket address counts, so a client cannot choose its own
/// address (and rate-limit bucket) by sending the header itself.
///
/// Proxies append the address they received the request from (`X-Forwarded-For`) or replace
/// the value (`X-Real-IP`). The list is read from the right, skipping trusted proxies; the first
/// other address is the client. Entries left of it were supplied by the client and are ignored.
fn client_ip(
    peer: Option<IpAddr>,
    headers: &HeaderMap,
    header: Option<&str>,
    trusted: &TrustedProxies,
) -> Option<IpAddr> {
    let peer = peer?.to_canonical();
    let Some(name) = header else {
        return Some(peer);
    };
    let mut client = peer;
    if !trusted.contains(client) {
        return Some(client);
    }
    let values: Vec<&str> = headers.get_all(name).iter().filter_map(|v| v.to_str().ok()).collect();
    for entry in values.iter().rev().flat_map(|value| value.rsplit(',')) {
        let Some(ip) = parse_forwarded_ip(entry) else {
            break;
        };
        client = ip;
        if !trusted.contains(ip) {
            break;
        }
    }
    Some(client)
}

/// An authenticated user with a live device session.
pub struct Auth {
    pub user: User,
    pub device_uuid: String,
    pub device_name: String,
    pub ip: String,
}

impl FromRequestParts<AppState> for Auth {
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let ClientIp(ip) = ClientIp::from_request_parts(parts, state).await?;
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(|| Error::Unauthorized("missing bearer token".into()))?
            .trim()
            .to_string();

        let claims = decode::<AccessClaims>(&token, &keys().1, &validation())
            .map_err(|_| Error::Unauthorized("invalid or expired token".into()))?
            .claims;
        if claims.typ != TYP_ACCESS {
            err!(Unauthorized, "invalid token type");
        }

        let (user, device_name) = state
            .db
            .run(move |conn| {
                let user = User::find(conn, &claims.sub)?;
                let device = Device::find(conn, &claims.dev, &claims.sub)?;
                Ok((user.map(|u| (u, claims.stamp, claims.dev)), device.map(|d| d.name)))
            })
            .await?;

        let Some((user, stamp, device_uuid)) = user else {
            err!(Unauthorized, "account no longer exists");
        };
        let Some(device_name) = device_name.filter(|_| user.security_stamp == stamp) else {
            err!(Unauthorized, "session was revoked, please log in again");
        };
        if !user.enabled {
            err!(Forbidden, "account is disabled");
        }
        Ok(Auth {
            user,
            device_uuid,
            device_name,
            ip,
        })
    }
}

/// An authenticated owner or admin.
pub struct AdminAuth(pub Auth);

impl FromRequestParts<AppState> for AdminAuth {
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let auth = Auth::from_request_parts(parts, state).await?;
        if !auth.user.is_admin() {
            err!(Forbidden, "administrator role required");
        }
        Ok(AdminAuth(auth))
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderName;

    use super::*;

    fn ip(raw: &str) -> Option<IpAddr> {
        Some(raw.parse().unwrap())
    }

    fn headers(name: &'static str, values: &[&str]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for value in values {
            map.append(HeaderName::from_static(name), value.parse().unwrap());
        }
        map
    }

    #[test]
    fn client_ip_trusts_the_header_from_proxies_only() {
        let default = TrustedProxies::parse(None);
        let (real, xff) = (Some("X-Real-IP"), Some("X-Forwarded-For"));

        // Exposed directly: a client cannot choose its address.
        assert_eq!(
            client_ip(ip("203.0.113.9"), &headers("x-real-ip", &["1.2.3.4"]), real, &default),
            ip("203.0.113.9")
        );
        // Behind a proxy on the same host that sets X-Real-IP.
        assert_eq!(
            client_ip(ip("127.0.0.1"), &headers("x-real-ip", &["198.51.100.7"]), real, &default),
            ip("198.51.100.7")
        );
        // The proxy appended the real client to a list the client forged.
        let forged = headers("x-forwarded-for", &["1.2.3.4, 198.51.100.7"]);
        assert_eq!(client_ip(ip("127.0.0.1"), &forged, xff, &default), ip("198.51.100.7"));
        // Two proxies (Docker network, then loopback); the header arrives as two lines.
        let chained = headers("x-forwarded-for", &["6.6.6.6, 198.51.100.7", "172.18.0.2"]);
        assert_eq!(client_ip(ip("::ffff:127.0.0.1"), &chained, xff, &default), ip("198.51.100.7"));
        // A LAN client behind the proxy keeps its own address.
        assert_eq!(
            client_ip(ip("127.0.0.1"), &headers("x-forwarded-for", &["192.168.1.5"]), xff, &default),
            ip("192.168.1.5")
        );
        // No header, an unreadable one, or IP_HEADER=none: the socket address.
        assert_eq!(client_ip(ip("127.0.0.1"), &HeaderMap::new(), real, &default), ip("127.0.0.1"));
        assert_eq!(client_ip(ip("127.0.0.1"), &headers("x-real-ip", &["unknown"]), real, &default), ip("127.0.0.1"));
        assert_eq!(
            client_ip(ip("127.0.0.1"), &headers("x-real-ip", &["198.51.100.7"]), None, &default),
            ip("127.0.0.1")
        );

        let only = TrustedProxies::parse(Some("10.1.2.3, fd00::/8"));
        assert_eq!(client_ip(ip("127.0.0.1"), &headers("x-real-ip", &["198.51.100.7"]), real, &only), ip("127.0.0.1"));
        assert_eq!(client_ip(ip("fd00::5"), &headers("x-real-ip", &["198.51.100.7"]), real, &only), ip("198.51.100.7"));
        let any = TrustedProxies::parse(Some("*"));
        assert_eq!(
            client_ip(ip("203.0.113.9"), &headers("x-real-ip", &["198.51.100.7"]), real, &any),
            ip("198.51.100.7")
        );
    }
}
