-- Human-authored edits have their own immutable provenance, never a Core apply receipt.
CREATE TABLE context_manual_edits (
    edit_id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    store_id uuid NOT NULL REFERENCES context_store(store_id),
    material_id uuid NOT NULL REFERENCES context_materials(material_id),
    expected_revision bigint NOT NULL CHECK (expected_revision > 0),
    expected_content_digest text NOT NULL CHECK (expected_content_digest ~ '^[0-9a-f]{64}$'),
    resulting_content_digest text NOT NULL CHECK (resulting_content_digest ~ '^[0-9a-f]{64}$'),
    recorded_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER context_manual_edit_gate BEFORE INSERT OR UPDATE OR DELETE ON context_manual_edits
    FOR EACH STATEMENT EXECUTE FUNCTION context_exclusive_gate();
CREATE TRIGGER context_manual_edit_immutable BEFORE UPDATE OR DELETE ON context_manual_edits
    FOR EACH ROW EXECUTE FUNCTION context_immutable_record();
ALTER TABLE context_materials ADD COLUMN last_manual_edit_id uuid REFERENCES context_manual_edits(edit_id);
ALTER TABLE context_material_versions ADD COLUMN manual_edit_id uuid UNIQUE REFERENCES context_manual_edits(edit_id);

CREATE OR REPLACE FUNCTION context_material_revision() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE apply_changed boolean; manual_changed boolean;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF ROW(NEW.material_id,NEW.scope,NEW.path,NEW.origin_kind,NEW.source_root,NEW.source_path,NEW.source_digest,NEW.imported_at,NEW.created_at)
            IS DISTINCT FROM ROW(OLD.material_id,OLD.scope,OLD.path,OLD.origin_kind,OLD.source_root,OLD.source_path,OLD.source_digest,OLD.imported_at,OLD.created_at) THEN
            RAISE EXCEPTION 'Immutable context identity or origin' USING ERRCODE = '23514';
        END IF;
        IF NEW.revision <> OLD.revision OR OLD.revision = 9223372036854775807 THEN
            RAISE EXCEPTION 'Invalid context revision' USING ERRCODE = '23514';
        END IF;
        apply_changed := NEW.last_apply_id IS DISTINCT FROM OLD.last_apply_id;
        manual_changed := NEW.last_manual_edit_id IS DISTINCT FROM OLD.last_manual_edit_id;
        IF apply_changed = manual_changed THEN
            RAISE EXCEPTION 'Exactly one context edit provenance is required' USING ERRCODE = '23514';
        END IF;
        IF manual_changed THEN
            IF NEW.last_manual_edit_id IS NULL OR OLD.deleted OR OLD.restricted OR NEW.deleted OR NEW.restricted
                OR NEW.deleted IS DISTINCT FROM OLD.deleted OR NEW.restricted IS DISTINCT FROM OLD.restricted
                OR NEW.scope NOT LIKE 'work/%' AND NEW.scope <> 'personal'
                OR NEW.path !~* '\.(md|markdown)$'
                OR NEW.path ~* '(^|/)(\.[^/]*|journal|raw)(/|$)'
                OR NEW.byte_len > 1048576
                OR NOT EXISTS (
                    SELECT 1 FROM context_manual_edits e JOIN context_store s ON s.store_id=e.store_id
                    WHERE e.edit_id=NEW.last_manual_edit_id AND e.material_id=OLD.material_id
                      AND e.expected_revision=OLD.revision AND e.expected_content_digest=OLD.content_digest
                      AND e.resulting_content_digest=NEW.content_digest
                ) THEN
                RAISE EXCEPTION 'Invalid manual context edit' USING ERRCODE = '23514';
            END IF;
        END IF;
        NEW.revision := OLD.revision + 1;
    ELSE
        IF NEW.revision <> 1 OR NEW.deleted OR NEW.last_manual_edit_id IS NOT NULL THEN
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
        apply_changed := true;
    END IF;
    IF apply_changed AND (NEW.last_apply_id IS NULL OR NOT EXISTS (
        SELECT 1 FROM context_apply_batches a JOIN context_store s ON s.store_id=a.store_id
        WHERE a.apply_id=NEW.last_apply_id AND a.state='pending'
    )) THEN
        RAISE EXCEPTION 'A matching pending apply is required' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END $$;

CREATE OR REPLACE FUNCTION context_record_version() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE version_apply_id uuid; version_manual_edit_id uuid;
BEGIN
    IF TG_OP='INSERT' THEN
        version_apply_id := NEW.last_apply_id;
        version_manual_edit_id := NULL;
    ELSIF NEW.last_manual_edit_id IS DISTINCT FROM OLD.last_manual_edit_id THEN
        version_apply_id := NULL;
        version_manual_edit_id := NEW.last_manual_edit_id;
    ELSE
        version_apply_id := NEW.last_apply_id;
        version_manual_edit_id := NULL;
    END IF;
    INSERT INTO context_material_versions(material_id,revision,content,content_digest,byte_len,restricted,search_text,deleted,apply_id,manual_edit_id,recorded_at)
        VALUES(NEW.material_id,NEW.revision,NEW.content,NEW.content_digest,NEW.byte_len,NEW.restricted,NEW.search_text,NEW.deleted,
            version_apply_id,version_manual_edit_id,
            CASE WHEN TG_OP='INSERT' THEN NEW.created_at ELSE clock_timestamp() END);
    RETURN NULL;
END $$;

CREATE FUNCTION context_manual_edit_complete() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM context_material_versions v
        JOIN context_projection_versions p ON p.material_id=v.material_id AND p.revision=v.revision
        WHERE v.manual_edit_id=NEW.edit_id AND v.material_id=NEW.material_id
          AND v.revision=NEW.expected_revision+1 AND v.content_digest=NEW.resulting_content_digest
          AND v.apply_id IS NULL AND p.payload->>'source_digest'=NEW.resulting_content_digest
    ) THEN
        RAISE EXCEPTION 'Manual context edit has no matching version' USING ERRCODE = '23514';
    END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER context_manual_edit_complete AFTER INSERT ON context_manual_edits
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION context_manual_edit_complete();
