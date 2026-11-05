use axum::{
    Json, Router,
    extract::{Query, State},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    api::{AppState, identity::validate_kdf},
    auth::Auth,
    crypto,
    db::models::{Attachment, Device, Event, Item, ROLE_OWNER, Send, User},
    err,
    error::{ApiJson, ApiResult, Error, JsonResult},
    util::{fmt_date, is_enc_string, parse_date},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/accounts/profile", get(get_profile).put(put_profile))
        .route("/accounts/password", post(change_password))
        .route("/accounts/delete", post(delete_account))
        .route("/accounts/revision-date", get(revision_date))
        .route("/accounts/stats", get(stats))
        .route("/events", get(events))
}

async fn get_profile(auth: Auth) -> JsonResult {
    Ok(Json(auth.user.profile_json()))
}

#[derive(Deserialize)]
struct ProfileData {
    name: String,
}

async fn put_profile(State(state): State<AppState>, auth: Auth, ApiJson(data): ApiJson<ProfileData>) -> JsonResult {
    let name = data.name.trim().to_string();
    if name.chars().count() > 100 {
        err!(BadRequest, "name is too long");
    }
    let uuid = auth.user.uuid;
    let user = state
        .db
        .write(move |conn| {
            User::set_name(conn, &uuid, &name)?;
            User::find(conn, &uuid)?.ok_or_else(|| Error::Unauthorized("account no longer exists".into()))
        })
        .await?;
    Ok(Json(user.profile_json()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PasswordData {
    auth_hash: String,
    new_auth_hash: String,
    new_protected_user_key: String,
    kdf_iterations: Option<i32>,
}

/// Verify the caller's current authHash.
async fn verify_current(user: &User, auth_hash: String) -> ApiResult<()> {
    if !crypto::verify_secret(auth_hash, user.password_hash.clone()).await? {
        err!(BadRequest, "master password is incorrect");
    }
    Ok(())
}

async fn change_password(
    State(state): State<AppState>,
    auth: Auth,
    ApiJson(data): ApiJson<PasswordData>,
) -> JsonResult {
    let iterations = data.kdf_iterations.unwrap_or(auth.user.kdf_iterations);
    validate_kdf(auth.user.kdf_type, iterations)?;
    if !crypto::is_valid_auth_hash(&data.new_auth_hash) {
        err!(BadRequest, "newAuthHash must be base64 of 32 bytes");
    }
    if !is_enc_string(&data.new_protected_user_key, 512) {
        err!(BadRequest, "newProtectedUserKey is not a v1 EncString");
    }
    verify_current(&auth.user, data.auth_hash).await?;

    let new_hash = crypto::hash_secret(data.new_auth_hash).await?;
    let Auth {
        user: verified,
        ip,
        device_name,
        ..
    } = auth;
    state
        .db
        .write(move |conn| {
            // Work on a fresh row: the copy from the auth guard is older than the argon2
            // runs above, and saving it would roll back writes made in the meantime.
            let Some(mut user) = User::find(conn, &verified.uuid)? else {
                err!(Unauthorized, "account no longer exists");
            };
            if user.password_hash != verified.password_hash || user.security_stamp != verified.security_stamp {
                err!(Unauthorized, "the session changed while the password was being updated, please log in again");
            }
            user.password_hash = new_hash;
            user.akey = data.new_protected_user_key;
            user.kdf_iterations = iterations;
            // Every session, including this one, must log in again with the new password.
            user.rotate_security_stamp();
            user.save(conn)?;
            Device::delete_all_for_user(conn, &user.uuid)?;
            Event::new(Some(&user.uuid), &user.email, "account.password_change", &user.uuid, &ip)
                .device(&device_name)
                .insert(conn)?;
            Ok(())
        })
        .await?;
    Ok(Json(json!({})))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteData {
    auth_hash: String,
}

async fn delete_account(State(state): State<AppState>, auth: Auth, ApiJson(data): ApiJson<DeleteData>) -> JsonResult {
    verify_current(&auth.user, data.auth_hash).await?;
    let Auth {
        user,
        ip,
        ..
    } = auth;
    let blobs = state
        .db
        .write(move |conn| {
            if user.role == ROLE_OWNER && User::count_owners(conn)? == 1 && User::count(conn)? > 1 {
                err!(Forbidden, "you are the only owner; promote another owner before deleting this account");
            }
            let mut blobs = Attachment::blob_ids_for_user(conn, &user.uuid)?;
            blobs.extend(Send::blob_ids_for_user(conn, &user.uuid)?);
            User::delete(conn, &user.uuid)?;
            Event::new(None, &user.email, "account.delete", &user.uuid, &ip).insert(conn)?;
            Ok(blobs)
        })
        .await?;
    state.storage.delete_many(blobs).await;
    Ok(Json(json!({})))
}

/// Cheap poll target: clients skip `/sync` while this value is unchanged.
async fn revision_date(auth: Auth) -> JsonResult {
    Ok(Json(json!({ "revisionDate": fmt_date(&auth.user.revision_date) })))
}

async fn stats(State(state): State<AppState>, auth: Auth) -> JsonResult {
    let quota = auth.user.effective_quota();
    let uuid = auth.user.uuid.clone();
    let (types, used, devices) = state
        .db
        .read(move |conn| {
            Ok((
                Item::type_counts(conn, &uuid)?,
                User::storage_used(conn, &uuid)?,
                Device::count_for_user(conn, &uuid)?,
            ))
        })
        .await?;
    let total: i64 = types.iter().map(|(_, c)| c).sum();
    Ok(Json(json!({
        "items": total,
        "itemsByType": types.into_iter().map(|(t, c)| json!({ "type": t, "count": c })).collect::<Vec<_>>(),
        "storageUsedBytes": used,
        "storageQuotaBytes": quota,
        "devices": devices,
        "revisionDate": fmt_date(&auth.user.revision_date),
    })))
}

#[derive(Deserialize)]
pub struct EventQuery {
    pub limit: Option<i64>,
    pub before: Option<String>,
}

impl EventQuery {
    pub fn parse(&self) -> ApiResult<(i64, Option<chrono::NaiveDateTime>)> {
        let limit = self.limit.unwrap_or(100).clamp(1, 500);
        let before = match &self.before {
            Some(raw) => Some(parse_date(raw).ok_or_else(|| crate::error::Error::BadRequest("invalid before".into()))?),
            None => None,
        };
        Ok((limit, before))
    }
}

async fn events(State(state): State<AppState>, auth: Auth, Query(query): Query<EventQuery>) -> JsonResult {
    let (limit, before) = query.parse()?;
    let uuid = auth.user.uuid;
    let events = state.db.run(move |conn| Event::page(conn, Some(&uuid), before, limit)).await?;
    Ok(Json(json!({ "data": events.iter().map(Event::to_json).collect::<Vec<_>>() })))
}
