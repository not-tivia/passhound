use crate::error::{Error, Result};
use crate::repo::common;
use crate::vault::Vault;
use chrono::{DateTime, Utc};
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Site {
    pub id: i64,
    pub name: String,
    pub url: Option<String>,
    pub category: Option<String>,
    pub abbreviations: Vec<String>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct NewSite {
    pub name: String,
    pub url: Option<String>,
    pub category: Option<String>,
    pub abbreviations: Vec<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateSite {
    pub name: String,
    pub url: Option<String>,
    pub category: Option<String>,
    pub abbreviations: Vec<String>,
    pub notes: Option<String>,
}

pub fn create(vault: &Vault, new: NewSite) -> Result<Site> {
    if new.name.trim().is_empty() {
        return Err(Error::InvalidInput("site name required".into()));
    }
    let abbr_json = serde_json::to_string(&new.abbreviations)
        .map_err(|e| Error::InvalidInput(format!("abbreviations: {e}")))?;
    let now = Utc::now();
    vault.conn().execute(
        "INSERT INTO sites (name, url, category, abbreviations, notes, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![new.name, new.url, new.category, abbr_json, new.notes, now.to_rfc3339()],
    )?;
    let id = vault.conn().last_insert_rowid();
    Ok(Site {
        id,
        name: new.name,
        url: new.url,
        category: new.category,
        abbreviations: new.abbreviations,
        notes: new.notes,
        created_at: now,
    })
}

pub fn update(vault: &Vault, id: i64, changes: UpdateSite) -> Result<()> {
    if changes.name.trim().is_empty() {
        return Err(Error::InvalidInput("site name required".into()));
    }
    let abbr_json = serde_json::to_string(&changes.abbreviations)
        .map_err(|e| Error::InvalidInput(format!("abbreviations: {e}")))?;
    let n = vault.conn().execute(
        "UPDATE sites SET name = ?1, url = ?2, category = ?3,
         abbreviations = ?4, notes = ?5 WHERE id = ?6",
        params![changes.name, changes.url, changes.category, abbr_json, changes.notes, id],
    )?;
    common::ensure_affected(n)
}

pub fn get(vault: &Vault, id: i64) -> Result<Site> {
    vault.conn().query_row(
        "SELECT id, name, url, category, abbreviations, notes, created_at FROM sites WHERE id = ?1",
        params![id],
        row_to_site,
    ).map_err(common::not_found_or_db)
}

pub fn find_by_name(vault: &Vault, name: &str) -> Result<Option<Site>> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let res = vault.conn().query_row(
        "SELECT id, name, url, category, abbreviations, notes, created_at
         FROM sites WHERE LOWER(name) = LOWER(?1)",
        params![trimmed],
        row_to_site,
    );
    match res {
        Ok(s) => Ok(Some(s)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn list(vault: &Vault) -> Result<Vec<Site>> {
    let mut stmt = vault.conn().prepare(
        "SELECT id, name, url, category, abbreviations, notes, created_at FROM sites ORDER BY name",
    )?;
    let rows = stmt
        .query_map([], row_to_site)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn row_to_site(row: &rusqlite::Row<'_>) -> rusqlite::Result<Site> {
    let abbr_json: String = row.get(4)?;
    let abbreviations: Vec<String> = serde_json::from_str(&abbr_json).unwrap_or_default();
    let created_str: String = row.get(6)?;
    let created_at = DateTime::parse_from_rfc3339(&created_str)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    Ok(Site {
        id: row.get(0)?,
        name: row.get(1)?,
        url: row.get(2)?,
        category: row.get(3)?,
        abbreviations,
        notes: row.get(5)?,
        created_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn vault() -> (TempDir, Vault) {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("v.db");
        let v = Vault::create(&path, b"hunter2").unwrap();
        (tmp, v)
    }

    #[test]
    fn create_and_get() {
        let (_tmp, v) = vault();
        let s = create(&v, NewSite {
            name: "RuneScape".into(),
            url: Some("runescape.com".into()),
            category: Some("Gaming".into()),
            abbreviations: vec!["RS".into(), "rs07".into()],
            notes: None,
        }).unwrap();
        assert_eq!(s.name, "RuneScape");
        let fetched = get(&v, s.id).unwrap();
        assert_eq!(fetched.id, s.id);
        assert_eq!(fetched.abbreviations, vec!["RS".to_string(), "rs07".to_string()]);
    }

    #[test]
    fn create_rejects_empty_name() {
        let (_tmp, v) = vault();
        let err = create(&v, NewSite::default()).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn list_returns_sites_ordered_by_name() {
        let (_tmp, v) = vault();
        for name in ["Zoom", "Amazon", "GitHub"] {
            create(&v, NewSite { name: name.into(), ..Default::default() }).unwrap();
        }
        let names: Vec<String> = list(&v).unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["Amazon", "GitHub", "Zoom"]);
    }

    #[test]
    fn get_returns_not_found_for_unknown_id() {
        let (_tmp, v) = vault();
        let err = get(&v, 999).unwrap_err();
        assert!(matches!(err, Error::NotFound));
    }

    #[test]
    fn find_by_name_is_case_insensitive_and_returns_none_for_missing() {
        let (_tmp, v) = vault();
        create(&v, NewSite { name: "RuneScape".into(), ..Default::default() }).unwrap();
        let found = find_by_name(&v, "runescape").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "RuneScape");
        let missing = find_by_name(&v, "nonexistent").unwrap();
        assert!(missing.is_none());
        let empty = find_by_name(&v, "   ").unwrap();
        assert!(empty.is_none());
    }

    #[test]
    fn update_changes_fields_round_trip() {
        let (_tmp, v) = vault();
        let s = create(&v, NewSite {
            name: "RuneScape".into(),
            url: Some("runescape.com".into()),
            category: Some("Gaming".into()),
            abbreviations: vec!["RS".into()],
            notes: None,
        }).unwrap();

        update(&v, s.id, UpdateSite {
            name: "Runescape Classic".into(),
            url: Some("classic.runescape.com".into()),
            category: Some("MMO".into()),
            abbreviations: vec!["RSC".into(), "rs07".into()],
            notes: Some("legacy account".into()),
        }).unwrap();

        let got = get(&v, s.id).unwrap();
        assert_eq!(got.name, "Runescape Classic");
        assert_eq!(got.url.as_deref(), Some("classic.runescape.com"));
        assert_eq!(got.category.as_deref(), Some("MMO"));
        assert_eq!(got.abbreviations, vec!["RSC".to_string(), "rs07".to_string()]);
        assert_eq!(got.notes.as_deref(), Some("legacy account"));
    }

    #[test]
    fn update_returns_not_found_for_missing_id() {
        let (_tmp, v) = vault();
        let err = update(&v, 999, UpdateSite {
            name: "x".into(), ..Default::default()
        }).unwrap_err();
        assert!(matches!(err, Error::NotFound));
    }
}
