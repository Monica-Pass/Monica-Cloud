//! Encrypted attachments: register, upload, download, delete (protocol §4.5).

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::header,
    response::{IntoResponse, Response},
    routing::post,
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    CONFIG,
    api::AppState,
    auth::Auth,
    db::{
        Conn,
        models::{Attachment, Event, Item, User},
    },
    err,
    error::{ApiJson, ApiResult, Error, JsonResult},
    util::is_enc_string,
};

/// Smallest possible chunked ciphertext: 8-byte header + one empty final chunk (12 + 16).
pub const MIN_ENCRYPTED_SIZE: i64 = 36;

pub fn routes() -> Router<AppState> {
    Router::new().route("/items/{id}/attachments", post(register_attachment)).route(
        "/items/{id}/attachments/{aid}",
        axum::routing::put(upload_attachment).get(download_attachment).delete(delete_attachment),
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttachmentData {
    file_name: String,
    key: String,
    size: i64,
}

/// Fail when `extra` more bytes would exceed the user's quota.
pub fn check_quota(conn: &mut Conn, user: &User, extra: i64) -> ApiResult<()> {
    if let Some(quota) = user.effective_quota() {
        let used = User::storage_used(conn, &user.uuid)?;
        if used.saturating_add(extra) > quota {
            err!(PayloadTooLarge, "storage quota exceeded");
        }
    }
    Ok(())
}

fn find_owned(conn: &mut Conn, item_id: &str, attachment_id: &str, user_uuid: &str) -> ApiResult<(Item, Attachment)> {
    let item = Item::find(conn, item_id, user_uuid)?.ok_or_else(|| Error::NotFound("item not found".into()))?;
    let attachment = Attachment::find(conn, attachment_id, &item.uuid)?
        .ok_or_else(|| Error::NotFound("attachment not found".into()))?;
    Ok((item, attachment))
}

async fn register_attachment(
    State(state): State<AppState>,
    auth: Auth,
    Path(item_id): Path<String>,
    ApiJson(data): ApiJson<AttachmentData>,
) -> JsonResult {
    if !is_enc_string(&data.file_name, 4096) || !is_enc_string(&data.key, 512) {
        err!(BadRequest, "fileName and key must be v1 EncStrings");
    }
    if data.size < MIN_ENCRYPTED_SIZE {
        err!(BadRequest, "size is smaller than any valid encrypted file");
    }
    if data.size > CONFIG.attachment_max_bytes() {
        err!(PayloadTooLarge, format!("attachments are limited to {} MiB", CONFIG.attachment_max_mb));
    }
    let attachment = state
        .db
        .write(move |conn| {
            let item =
                Item::find(conn, &item_id, &auth.user.uuid)?.ok_or_else(|| Error::NotFound("item not found".into()))?;
            check_quota(conn, &auth.user, data.size)?;
            let attachment = Attachment::new(item.uuid, auth.user.uuid.clone(), data.file_name, data.key, data.size);
            attachment.insert(conn)?;
            Ok(attachment)
        })
        .await?;
    Ok(Json(json!({
        "id": attachment.uuid,
        "uploadUrl": format!("/api/v1/items/{}/attachments/{}", attachment.item_uuid, attachment.uuid),
        "attachment": attachment.to_json(),
    })))
}

async fn upload_attachment(
    State(state): State<AppState>,
    auth: Auth,
    Path((item_id, attachment_id)): Path<(String, String)>,
    body: Body,
) -> JsonResult {
    let user_uuid = auth.user.uuid.clone();
    let (ids, attachment) = {
        let (item_id, attachment_id) = (item_id.clone(), attachment_id.clone());
        state
            .db
            .run(move |conn| {
                let (_, attachment) = find_owned(conn, &item_id, &attachment_id, &user_uuid)?;
                Ok(((item_id, attachment_id), attachment))
            })
            .await?
    };
    if attachment.uploaded {
        err!(BadRequest, "attachment content was already uploaded");
    }

    state.storage.store(&attachment.blob_id, body, attachment.size).await?;

    let result = state
        .db
        .write(move |conn| {
            let (mut item, attachment) = find_owned(conn, &ids.0, &ids.1, &auth.user.uuid)?;
            Attachment::mark_uploaded(conn, &attachment.uuid)?;
            item.revision_date = User::next_revision(conn, &auth.user.uuid)?;
            item.save(conn)?;
            Event::new(Some(&auth.user.uuid), &auth.user.email, "attachment.upload", &attachment.uuid, &auth.ip)
                .device(&auth.device_name)
                .insert(conn)?;
            item.to_json_full(conn)
        })
        .await;
    // The item or attachment was deleted while the bytes were streaming in; nothing
    // references the blob any more, so do not leave it on disk.
    if let Err(Error::NotFound(_)) = &result {
        state.storage.delete(&attachment.blob_id).await;
    }
    Ok(Json(result?))
}

async fn download_attachment(
    State(state): State<AppState>,
    auth: Auth,
    Path((item_id, attachment_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let user_uuid = auth.user.uuid;
    let attachment = state.db.run(move |conn| Ok(find_owned(conn, &item_id, &attachment_id, &user_uuid)?.1)).await?;
    if !attachment.uploaded {
        err!(NotFound, "attachment content was not uploaded yet");
    }
    let (body, len) = state.storage.open(&attachment.blob_id).await?;
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

async fn delete_attachment(
    State(state): State<AppState>,
    auth: Auth,
    Path((item_id, attachment_id)): Path<(String, String)>,
) -> JsonResult {
    let (item, blob_id) = state
        .db
        .write(move |conn| {
            let (mut item, attachment) = find_owned(conn, &item_id, &attachment_id, &auth.user.uuid)?;
            Attachment::delete(conn, &attachment.uuid)?;
            item.revision_date = User::next_revision(conn, &auth.user.uuid)?;
            item.save(conn)?;
            Event::new(Some(&auth.user.uuid), &auth.user.email, "attachment.delete", &attachment.uuid, &auth.ip)
                .device(&auth.device_name)
                .insert(conn)?;
            Ok((item.to_json_full(conn)?, attachment.blob_id))
        })
        .await?;
    state.storage.delete(&blob_id).await;
    Ok(Json(item))
}
