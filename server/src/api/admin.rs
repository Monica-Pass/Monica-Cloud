//! Instance administration for owner/admin accounts (vaultwarden's `/admin`).
//! Unlike vaultwarden there is no separate ADMIN_TOKEN: the first account is the
//! owner and can promote others.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};

use super::{AppState, core::EventQuery};
use crate::{
    CONFIG,
    auth::AdminAuth,
    config::{RuntimeSettings, VERSION},
    db::{
        Conn,
        models::{Attachment, Device, Event, Item, ROLE_ADMIN, ROLE_OWNER, ROLE_USER, Send, ServerSettings, User},
    },
    err,
    error::{ApiJson, ApiResult, Error, JsonResult},
    util::fmt_date,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/users", get(list_users))
        .route("/admin/users/{id}", axum::routing::patch(update_user).delete(delete_user))
        .route("/admin/users/{id}/deauth", post(deauth_user))
        .route("/admin/diagnostics", get(diagnostics))
        .route("/admin/settings", get(get_settings).put(put_settings))
        .route("/admin/events", get(all_events))
}

fn user_row(conn: &mut Conn, user: &User) -> ApiResult<Value> {
    let mut row = user.profile_json();
    row["enabled"] = json!(user.enabled);
    row["quotaBytes"] = json!(user.quota_bytes);
    row["effectiveQuotaBytes"] = json!(user.effective_quota());
    row["storageUsedBytes"] = json!(User::storage_used(conn, &user.uuid)?);
    row["itemCount"] = json!(Item::count_for_user(conn, &user.uuid)?);
    row["deviceCount"] = json!(Device::count_for_user(conn, &user.uuid)?);
    Ok(row)
}

async fn list_users(State(state): State<AppState>, _admin: AdminAuth) -> JsonResult {
    let rows = state
        .db
        .read(|conn| {
            let users = User::all(conn)?;
            users.iter().map(|u| user_row(conn, u)).collect::<ApiResult<Vec<_>>>()
        })
        .await?;
    Ok(Json(json!({ "data": rows })))
}

/// Distinguishes "absent" (`None`) from explicit `null` (`Some(None)`).
fn double_option<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserUpdate {
    enabled: Option<bool>,
    role: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    quota_bytes: Option<Option<i64>>,
}

fn find_user(conn: &mut Conn, id: &str) -> ApiResult<User> {
    User::find(conn, id)?.ok_or_else(|| Error::NotFound("user not found".into()))
}

async fn update_user(
    State(state): State<AppState>,
    AdminAuth(admin): AdminAuth,
    Path(id): Path<String>,
    ApiJson(data): ApiJson<UserUpdate>,
) -> JsonResult {
    let row = state
        .db
        .write(move |conn| {
            let mut user = find_user(conn, &id)?;
            let is_self = user.uuid == admin.user.uuid;

            if let Some(role) = data.role.as_deref() {
                if ![ROLE_OWNER, ROLE_ADMIN, ROLE_USER].contains(&role) {
                    err!(BadRequest, "role must be owner, admin or user");
                }
                if admin.user.role != ROLE_OWNER {
                    err!(Forbidden, "only owners can change roles");
                }
                if user.role == ROLE_OWNER && role != ROLE_OWNER && User::count_owners(conn)? == 1 {
                    err!(BadRequest, "the server needs at least one owner");
                }
                user.role = role.into();
            }
            if user.role == ROLE_OWNER
                && admin.user.role != ROLE_OWNER
                && (data.enabled.is_some() || data.quota_bytes.is_some())
            {
                err!(Forbidden, "only owners can modify an owner account");
            }
            if let Some(enabled) = data.enabled {
                if is_self && !enabled {
                    err!(BadRequest, "you cannot disable your own account");
                }
                if user.enabled && !enabled {
                    user.rotate_security_stamp();
                    Device::delete_all_for_user(conn, &user.uuid)?;
                }
                user.enabled = enabled;
            }
            if let Some(quota) = data.quota_bytes {
                if quota.is_some_and(|q| q < 0) {
                    err!(BadRequest, "quotaBytes cannot be negative");
                }
                user.quota_bytes = quota;
            }
            user.save(conn)?;
            Event::new(Some(&admin.user.uuid), &admin.user.email, "admin.user_update", &user.email, &admin.ip)
                .insert(conn)?;
            user_row(conn, &user)
        })
        .await?;
    Ok(Json(row))
}

async fn deauth_user(State(state): State<AppState>, AdminAuth(admin): AdminAuth, Path(id): Path<String>) -> JsonResult {
    state
        .db
        .write(move |conn| {
            let mut user = find_user(conn, &id)?;
            // Same rule as disabling or deleting: an admin cannot act on an owner account.
            if user.role == ROLE_OWNER && admin.user.role != ROLE_OWNER {
                err!(Forbidden, "only owners can sign out an owner account");
            }
            user.rotate_security_stamp();
            user.save(conn)?;
            Device::delete_all_for_user(conn, &user.uuid)?;
            Event::new(Some(&admin.user.uuid), &admin.user.email, "admin.user_deauth", &user.email, &admin.ip)
                .insert(conn)?;
            Ok(())
        })
        .await?;
    Ok(Json(json!({})))
}

async fn delete_user(State(state): State<AppState>, AdminAuth(admin): AdminAuth, Path(id): Path<String>) -> JsonResult {
    let blobs = state
        .db
        .write(move |conn| {
            let user = find_user(conn, &id)?;
            if user.uuid == admin.user.uuid {
                err!(BadRequest, "delete your own account from the account settings instead");
            }
            if user.role == ROLE_OWNER && admin.user.role != ROLE_OWNER {
                err!(Forbidden, "only owners can delete an owner account");
            }
            let mut blobs = Attachment::blob_ids_for_user(conn, &user.uuid)?;
            blobs.extend(Send::blob_ids_for_user(conn, &user.uuid)?);
            User::delete(conn, &user.uuid)?;
            Event::new(Some(&admin.user.uuid), &admin.user.email, "admin.user_delete", &user.email, &admin.ip)
                .insert(conn)?;
            Ok(blobs)
        })
        .await?;
    state.storage.delete_many(blobs).await;
    Ok(Json(json!({})))
}

fn file_size(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

async fn diagnostics(State(state): State<AppState>, _admin: AdminAuth) -> JsonResult {
    let (users, items, sends, (attachments, attachment_bytes)) = state
        .db
        .read(|conn| {
            Ok((User::count(conn)?, Item::count_all(conn)?, Send::count_all(conn)?, Attachment::count_all(conn)?))
        })
        .await?;
    let db_path = std::path::PathBuf::from(&CONFIG.database_url);
    let db_bytes = file_size(&db_path) + file_size(&db_path.with_extension("sqlite3-wal"));
    let web_vault_index = CONFIG.web_vault_folder.join("index.html");

    Ok(Json(json!({
        "version": VERSION,
        "uptimeSeconds": state.started_at.elapsed().as_secs(),
        "serverTime": fmt_date(&crate::util::now()),
        "database": { "backend": "sqlite", "path": CONFIG.database_url, "sizeBytes": db_bytes },
        "storage": {
            "dataFolder": CONFIG.data_folder.to_string_lossy(),
            "attachments": attachments,
            "attachmentBytes": attachment_bytes,
        },
        "counts": { "users": users, "items": items, "sends": sends },
        "webVault": {
            "enabled": CONFIG.web_vault_enabled,
            "folder": CONFIG.web_vault_folder.to_string_lossy(),
            "present": web_vault_index.exists(),
        },
        "config": {
            "domain": CONFIG.domain,
            "ipHeader": CONFIG.ip_header,
            "attachmentMaxMb": CONFIG.attachment_max_mb,
            "kdfIterationsDefault": CONFIG.kdf_iterations_default,
            "accessTokenTtlSecs": CONFIG.access_token_ttl_secs,
            "refreshTokenDays": CONFIG.refresh_token_days,
            "tombstoneRetentionDays": CONFIG.tombstone_retention_days,
            "signupsDomainsWhitelist": CONFIG.signups_domains_whitelist,
        },
    })))
}

async fn get_settings(_admin: AdminAuth) -> JsonResult {
    Ok(Json(serde_json::to_value(RuntimeSettings::current())?))
}

async fn put_settings(
    State(state): State<AppState>,
    AdminAuth(admin): AdminAuth,
    ApiJson(settings): ApiJson<RuntimeSettings>,
) -> JsonResult {
    if settings.event_retention_days < 1 {
        err!(BadRequest, "eventRetentionDays must be at least 1");
    }
    if settings.default_quota_bytes.is_some_and(|q| q < 0) {
        err!(BadRequest, "defaultQuotaBytes cannot be negative");
    }
    let stored = settings.clone();
    state
        .db
        .write(move |conn| {
            ServerSettings::store(conn, &stored)?;
            Event::new(Some(&admin.user.uuid), &admin.user.email, "admin.settings_update", "", &admin.ip)
                .insert(conn)?;
            Ok(())
        })
        .await?;
    RuntimeSettings::replace(settings.clone());
    Ok(Json(serde_json::to_value(settings)?))
}

async fn all_events(State(state): State<AppState>, _admin: AdminAuth, Query(query): Query<EventQuery>) -> JsonResult {
    let (limit, before) = query.parse()?;
    let events = state.db.run(move |conn| Event::page(conn, None, before, limit)).await?;
    Ok(Json(json!({ "data": events.iter().map(Event::to_json).collect::<Vec<_>>() })))
}
