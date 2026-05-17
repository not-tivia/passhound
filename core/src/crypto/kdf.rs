use crate::error::{Error, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore;
use zeroize::Zeroizing;

/// Argon2id parameters: 64 MiB memory, 3 iterations, 1 lane. ~250ms on modern desktop.
///
/// OWASP 2025 cheatsheet tiers (https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html):
///   minimum:        19 MiB / 2 iter / 1 lane
///   standard:       46 MiB / 1 iter / 1 lane
///   high-security: 128 MiB / 3 iter / 4 lanes
///
/// PassHound sits between standard and high-security — explicitly conservative for a desktop
/// tool with no UX sensitivity to ~250ms unlocks. Per the 2026-05-17 security audit
/// (docs/superpowers/audits/2026-05-17-passhound-security-audit.md), these params are stronger
/// than current published guidance. DO NOT REDUCE WITHOUT EXPLICIT SECURITY REVIEW.
fn params() -> Params {
    Params::new(64 * 1024, 3, 1, Some(32)).expect("valid argon2 params")
}

/// Derive a 32-byte key from a master password and a salt.
///
/// Uses Argon2id with parameters chosen to take ~250ms on a modern desktop.
/// Salt MUST be at least 16 bytes and unique per vault.
///
/// The intermediate output buffer is wrapped in `Zeroizing` so it zeros on drop;
/// the returned `[u8; 32]` is a copy that the caller (typically `MasterKey::new`)
/// is responsible for zeroizing.
pub fn derive_key(password: &[u8], salt: &[u8]) -> Result<[u8; 32]> {
    if salt.len() < 16 {
        return Err(Error::InvalidInput("salt must be at least 16 bytes".into()));
    }
    let mut out = Zeroizing::new([0u8; 32]);
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params());
    argon
        .hash_password_into(password, salt, out.as_mut_slice())
        .map_err(|e| Error::Argon2(e.to_string()))?;
    Ok(*out)
}

/// Generate a fresh random 16-byte salt for a new vault.
pub fn generate_salt() -> [u8; 16] {
    let mut s = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut s);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_key_is_deterministic() {
        let salt = [0u8; 16];
        let k1 = derive_key(b"hunter2", &salt).unwrap();
        let k2 = derive_key(b"hunter2", &salt).unwrap();
        assert_eq!(k1, k2);
    }

    #[test]
    fn derive_key_differs_on_different_passwords() {
        let salt = [0u8; 16];
        let k1 = derive_key(b"hunter2", &salt).unwrap();
        let k2 = derive_key(b"correcthorsebatterystaple", &salt).unwrap();
        assert_ne!(k1, k2);
    }

    #[test]
    fn derive_key_differs_on_different_salts() {
        let k1 = derive_key(b"hunter2", &[1u8; 16]).unwrap();
        let k2 = derive_key(b"hunter2", &[2u8; 16]).unwrap();
        assert_ne!(k1, k2);
    }

    #[test]
    fn derive_key_rejects_short_salt() {
        let err = derive_key(b"hunter2", &[0u8; 4]).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn generate_salt_is_random() {
        let s1 = generate_salt();
        let s2 = generate_salt();
        assert_ne!(s1, s2);
    }
}
