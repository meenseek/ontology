-- Document identity, source currency and manual intent cannot use memory's FK
-- queue. Membership/history remain in document_subjects; no bodies are copied.
CREATE TABLE document_grouping (
    scope text NOT NULL REFERENCES scopes(id),
    material_id uuid REFERENCES context_materials(material_id) ON DELETE CASCADE,
    entity_id text,
    source_id text GENERATED ALWAYS AS (COALESCE(material_id::text,entity_id)) STORED,
    mode text NOT NULL CHECK(mode IN ('auto','manual','off')),
    state text NOT NULL CHECK(state IN ('pending','processing','assigned','suggested','unmatched','error','manual','off','ineligible')),
    source_revision text NOT NULL,
    content_digest text NOT NULL CHECK(content_digest ~ '^[0-9a-f]{64}$'),
    membership_revision bigint NOT NULL CHECK(membership_revision BETWEEN 0 AND 9007199254740991),
    attempts integer NOT NULL DEFAULT 0 CHECK(attempts >= 0),
    claim_token uuid,
    lease_until timestamptz,
    suggestions jsonb NOT NULL DEFAULT '{}'::jsonb CHECK(jsonb_typeof(suggestions)='object'),
    reason text,
    policy_digest text,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK((material_id IS NULL) <> (entity_id IS NULL)),
    CHECK((state='processing') = (claim_token IS NOT NULL AND lease_until IS NOT NULL)),
    PRIMARY KEY(scope,source_id),
    FOREIGN KEY(scope,entity_id) REFERENCES entities(scope,id) ON DELETE CASCADE
);
CREATE INDEX document_grouping_queue_idx ON document_grouping(state,lease_until,updated_at) WHERE mode='auto';

-- Protect every old UUID in both app scopes, including delayed projections and
-- future company bindings. Old Git IDs are protected even if currently bound.
-- An old unclassified document is not a request for automatic classification.
WITH originals AS (
    SELECT a.id AS scope,m.material_id,NULL::text AS entity_id,m.revision::text AS source_revision,m.content_digest
    FROM context_materials m CROSS JOIN scopes a WHERE a.id IN ('personal','meenseek')
    UNION ALL
    SELECT e.scope,NULL::uuid,e.id,COALESCE(p.source_revision,s.verified_revision,'baseline'),
           encode(sha256(convert_to(COALESCE(p.content,''),'UTF8')),'hex')
    FROM entities e JOIN sources s ON s.id=e.source_id AND s.scope=e.scope
    LEFT JOIN source_records p ON p.entity_id=e.id AND p.scope=e.scope WHERE s.kind='git'
)
INSERT INTO document_grouping(scope,material_id,entity_id,mode,state,source_revision,content_digest,membership_revision)
SELECT o.scope,o.material_id,o.entity_id,
       CASE WHEN d.subject_id IS NULL THEN 'off' ELSE 'manual' END,
       CASE WHEN d.subject_id IS NULL THEN 'off' ELSE 'manual' END,
       o.source_revision,o.content_digest,COALESCE(d.revision,0)
FROM originals o LEFT JOIN document_subjects d ON d.scope=o.scope AND d.source_id=COALESCE(o.material_id::text,o.entity_id);
INSERT INTO document_grouping(scope,material_id,entity_id,mode,state,source_revision,content_digest,membership_revision)
SELECT scope,material_id,entity_id,CASE WHEN subject_id IS NULL THEN 'off' ELSE 'manual' END,
       CASE WHEN subject_id IS NULL THEN 'off' ELSE 'manual' END,source_revision,content_digest,revision
FROM document_subjects ON CONFLICT(scope,source_id) DO NOTHING;
