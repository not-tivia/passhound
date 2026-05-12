-- tags already exists from 001_initial.sql but lacks created_at
ALTER TABLE tags ADD COLUMN created_at TEXT NOT NULL DEFAULT '';

-- account_tags already exists from 001_initial.sql but lacks indexes
CREATE INDEX idx_account_tags_tag      ON account_tags(tag_id);
CREATE INDEX idx_account_tags_account  ON account_tags(account_id);
