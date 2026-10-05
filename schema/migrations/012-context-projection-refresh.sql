-- The app owns regenerable, redacted current search projections. This deployed
-- boundary repairs eligible originals previously excluded by a path/parser bug.
-- No originals, revisions, history, identities, or classifications are migrated.
-- Only explicit project may refresh a current row; import/commit remain append-only.
CREATE FUNCTION context_projection_refresh_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'Immutable context projection history' USING ERRCODE = '23514';
    END IF;
    IF current_setting('ontology.projection_refresh', true) IS DISTINCT FROM 'on'
       OR ROW(NEW.material_id, NEW.revision) IS DISTINCT FROM ROW(OLD.material_id, OLD.revision)
       OR NOT EXISTS (
           SELECT 1 FROM context_materials
           WHERE material_id=OLD.material_id AND revision=OLD.revision
       )
       OR OLD.payload_digest IS DISTINCT FROM encode(sha256(convert_to(OLD.payload::text,'UTF8')),'hex') THEN
        RAISE EXCEPTION 'Invalid current context projection refresh' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END $$;
DROP TRIGGER context_projection_immutable ON context_projection_versions;
CREATE TRIGGER context_projection_immutable BEFORE UPDATE OR DELETE ON context_projection_versions
    FOR EACH ROW EXECUTE FUNCTION context_projection_refresh_guard();
DROP TRIGGER context_projection_check ON context_projection_versions;
CREATE TRIGGER context_projection_check BEFORE INSERT OR UPDATE ON context_projection_versions
    FOR EACH ROW EXECUTE FUNCTION context_projection_validate();
