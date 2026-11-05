//! Discovery, registration and token endpoints (protocol §4.1).

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::AppState;
use crate::{
    CONFIG,
    auth::{self, Auth, ClientIp},
    config::{KDF_ITERATIONS_MAX, KDF_ITERATIONS_MIN, KDF_PBKDF2, RuntimeSettings, VERSION},
    crypto,
    db::models::{DEVICE_TYPES, Device, Event, ROLE_OWNER, User},
    err,
    error::{ApiJson, ApiResult, JsonResult},
    util::{is_enc_string, is_valid_email, normalize_email, now},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/config", get(config))
        .route("/accounts/prelogin", post(prelogin))
        .route("/accounts/register", post(register))
        .route("/auth/token", post(token))
        .route("/auth/logout", post(logout))
}

async fn config(State(state): State<AppState>) -> JsonResult {
    let settings = RuntimeSettings::current();
    let users = state.db.run(User::count).await?;
    Ok(Json(json!({
        "name": "Monica Cloud",
        "version": VERSION,
        "protocolVersion": 1,
        "domain": CONFIG.domain,
        "initialized": users > 0,
        // The very first account can always be created; it becomes the owner.
        "signupsAllowed": settings.signups_allowed || users == 0,
        "sendsAllowed": settings.sends_allowed,
        "kdf": KDF_PBKDF2,
        "kdfIterations": CONFIG.kdf_iterations_default,
        "attachmentMaxBytes": CONFIG.attachment_max_bytes(),
        "features": ["items", "folders", "attachments", "sends", "incremental-sync"],
    })))
}

#[derive(Deserialize)]
struct PreloginData {
    email: String,
}

async fn prelogin(State(state): State<AppState>, ApiJson(data): ApiJson<PreloginData>) -> JsonResult {
    let email = normalize_email(&data.email);
    let user = state.db.run(move |conn| User::find_by_email(conn, &email)).await?;
    // Unknown accounts get the defaults so prelogin cannot be used to enumerate users.
    let (kdf, iterations) = match user {
        Some(user) => (user.kdf_type, user.kdf_iterations),
        None => (KDF_PBKDF2, CONFIG.kdf_iterations_default),
    };
    Ok(Json(json!({ "kdf": kdf, "kdfIterations": iterations })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegisterData {
    email: String,
    name: Option<String>,
    auth_hash: String,
    kdf: i32,
    kdf_iterations: i32,
    protected_user_key: String,
}

pub fn validate_kdf(kdf: i32, iterations: i32) -> ApiResult<()> {
    if kdf != KDF_PBKDF2 {
        err!(BadRequest, "unsupported kdf; protocol v1 only supports PBKDF2-SHA256 (0)");
    }
    if !(KDF_ITERATIONS_MIN..=KDF_ITERATIONS_MAX).contains(&iterations) {
        err!(BadRequest, format!("kdfIterations must be between {KDF_ITERATIONS_MIN} and {KDF_ITERATIONS_MAX}"));
    }
    Ok(())
}

/// Whether `email` may create an account on a server that already has `existing_users`.
fn check_signup_allowed(existing_users: i64, email: &str) -> ApiResult<()> {
    if existing_users == 0 {
        return Ok(()); // the first account is always allowed and becomes the owner
    }
    if !RuntimeSettings::current().signups_allowed {
        err!(Forbidden, "registration is disabled on this server");
    }
    let domain = email.rsplit('@').next().unwrap_or_default();
    if !CONFIG.signups_domains_whitelist.is_empty() && !CONFIG.signups_domains_whitelist.iter().any(|d| d == domain) {
        err!(Forbidden, "this email domain is not allowed to register");
    }
    Ok(())
}

async fn register(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(data): ApiJson<RegisterData>,
) -> JsonResult {
    state.login_limiter.check_ip("auth", &ip)?;

    let email = normalize_email(&data.email);
    if !is_valid_email(&email) {
        err!(BadRequest, "invalid email address");
    }
    let name = data.name.unwrap_or_default().trim().to_string();
    if name.chars().count() > 100 {
        err!(BadRequest, "name is too long");
    }
    if !crypto::is_valid_auth_hash(&data.auth_hash) {
        err!(BadRequest, "authHash must be base64 of 32 bytes");
    }
    validate_kdf(data.kdf, data.kdf_iterations)?;
    if !is_enc_string(&data.protected_user_key, 512) {
        err!(BadRequest, "protectedUserKey is not a v1 EncString");
    }

    // Reject what can be rejected cheaply before paying for argon2, so a closed server cannot be
    // made to hash for every request. The write transaction below re-checks all of it.
    let (existing, taken) = {
        let email = email.clone();
        state.db.run(move |conn| Ok((User::count(conn)?, User::find_by_email(conn, &email)?.is_some()))).await?
    };
    check_signup_allowed(existing, &email)?;
    if taken {
        err!(BadRequest, "an account with this email already exists");
    }

    // Hash outside the write transaction; argon2 is deliberately slow.
    let password_hash = crypto::hash_secret(data.auth_hash).await?;
    let user = User::new(email, name, password_hash, data.kdf, data.kdf_iterations, data.protected_user_key);

    let user = state
        .db
        .write(move |conn| {
            let mut user = user;
            let existing = User::count(conn)?;
            check_signup_allowed(existing, &user.email)?;
            if existing == 0 {
                user.role = ROLE_OWNER.into();
            }
            if User::find_by_email(conn, &user.email)?.is_some() {
                err!(BadRequest, "an account with this email already exists");
            }
            user.insert(conn)?;
            Event::new(Some(&user.uuid), &user.email, "account.register", &user.uuid, &ip).insert(conn)?;
            Ok(user)
        })
        .await?;

    Ok(Json(json!({ "id": user.uuid, "email": user.email, "role": user.role })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenData {
    grant_type: String,
    email: Option<String>,
    auth_hash: Option<String>,
    refresh_token: Option<String>,
    device: Option<DeviceData>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceData {
    identifier: String,
    name: String,
    #[serde(rename = "type")]
    dtype: String,
    app_version: Option<String>,
}

fn token_response(user: &User, device: &Device, refresh_token: String) -> Json<Value> {
    Json(json!({
        "accessToken": auth::issue_access_token(user, device),
        "expiresIn": CONFIG.access_token_ttl_secs,
        "tokenType": "Bearer",
        "refreshToken": refresh_token,
        "protectedUserKey": user.akey,
        "kdf": user.kdf_type,
        "kdfIterations": user.kdf_iterations,
        "account": user.account_json(),
        "deviceId": device.uuid,
    }))
}

async fn token(State(state): State<AppState>, ClientIp(ip): ClientIp, ApiJson(data): ApiJson<TokenData>) -> JsonResult {
    match data.grant_type.as_str() {
        "password" => password_grant(state, ip, data).await,
        "refresh_token" => refresh_grant(state, ip, data).await,
        _ => err!(BadRequest, "grantType must be \"password\" or \"refresh_token\""),
    }
}

async fn password_grant(state: AppState, ip: String, data: TokenData) -> JsonResult {
    state.login_limiter.check_ip("auth", &ip)?;

    let (Some(email), Some(auth_hash), Some(device)) = (data.email, data.auth_hash, data.device) else {
        err!(BadRequest, "email, authHash and device are required");
    };
    let identifier = device.identifier.trim().to_string();
    let device_name = device.name.trim().chars().take(128).collect::<String>();
    let dtype = device.dtype.trim().to_lowercase();
    if identifier.is_empty() || identifier.len() > 128 {
        err!(BadRequest, "device.identifier must be 1-128 characters");
    }
    if !DEVICE_TYPES.contains(&dtype.as_str()) {
        err!(BadRequest, format!("device.type must be one of {}", DEVICE_TYPES.join(", ")));
    }
    let app_version = device.app_version.unwrap_or_default().chars().take(64).collect::<String>();

    let email = normalize_email(&email);
    let user = state.db.run(move |conn| User::find_by_email(conn, &email)).await?;
    // Verify after the connection is back in the pool: argon2 takes tens of milliseconds, and
    // holding a connection meanwhile let a handful of logins stall every other request.
    let verified = match user {
        Some(user) => crypto::verify_secret(auth_hash, user.password_hash.clone()).await?.then_some(user),
        None => {
            crypto::dummy_verify().await?;
            None
        }
    };
    let Some(user) = verified else {
        err!(Unauthorized, "invalid email or master password");
    };
    if !user.enabled {
        err!(Forbidden, "account is disabled");
    }

    let (user, device, refresh_token) = state
        .db
        .write(move |conn| {
            // Re-read inside the transaction: the password may have been changed or the
            // account disabled while argon2 was running, and the stale copy must not be
            // written back.
            let Some(mut user) = User::find(conn, &user.uuid)?.filter(|u| u.password_hash == user.password_hash) else {
                err!(Unauthorized, "invalid email or master password");
            };
            if !user.enabled {
                err!(Forbidden, "account is disabled");
            }
            let mut device = match Device::find_by_identifier(conn, &user.uuid, &identifier)? {
                Some(existing) => existing,
                None => Device::new(user.uuid.clone(), identifier),
            };
            device.name = if device_name.is_empty() {
                dtype.clone()
            } else {
                device_name
            };
            device.dtype = dtype;
            device.app_version = app_version;
            device.last_seen_at = now();
            device.last_ip = ip.clone();
            let refresh_token = device.rotate_refresh_token();
            device.upsert(conn)?;

            user.last_login_at = Some(User::touch_login(conn, &user.uuid)?);
            Event::new(Some(&user.uuid), &user.email, "auth.login", &device.uuid, &ip)
                .device(&device.name)
                .insert(conn)?;
            Ok((user, device, refresh_token))
        })
        .await?;

    Ok(token_response(&user, &device, refresh_token))
}

async fn refresh_grant(state: AppState, ip: String, data: TokenData) -> JsonResult {
    let Some(presented) = data.refresh_token else {
        err!(BadRequest, "refreshToken is required");
    };
    let (user, device, refresh_token) = state
        .db
        .write(move |conn| {
            let Some(mut device) = Device::find_by_refresh_token(conn, presented.trim())? else {
                err!(Unauthorized, "refresh token is invalid or was already used");
            };
            let Some(user) = User::find(conn, &device.user_uuid)? else {
                err!(Unauthorized, "account no longer exists");
            };
            if !user.enabled {
                err!(Forbidden, "account is disabled");
            }
            let idle = now() - device.last_seen_at;
            if idle > chrono::Duration::days(CONFIG.refresh_token_days) {
                Device::delete(conn, &device.uuid)?;
                err!(Unauthorized, "session expired after inactivity, please log in again");
            }
            device.last_seen_at = now();
            device.last_ip = ip;
            let refresh_token = device.rotate_refresh_token();
            device.upsert(conn)?;
            Ok((user, device, refresh_token))
        })
        .await?;

    Ok(token_response(&user, &device, refresh_token))
}

async fn logout(State(state): State<AppState>, auth: Auth) -> JsonResult {
    let Auth {
        user,
        device_uuid,
        ip,
        ..
    } = auth;
    state
        .db
        .write(move |conn| {
            Device::delete(conn, &device_uuid)?;
            Event::new(Some(&user.uuid), &user.email, "auth.logout", &device_uuid, &ip).insert(conn)?;
            Ok(())
        })
        .await?;
    Ok(Json(json!({})))
}
