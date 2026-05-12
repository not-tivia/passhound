CREATE TABLE attachments (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  filename TEXT NOT NULL,
  mime_type TEXT NOT NULL,
  size_bytes INTEGER NOT NULL,
  blob_encrypted BLOB NOT NULL,
  blob_nonce BLOB NOT NULL,
  created_at TEXT NOT NULL
) STRICT;
CREATE INDEX idx_attachments_account_id ON attachments(account_id);
