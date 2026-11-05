use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde_json::{Value, json};

use crate::{
    crypto,
    db::{Conn, schema::sends},
    error::ApiResult,
    util::{fmt_date, fmt_opt_date, new_uuid, now},
};

pub const SEND_TYPE_TEXT: i32 = 0;
pub const SEND_TYPE_FILE: i32 = 1;

#[derive(Debug, Clone, Queryable, Selectable, Insertable, AsChangeset, Identifiable)]
#[diesel(table_name = sends, primary_key(uuid), treat_none_as_null = true)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Send {
    pub uuid: String,
    pub user_uuid: String,
    pub access_id: String,
    pub atype: i32,
    pub akey: String,
    pub name: String,
    pub notes: Option<String>,
    pub text: Option<String>,
    pub file_name: Option<String>,
    pub size_bytes: Option<i64>,
    pub blob_id: Option<String>,
    pub uploaded: bool,
    pub password_hash: Option<String>,
    pub max_access_count: Option<i32>,
    pub access_count: i32,
    pub disabled: bool,
    pub hide_email: bool,
    pub expiration_at: Option<NaiveDateTime>,
    pub deletion_at: NaiveDateTime,
    pub created_at: NaiveDateTime,
    pub revision_date: NaiveDateTime,
}

impl Send {
    pub fn new(user_uuid: String, atype: i32, deletion_at: NaiveDateTime, revision: NaiveDateTime) -> Self {
        Self {
            uuid: new_uuid(),
            user_uuid,
            access_id: crypto::random_token::<16>(),
            atype,
            akey: String::new(),
            name: String::new(),
            notes: None,
            text: None,
            file_name: None,
            size_bytes: None,
            blob_id: None,
            // Text sends are complete on creation; file sends wait for their upload.
            uploaded: atype == SEND_TYPE_TEXT,
            password_hash: None,
            max_access_count: None,
            access_count: 0,
            disabled: false,
            hide_email: false,
            expiration_at: None,
            deletion_at,
            created_at: now(),
            revision_date: revision,
        }
    }

    /// Owner view: everything the owner's client needs to re-open the Send.
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.uuid,
            "accessId": self.access_id,
            "type": self.atype,
            "key": self.akey,
            "name": self.name,
            "notes": self.notes,
            "text": self.text,
            "fileName": self.file_name,
            "size": self.size_bytes,
            "uploaded": self.uploaded,
            "passwordProtected": self.password_hash.is_some(),
            "maxAccessCount": self.max_access_count,
            "accessCount": self.access_count,
            "disabled": self.disabled,
            "hideEmail": self.hide_email,
            "expirationDate": fmt_opt_date(&self.expiration_at),
            "deletionDate": fmt_date(&self.deletion_at),
            "creationDate": fmt_date(&self.created_at),
            "revisionDate": fmt_date(&self.revision_date),
        })
    }

    /// Recipient view: no owner notes, no key, no counters.
    pub fn to_public_json(&self, creator: Option<&str>) -> Value {
        json!({
            "id": self.uuid,
            "accessId": self.access_id,
            "type": self.atype,
            "name": self.name,
            "text": self.text,
            "fileName": self.file_name,
            "size": self.size_bytes,
            "expirationDate": fmt_opt_date(&self.expiration_at),
            "creatorIdentifier": creator,
        })
    }

    /// Whether a recipient may open the Send right now.
    pub fn is_accessible(&self) -> bool {
        let now = now();
        !self.disabled
            && self.uploaded
            && self.deletion_at > now
            && self.expiration_at.is_none_or(|exp| exp > now)
            && self.max_access_count.is_none_or(|max| self.access_count < max)
    }

    pub fn insert(&self, conn: &mut Conn) -> ApiResult<()> {
        diesel::insert_into(sends::table).values(self).execute(conn)?;
        Ok(())
    }

    pub fn save(&self, conn: &mut Conn) -> ApiResult<()> {
        diesel::update(sends::table.find(&self.uuid)).set(self).execute(conn)?;
        Ok(())
    }

    pub fn delete(conn: &mut Conn, uuid: &str) -> ApiResult<()> {
        diesel::delete(sends::table.find(uuid)).execute(conn)?;
        Ok(())
    }

    pub fn find(conn: &mut Conn, uuid: &str, user_uuid: &str) -> ApiResult<Option<Self>> {
        Ok(sends::table
            .filter(sends::uuid.eq(uuid))
            .filter(sends::user_uuid.eq(user_uuid))
            .select(Send::as_select())
            .first(conn)
            .optional()?)
    }

    pub fn find_by_uuid(conn: &mut Conn, uuid: &str) -> ApiResult<Option<Self>> {
        Ok(sends::table.find(uuid).select(Send::as_select()).first(conn).optional()?)
    }

    pub fn find_by_access_id(conn: &mut Conn, access_id: &str) -> ApiResult<Option<Self>> {
        Ok(sends::table.filter(sends::access_id.eq(access_id)).select(Send::as_select()).first(conn).optional()?)
    }

    pub fn changed_since(conn: &mut Conn, user_uuid: &str, since: Option<NaiveDateTime>) -> ApiResult<Vec<Self>> {
        let mut query = sends::table.filter(sends::user_uuid.eq(user_uuid)).into_boxed();
        if let Some(since) = since {
            query = query.filter(sends::revision_date.gt(since));
        }
        Ok(query.order(sends::created_at.desc()).select(Send::as_select()).load(conn)?)
    }

    pub fn blob_ids_for_user(conn: &mut Conn, user_uuid: &str) -> ApiResult<Vec<String>> {
        let ids: Vec<Option<String>> =
            sends::table.filter(sends::user_uuid.eq(user_uuid)).select(sends::blob_id).load(conn)?;
        Ok(ids.into_iter().flatten().collect())
    }

    /// Sends past their deletion date, plus file Sends whose upload never finished
    /// (created before `pending_before`); those would otherwise hold quota until deletion.
    pub fn find_expired(conn: &mut Conn, pending_before: NaiveDateTime) -> ApiResult<Vec<Self>> {
        Ok(sends::table
            .filter(
                sends::deletion_at.le(now()).or(sends::uploaded.eq(false).and(sends::created_at.lt(pending_before))),
            )
            .select(Send::as_select())
            .load(conn)?)
    }

    pub fn count_all(conn: &mut Conn) -> ApiResult<i64> {
        Ok(sends::table.count().get_result(conn)?)
    }
}
