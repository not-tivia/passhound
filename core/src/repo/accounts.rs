use crate::error::{Error, Result};
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
    ).map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Error::NotFound,
        other => Error::from(other),
    })
}

pub fn delete(vault: &Vault, id: i64) -> Result<()> {
    let n = vault.conn().execute(
        "DELETE FROM accounts WHERE id = ?1",
        params![id],
    )?;
    if n == 0 {
        return Err(Error::NotFound);
    }
    Ok(())
}

pub fn list_for_site(vault: &Vault, site_id: i64) -> Result<Vec<Account>> {
    let mut stmt = vault.conn().prepare(
        "SELECT id, site_id, username, display_name, alias, notes, created_at FROM accounts
         WHERE site_id = ?1 ORDER BY created_at",
    )?;
    let rows = stmt
        .query_map(params![site_id], row_to_account)?
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
        let listed = list_for_site(&v, sid).unwrap();
        assert_eq!(listed[0].display_name.as_deref(), Some("DragonSlayer"));
    }

    #[test]
    fn list_for_site_returns_only_that_sites_accounts() {
        let (_t, v, sid) = setup();
        let other = sites::create(&v, NewSite { name: "Other".into(), ..Default::default() }).unwrap();
        create(&v, NewAccount { site_id: sid, alias: Some("a".into()), ..Default::default() }).unwrap();
        create(&v, NewAccount { site_id: sid, alias: Some("b".into()), ..Default::default() }).unwrap();
        create(&v, NewAccount { site_id: other.id, alias: Some("c".into()), ..Default::default() }).unwrap();
        let mine = list_for_site(&v, sid).unwrap();
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
}
