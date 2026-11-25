-- REQ-COMP-010: brouillon de workflow propre à un projet, non exécutable.
-- Le texte est retenu sous forme d'octets UTF-8 exacts, sans analyse ni normalisation.
CREATE TABLE workflow_drafts (
 draft_id TEXT PRIMARY KEY,
 project_id TEXT NOT NULL REFERENCES projects(project_id),
 source_kind TEXT NOT NULL CHECK(source_kind IN ('text')),
 text_hash TEXT NOT NULL,
 text_utf8 BLOB NOT NULL CHECK(length(text_utf8)<=1048576),
 revision INTEGER NOT NULL CHECK(revision>=1),
 created_by_actor TEXT NOT NULL,
 created_at_ms INTEGER NOT NULL,
 updated_at_ms INTEGER NOT NULL
) STRICT;
CREATE INDEX workflow_drafts_project ON workflow_drafts(project_id, draft_id);
