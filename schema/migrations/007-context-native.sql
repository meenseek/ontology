-- Version-bound, redacted projections. Original and history columns are untouched.
CREATE TABLE context_projection_versions (
    material_id uuid NOT NULL,
    revision bigint NOT NULL,
    payload jsonb NOT NULL,
    payload_digest text NOT NULL,
    PRIMARY KEY (material_id, revision),
    FOREIGN KEY (material_id, revision) REFERENCES context_material_versions(material_id, revision),
    CHECK (octet_length(payload::text) <= 33554432),
    CHECK (payload_digest = encode(sha256(convert_to(payload::text,'UTF8')),'hex')),
    CHECK (jsonb_typeof(payload) = 'object'),
    CHECK (payload->>'status' IN ('searchable','excluded','unavailable','tombstone')),
    CHECK (payload ?& ARRAY['status','source_digest','terms']),
    CHECK (jsonb_typeof(payload->'terms') = 'array'),
    CHECK (payload->>'status'<>'searchable' OR (
        payload ?& ARRAY['title','body','language','aliases','exportable','ontology']
        AND jsonb_typeof(payload->'title')='string'
        AND jsonb_typeof(payload->'body')='string'
        AND jsonb_typeof(payload->'aliases')='array'
        AND jsonb_typeof(payload->'exportable')='boolean'
        AND jsonb_typeof(payload->'ontology')='object'))
);
CREATE INDEX context_projection_terms ON context_projection_versions USING gin ((payload->'terms'));
CREATE FUNCTION context_projection_validate() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE source_sha text; source_deleted boolean;
BEGIN
    SELECT content_digest, deleted INTO STRICT source_sha, source_deleted
      FROM context_material_versions WHERE material_id=NEW.material_id AND revision=NEW.revision;
    IF NEW.payload->>'source_digest' IS DISTINCT FROM source_sha
       OR ((NEW.payload->>'status'='tombstone') IS DISTINCT FROM source_deleted)
       OR (NEW.payload->>'status'<>'searchable' AND
           (NEW.payload ? 'body' OR NEW.payload ? 'title' OR NEW.payload->'terms'<>'[]'::jsonb)) THEN
        RAISE EXCEPTION 'Invalid context projection';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER context_projection_gate BEFORE INSERT OR UPDATE OR DELETE ON context_projection_versions
    FOR EACH STATEMENT EXECUTE FUNCTION context_exclusive_gate();
CREATE TRIGGER context_projection_check BEFORE INSERT ON context_projection_versions
    FOR EACH ROW EXECUTE FUNCTION context_projection_validate();
CREATE TRIGGER context_projection_immutable BEFORE UPDATE OR DELETE ON context_projection_versions
    FOR EACH ROW EXECUTE FUNCTION context_immutable_record();
ALTER TABLE context_apply_batches ADD COLUMN core_contract bytea,
    ADD COLUMN core_contract_digest text,
    ADD CONSTRAINT context_core_contract_bytes CHECK (
        (core_contract IS NULL AND core_contract_digest IS NULL) OR
        (core_contract IS NOT NULL AND core_contract_digest IS NOT NULL
         AND octet_length(core_contract) BETWEEN 1 AND 1048576
         AND core_contract_digest=encode(sha256(core_contract),'hex'))
    );
-- Once stored, the exact Core bytes are part of the pending effect identity.
CREATE FUNCTION context_core_contract_immutable() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF ROW(NEW.core_contract,NEW.core_contract_digest) IS DISTINCT FROM
       ROW(OLD.core_contract,OLD.core_contract_digest) THEN
        RAISE EXCEPTION 'Immutable Core contract';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER context_core_contract_immutable BEFORE UPDATE ON context_apply_batches
    FOR EACH ROW EXECUTE FUNCTION context_core_contract_immutable();
