use chrono::{DateTime, Utc};
use rusqlite::params;
use crate::error::{Error, Result};
use crate::repo::common;
use crate::vault::Vault;

pub struct Tag {
    pub id: i64,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

pub struct TagWithCount {
    pub id: i64,
    pub name: String,
    pub account_count: i64,
}

pub struct NewTag<'a> {
    pub name: &'a str,
    pub created_at: Option<DateTime<Utc>>,
}

pub fn create(vault: &Vault, new: NewTag) -> Result<Tag> {
    let name = validate_name(new.name)?;
    let created_at = new.created_at.unwrap_or_else(Utc::now);
    let id = vault.conn().query_row(
        "INSERT INTO tags (name, created_at) VALUES (?1, ?2) RETURNING id",
        params![name, created_at.to_rfc3339()],
        |row| row.get(0),
    )?;
    Ok(Tag { id, name, created_at })
}

pub fn get(vault: &Vault, id: i64) -> Result<Tag> {
    vault.conn().query_row(
        "SELECT id, name, created_at FROM tags WHERE id = ?1",
        params![id],
        |row| {
            let created_at: String = row.get(2)?;
            Ok(Tag {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: DateTime::parse_from_rfc3339(&created_at)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
        },
    ).map_err(common::not_found_or_db)
}

pub fn list(vault: &Vault) -> Result<Vec<Tag>> {
    let mut stmt = vault.conn().prepare(
        "SELECT id, name, created_at FROM tags ORDER BY LOWER(name)"
    )?;
    let rows = stmt.query_map([], |row| {
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

pub fn list_with_counts(vault: &Vault) -> Result<Vec<TagWithCount>> {
    let mut stmt = vault.conn().prepare(
        "SELECT t.id, t.name, COUNT(at.account_id) AS account_count
         FROM tags t
         LEFT JOIN account_tags at ON at.tag_id = t.id
         GROUP BY t.id
         ORDER BY LOWER(t.name)"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(TagWithCount {
            id: row.get(0)?,
            name: row.get(1)?,
            account_count: row.get(2)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub fn find_by_name(vault: &Vault, name: &str) -> Result<Option<Tag>> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let res = vault.conn().query_row(
        "SELECT id, name, created_at FROM tags WHERE LOWER(name) = LOWER(?1)",
        params![trimmed],
        |row| {
            let created_at: String = row.get(2)?;
            Ok(Tag {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: DateTime::parse_from_rfc3339(&created_at)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
        },
    );
    match res {
        Ok(t) => Ok(Some(t)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn rename(vault: &Vault, id: i64, new_name: &str) -> Result<()> {
    let name = validate_name(new_name)?;
    let n = vault.conn().execute(
        "UPDATE tags SET name = ?1 WHERE id = ?2",
        params![name, id],
    )?;
    common::ensure_affected(n)
}

pub fn delete(vault: &Vault, id: i64) -> Result<()> {
    let n = vault.conn().execute(
        "DELETE FROM tags WHERE id = ?1",
        params![id],
    )?;
    common::ensure_affected(n)
}

fn validate_name(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Error::Validation("tag name must not be empty".into()));
    }
    if trimmed.len() > 64 {
        return Err(Error::Validation("tag name must be 64 chars or fewer".into()));
    }
    if trimmed != raw {
        return Err(Error::Validation("tag name must not have leading or trailing whitespace".into()));
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::Vault;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Vault) {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("vault.db");
        let mut v = Vault::create(&path, b"pw").unwrap();
        v.unlock(b"pw").unwrap();
        (tmp, v)
    }

    #[test]
    fn create_then_get_round_trip() {
        let (_t, v) = setup();
        let t = create(&v, NewTag { name: "runescape", created_at: None }).unwrap();
        let got = get(&v, t.id).unwrap();
        assert_eq!(got.name, "runescape");
    }

    #[test]
    fn find_by_name_is_case_insensitive() {
        let (_t, v) = setup();
        create(&v, NewTag { name: "RuneScape", created_at: None }).unwrap();
        let found = find_by_name(&v, "runescape").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "RuneScape");
    }

    #[test]
    fn rename_updates_row() {
        let (_t, v) = setup();
        let t = create(&v, NewTag { name: "old", created_at: None }).unwrap();
        rename(&v, t.id, "new").unwrap();
        let got = get(&v, t.id).unwrap();
        assert_eq!(got.name, "new");
    }

    #[test]
    fn delete_returns_not_found_on_second_call() {
        let (_t, v) = setup();
        let t = create(&v, NewTag { name: "tmp", created_at: None }).unwrap();
        delete(&v, t.id).unwrap();
        assert!(matches!(delete(&v, t.id), Err(Error::NotFound)));
    }

    #[test]
    fn list_with_counts_includes_zero_count() {
        let (_t, v) = setup();
        create(&v, NewTag { name: "unused", created_at: None }).unwrap();
        let list = list_with_counts(&v).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].account_count, 0);
    }
}
