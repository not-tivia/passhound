//! Source-shred helper: overwrite-and-unlink for file-based imports.

use rand::RngCore;
use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

/// Overwrite the file with three passes (0xFF, 0x00, random), fsync between
/// passes, then unlink. On non-Unix targets, falls back to plain unlink.
///
/// Errors:
/// - File does not exist → `io::ErrorKind::NotFound`.
/// - Permission / I/O failures → underlying `io::Error`.
pub fn shred_file(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "shred target does not exist",
        ));
    }
    #[cfg(unix)]
    {
        let len = std::fs::metadata(path)?.len();
        if len > 0 {
            for pass in 0..3u8 {
                let mut f = OpenOptions::new().write(true).open(path)?;
                f.seek(SeekFrom::Start(0))?;
                let mut buf = vec![0u8; 4096];
                let mut written: u64 = 0;
                while written < len {
                    let chunk = std::cmp::min(buf.len() as u64, len - written) as usize;
                    match pass {
                        0 => buf[..chunk].iter_mut().for_each(|b| *b = 0xFF),
                        1 => buf[..chunk].iter_mut().for_each(|b| *b = 0x00),
                        _ => rand::thread_rng().fill_bytes(&mut buf[..chunk]),
                    }
                    f.write_all(&buf[..chunk])?;
                    written += chunk as u64;
                }
                f.sync_all()?;
            }
        }
    }
    std::fs::remove_file(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use tempfile::NamedTempFile;

    #[test]
    fn shred_removes_file() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "secret content").unwrap();
        let p = f.path().to_path_buf();
        // Persist the tempfile so dropping the handle doesn't delete it first.
        f.persist(&p).unwrap();
        assert!(p.exists());

        shred_file(&p).unwrap();
        assert!(!p.exists());
    }

    #[test]
    fn shred_missing_file_returns_err() {
        let p = std::path::PathBuf::from("/tmp/passhound-shred-does-not-exist-XYZ");
        let err = shred_file(&p).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn shred_empty_file_still_unlinks() {
        let f = NamedTempFile::new().unwrap();
        let p = f.path().to_path_buf();
        f.persist(&p).unwrap();
        assert!(p.exists());

        shred_file(&p).unwrap();
        assert!(!p.exists());
    }
}
