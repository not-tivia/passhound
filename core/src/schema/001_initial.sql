CREATE TABLE vault_meta (
    key TEXT PRIMARY KEY,
    value BLOB NOT NULL
) STRICT;

CREATE TABLE sites (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    url TEXT,
    category TEXT,
    abbreviations TEXT NOT NULL DEFAULT '[]',
    notes TEXT,
    created_at TEXT NOT NULL
) STRICT;
CREATE INDEX idx_sites_name ON sites(name);
CREATE INDEX idx_sites_category ON sites(category);

CREATE TABLE accounts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    site_id INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    username TEXT,
    alias TEXT,
    notes TEXT,
    created_at TEXT NOT NULL
) STRICT;
CREATE INDEX idx_accounts_site ON accounts(site_id);

CREATE TABLE password_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    password_encrypted BLOB NOT NULL,
    password_nonce BLOB NOT NULL,
    created_at TEXT NOT NULL,
    retired_at TEXT,
    source TEXT NOT NULL,
    confidence TEXT NOT NULL DEFAULT 'certain',
    notes TEXT
) STRICT;
CREATE INDEX idx_pw_account ON password_history(account_id);
CREATE INDEX idx_pw_current ON password_history(account_id) WHERE retired_at IS NULL;

CREATE TABLE base_words (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    word_encrypted BLOB NOT NULL,
    word_nonce BLOB NOT NULL,
    is_favorite INTEGER NOT NULL DEFAULT 0,
    first_seen_at TEXT,
    last_seen_at TEXT,
    usage_count INTEGER NOT NULL DEFAULT 0,
    notes TEXT
) STRICT;

CREATE TABLE eras (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    start_date TEXT,
    end_date TEXT,
    notes TEXT
) STRICT;

CREATE TABLE tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE
) STRICT;

CREATE TABLE account_tags (
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (account_id, tag_id)
) STRICT;

CREATE TABLE imports (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source TEXT NOT NULL,
    file_path TEXT,
    imported_at TEXT NOT NULL,
    entries_added INTEGER NOT NULL DEFAULT 0,
    notes TEXT
) STRICT;
