ALTER TABLE password_history ADD COLUMN source_import_id INTEGER REFERENCES imports(id) ON DELETE SET NULL;
CREATE INDEX idx_pw_source_import ON password_history(source_import_id);
