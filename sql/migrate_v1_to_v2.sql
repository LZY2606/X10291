CREATE TABLE IF NOT EXISTS meta_old (key TEXT PRIMARY KEY, value TEXT NOT NULL);
INSERT INTO meta_old (key, value) VALUES ('current_branch', 'main');
ALTER TABLE versions RENAME TO versions_old;
CREATE TABLE versions (
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
INSERT INTO versions (id, parent_id, label, kind, status, parameters_json, branch_id, snapshot_json, complete)
SELECT id, parent_id, label, COALESCE(kind, 'import'), 'complete',
       COALESCE(parameters_json, '{"rule_version":"rules-legacy.1","pressure_unit":"bar"}'),
       COALESCE(branch_id, 'main'), snapshot_json, 1
FROM versions_old;
DROP TABLE versions_old;
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
  changes_json TEXT NOT NULL DEFAULT '[]'
);
CREATE TABLE IF NOT EXISTS branch_pointers (
  branch_id TEXT PRIMARY KEY,
  version_id INTEGER NOT NULL REFERENCES versions(id)
);
INSERT OR IGNORE INTO branch_pointers (branch_id, version_id)
SELECT 'main', MAX(id) FROM versions WHERE complete = 1;
