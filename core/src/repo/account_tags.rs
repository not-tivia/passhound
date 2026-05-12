use chrono::{DateTime, Utc};
use rusqlite::params;
use crate::error::Result;
use crate::vault::Vault;
use crate::repo::tags::Tag;

pub fn assign(vault: &Vault, account_id: i64, tag_id: i64) -> Result<()> {
    vault.conn().execute(
        "INSERT OR IGNORE INTO account_tags (account_id, tag_id) VALUES (?1, ?2)",
        params![account_id, tag_id],
    )?;
    Ok(())
}

pub fn unassign(vault: &Vault, account_id: i64, tag_id: i64) -> Result<()> {
    vault.conn().execute(
        "DELETE FROM account_tags WHERE account_id = ?1 AND tag_id = ?2",
        params![account_id, tag_id],
    )?;
    Ok(())
}

pub fn list_for_account(vault: &Vault, account_id: i64) -> Result<Vec<Tag>> {
    let mut stmt = vault.conn().prepare(
        "SELECT t.id, t.name, t.created_at
         FROM tags t
         JOIN account_tags at ON at.tag_id = t.id
         WHERE at.account_id = ?1
         ORDER BY LOWER(t.name)"
    )?;
    let rows = stmt.query_map(params![account_id], |row| {
        let created_at: String = row.get(2)?;
        Ok(Tag {
            id: row.get(0)?,
            name: row.get(1)?,
            created_at: DateTime::parse_from_rfc3339(&created_at)
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub fn bulk_assign(vault: &Vault, account_ids: &[i64], tag_id: i64) -> Result<usize> {
    let tx = vault.conn().unchecked_transaction()?;
    let mut total = 0usize;
    {
        let mut stmt = tx.prepare(
            "INSERT OR IGNORE INTO account_tags (account_id, tag_id) VALUES (?1, ?2)"
        )?;
        for &aid in account_ids {
            total += stmt.execute(params![aid, tag_id])?;
        }
    }
    tx.commit()?;
    Ok(total)
}

pub fn bulk_unassign(vault: &Vault, account_ids: &[i64], tag_id: i64) -> Result<usize> {
    let tx = vault.conn().unchecked_transaction()?;
    let mut total = 0usize;
    {
        let mut stmt = tx.prepare(
            "DELETE FROM account_tags WHERE account_id = ?1 AND tag_id = ?2"
        )?;
        for &aid in account_ids {
            total += stmt.execute(params![aid, tag_id])?;
        }
    }
    tx.commit()?;
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::{tags, sites, accounts};
    use crate::vault::Vault;
    use tempfile::TempDir;

    fn setup_with_accounts(n: usize) -> (TempDir, Vault, i64, Vec<i64>) {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("vault.db");
        let mut v = Vault::create(&path, b"pw").unwrap();
        v.unlock(b"pw").unwrap();
        let s = sites::create(&v, sites::NewSite { name: "S".into(), ..Default::default() }).unwrap();
        let tag = tags::create(&v, tags::NewTag { name: "throwaway", created_at: None }).unwrap();
        let ids: Vec<i64> = (0..n).map(|i| {
            accounts::create(&v, accounts::NewAccount {
                site_id: s.id,
                username: Some(format!("u{i}")),
                ..Default::default()
            }).unwrap().id
        }).collect();
        (tmp, v, tag.id, ids)
    }

    #[test]
    fn assign_then_list_for_account() {
        let (_t, v, tid, ids) = setup_with_accounts(1);
        let t2 = tags::create(&v, tags::NewTag { name: "main", created_at: None }).unwrap();
        assign(&v, ids[0], tid).unwrap();
        assign(&v, ids[0], t2.id).unwrap();
        let listed = list_for_account(&v, ids[0]).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].name, "main"); // ordered by LOWER(name)
        assert_eq!(listed[1].name, "throwaway");
    }

    #[test]
    fn assign_is_idempotent() {
        let (_t, v, tid, ids) = setup_with_accounts(1);
        assign(&v, ids[0], tid).unwrap();
        assign(&v, ids[0], tid).unwrap();
        let listed = list_for_account(&v, ids[0]).unwrap();
        assert_eq!(listed.len(), 1);
    }

    #[test]
    fn unassign_removes_pair() {
        let (_t, v, tid, ids) = setup_with_accounts(1);
        assign(&v, ids[0], tid).unwrap();
        unassign(&v, ids[0], tid).unwrap();
        assert!(list_for_account(&v, ids[0]).unwrap().is_empty());
    }

    #[test]
    fn bulk_assign_returns_inserted_count() {
        let (_t, v, tid, ids) = setup_with_accounts(3);
        let n = bulk_assign(&v, &ids, tid).unwrap();
        assert_eq!(n, 3);
        let n2 = bulk_assign(&v, &ids, tid).unwrap();
        assert_eq!(n2, 0, "idempotent re-run should insert nothing");
    }

    #[test]
    fn bulk_unassign_returns_deleted_count() {
        let (_t, v, tid, ids) = setup_with_accounts(3);
        // Only 2 of the 3 have the tag.
        assign(&v, ids[0], tid).unwrap();
        assign(&v, ids[1], tid).unwrap();
        let deleted = bulk_unassign(&v, &ids, tid).unwrap();
        assert_eq!(deleted, 2);
    }
}
