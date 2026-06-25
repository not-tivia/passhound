use crate::error::Result;
use crate::vault::Vault;
use rusqlite::{params, OptionalExtension};

#[derive(Debug, Clone)]
pub struct SiteAlias {
    pub id: i64,
    pub alias_canonical: String,
    pub site_id: i64,
    pub original_name: String,
}

/// Insert or update the alias `alias_canonical -> site_id` (UNIQUE upsert).
pub fn record(vault: &Vault, alias_canonical: &str, site_id: i64, original_name: &str) -> Result<()> {
    vault.conn().execute(
        "INSERT INTO site_aliases (alias_canonical, site_id, original_name, created_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(alias_canonical) DO UPDATE SET site_id = excluded.site_id, original_name = excluded.original_name",
        params![alias_canonical, site_id, original_name, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

/// Survivor site id for a canonical name, if aliased.
pub fn resolve(vault: &Vault, canonical: &str) -> Result<Option<i64>> {
    let id = vault.conn().query_row(
        "SELECT site_id FROM site_aliases WHERE alias_canonical = ?1",
        params![canonical],
        |r| r.get(0),
    ).optional()?;
    Ok(id)
}

/// Aliases pointing at a site (for "also known as" display).
pub fn list_for_site(vault: &Vault, site_id: i64) -> Result<Vec<SiteAlias>> {
    let mut stmt = vault.conn().prepare(
        "SELECT id, alias_canonical, site_id, original_name FROM site_aliases WHERE site_id = ?1 ORDER BY original_name",
    )?;
    let rows = stmt.query_map(params![site_id], |r| Ok(SiteAlias {
        id: r.get(0)?, alias_canonical: r.get(1)?, site_id: r.get(2)?, original_name: r.get(3)?,
    }))?.collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Re-point every alias from one site to another (used when a survivor is itself merged).
pub fn repoint(vault: &Vault, from_site_id: i64, to_site_id: i64) -> Result<()> {
    vault.conn().execute(
        "UPDATE site_aliases SET site_id = ?1 WHERE site_id = ?2",
        params![to_site_id, from_site_id],
    )?;
    Ok(())
}

/// Remove an alias.
pub fn delete(vault: &Vault, id: i64) -> Result<()> {
    vault.conn().execute("DELETE FROM site_aliases WHERE id = ?1", params![id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::sites::{self, NewSite};
    use crate::vault::Vault;
    use tempfile::TempDir;

    fn vault() -> (TempDir, Vault) {
        let tmp = TempDir::new().unwrap();
        let v = Vault::create(&tmp.path().join("v.db"), b"x").unwrap();
        (tmp, v)
    }

    #[test]
    fn record_and_resolve() {
        let (_t, v) = vault();
        let s = sites::create(&v, NewSite { name: "RuneScape".into(), ..Default::default() }).unwrap();
        record(&v, "jagex", s.id, "Jagex").unwrap();
        assert_eq!(resolve(&v, "jagex").unwrap(), Some(s.id));
        assert_eq!(resolve(&v, "unknown").unwrap(), None);
    }

    #[test]
    fn record_upserts_on_duplicate_canonical() {
        let (_t, v) = vault();
        let a = sites::create(&v, NewSite { name: "A".into(), ..Default::default() }).unwrap();
        let b = sites::create(&v, NewSite { name: "B".into(), ..Default::default() }).unwrap();
        record(&v, "jagex", a.id, "Jagex").unwrap();
        record(&v, "jagex", b.id, "Jagex").unwrap(); // same canonical -> updates target
        assert_eq!(resolve(&v, "jagex").unwrap(), Some(b.id));
    }

    #[test]
    fn list_for_site_and_repoint_and_delete() {
        let (_t, v) = vault();
        let a = sites::create(&v, NewSite { name: "A".into(), ..Default::default() }).unwrap();
        let b = sites::create(&v, NewSite { name: "B".into(), ..Default::default() }).unwrap();
        record(&v, "jagex", a.id, "Jagex").unwrap();
        let listed = list_for_site(&v, a.id).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].original_name, "Jagex");
        repoint(&v, a.id, b.id).unwrap();
        assert_eq!(resolve(&v, "jagex").unwrap(), Some(b.id));
        let id = list_for_site(&v, b.id).unwrap()[0].id;
        delete(&v, id).unwrap();
        assert_eq!(resolve(&v, "jagex").unwrap(), None);
    }
}
