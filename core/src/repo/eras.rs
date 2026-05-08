use crate::error::{Error, Result};
use crate::vault::Vault;
use chrono::NaiveDate;
use rusqlite::params;

#[derive(Debug, Clone)]
pub struct Era {
    pub id: i64,
    pub name: String,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub notes: Option<String>,
}

pub fn list(vault: &Vault) -> Result<Vec<Era>> {
    let mut stmt = vault.conn().prepare(
        "SELECT id, name, start_date, end_date, notes FROM eras ORDER BY start_date NULLS LAST, name",
    )?;
    let rows = stmt.query_map([], row_to_era)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn add(
    vault: &Vault,
    name: &str,
    start: Option<NaiveDate>,
    end: Option<NaiveDate>,
    notes: Option<&str>,
) -> Result<i64> {
    if name.trim().is_empty() {
        return Err(Error::InvalidInput("era name required".into()));
    }
    vault.conn().execute(
        "INSERT INTO eras (name, start_date, end_date, notes) VALUES (?1, ?2, ?3, ?4)",
        params![
            name,
            start.map(|d| d.format("%Y-%m-%d").to_string()),
            end.map(|d| d.format("%Y-%m-%d").to_string()),
            notes,
        ],
    )?;
    Ok(vault.conn().last_insert_rowid())
}

pub fn find_by_name(vault: &Vault, name: &str) -> Result<Option<Era>> {
    match vault.conn().query_row(
        "SELECT id, name, start_date, end_date, notes FROM eras WHERE name = ?1",
        params![name],
        row_to_era,
    ) {
        Ok(e) => Ok(Some(e)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(other) => Err(Error::from(other)),
    }
}

fn row_to_era(row: &rusqlite::Row<'_>) -> rusqlite::Result<Era> {
    let start: Option<String> = row.get(2)?;
    let end: Option<String> = row.get(3)?;
    Ok(Era {
        id: row.get(0)?,
        name: row.get(1)?,
        start_date: start.and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()),
        end_date: end.and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()),
        notes: row.get(4)?,
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
    fn add_then_find() {
        let (_t, v) = vault();
        let start = NaiveDate::from_ymd_opt(2010, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2015, 12, 31).unwrap();
        add(&v, "RuneScape years", Some(start), Some(end), None).unwrap();
        let e = find_by_name(&v, "RuneScape years").unwrap().unwrap();
        assert_eq!(e.start_date, Some(start));
        assert_eq!(e.end_date, Some(end));
    }

    #[test]
    fn add_rejects_empty_name() {
        let (_t, v) = vault();
        let err = add(&v, "  ", None, None, None).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn find_returns_none_for_unknown() {
        let (_t, v) = vault();
        assert!(find_by_name(&v, "nope").unwrap().is_none());
    }

    #[test]
    fn list_returns_eras_ordered_by_start_then_name() {
        let (_t, v) = vault();
        add(&v, "Modern", Some(NaiveDate::from_ymd_opt(2020, 1, 1).unwrap()), None, None).unwrap();
        add(&v, "RuneScape years", Some(NaiveDate::from_ymd_opt(2010, 1, 1).unwrap()), None, None).unwrap();
        add(&v, "College", Some(NaiveDate::from_ymd_opt(2016, 1, 1).unwrap()), None, None).unwrap();
        let names: Vec<String> = list(&v).unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["RuneScape years", "College", "Modern"]);
    }
}
