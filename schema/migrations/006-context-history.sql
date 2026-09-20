-- Additive native storage foundation. These records are not proof of Core acceptance.
CREATE TABLE context_store (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    store_id uuid NOT NULL UNIQUE DEFAULT gen_random_uuid()
);
INSERT INTO context_store(singleton) VALUES (true);

CREATE TABLE context_apply_batches (
    apply_id uuid PRIMARY KEY,
    store_id uuid NOT NULL REFERENCES context_store(store_id),
    core_run_id text NOT NULL CHECK (length(core_run_id) BETWEEN 1 AND 1024),
    prepared_run_digest text NOT NULL CHECK (prepared_run_digest ~ '^[0-9a-f]{64}$'),
    candidate_digest text NOT NULL CHECK (candidate_digest ~ '^[0-9a-f]{64}$'),
    expected_source_versions jsonb NOT NULL CHECK (jsonb_typeof(expected_source_versions) = 'object' AND octet_length(expected_source_versions::text) <= 1048576),
    context_targets jsonb NOT NULL CHECK (jsonb_typeof(context_targets) = 'array' AND jsonb_array_length(context_targets) BETWEEN 1 AND 10000 AND octet_length(context_targets::text) <= 1048576),
    core_apply_attempt_id text NOT NULL UNIQUE CHECK (length(core_apply_attempt_id) BETWEEN 1 AND 1024),
    expected_batch_id text NOT NULL UNIQUE CHECK (length(expected_batch_id) BETWEEN 1 AND 1024),
    expected_journal_locator text NOT NULL UNIQUE CHECK (length(expected_journal_locator) BETWEEN 1 AND 4096),
    actual_batch_id text CHECK (actual_batch_id = expected_batch_id),
    actual_journal_locator text CHECK (actual_journal_locator = expected_journal_locator),
    state text NOT NULL CHECK (state IN ('pending','committed','finalized','aborted')),
    commit_receipt jsonb CHECK (jsonb_typeof(commit_receipt) = 'object' AND octet_length(commit_receipt::text) <= 1048576),
    final_core_receipt_digest text CHECK (final_core_receipt_digest ~ '^[0-9a-f]{64}$'),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((actual_batch_id IS NULL) = (actual_journal_locator IS NULL)),
    CHECK (
        (state = 'pending' AND commit_receipt IS NULL AND final_core_receipt_digest IS NULL) OR
        (state = 'committed' AND actual_batch_id IS NOT NULL AND actual_journal_locator IS NOT NULL AND commit_receipt IS NOT NULL AND final_core_receipt_digest IS NULL) OR
        (state = 'finalized' AND actual_batch_id IS NOT NULL AND actual_journal_locator IS NOT NULL AND commit_receipt IS NOT NULL AND final_core_receipt_digest IS NOT NULL) OR
        (state = 'aborted' AND commit_receipt IS NULL AND final_core_receipt_digest IS NULL)
    )
);
CREATE UNIQUE INDEX context_one_unresolved_apply ON context_apply_batches(store_id)
    WHERE state IN ('pending','committed');

ALTER TABLE context_materials
    ALTER COLUMN source_root DROP NOT NULL,
    ALTER COLUMN source_digest DROP NOT NULL,
    ALTER COLUMN imported_at DROP NOT NULL,
    ADD COLUMN material_id uuid NOT NULL UNIQUE DEFAULT gen_random_uuid(),
    ADD COLUMN revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0),
    ADD COLUMN deleted boolean NOT NULL DEFAULT false,
    ADD COLUMN origin_kind text NOT NULL DEFAULT 'imported-file',
    ADD COLUMN created_at timestamptz,
    ADD COLUMN last_apply_id uuid REFERENCES context_apply_batches(apply_id);
UPDATE context_materials SET created_at = imported_at;
ALTER TABLE context_materials
    ALTER COLUMN created_at SET NOT NULL,
    ALTER COLUMN created_at SET DEFAULT now(),
    ADD CONSTRAINT context_origin_shape CHECK (
        (origin_kind = 'imported-file' AND source_root IS NOT NULL AND source_digest IS NOT NULL AND imported_at IS NOT NULL AND created_at = imported_at) OR
        (origin_kind = 'native' AND source_root IS NULL AND source_digest IS NULL AND imported_at IS NULL AND last_apply_id IS NOT NULL)
    ),
    ADD CONSTRAINT context_exact_bytes CHECK (byte_len = octet_length(content) AND content_digest = encode(sha256(content),'hex')),
    ADD CONSTRAINT context_tombstone CHECK (NOT deleted OR (byte_len = 0 AND content = ''::bytea AND search_text IS NULL));

CREATE TABLE context_material_versions (
    material_id uuid NOT NULL REFERENCES context_materials(material_id),
    revision bigint NOT NULL CHECK (revision > 0),
    content bytea NOT NULL,
    content_digest text NOT NULL CHECK (content_digest = encode(sha256(content),'hex')),
    byte_len bigint NOT NULL CHECK (byte_len BETWEEN 0 AND 16777216 AND byte_len = octet_length(content)),
    restricted boolean NOT NULL,
    search_text text,
    deleted boolean NOT NULL,
    apply_id uuid REFERENCES context_apply_batches(apply_id),
    recorded_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY(material_id, revision),
    CHECK (NOT restricted OR search_text IS NULL),
    CHECK (NOT deleted OR (byte_len = 0 AND content = ''::bytea AND search_text IS NULL))
);
INSERT INTO context_material_versions(material_id,revision,content,content_digest,byte_len,restricted,search_text,deleted,apply_id,recorded_at)
    SELECT material_id,revision,content,content_digest,byte_len,restricted,search_text,deleted,last_apply_id,created_at FROM context_materials;

-- Native apply/recovery mutations serialize with readers on their actual connection.
CREATE FUNCTION context_exclusive_gate() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NOT pg_try_advisory_xact_lock(478310003) THEN
        RAISE EXCEPTION 'Context operation in progress' USING ERRCODE = '55P03';
    END IF;
    RETURN NULL;
END $$;
CREATE TRIGGER context_apply_gate BEFORE INSERT OR UPDATE OR DELETE ON context_apply_batches
    FOR EACH STATEMENT EXECUTE FUNCTION context_exclusive_gate();
CREATE TRIGGER context_material_gate BEFORE INSERT OR UPDATE OR DELETE ON context_materials
    FOR EACH STATEMENT EXECUTE FUNCTION context_exclusive_gate();

CREATE FUNCTION context_immutable_record() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'Immutable context record' USING ERRCODE = '23514';
END $$;
CREATE TRIGGER context_store_immutable BEFORE UPDATE OR DELETE ON context_store
    FOR EACH ROW EXECUTE FUNCTION context_immutable_record();
CREATE TRIGGER context_history_immutable BEFORE UPDATE OR DELETE ON context_material_versions
    FOR EACH ROW EXECUTE FUNCTION context_immutable_record();
CREATE TRIGGER context_no_physical_delete BEFORE DELETE ON context_materials
    FOR EACH ROW EXECUTE FUNCTION context_immutable_record();

CREATE FUNCTION context_material_revision() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF ROW(NEW.material_id,NEW.scope,NEW.path,NEW.origin_kind,NEW.source_root,NEW.source_path,NEW.source_digest,NEW.imported_at,NEW.created_at)
            IS DISTINCT FROM ROW(OLD.material_id,OLD.scope,OLD.path,OLD.origin_kind,OLD.source_root,OLD.source_path,OLD.source_digest,OLD.imported_at,OLD.created_at) THEN
            RAISE EXCEPTION 'Immutable context identity or origin' USING ERRCODE = '23514';
        END IF;
        IF NEW.revision <> OLD.revision OR OLD.revision = 9223372036854775807 THEN
            RAISE EXCEPTION 'Invalid context revision' USING ERRCODE = '23514';
        END IF;
        IF NEW.last_apply_id IS NOT DISTINCT FROM OLD.last_apply_id THEN
            RAISE EXCEPTION 'A new pending apply is required' USING ERRCODE = '23514';
        END IF;
        NEW.revision := OLD.revision + 1;
    ELSE
        IF NEW.revision <> 1 OR NEW.deleted THEN
            RAISE EXCEPTION 'Invalid initial context version' USING ERRCODE = '23514';
        END IF;
        IF NEW.origin_kind = 'imported-file' THEN
            IF NEW.last_apply_id IS NOT NULL OR NEW.source_digest IS DISTINCT FROM NEW.content_digest THEN
                RAISE EXCEPTION 'Imported originals have no originating apply' USING ERRCODE = '23514';
            END IF;
            NEW.created_at := NEW.imported_at;
            RETURN NEW;
        END IF;
        NEW.created_at := clock_timestamp();
    END IF;
    IF NEW.last_apply_id IS NULL OR NOT EXISTS (
        SELECT 1 FROM context_apply_batches a JOIN context_store s ON s.store_id=a.store_id
        WHERE a.apply_id=NEW.last_apply_id AND a.state='pending'
    ) THEN
        RAISE EXCEPTION 'A matching pending apply is required' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER context_revision BEFORE INSERT OR UPDATE ON context_materials
    FOR EACH ROW EXECUTE FUNCTION context_material_revision();

CREATE FUNCTION context_record_version() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO context_material_versions(material_id,revision,content,content_digest,byte_len,restricted,search_text,deleted,apply_id,recorded_at)
        VALUES(NEW.material_id,NEW.revision,NEW.content,NEW.content_digest,NEW.byte_len,NEW.restricted,NEW.search_text,NEW.deleted,NEW.last_apply_id,
            CASE WHEN TG_OP='INSERT' THEN NEW.created_at ELSE clock_timestamp() END);
    RETURN NULL;
END $$;
CREATE TRIGGER context_version AFTER INSERT OR UPDATE ON context_materials
    FOR EACH ROW EXECUTE FUNCTION context_record_version();
