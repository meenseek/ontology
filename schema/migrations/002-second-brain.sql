-- Append to the digest-checked migration chain; retain imported records and confirmations.
ALTER TABLE sources ADD COLUMN generation bigint NOT NULL DEFAULT 0 CHECK (generation >= 0);
CREATE TABLE subjects (
    id text PRIMARY KEY,
    scope text NOT NULL REFERENCES scopes(id),
    name text NOT NULL CHECK (octet_length(name) BETWEEN 1 AND 320),
    key_digest text NOT NULL,
    payload_digest text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE(scope,id), UNIQUE(scope,key_digest)
);
-- This survives erasure. It holds identifiers and digests only, never request content.
CREATE TABLE memory_creations (
    scope text NOT NULL REFERENCES scopes(id),
    key_digest text NOT NULL,
    payload_digest text NOT NULL,
    memory_id text NOT NULL UNIQUE,
    PRIMARY KEY(scope,key_digest)
);
CREATE TABLE memories (
    id text PRIMARY KEY,
    scope text NOT NULL REFERENCES scopes(id),
    subject_id text,
    revision bigint NOT NULL CHECK (revision BETWEEN 1 AND 9007199254740991),
    status text NOT NULL CHECK (status IN ('accepted','proposed','withdrawn')),
    document jsonb NOT NULL CHECK (octet_length(document::text) <= 24576),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE(scope,id),
    FOREIGN KEY(scope,subject_id) REFERENCES subjects(scope,id),
    FOREIGN KEY(id) REFERENCES memory_creations(memory_id)
);
CREATE TABLE memory_history (
    scope text NOT NULL,
    memory_id text NOT NULL,
    revision bigint NOT NULL,
    status text NOT NULL,
    subject_id text,
    document jsonb NOT NULL,
    changed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY(scope,memory_id,revision),
    FOREIGN KEY(scope,memory_id) REFERENCES memories(scope,id) ON DELETE CASCADE
);
CREATE INDEX memories_scope_status_idx ON memories(scope,status,subject_id,id);
