-- Owner: ontology native context. The existing 009 migration is already applied.
-- Target: user-authored edits of visible profile Markdown and Markdown up to the
-- 16 MiB import limit use the same revision, history, projection and conflict checks.
-- No temporary
-- compatibility path or data backfill remains after this function replacement.
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
                OR NEW.path !~* '\.(md|markdown)$'
                OR NEW.path ~* '(^|/)(\.[^/]*|journal|raw)(/|$)'
                OR NEW.byte_len > 16777216
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
