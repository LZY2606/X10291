CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS versions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  parent_id INTEGER REFERENCES versions(id) DEFERRABLE INITIALLY DEFERRED,
  label TEXT NOT NULL,
  kind TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('pending', 'complete')),
  parameters_json TEXT NOT NULL,
  branch_id TEXT NOT NULL,
  snapshot_json TEXT NOT NULL,
  complete INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS sources (
  version_id INTEGER NOT NULL REFERENCES versions(id) ON DELETE CASCADE,
  source_id TEXT NOT NULL,
  ordinal INTEGER NOT NULL,
  file_name TEXT NOT NULL,
  content TEXT NOT NULL,
  content_sha TEXT NOT NULL,
  PRIMARY KEY (version_id, source_id)
);
CREATE TABLE IF NOT EXISTS changes (
  version_id INTEGER PRIMARY KEY REFERENCES versions(id) ON DELETE CASCADE,
  changes_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS branch_pointers (
  branch_id TEXT PRIMARY KEY,
  version_id INTEGER NOT NULL REFERENCES versions(id)
);
CREATE INDEX IF NOT EXISTS idx_versions_branch ON versions(branch_id, complete);
CREATE INDEX IF NOT EXISTS idx_sources_source_id ON sources(source_id);
INSERT OR IGNORE INTO meta (key, value) VALUES ('current_branch', 'main');
