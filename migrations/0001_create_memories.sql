CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE memories (
    id          UUID PRIMARY KEY,
    user_id     TEXT NOT NULL,
    content     TEXT NOT NULL,
    embedding   vector(1536) NOT NULL,
    metadata    JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at  TIMESTAMPTZ NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL
);

CREATE INDEX memories_user_id_idx ON memories (user_id);
CREATE INDEX memories_embedding_idx ON memories USING hnsw (embedding vector_cosine_ops);
