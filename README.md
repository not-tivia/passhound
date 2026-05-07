# PassHound

A local, encrypted, offline personal password vault and recovery tool.

**Phase 1 (this build):** CLI with `init` / `add` / `list` / `get`. Foundational vault and encryption.

Subsequent phases will add: bulk importers (Phase 2), a recovery generator that produces ranked candidates from your historical patterns (Phase 3), and a Tauri desktop GUI (Phase 4).

## Quick start

```bash
# Build
cargo build --release

# Create a vault (default location: OS user data dir; override with --vault)
./target/release/passhound init

# Add a password
./target/release/passhound add --site "RuneScape" --username chris --category Gaming

# List accounts
./target/release/passhound list

# Reveal a password (use the account id from `list`)
./target/release/passhound get --account 1
```

## Threat model (Phase 1)

- The vault file is encrypted such that password ciphertext requires the master password to read.
- Site names, usernames, and timestamps are stored in plaintext metadata for queryability. Acceptable for a single-user local tool; matches KeePass's default trade-off.
- The master password is processed via Argon2id (~250ms work factor). Each password blob is encrypted with XChaCha20-Poly1305 with a fresh nonce.
- Master keys are zeroized from memory on lock or app exit.
- The vault is NOT designed for multi-user/cloud sync scenarios.

## Repo layout

- `core/` — library crate. All vault, encryption, schema, and repository logic. No I/O beyond the SQLite DB.
- `cli/` — `passhound` binary, thin wrapper over `core`.
- `core/tests/` — integration tests.

## Running tests

```bash
cargo test
```
