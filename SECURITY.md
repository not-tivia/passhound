# PassHound — Security Model

## What's encrypted

- Account passwords (XChaCha20-Poly1305 + per-vault Argon2id-derived key)
- Password history blobs
- Recovery base words
- File attachment contents

## What's plaintext (deliberate tradeoff)

The following fields are stored unencrypted in the SQLite vault file:

- Site names, URLs, categories, abbreviations, and site-level notes
- Account usernames, aliases, display names, and per-account notes
- Attachment filenames, MIME types, and byte sizes
- Import records: source label, file path, import timestamp, entry count
- Password history metadata: source label, confidence, timestamps (created_at, retired_at)
- Recovery feedback traits: worked/didn't-work flags, length/character-class features, score, rank
- Base-word metadata: favorite flag, first/last seen timestamps, usage count
- All timestamps (created_at, updated_at, last_used, feedback_at)
- Tags (name, created_at) and tag-to-account assignments
- Eras (name, start/end dates, notes)
- Per-vault settings keys and values (vault_meta)

This is a privacy/usability tradeoff: it lets PassHound search, filter, and display
without unlocking each row individually, and shrinks the cryptographic surface area.
The encrypted columns are exactly the columns that would let an attacker authenticate
as the user on a third-party site. Metadata that only identifies WHICH services the
user has accounts at remains visible to anyone with the raw file.

## Crypto choices

- KDF: Argon2id, 64 MiB / 3 iter / 1 lane (stronger than OWASP 2025 "standard" tier)
- AEAD: XChaCha20-Poly1305 with random 24-byte nonces
- Salt: 16 random bytes per vault
- File perms: 0600 on Unix (main vault + rollback journal sidecar)
- Master-key zeroization on drop (ZeroizeOnDrop, forbid(unsafe_code))

## Threat model

PassHound DEFENDS against:
- Offline disk-image attackers with the raw vault file but no master password
- Multi-user Unix systems where the vault file would otherwise be world-readable
- Crash-recovery journal exposure (sidecar is also 0600)
- Casual over-the-shoulder observation (optional reveal_clear_seconds auto-mask)

PassHound does NOT defend against:
- An attacker with code execution as your user (keyloggers, screen capture, memory dumping)
- A compromised WebView / supply-chain compromise of the Tauri runtime
- Phishing of the master password itself
- Network adversaries (PassHound is offline by design — irrelevant)
- A targeted attacker with both your vault file AND a way to test guesses against you

## Reporting issues

This is a personal tool maintained by one person. If you find a bug, open an issue or
PR at https://github.com/not-tivia/passhound. No bounty program.
