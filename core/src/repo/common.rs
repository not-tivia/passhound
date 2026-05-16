//! Shared helpers for the repo modules — DRYs the `Error::NotFound`
//! mapping that recurs across passwords, accounts, sites, tags,
//! attachments, base_words.

use crate::error::{Error, Result};

/// Map a `rusqlite::Error` to `Error::NotFound` when it is
/// `QueryReturnedNoRows`, otherwise wrap it as `Error::Sqlite` (via the
/// existing `From<rusqlite::Error> for Error` impl).
///
/// Use after `query_row`:
///
/// ```ignore
/// vault.conn().query_row(SQL, params, ROW_FN)
///     .map_err(crate::repo::common::not_found_or_db)?;
/// ```
pub fn not_found_or_db(e: rusqlite::Error) -> Error {
    match e {
        rusqlite::Error::QueryReturnedNoRows => Error::NotFound,
        other => Error::from(other),
    }
}

/// Return `Error::NotFound` if the affected-rows count is zero.
///
/// Use after `execute()` for UPDATE / DELETE statements that target a
/// specific row:
///
/// ```ignore
/// let n = vault.conn().execute(SQL, params)?;
/// crate::repo::common::ensure_affected(n)?;
/// ```
pub fn ensure_affected(n: usize) -> Result<()> {
    if n == 0 { Err(Error::NotFound) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_or_db_maps_query_returned_no_rows() {
        let e = rusqlite::Error::QueryReturnedNoRows;
        assert!(matches!(not_found_or_db(e), Error::NotFound));
    }

    #[test]
    fn not_found_or_db_wraps_other_errors_as_db() {
        let e = rusqlite::Error::InvalidQuery;
        assert!(matches!(not_found_or_db(e), Error::Sqlite(_)));
    }

    #[test]
    fn ensure_affected_returns_ok_on_nonzero() {
        assert!(ensure_affected(1).is_ok());
        assert!(ensure_affected(7).is_ok());
    }

    #[test]
    fn ensure_affected_returns_not_found_on_zero() {
        assert!(matches!(ensure_affected(0), Err(Error::NotFound)));
    }
}
