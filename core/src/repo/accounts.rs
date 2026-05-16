use crate::error::{Error, Result};
use crate::repo::common;
use crate::vault::Vault;
use chrono::{DateTime, Utc};
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: i64,
    pub site_id: i64,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub alias: Option<String>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct NewAccount {
    pub site_id: i64,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub alias: Option<String>,
    pub notes: Option<String>,
}

pub fn create(vault: &Vault, new: NewAccount) -> Result<Account> {
    if new.site_id <= 0 {
        return Err(Error::InvalidInput("site_id required".into()));
    }
    let now = Utc::now();
    vault.conn().execute(
        "INSERT INTO accounts (site_id, username, display_name, alias, notes, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![new.site_id, new.username, new.display_name, new.alias, new.notes, now.to_rfc3339()],
    )?;
    let id = vault.conn().last_insert_rowid();
    Ok(Account {
        id,
        site_id: new.site_id,
        username: new.username,
        display_name: new.display_name,
        alias: new.alias,
        notes: new.notes,
        created_at: now,
    })
}

pub fn get(vault: &Vault, id: i64) -> Result<Account> {
    vault.conn().query_row(
        "SELECT id, site_id, username, display_name, alias, notes, created_at FROM accounts WHERE id = ?1",
        params![id],
        row_to_account,
    ).map_err(common::not_found_or_db)
}

/// Fields to update on an existing account. Each `Option<String>` is the new value
/// (including `None` meaning NULL / clear the field). All four fields are always written.
#[derive(Debug, Clone, Default)]
pub struct UpdateAccount {
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub alias: Option<String>,
    pub notes: Option<String>,
}

pub fn update(vault: &Vault, id: i64, changes: UpdateAccount) -> Result<()> {
    let n = vault.conn().execute(
        "UPDATE accounts SET username = ?1, display_name = ?2, alias = ?3, notes = ?4
         WHERE id = ?5",
        params![changes.username, changes.display_name, changes.alias, changes.notes, id],
    )?;
    common::ensure_affected(n)
}

pub fn delete(vault: &Vault, id: i64) -> Result<()> {
    let n = vault.conn().execute(
        "DELETE FROM accounts WHERE id = ?1",
        params![id],
    )?;
    common::ensure_affected(n)
}

pub fn list_all(vault: &Vault, tag_ids: &[i64]) -> Result<Vec<Account>> {
    if tag_ids.is_empty() {
        let mut stmt = vault.conn().prepare(
            "SELECT id, site_id, username, display_name, alias, notes, created_at
             FROM accounts ORDER BY id",
        )?;
        let rows = stmt
            .query_map([], row_to_account)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        return Ok(rows);
    }
    let placeholders: String = tag_ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT a.id, a.site_id, a.username, a.display_name, a.alias, a.notes, a.created_at
         FROM accounts a
         JOIN account_tags at ON at.account_id = a.id
         WHERE at.tag_id IN ({})
         GROUP BY a.id
         HAVING COUNT(DISTINCT at.tag_id) = ?{}
         ORDER BY a.id",
        placeholders,
        tag_ids.len() + 1,
    );
    let mut stmt = vault.conn().prepare(&sql)?;
    let mut params_vec: Vec<&dyn rusqlite::ToSql> =
        tag_ids.iter().map(|x| x as &dyn rusqlite::ToSql).collect();
    let len_bind: i64 = tag_ids.len() as i64;
    params_vec.push(&len_bind);
    let rows = stmt
        .query_map(params_vec.as_slice(), row_to_account)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn list_for_site(vault: &Vault, site_id: i64, tag_ids: &[i64]) -> Result<Vec<Account>> {
    if tag_ids.is_empty() {
        let mut stmt = vault.conn().prepare(
            "SELECT id, site_id, username, display_name, alias, notes, created_at FROM accounts
             WHERE site_id = ?1 ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map(params![site_id], row_to_account)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        return Ok(rows);
    }
    let placeholders: String = tag_ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let n = tag_ids.len();
    let sql = format!(
        "SELECT a.id, a.site_id, a.username, a.display_name, a.alias, a.notes, a.created_at
         FROM accounts a
         JOIN account_tags at ON at.account_id = a.id
         WHERE at.tag_id IN ({})
           AND a.site_id = ?{}
         GROUP BY a.id
         HAVING COUNT(DISTINCT at.tag_id) = ?{}
         ORDER BY a.id",
        placeholders,
        n + 1,
        n + 2,
    );
    let mut stmt = vault.conn().prepare(&sql)?;
    let mut params_vec: Vec<&dyn rusqlite::ToSql> =
        tag_ids.iter().map(|x| x as &dyn rusqlite::ToSql).collect();
    let len_bind: i64 = n as i64;
    params_vec.push(&site_id);
    params_vec.push(&len_bind);
    let rows = stmt
        .query_map(params_vec.as_slice(), row_to_account)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn row_to_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    let created_str: String = row.get(6)?;
    let created_at = DateTime::parse_from_rfc3339(&created_str)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    Ok(Account {
        id: row.get(0)?,
        site_id: row.get(1)?,
        username: row.get(2)?,
        display_name: row.get(3)?,
        alias: row.get(4)?,
        notes: row.get(5)?,
        created_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::sites::{self, NewSite};
    use tempfile::TempDir;

    fn setup() -> (TempDir, Vault, i64) {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("v.db");
        let v = Vault::create(&path, b"hunter2").unwrap();
        let s = sites::create(&v, NewSite { name: "RS".into(), ..Default::default() }).unwrap();
        (tmp, v, s.id)
    }

    #[test]
    fn create_and_get() {
        let (_t, v, sid) = setup();
        let a = create(&v, NewAccount {
            site_id: sid,
            username: Some("chris".into()),
            display_name: None,
            alias: Some("main".into()),
            notes: None,
        }).unwrap();
        let fetched = get(&v, a.id).unwrap();
        assert_eq!(fetched.username.as_deref(), Some("chris"));
    }

    #[test]
    fn create_round_trips_display_name() {
        let (_t, v, sid) = setup();
        let a = create(&v, NewAccount {
            site_id: sid,
            username: Some("xXdragonSlayerXx".into()),
            display_name: Some("DragonSlayer".into()),
            ..Default::default()
        }).unwrap();
        assert_eq!(a.display_name.as_deref(), Some("DragonSlayer"));
        let fetched = get(&v, a.id).unwrap();
        assert_eq!(fetched.display_name.as_deref(), Some("DragonSlayer"));
        let listed = list_for_site(&v, sid, &[]).unwrap();
        assert_eq!(listed[0].display_name.as_deref(), Some("DragonSlayer"));
    }

    #[test]
    fn list_for_site_returns_only_that_sites_accounts() {
        let (_t, v, sid) = setup();
        let other = sites::create(&v, NewSite { name: "Other".into(), ..Default::default() }).unwrap();
        create(&v, NewAccount { site_id: sid, alias: Some("a".into()), ..Default::default() }).unwrap();
        create(&v, NewAccount { site_id: sid, alias: Some("b".into()), ..Default::default() }).unwrap();
        create(&v, NewAccount { site_id: other.id, alias: Some("c".into()), ..Default::default() }).unwrap();
        let mine = list_for_site(&v, sid, &[]).unwrap();
        assert_eq!(mine.len(), 2);
    }

    #[test]
    fn delete_removes_account_and_cascades_passwords() {
        let (_t, v, sid) = setup();
        let a = create(&v, NewAccount {
            site_id: sid,
            username: Some("alice".into()),
            display_name: None,
            alias: None,
            notes: None,
        }).unwrap();
        crate::repo::passwords::insert(&v, crate::repo::passwords::NewPassword {
            account_id: a.id,
            plaintext: "secret",
            source: "manual".into(),
            confidence: crate::repo::passwords::Confidence::Certain,
            notes: None,
            created_at: None,
        }).unwrap();

        delete(&v, a.id).unwrap();

        assert!(matches!(get(&v, a.id), Err(crate::error::Error::NotFound)));
        let hist = crate::repo::passwords::list_history(&v, a.id).unwrap();
        assert!(hist.is_empty(), "password history should cascade-delete");
        assert!(matches!(delete(&v, a.id), Err(crate::error::Error::NotFound)));
    }

    #[test]
    fn update_overwrites_fields_and_returns_not_found_for_missing() {
        let (_t, v, sid) = setup();
        let a = create(&v, NewAccount {
            site_id: sid,
            username: Some("original".into()),
            display_name: None,
            alias: None,
            notes: None,
        }).unwrap();
        update(&v, a.id, UpdateAccount {
            username: Some("updated".into()),
            display_name: Some("Display Name".into()),
            alias: None,
            notes: None,
        }).unwrap();
        let fetched = get(&v, a.id).unwrap();
        assert_eq!(fetched.username.as_deref(), Some("updated"));
        assert_eq!(fetched.display_name.as_deref(), Some("Display Name"));
        assert!(fetched.alias.is_none());

        // NULL out username
        update(&v, a.id, UpdateAccount { username: None, ..Default::default() }).unwrap();
        let fetched2 = get(&v, a.id).unwrap();
        assert!(fetched2.username.is_none());

        // NotFound for non-existent id
        assert!(matches!(update(&v, 999, UpdateAccount::default()), Err(crate::error::Error::NotFound)));
    }

    #[test]
    fn list_all_with_empty_tag_filter_unchanged() {
        let (_t, v, sid) = setup();
        create(&v, NewAccount { site_id: sid, ..Default::default() }).unwrap();
        create(&v, NewAccount { site_id: sid, ..Default::default() }).unwrap();
        let all = list_all(&v, &[]).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn list_all_with_one_tag_filters_to_tagged_accounts() {
        let (_t, v, sid) = setup();
        let a1 = create(&v, NewAccount { site_id: sid, ..Default::default() }).unwrap();
        let _a2 = create(&v, NewAccount { site_id: sid, ..Default::default() }).unwrap();
        let tag = crate::repo::tags::create(&v, crate::repo::tags::NewTag { name: "x", created_at: None }).unwrap();
        crate::repo::account_tags::assign(&v, a1.id, tag.id).unwrap();
        let filtered = list_all(&v, &[tag.id]).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, a1.id);
    }

    #[test]
    fn list_all_with_two_tags_and_filters_intersection() {
        let (_t, v, sid) = setup();
        let a_both = create(&v, NewAccount { site_id: sid, ..Default::default() }).unwrap();
        let a_only_a = create(&v, NewAccount { site_id: sid, ..Default::default() }).unwrap();
        let a_only_b = create(&v, NewAccount { site_id: sid, ..Default::default() }).unwrap();
        let _a_none = create(&v, NewAccount { site_id: sid, ..Default::default() }).unwrap();
        let ta = crate::repo::tags::create(&v, crate::repo::tags::NewTag { name: "a", created_at: None }).unwrap();
        let tb = crate::repo::tags::create(&v, crate::repo::tags::NewTag { name: "b", created_at: None }).unwrap();
        crate::repo::account_tags::assign(&v, a_both.id, ta.id).unwrap();
        crate::repo::account_tags::assign(&v, a_both.id, tb.id).unwrap();
        crate::repo::account_tags::assign(&v, a_only_a.id, ta.id).unwrap();
        crate::repo::account_tags::assign(&v, a_only_b.id, tb.id).unwrap();

        let filtered = list_all(&v, &[ta.id, tb.id]).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, a_both.id);
    }
}
