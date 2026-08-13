-- Hybrid search full-text leg (issue #20).
--
-- A generated tsvector column keeps the full-text index in lockstep with
-- `content` for free: PostgreSQL recomputes it on every insert/update, so the
-- storage provider does not need separate index maintenance.

ALTER TABLE memories
    ADD COLUMN content_tsv tsvector
    GENERATED ALWAYS AS (to_tsvector('english', content)) STORED;

CREATE INDEX memories_content_tsv_idx ON memories USING gin (content_tsv);
