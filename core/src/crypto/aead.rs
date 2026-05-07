use crate::error::{Error, Result};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    Key, XChaCha20Poly1305, XNonce,
};
use rand::RngCore;

pub const NONCE_LEN: usize = 24;

/// Encrypt `plaintext` under `key`. Returns (ciphertext, nonce).
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<(Vec<u8>, [u8; NONCE_LEN])> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = XNonce::from_slice(&nonce_bytes);
    let ct = cipher.encrypt(nonce, plaintext).map_err(|_| Error::Aead)?;
    Ok((ct, nonce_bytes))
}

/// Decrypt `ciphertext` with `key` and `nonce`.
pub fn decrypt(key: &[u8; 32], ciphertext: &[u8], nonce: &[u8; NONCE_LEN]) -> Result<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    let nonce = XNonce::from_slice(nonce);
    cipher.decrypt(nonce, ciphertext).map_err(|_| Error::Aead)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> [u8; 32] {
        let mut k = [0u8; 32];
        for (i, b) in k.iter_mut().enumerate() {
            *b = i as u8;
        }
        k
    }

    #[test]
    fn round_trip() {
        let pt = b"hunter2";
        let (ct, nonce) = encrypt(&key(), pt).unwrap();
        let dec = decrypt(&key(), &ct, &nonce).unwrap();
        assert_eq!(dec, pt);
    }

    #[test]
    fn nonces_are_unique() {
        let pt = b"hello";
        let (_, n1) = encrypt(&key(), pt).unwrap();
        let (_, n2) = encrypt(&key(), pt).unwrap();
        assert_ne!(n1, n2);
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let pt = b"sekret";
        let (mut ct, nonce) = encrypt(&key(), pt).unwrap();
        ct[0] ^= 0x01;
        let err = decrypt(&key(), &ct, &nonce).unwrap_err();
        assert!(matches!(err, Error::Aead));
    }

    #[test]
    fn wrong_key_fails() {
        let pt = b"sekret";
        let (ct, nonce) = encrypt(&key(), pt).unwrap();
        let mut k2 = key();
        k2[0] ^= 0xFF;
        let err = decrypt(&k2, &ct, &nonce).unwrap_err();
        assert!(matches!(err, Error::Aead));
    }
}
