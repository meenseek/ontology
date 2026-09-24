-- Existing memory and history are preserved. This table is both the durable
-- classification queue and the user's manual/automatic grouping choice.
CREATE TABLE memory_grouping (
    memory_id text PRIMARY KEY REFERENCES memories(id) ON DELETE CASCADE,
    mode text NOT NULL CHECK (mode IN ('auto','manual','off')),
    state text NOT NULL CHECK (state IN ('pending','processing','assigned','suggested','unmatched','error','manual','off')),
    source_revision bigint NOT NULL CHECK (source_revision > 0),
    attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    lease_until timestamptz,
    suggestions jsonb NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(suggestions) = 'object'),
    reason text,
    policy_digest text,
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX memory_grouping_queue_idx ON memory_grouping(state,lease_until,updated_at)
    WHERE mode = 'auto';
INSERT INTO memory_grouping(memory_id,mode,state,source_revision)
SELECT id, CASE WHEN subject_id IS NULL THEN 'auto' ELSE 'manual' END,
       CASE WHEN subject_id IS NULL THEN 'pending' ELSE 'manual' END, revision
FROM memories WHERE scope = 'personal' AND status IN ('accepted','proposed');
