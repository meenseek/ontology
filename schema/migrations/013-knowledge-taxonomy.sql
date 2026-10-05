-- The app classification owner extends existing subject identities. Deployed
-- source, memory, evidence and migration histories remain unchanged. Completion
-- requires restored-copy preservation and an explicit operating transition.
-- This unpublished draft also extends the existing Git projection owner with
-- authored reference paths. Backfill is explicit; original bytes are untouched.
ALTER TABLE source_records ADD COLUMN reference_paths text[] NOT NULL DEFAULT '{}';
ALTER TABLE subjects ADD COLUMN revision bigint NOT NULL DEFAULT 0
    CHECK (revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE subjects ADD COLUMN definition jsonb CHECK (definition IS NULL OR (
    jsonb_typeof(definition)='object'
    AND definition ?& ARRAY['purpose','include','exclude']
    AND definition-ARRAY['purpose','include','exclude']='{}'::jsonb
    AND jsonb_typeof(definition->'purpose')='string'
    AND jsonb_typeof(definition->'include')='string'
    AND jsonb_typeof(definition->'exclude')='string'
    AND octet_length(definition->>'purpose') BETWEEN 1 AND 1024
    AND octet_length(definition->>'include') BETWEEN 1 AND 2048
    AND octet_length(definition->>'exclude') BETWEEN 1 AND 2048));
CREATE TABLE subject_history (
    scope text NOT NULL REFERENCES scopes(id),
    subject_id text NOT NULL,
    revision bigint NOT NULL,
    name text NOT NULL,
    definition jsonb,
    action text NOT NULL CHECK (action IN ('create','define','delete')),
    changed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY(scope,subject_id,revision)
);
INSERT INTO subject_history(scope,subject_id,revision,name,definition,action,changed_at)
    SELECT scope,id,revision,name,definition,'create',created_at FROM subjects;
CREATE TABLE document_subjects (
    scope text NOT NULL REFERENCES scopes(id),
    material_id uuid REFERENCES context_materials(material_id) ON DELETE CASCADE,
    entity_id text,
    source_id text GENERATED ALWAYS AS (COALESCE(material_id::text,entity_id)) STORED,
    subject_id text,
    subject_revision bigint,
    revision bigint NOT NULL CHECK (revision BETWEEN 1 AND 9007199254740991),
    source_revision text NOT NULL,
    content_digest text NOT NULL CHECK (content_digest ~ '^[0-9a-f]{64}$'),
    reason text NOT NULL CHECK (octet_length(reason) BETWEEN 1 AND 2048),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((material_id IS NULL) <> (entity_id IS NULL)),
    CHECK ((subject_id IS NULL) = (subject_revision IS NULL)),
    CHECK (subject_revision IS NULL OR subject_revision >= 0),
    PRIMARY KEY(scope,source_id),
    FOREIGN KEY(scope,entity_id) REFERENCES entities(scope,id) ON DELETE CASCADE,
    FOREIGN KEY(scope,subject_id) REFERENCES subjects(scope,id)
);
CREATE INDEX document_subjects_subject_idx ON document_subjects(scope,subject_id);
-- Decision provenance survives source/group deletion; bodies are never copied.
CREATE TABLE document_subject_history (
    scope text NOT NULL REFERENCES scopes(id),
    source_id text NOT NULL,
    material_id uuid,
    entity_id text,
    revision bigint NOT NULL,
    subject_id text,
    subject_revision bigint,
    source_revision text NOT NULL,
    content_digest text NOT NULL,
    reason text NOT NULL,
    changed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY(scope,source_id,revision)
);
