CREATE TABLE site_aliases (
    id              INTEGER PRIMARY KEY,
    alias_canonical TEXT NOT NULL UNIQUE,
    site_id         INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    original_name   TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT ''
) STRICT;
CREATE INDEX idx_site_aliases_site ON site_aliases(site_id);
