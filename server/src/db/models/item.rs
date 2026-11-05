use std::collections::HashMap;

use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde_json::{Value, json};

use super::Attachment;
use crate::{
    db::{
        Conn,
        schema::{attachments, items},
    },
    error::ApiResult,
    util::{fmt_date, fmt_opt_date, new_uuid, now},
};

/// Item types from protocol §2. The server only uses them for counting.
pub const ITEM_TYPE_MIN: i32 = 1;
pub const ITEM_TYPE_MAX: i32 = 64;

#[derive(Debug, Clone, Queryable, Selectable, Insertable, AsChangeset, Identifiable)]
#[diesel(table_name = items, primary_key(uuid), treat_none_as_null = true)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Item {
    pub uuid: String,
    pub user_uuid: String,
    pub folder_uuid: Option<String>,
    pub atype: i32,
    pub favorite: bool,
    pub akey: String,
    pub data: String,
    pub created_at: NaiveDateTime,
    pub revision_date: NaiveDateTime,
    pub deleted_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
}

impl Item {
    pub fn new(user_uuid: String, revision: NaiveDateTime) -> Self {
        Self {
            uuid: new_uuid(),
            user_uuid,
            folder_uuid: None,
            atype: 1,
            favorite: false,
            akey: String::new(),
            data: String::new(),
            created_at: now(),
            revision_date: revision,
            deleted_at: None,
            archived_at: None,
        }
    }

    pub fn to_json(&self, attachments: &[Attachment]) -> Value {
        json!({
            "id": self.uuid,
            "type": self.atype,
            "folderId": self.folder_uuid,
            "favorite": self.favorite,
            "key": self.akey,
            "data": self.data,
            "attachments": attachments.iter().map(Attachment::to_json).collect::<Vec<_>>(),
            "creationDate": fmt_date(&self.created_at),
            "revisionDate": fmt_date(&self.revision_date),
            "deletedDate": fmt_opt_date(&self.deleted_at),
            "archivedDate": fmt_opt_date(&self.archived_at),
        })
    }

    /// JSON including the item's uploaded attachments.
    pub fn to_json_full(&self, conn: &mut Conn) -> ApiResult<Value> {
        let attachments = Attachment::uploaded_for_item(conn, &self.uuid)?;
        Ok(self.to_json(&attachments))
    }

    pub fn insert(&self, conn: &mut Conn) -> ApiResult<()> {
        diesel::insert_into(items::table).values(self).execute(conn)?;
        Ok(())
    }

    pub fn save(&self, conn: &mut Conn) -> ApiResult<()> {
        diesel::update(items::table.find(&self.uuid)).set(self).execute(conn)?;
        Ok(())
    }

    pub fn delete(conn: &mut Conn, uuid: &str) -> ApiResult<()> {
        diesel::delete(items::table.find(uuid)).execute(conn)?;
        Ok(())
    }

    pub fn find(conn: &mut Conn, uuid: &str, user_uuid: &str) -> ApiResult<Option<Self>> {
        Ok(items::table
            .filter(items::uuid.eq(uuid))
            .filter(items::user_uuid.eq(user_uuid))
            .select(Item::as_select())
            .first(conn)
            .optional()?)
    }

    pub fn find_many(conn: &mut Conn, uuids: &[String], user_uuid: &str) -> ApiResult<Vec<Self>> {
        Ok(items::table
            .filter(items::uuid.eq_any(uuids))
            .filter(items::user_uuid.eq(user_uuid))
            .select(Item::as_select())
            .load(conn)?)
    }

    pub fn in_folder(conn: &mut Conn, folder_uuid: &str) -> ApiResult<Vec<Self>> {
        Ok(items::table.filter(items::folder_uuid.eq(folder_uuid)).select(Item::as_select()).load(conn)?)
    }

    pub fn count_for_user(conn: &mut Conn, user_uuid: &str) -> ApiResult<i64> {
        Ok(items::table.filter(items::user_uuid.eq(user_uuid)).count().get_result(conn)?)
    }

    pub fn count_all(conn: &mut Conn) -> ApiResult<i64> {
        Ok(items::table.count().get_result(conn)?)
    }

    /// Changed items with their uploaded attachments, in one extra query.
    pub fn changed_since_json(conn: &mut Conn, user_uuid: &str, since: Option<NaiveDateTime>) -> ApiResult<Vec<Value>> {
        let mut query = items::table.filter(items::user_uuid.eq(user_uuid)).into_boxed();
        if let Some(since) = since {
            query = query.filter(items::revision_date.gt(since));
        }
        let rows: Vec<Item> = query.order(items::created_at.asc()).select(Item::as_select()).load(conn)?;

        let mut attachment_query = attachments::table
            .filter(attachments::user_uuid.eq(user_uuid))
            .filter(attachments::uploaded.eq(true))
            .into_boxed();
        if let Some(since) = since {
            // A subquery instead of binding every changed id: SQLite caps bound parameters
            // (32766), and a large import or folder deletion can change more items than that.
            let changed = items::table
                .filter(items::user_uuid.eq(user_uuid))
                .filter(items::revision_date.gt(since))
                .select(items::uuid);
            attachment_query = attachment_query.filter(attachments::item_uuid.eq_any(changed));
        }
        let mut by_item: HashMap<String, Vec<Attachment>> = HashMap::new();
        for attachment in attachment_query.select(Attachment::as_select()).load(conn)? {
            by_item.entry(attachment.item_uuid.clone()).or_default().push(attachment);
        }

        Ok(rows.iter().map(|item| item.to_json(by_item.get(&item.uuid).map(Vec::as_slice).unwrap_or(&[]))).collect())
    }

    /// Item counts per type for the dashboard (active, trashed excluded).
    pub fn type_counts(conn: &mut Conn, user_uuid: &str) -> ApiResult<Vec<(i32, i64)>> {
        use diesel::dsl::count_star;
        Ok(items::table
            .filter(items::user_uuid.eq(user_uuid))
            .filter(items::deleted_at.is_null())
            .group_by(items::atype)
            .select((items::atype, count_star()))
            .load(conn)?)
    }
}
