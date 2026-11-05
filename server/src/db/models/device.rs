use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde_json::{Value, json};

use crate::{
    crypto,
    db::{Conn, schema::devices},
    error::ApiResult,
    util::{fmt_date, new_uuid, now},
};

pub const DEVICE_TYPES: &[&str] = &["android", "web", "desktop", "cli", "ios"];

#[derive(Debug, Clone, Queryable, Selectable, Insertable, AsChangeset, Identifiable)]
#[diesel(table_name = devices, primary_key(uuid), treat_none_as_null = true)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Device {
    pub uuid: String,
    pub user_uuid: String,
    pub identifier: String,
    pub name: String,
    pub dtype: String,
    pub app_version: String,
    pub refresh_token_hash: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub last_seen_at: NaiveDateTime,
    pub last_ip: String,
}

impl Device {
    pub fn new(user_uuid: String, identifier: String) -> Self {
        let now = now();
        Self {
            uuid: new_uuid(),
            user_uuid,
            identifier,
            name: String::new(),
            dtype: "web".into(),
            app_version: String::new(),
            refresh_token_hash: None,
            created_at: now,
            updated_at: now,
            last_seen_at: now,
            last_ip: String::new(),
        }
    }

    /// Issue a fresh refresh token; only its hash is stored.
    pub fn rotate_refresh_token(&mut self) -> String {
        let token = crypto::random_token::<48>();
        self.refresh_token_hash = Some(crypto::sha256_hex(token.as_bytes()));
        token
    }

    pub fn to_json(&self, current_device: Option<&str>) -> Value {
        json!({
            "id": self.uuid,
            "identifier": self.identifier,
            "name": self.name,
            "type": self.dtype,
            "appVersion": self.app_version,
            "creationDate": fmt_date(&self.created_at),
            "lastSeenDate": fmt_date(&self.last_seen_at),
            "lastIp": self.last_ip,
            "current": current_device == Some(self.uuid.as_str()),
        })
    }

    pub fn upsert(&mut self, conn: &mut Conn) -> ApiResult<()> {
        self.updated_at = now();
        diesel::insert_into(devices::table)
            .values(&*self)
            .on_conflict(devices::uuid)
            .do_update()
            .set(&*self)
            .execute(conn)?;
        Ok(())
    }

    pub fn find(conn: &mut Conn, uuid: &str, user_uuid: &str) -> ApiResult<Option<Self>> {
        Ok(devices::table
            .filter(devices::uuid.eq(uuid))
            .filter(devices::user_uuid.eq(user_uuid))
            .select(Device::as_select())
            .first(conn)
            .optional()?)
    }

    pub fn find_by_identifier(conn: &mut Conn, user_uuid: &str, identifier: &str) -> ApiResult<Option<Self>> {
        Ok(devices::table
            .filter(devices::user_uuid.eq(user_uuid))
            .filter(devices::identifier.eq(identifier))
            .select(Device::as_select())
            .first(conn)
            .optional()?)
    }

    pub fn find_by_refresh_token(conn: &mut Conn, token: &str) -> ApiResult<Option<Self>> {
        let hash = crypto::sha256_hex(token.as_bytes());
        Ok(devices::table
            .filter(devices::refresh_token_hash.eq(hash))
            .select(Device::as_select())
            .first(conn)
            .optional()?)
    }

    pub fn for_user(conn: &mut Conn, user_uuid: &str) -> ApiResult<Vec<Self>> {
        Ok(devices::table
            .filter(devices::user_uuid.eq(user_uuid))
            .order(devices::last_seen_at.desc())
            .select(Device::as_select())
            .load(conn)?)
    }

    pub fn count_for_user(conn: &mut Conn, user_uuid: &str) -> ApiResult<i64> {
        Ok(devices::table.filter(devices::user_uuid.eq(user_uuid)).count().get_result(conn)?)
    }

    pub fn delete(conn: &mut Conn, uuid: &str) -> ApiResult<()> {
        diesel::delete(devices::table.find(uuid)).execute(conn)?;
        Ok(())
    }

    pub fn delete_all_for_user(conn: &mut Conn, user_uuid: &str) -> ApiResult<()> {
        diesel::delete(devices::table.filter(devices::user_uuid.eq(user_uuid))).execute(conn)?;
        Ok(())
    }

    /// Sessions that have not refreshed since `cutoff` can never refresh again; drop them
    /// so they stop showing up in the device list.
    pub fn purge_idle(conn: &mut Conn, cutoff: NaiveDateTime) -> ApiResult<usize> {
        Ok(diesel::delete(devices::table.filter(devices::last_seen_at.lt(cutoff))).execute(conn)?)
    }
}
