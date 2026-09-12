-- Preserve evidence used by existing records without changing their historical metadata.
-- Only matching retained bytes can recover an old reference; other old evidence stays unavailable.
CREATE TABLE evidence_contents (
    scope text NOT NULL REFERENCES scopes(id),
    digest text NOT NULL CHECK (digest ~ '^[0-9a-f]{64}$'),
    content text NOT NULL CHECK (octet_length(content) <= 65536),
    PRIMARY KEY(scope,digest),
    CHECK (digest = encode(sha256(convert_to(content,'UTF8')),'hex'))
);
CREATE TABLE evidence_snapshots (
    scope text NOT NULL,
    memory_id text NOT NULL,
    revision bigint NOT NULL,
    entity_id text NOT NULL,
    digest text NOT NULL,
    PRIMARY KEY(scope,memory_id,revision,entity_id),
    FOREIGN KEY(scope,memory_id,revision) REFERENCES memory_history(scope,memory_id,revision) ON DELETE CASCADE,
    FOREIGN KEY(scope,digest) REFERENCES evidence_contents(scope,digest)
);
CREATE INDEX evidence_snapshots_content_idx ON evidence_snapshots(scope,digest);
WITH recovered AS MATERIALIZED (
    SELECT h.scope,h.memory_id,h.revision,x->>'entity_id' AS entity_id,p.content_digest,p.content
    FROM memory_history h CROSS JOIN LATERAL jsonb_array_elements(h.document->'evidence') x
    JOIN source_records p ON p.scope=h.scope AND p.entity_id=x->>'entity_id'
      AND p.content_digest=x->>'content_digest'
    WHERE p.content IS NOT NULL
), bodies AS (
    INSERT INTO evidence_contents(scope,digest,content)
    SELECT DISTINCT scope,content_digest,content FROM recovered
    RETURNING scope,digest
)
INSERT INTO evidence_snapshots(scope,memory_id,revision,entity_id,digest)
SELECT r.scope,r.memory_id,r.revision,r.entity_id,r.content_digest
FROM recovered r JOIN bodies b ON b.scope=r.scope AND b.digest=r.content_digest;
