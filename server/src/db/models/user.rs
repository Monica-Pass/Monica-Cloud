use chrono::{Duration, NaiveDateTime};
use diesel::prelude::*;
use serde_json::{Value, json};

use crate::{
    db::{
        Conn,
        schema::{attachments, sends, users},
    },
    error::ApiResult,
    util::{fmt_date, fmt_opt_date, new_uuid, now},
};

pub const ROLE_OWNER: &str = "owner";
pub const ROLE_ADMIN: &str = "admin";
pub const ROLE_USER: &str = "user";

#[derive(Debug, Clone, Queryable, Selectable, Insertable, AsChangeset, Identifiable)]
#[diesel(table_name = users, primary_key(uuid), treat_none_as_null = true)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct User {
    pub uuid: String,
    pub email: String,
    pub name: String,
    pub role: String,
    pub enabled: bool,
    pub password_hash: String,
    pub kdf_type: i32,
    pub kdf_iterations: i32,
    pub akey: String,
    pub security_stamp: String,
    pub revision_date: NaiveDateTime,
    pub quota_bytes: Option<i64>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub last_login_at: Option<NaiveDateTime>,
}

impl User {
    /// `password_hash` is the Argon2id PHC string of the client's authHash ([`crate::crypto::hash_secret`]).
    pub fn new(
        email: String,
        name: String,
        password_hash: String,
        kdf_type: i32,
        kdf_iterations: i32,
        akey: String,
    ) -> Self {
        let now = now();
        Self {
            uuid: new_uuid(),
            email,
            name,
            role: ROLE_USER.into(),
            enabled: true,
            password_hash,
            kdf_type,
            kdf_iterations,
            akey,
            security_stamp: new_uuid(),
            revision_date: now,
            quota_bytes: None,
            created_at: now,
            updated_at: now,
            last_login_at: None,
        }
    }

    pub fn is_admin(&self) -> bool {
        self.role == ROLE_OWNER || self.role == ROLE_ADMIN
    }

    /// Invalidates every access token issued so far.
    pub fn rotate_security_stamp(&mut self) {
        self.security_stamp = new_uuid();
    }

    pub fn profile_json(&self) -> Value {
        json!({
            "id": self.uuid,
            "email": self.email,
            "name": self.name,
            "role": self.role,
            "kdf": self.kdf_type,
            "kdfIterations": self.kdf_iterations,
            "creationDate": fmt_date(&self.created_at),
            "revisionDate": fmt_date(&self.revision_date),
            "lastLoginDate": fmt_opt_date(&self.last_login_at),
        })
    }

    pub fn account_json(&self) -> Value {
        json!({ "id": self.uuid, "email": self.email, "name": self.name, "role": self.role })
    }

    pub fn insert(&self, conn: &mut Conn) -> ApiResult<()> {
        diesel::insert_into(users::table).values(self).execute(conn)?;
        Ok(())
    }

    /// Writes every column. Only call this on a row read inside the same write
    /// transaction; a copy loaded earlier (e.g. by the `Auth` guard) would roll back
    /// concurrent changes such as a newer `revision_date` or an admin disabling the account.
    pub fn save(&mut self, conn: &mut Conn) -> ApiResult<()> {
        self.updated_at = now();
        diesel::update(users::table.find(&self.uuid)).set(&*self).execute(conn)?;
        Ok(())
    }

    /// Records a successful login without touching any other column.
    pub fn touch_login(conn: &mut Conn, uuid: &str) -> ApiResult<NaiveDateTime> {
        let now = now();
        diesel::update(users::table.find(uuid))
            .set((users::last_login_at.eq(Some(now)), users::updated_at.eq(now)))
            .execute(conn)?;
        Ok(now)
    }

    /// Changes the display name without touching any other column.
    pub fn set_name(conn: &mut Conn, uuid: &str, name: &str) -> ApiResult<()> {
        diesel::update(users::table.find(uuid))
            .set((users::name.eq(name), users::updated_at.eq(now())))
            .execute(conn)?;
        Ok(())
    }

    pub fn find(conn: &mut Conn, uuid: &str) -> ApiResult<Option<Self>> {
        Ok(users::table.find(uuid).select(User::as_select()).first(conn).optional()?)
    }

    pub fn find_by_email(conn: &mut Conn, email: &str) -> ApiResult<Option<Self>> {
        Ok(users::table.filter(users::email.eq(email)).select(User::as_select()).first(conn).optional()?)
    }

    pub fn all(conn: &mut Conn) -> ApiResult<Vec<Self>> {
        Ok(users::table.order(users::created_at.asc()).select(User::as_select()).load(conn)?)
    }

    pub fn count(conn: &mut Conn) -> ApiResult<i64> {
        Ok(users::table.count().get_result(conn)?)
    }

    pub fn count_owners(conn: &mut Conn) -> ApiResult<i64> {
        Ok(users::table.filter(users::role.eq(ROLE_OWNER)).count().get_result(conn)?)
    }

    pub fn delete(conn: &mut Conn, uuid: &str) -> ApiResult<()> {
        diesel::delete(users::table.find(uuid)).execute(conn)?;
        Ok(())
    }

    /// Allocate the next account revision. Revisions are strictly increasing per
    /// user, and every object written in one transaction shares the same value,
    /// which is what makes `GET /sync?since=` lossless.
    pub fn next_revision(conn: &mut Conn, uuid: &str) -> ApiResult<NaiveDateTime> {
        let previous: NaiveDateTime = users::table.find(uuid).select(users::revision_date).first(conn)?;
        let mut next = now();
        if next <= previous {
            next = previous + Duration::microseconds(1);
        }
        diesel::update(users::table.find(uuid)).set(users::revision_date.eq(next)).execute(conn)?;
        Ok(next)
    }

    /// Bytes of uploaded blobs (attachments + Send files) owned by the user.
    pub fn storage_used(conn: &mut Conn, uuid: &str) -> ApiResult<i64> {
        use diesel::{dsl::sql, sql_types::BigInt};
        // SUM() over BIGINT maps to Numeric in diesel; COALESCE keeps it an integer on SQLite.
        let attachments: i64 = attachments::table
            .filter(attachments::user_uuid.eq(uuid))
            .select(sql::<BigInt>("COALESCE(SUM(size), 0)"))
            .first(conn)?;
        let sends: i64 = sends::table
            .filter(sends::user_uuid.eq(uuid))
            .select(sql::<BigInt>("COALESCE(SUM(size_bytes), 0)"))
            .first(conn)?;
        Ok(attachments + sends)
    }

    /// Effective quota: per-user override, else the server default. `None` = unlimited.
    pub fn effective_quota(&self) -> Option<i64> {
        self.quota_bytes.or(crate::config::RuntimeSettings::current().default_quota_bytes)
    }
}
