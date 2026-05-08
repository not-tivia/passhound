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

## Import existing passwords

PassHound can ingest your backlog from generic CSV files (Google Password Manager
exports, hand-rolled CSVs) or pasted text from old notes / Google Docs.

### CSV

```bash
# Dry run: parse + preview, show summary
./target/release/passhound import csv ~/Downloads/passwords.csv

# Inspect every classified row before committing
./target/release/passhound import csv ~/Downloads/passwords.csv --show-conflicts

# Apply the import
./target/release/passhound import csv ~/Downloads/passwords.csv --commit

# After commit, you'll be prompted: Shred /path/to/passwords.csv? [Y/n]
# Pressing Y overwrites and deletes the source file. Default-on; pass --no-shred to skip.
```

If PassHound can't auto-detect the column shape from your CSV's headers, it
drops into an interactive prompt (`Column index for site (required):`) and
saves the mapping per-fingerprint inside the vault — so re-importing the
same export shape later doesn't ask again.

You can also pass an explicit mapping: `--mapping site=Name,password=Pass`.

### Paste-and-parse

For old Google Docs entries or markdown snippets shaped like:

```
site: RuneScape
username: chris
password: Fluffy!2014

site: Amazon
username: chris@example.com
password: Bezos$Buy1
```

```bash
# From a file
./target/release/passhound import paste --file ~/old-passwords.md --commit

# Piped from stdin
cat ~/old-passwords.md | ./target/release/passhound import paste --commit

# From $EDITOR (when neither --file nor stdin is provided)
./target/release/passhound import paste --commit
```

The parser recognises `site:` / `name:` / `website:` / `title:` / `service:`,
`url:`, `username:` / `user:` / `login:` / `email:`, `password:` / `pass:`,
and `notes:` / `note:` / `comment:` (case-insensitive). Blocks are separated
by blank lines.

### Reversing an import

Every import gets a numeric id printed after `--commit`. To reverse it
(deletes only that import's password rows + orphan accounts + orphan sites):

```bash
./target/release/passhound import undo 7
```

Imports default to `confidence: uncertain`. Source rows are tagged with the
import id via the `password_history.source_import_id` column.

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
