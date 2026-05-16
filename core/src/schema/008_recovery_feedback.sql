CREATE TABLE recovery_feedback (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER REFERENCES accounts(id) ON DELETE SET NULL,
    provenance TEXT NOT NULL,
    score REAL NOT NULL,
    rank INTEGER NOT NULL,
    worked INTEGER NOT NULL,
    length INTEGER NOT NULL,
    has_digit INTEGER NOT NULL,
    has_symbol INTEGER NOT NULL,
    has_upper INTEGER NOT NULL,
    has_lower INTEGER NOT NULL,
    feedback_at TEXT NOT NULL
) STRICT;

CREATE INDEX idx_recovery_feedback_worked ON recovery_feedback(worked);
