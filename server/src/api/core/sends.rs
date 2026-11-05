//! Monica Send: owner management and public access (protocol §4.6).

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::header,
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use chrono::Duration;
use serde::Deserialize;
use serde_json::json;

use super::attachments::{MIN_ENCRYPTED_SIZE, check_quota};
use crate::{
    CONFIG,
    api::AppState,
    auth::{self, Auth, ClientIp},
    config::RuntimeSettings,
    crypto,
    db::{
        Conn,
        models::{Event, OBJECT_SEND, SEND_TYPE_FILE, SEND_TYPE_TEXT, Send, Tombstone, User},
    },
    err,
    error::{ApiJson, ApiResult, Error, JsonResult},
    util::{is_enc_string, now, parse_date},
};

const SEND_MAX_LIFETIME_DAYS: i64 = 31;
const SEND_TEXT_MAX: usize = 1024 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/sends", get(list_sends).post(create_send))
        .route("/sends/{id}", put(update_send).delete(delete_send))
        .route("/sends/{id}/file", put(upload_send_file))
        .route("/public/sends/{access_id}", post(access_send))
        .route("/public/sends/{access_id}/file/{token}", get(download_send_file))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendData {
    #[serde(rename = "type")]
    atype: i32,
    key: String,
    name: String,
    notes: Option<String>,
    text: Option<String>,
    file_name: Option<String>,
    size: Option<i64>,
    password_hash: Option<String>,
    #[serde(default)]
    remove_password: bool,
    max_access_count: Option<i32>,
    expiration_date: Option<String>,
    deletion_date: String,
    #[serde(default)]
    hide_email: bool,
    #[serde(default)]
    disabled: bool,
}

struct ValidatedSend {
    data: SendData,
    deletion_at: chrono::NaiveDateTime,
    expiration_at: Option<chrono::NaiveDateTime>,
    password_hash: Option<String>,
}

/// Validate a create/update request. Hashes a new Send password with argon2.
/// `creating` is false for updates, where an already-passed expiration date may be kept.
async fn validate(data: SendData, creating: bool) -> ApiResult<ValidatedSend> {
    if !RuntimeSettings::current().sends_allowed {
        err!(Forbidden, "Send is disabled on this server");
    }
    if !is_enc_string(&data.key, 512) || !is_enc_string(&data.name, 4096) {
        err!(BadRequest, "key and name must be v1 EncStrings");
    }
    if let Some(notes) = &data.notes
        && !is_enc_string(notes, 64 * 1024)
    {
        err!(BadRequest, "notes must be a v1 EncString");
    }
    match data.atype {
        SEND_TYPE_TEXT => {
            if !data.text.as_deref().is_some_and(|t| is_enc_string(t, SEND_TEXT_MAX)) {
                err!(BadRequest, "text sends need an EncString text of at most 1 MiB");
            }
        }
        SEND_TYPE_FILE => {
            if !data.file_name.as_deref().is_some_and(|f| is_enc_string(f, 4096)) {
                err!(BadRequest, "file sends need an EncString fileName");
            }
            let size = data.size.unwrap_or(0);
            if size < MIN_ENCRYPTED_SIZE {
                err!(BadRequest, "file sends need the encrypted size");
            }
            if size > CONFIG.attachment_max_bytes() {
                err!(PayloadTooLarge, format!("files are limited to {} MiB", CONFIG.attachment_max_mb));
            }
        }
        _ => err!(BadRequest, "type must be 0 (text) or 1 (file)"),
    }
    if data.max_access_count.is_some_and(|max| max < 1) {
        err!(BadRequest, "maxAccessCount must be at least 1");
    }

    let now = now();
    let Some(deletion_at) = parse_date(&data.deletion_date) else {
        err!(BadRequest, "deletionDate must be an RFC 3339 timestamp");
    };
    if deletion_at <= now || deletion_at > now + Duration::days(SEND_MAX_LIFETIME_DAYS) {
        err!(BadRequest, format!("deletionDate must be within the next {SEND_MAX_LIFETIME_DAYS} days"));
    }
    let expiration_at = match &data.expiration_date {
        Some(raw) => match parse_date(raw) {
            Some(exp) if (exp > now || !creating) && exp <= deletion_at => Some(exp),
            _ => err!(BadRequest, "expirationDate must be in the future and not after deletionDate"),
        },
        None => None,
    };

    let password_hash = match &data.password_hash {
        Some(hash) if crypto::is_valid_auth_hash(hash) => Some(crypto::hash_secret(hash.clone()).await?),
        Some(_) => err!(BadRequest, "passwordHash must be base64 of 32 bytes"),
        None => None,
    };
    Ok(ValidatedSend {
        data,
        deletion_at,
        expiration_at,
        password_hash,
    })
}

fn find_send(conn: &mut Conn, id: &str, user_uuid: &str) -> ApiResult<Send> {
    Send::find(conn, id, user_uuid)?.ok_or_else(|| Error::NotFound("send not found".into()))
}

async fn list_sends(State(state): State<AppState>, auth: Auth) -> JsonResult {
    let uuid = auth.user.uuid;
    let sends = state.db.run(move |conn| Send::changed_since(conn, &uuid, None)).await?;
    Ok(Json(json!({ "data": sends.iter().map(Send::to_json).collect::<Vec<_>>() })))
}

async fn create_send(State(state): State<AppState>, auth: Auth, ApiJson(data): ApiJson<SendData>) -> JsonResult {
    let v = validate(data, true).await?;
    let send = state
        .db
        .write(move |conn| {
            if v.data.atype == SEND_TYPE_FILE {
                check_quota(conn, &auth.user, v.data.size.unwrap_or(0))?;
            }
            let revision = User::next_revision(conn, &auth.user.uuid)?;
            let mut send = Send::new(auth.user.uuid.clone(), v.data.atype, v.deletion_at, revision);
            send.akey = v.data.key;
            send.name = v.data.name;
            send.notes = v.data.notes;
            send.text = v.data.text.filter(|_| v.data.atype == SEND_TYPE_TEXT);
            if v.data.atype == SEND_TYPE_FILE {
                send.file_name = v.data.file_name;
                send.size_bytes = v.data.size;
                send.blob_id = Some(crate::storage::new_blob_id());
            }
            send.password_hash = v.password_hash;
            send.max_access_count = v.data.max_access_count;
            send.expiration_at = v.expiration_at;
            send.hide_email = v.data.hide_email;
            send.disabled = v.data.disabled;
            send.insert(conn)?;
            Event::new(Some(&auth.user.uuid), &auth.user.email, "send.create", &send.uuid, &auth.ip)
                .device(&auth.device_name)
                .insert(conn)?;
            Ok(send)
        })
        .await?;
    let upload_url = (send.atype == SEND_TYPE_FILE).then(|| format!("/api/v1/sends/{}/file", send.uuid));
    let mut body = send.to_json();
    body["uploadUrl"] = json!(upload_url);
    Ok(Json(body))
}

async fn update_send(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    ApiJson(data): ApiJson<SendData>,
) -> JsonResult {
    let v = validate(data, false).await?;
    let send = state
        .db
        .write(move |conn| {
            let mut send = find_send(conn, &id, &auth.user.uuid)?;
            if send.atype != v.data.atype {
                err!(BadRequest, "the type of a Send cannot change");
            }
            send.akey = v.data.key;
            send.name = v.data.name;
            send.notes = v.data.notes;
            if send.atype == SEND_TYPE_TEXT {
                send.text = v.data.text;
            }
            if v.data.remove_password {
                send.password_hash = None;
            } else if v.password_hash.is_some() {
                send.password_hash = v.password_hash;
            }
            send.max_access_count = v.data.max_access_count;
            send.expiration_at = v.expiration_at;
            send.deletion_at = v.deletion_at;
            send.hide_email = v.data.hide_email;
            send.disabled = v.data.disabled;
            send.revision_date = User::next_revision(conn, &auth.user.uuid)?;
            send.save(conn)?;
            Ok(send)
        })
        .await?;
    Ok(Json(send.to_json()))
}

async fn upload_send_file(State(state): State<AppState>, auth: Auth, Path(id): Path<String>, body: Body) -> JsonResult {
    let user_uuid = auth.user.uuid.clone();
    let send = {
        let id = id.clone();
        state.db.run(move |conn| find_send(conn, &id, &user_uuid)).await?
    };
    let (Some(blob_id), Some(size)) = (send.blob_id.clone(), send.size_bytes) else {
        err!(BadRequest, "this Send has no file");
    };
    if send.uploaded {
        err!(BadRequest, "the file was already uploaded");
    }
    state.storage.store(&blob_id, body, size).await?;

    let result = state
        .db
        .write(move |conn| {
            let mut send = find_send(conn, &id, &auth.user.uuid)?;
            send.uploaded = true;
            send.revision_date = User::next_revision(conn, &auth.user.uuid)?;
            send.save(conn)?;
            Ok(send)
        })
        .await;
    // Deleted (or purged) while uploading: the blob is unreferenced now.
    if let Err(Error::NotFound(_)) = &result {
        state.storage.delete(&blob_id).await;
    }
    Ok(Json(result?.to_json()))
}

async fn delete_send(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> JsonResult {
    let blob = state
        .db
        .write(move |conn| {
            let send = find_send(conn, &id, &auth.user.uuid)?;
            let revision = User::next_revision(conn, &auth.user.uuid)?;
            Send::delete(conn, &send.uuid)?;
            Tombstone::record(conn, &auth.user.uuid, OBJECT_SEND, &send.uuid, revision)?;
            Event::new(Some(&auth.user.uuid), &auth.user.email, "send.delete", &send.uuid, &auth.ip)
                .device(&auth.device_name)
                .insert(conn)?;
            Ok(send.blob_id)
        })
        .await?;
    if let Some(blob) = blob {
        state.storage.delete(&blob).await;
    }
    Ok(Json(json!({})))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AccessData {
    password_hash: Option<String>,
}

/// Count one access, re-checking availability inside the write transaction so
/// concurrent recipients cannot exceed `maxAccessCount`.
fn record_access(conn: &mut Conn, send_uuid: &str) -> ApiResult<Send> {
    let mut send = Send::find_by_uuid(conn, send_uuid)?.ok_or_else(|| Error::NotFound("send not found".into()))?;
    if !send.is_accessible() {
        err!(NotFound, "send not found");
    }
    send.access_count += 1;
    send.revision_date = User::next_revision(conn, &send.user_uuid)?;
    send.save(conn)?;
    Ok(send)
}

async fn access_send(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Path(access_id): Path<String>,
    body: bytes::Bytes,
) -> JsonResult {
    state.login_limiter.check_ip("send", &ip)?;
    // The body is optional: recipients without a password may POST nothing.
    let data: AccessData = if body.iter().all(u8::is_ascii_whitespace) {
        AccessData::default()
    } else {
        serde_json::from_slice(&body).map_err(|e| Error::BadRequest(format!("invalid JSON: {e}")))?
    };

    let (send, owner_email) = state
        .db
        .run(move |conn| {
            let send = Send::find_by_access_id(conn, &access_id)?.filter(Send::is_accessible);
            let Some(send) = send else {
                err!(NotFound, "send not found")
            };
            let owner = User::find(conn, &send.user_uuid)?.map(|u| u.email);
            Ok((send, owner))
        })
        .await?;

    if let Some(phc) = send.password_hash.clone() {
        let Some(presented) = data.password_hash else {
            err!(Unauthorized, "password required");
        };
        if !crypto::verify_secret(presented, phc).await? {
            err!(Unauthorized, "invalid password");
        }
    }

    let creator = if send.hide_email {
        None
    } else {
        owner_email
    };
    if send.atype == SEND_TYPE_TEXT {
        let uuid = send.uuid.clone();
        let send = state.db.write(move |conn| record_access(conn, &uuid)).await?;
        return Ok(Json(send.to_public_json(creator.as_deref())));
    }
    let mut body = send.to_public_json(creator.as_deref());
    body["downloadToken"] = json!(auth::issue_send_file_token(&send.uuid));
    Ok(Json(body))
}

async fn download_send_file(
    State(state): State<AppState>,
    Path((access_id, token)): Path<(String, String)>,
) -> ApiResult<Response> {
    let send_uuid = auth::verify_send_file_token(&token)?;
    let send = state
        .db
        .write(move |conn| {
            let send = record_access(conn, &send_uuid)?;
            if send.access_id != access_id {
                err!(NotFound, "send not found");
            }
            Ok(send)
        })
        .await?;
    let Some(blob_id) = send.blob_id else {
        err!(NotFound, "send has no file")
    };
    let (body, len) = state.storage.open(&blob_id).await?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (header::CONTENT_LENGTH, len.to_string()),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        body,
    )
        .into_response())
}

/// Background job: drop Sends past their deletion date and file Sends that were
/// never uploaded within a day.
pub async fn purge_expired_sends(state: &AppState) -> ApiResult<usize> {
    let pending_before = now() - Duration::hours(24);
    let blobs = state
        .db
        .write(move |conn| {
            let expired = Send::find_expired(conn, pending_before)?;
            let mut blobs = Vec::new();
            for send in &expired {
                let revision = User::next_revision(conn, &send.user_uuid)?;
                Send::delete(conn, &send.uuid)?;
                Tombstone::record(conn, &send.user_uuid, OBJECT_SEND, &send.uuid, revision)?;
                blobs.extend(send.blob_id.clone());
            }
            Ok((expired.len(), blobs))
        })
        .await?;
    let (count, blobs) = blobs;
    state.storage.delete_many(blobs).await;
    Ok(count)
}
