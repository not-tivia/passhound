# PassHound

A local, encrypted, offline personal password vault and recovery tool. Argon2id KDF + XChaCha20-Poly1305 AEAD. Rust core, Tauri/React desktop GUI, CLI binary. See [SECURITY.md](SECURITY.md) for the threat model.

## Download

Latest release: **[v1.6.0](https://github.com/not-tivia/passhound/releases/latest)**

| File | Platform |
|---|---|
| `PassHound_1.6.0_x64_en-US.msi` | Windows (MSI installer) |
| `PassHound_1.6.0_x64-setup.exe` | Windows (NSIS installer) |
| `PassHound_1.6.0_amd64.deb` | Linux Debian/Ubuntu |
| `PassHound-1.6.0-1.x86_64.rpm` | Linux Fedora/RHEL |
| `PassHound_1.6.0_amd64.AppImage` | Linux portable |

Source tarball and the CLI binary can be built locally — see Quick start below.

## CLI quick start

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

## Recovering forgotten passwords

When you can't remember a password for one of your own accounts, PassHound can
generate a ranked list of candidates derived entirely from your own history.
The tool NEVER touches the network — it just prints candidates; you copy and
try them manually.

### Setup (one-time)

```bash
# Tokenize all imported password history into base_words and auto-favorite the top 10.
./target/release/passhound analyze

# Inspect what was extracted; promote/demote favorites as needed (manual flags
# are preserved across future `analyze` re-runs).
./target/release/passhound base-word list
./target/release/passhound base-word promote 12
./target/release/passhound base-word demote 5

# Define your life eras.
./target/release/passhound era add --name "RuneScape years" --start 2010-01-01 --end 2015-12-31
./target/release/passhound era add --name "College" --start 2016-01-01 --end 2019-12-31
./target/release/passhound era add --name "Modern" --start 2020-01-01
./target/release/passhound era list
```

### Generating candidates

```bash
# Top 100 candidates with full hints.
./target/release/passhound recover --site Reddit --era College --hint moon

# Loose hints — just a partial recollection.
./target/release/passhound recover --hint flu

# Tighten constraints.
./target/release/passhound recover --hint thunder --min-length 12 --require-symbol
```

### Output

```
RANK  SCORE  CANDIDATE                      WHY
1     0.92   MoonBeam$2019Rd                G+E+F+H: BaseWordPool + SiteAffix + NumberIncrement + EraBoost
2     0.88   MoonBeam$2018Rd                G+E+F+H: BaseWordPool + SiteAffix + NumberIncrement + EraBoost
...
```

- `RANK` is the position (1-based) in the ranked list.
- `SCORE` is the weighted-sum score; higher is more confidently a match.
- `CANDIDATE` is the password to try.
- `WHY` lists the rules that produced it. Tag letters mirror the project's pattern letters (B=special-suffix, D=word-combine, E=site-affix, F=number-increment, G=base-word-pool, H=era-boost, plus CASE and LEET).

### Hard guardrail

The recovery tool does not and will never make network requests against third-party services. It produces candidates; you try them manually.

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
