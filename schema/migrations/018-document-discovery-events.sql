-- Durable invalidations contain identities only. Source bodies, manual intent,
-- membership history and existing classification claims stay in their owners.
CREATE TABLE document_grouping_dirty (
    scope text NOT NULL REFERENCES scopes(id) CHECK (scope IN ('personal','meenseek')),
    material_id uuid,
    entity_id text,
    source_id text GENERATED ALWAYS AS (COALESCE(material_id::text,entity_id)) STORED,
    CHECK ((material_id IS NULL) <> (entity_id IS NULL)),
    PRIMARY KEY (scope,source_id)
);

CREATE FUNCTION document_grouping_invalidate(app_scope text, material uuid, entity text)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO document_grouping_dirty(scope,material_id,entity_id)
    SELECT id,material,entity FROM scopes
    WHERE id IN ('personal','meenseek') AND (app_scope IS NULL OR id=app_scope)
    ORDER BY id
    -- UPDATE must acquire the row lock, even for an already-dirty identity.
    -- DO NOTHING could swallow a change while a consumer is acknowledging it.
    ON CONFLICT(scope,source_id) DO UPDATE SET
        material_id=EXCLUDED.material_id,entity_id=EXCLUDED.entity_id;
    PERFORM pg_notify('ontology_document_discovery','');
END $$;

CREATE FUNCTION document_grouping_source_changed() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    source_ids text[];
    item record;
BEGIN
    IF TG_OP='UPDATE' THEN
        CASE TG_TABLE_NAME
        WHEN 'context_materials' THEN
            IF (OLD.scope,OLD.path,OLD.revision,OLD.content_digest,OLD.deleted,OLD.restricted)
                IS NOT DISTINCT FROM (NEW.scope,NEW.path,NEW.revision,NEW.content_digest,NEW.deleted,NEW.restricted) THEN RETURN NULL; END IF;
        WHEN 'context_projection_versions' THEN
            IF (OLD.material_id,OLD.revision,OLD.payload_digest)
                IS NOT DISTINCT FROM (NEW.material_id,NEW.revision,NEW.payload_digest) THEN RETURN NULL; END IF;
        WHEN 'sources' THEN
            IF (OLD.id,OLD.scope,OLD.kind,OLD.status,OLD.verified_revision)
                IS NOT DISTINCT FROM (NEW.id,NEW.scope,NEW.kind,NEW.status,NEW.verified_revision) THEN RETURN NULL; END IF;
        WHEN 'entities' THEN
            IF (OLD.id,OLD.scope,OLD.source_id) IS NOT DISTINCT FROM (NEW.id,NEW.scope,NEW.source_id) THEN RETURN NULL; END IF;
        WHEN 'source_records' THEN
            IF (OLD.scope,OLD.entity_id,OLD.source_revision,OLD.content_digest,OLD.present)
                IS NOT DISTINCT FROM (NEW.scope,NEW.entity_id,NEW.source_revision,NEW.content_digest,NEW.present) THEN RETURN NULL; END IF;
        WHEN 'context_source_bindings' THEN
            IF (OLD.source_id,OLD.material_id) IS NOT DISTINCT FROM (NEW.source_id,NEW.material_id) THEN RETURN NULL; END IF;
        WHEN 'document_subjects' THEN
            IF (OLD.scope,OLD.material_id,OLD.entity_id,OLD.revision)
                IS NOT DISTINCT FROM (NEW.scope,NEW.material_id,NEW.entity_id,NEW.revision) THEN RETURN NULL; END IF;
        END CASE;
    END IF;

    IF TG_TABLE_NAME IN ('context_materials','context_projection_versions') THEN
        IF TG_OP<>'INSERT' AND (TG_OP='DELETE' OR OLD.material_id IS DISTINCT FROM NEW.material_id) THEN
            PERFORM document_grouping_invalidate(NULL,OLD.material_id,NULL);
        END IF;
        IF TG_OP<>'DELETE' THEN PERFORM document_grouping_invalidate(NULL,NEW.material_id,NULL); END IF;
    ELSIF TG_TABLE_NAME IN ('source_records','entities','document_subjects') THEN
        IF TG_TABLE_NAME='entities' THEN
            IF TG_OP<>'INSERT' THEN PERFORM document_grouping_invalidate(OLD.scope,NULL,OLD.id); END IF;
            IF TG_OP<>'DELETE' THEN PERFORM document_grouping_invalidate(NEW.scope,NULL,NEW.id); END IF;
        ELSE
            IF TG_OP<>'INSERT' THEN
                IF TG_TABLE_NAME='document_subjects' THEN
                    PERFORM document_grouping_invalidate(OLD.scope,OLD.material_id,OLD.entity_id);
                ELSE PERFORM document_grouping_invalidate(OLD.scope,NULL,OLD.entity_id); END IF;
            END IF;
            IF TG_OP<>'DELETE' THEN
                IF TG_TABLE_NAME='document_subjects' THEN
                    PERFORM document_grouping_invalidate(NEW.scope,NEW.material_id,NEW.entity_id);
                ELSE PERFORM document_grouping_invalidate(NEW.scope,NULL,NEW.entity_id); END IF;
            END IF;
        END IF;
    ELSE
        IF TG_TABLE_NAME='context_source_bindings' THEN
            source_ids=ARRAY[CASE WHEN TG_OP<>'INSERT' THEN OLD.source_id END,CASE WHEN TG_OP<>'DELETE' THEN NEW.source_id END];
            IF TG_OP<>'INSERT' THEN PERFORM document_grouping_invalidate(NULL,OLD.material_id,NULL); END IF;
            IF TG_OP<>'DELETE' THEN PERFORM document_grouping_invalidate(NULL,NEW.material_id,NULL); END IF;
        ELSE
            source_ids=ARRAY[CASE WHEN TG_OP<>'INSERT' THEN OLD.id END,CASE WHEN TG_OP<>'DELETE' THEN NEW.id END];
        END IF;
        FOR item IN SELECT scope,id FROM entities WHERE scope IN ('personal','meenseek') AND source_id=ANY(source_ids) ORDER BY scope,id LOOP
            PERFORM document_grouping_invalidate(item.scope,NULL,item.id);
        END LOOP;
        FOR item IN SELECT DISTINCT material_id FROM context_source_bindings WHERE source_id=ANY(source_ids) ORDER BY material_id LOOP
            PERFORM document_grouping_invalidate(NULL,item.material_id,NULL);
        END LOOP;
    END IF;
    RETURN NULL;
END $$;

CREATE TRIGGER document_grouping_material_changed AFTER INSERT OR UPDATE OR DELETE ON context_materials
FOR EACH ROW EXECUTE FUNCTION document_grouping_source_changed();
CREATE TRIGGER document_grouping_projection_changed AFTER INSERT OR UPDATE OR DELETE ON context_projection_versions
FOR EACH ROW EXECUTE FUNCTION document_grouping_source_changed();
CREATE TRIGGER document_grouping_source_changed AFTER INSERT OR UPDATE OR DELETE ON sources
FOR EACH ROW EXECUTE FUNCTION document_grouping_source_changed();
CREATE TRIGGER document_grouping_entity_changed AFTER INSERT OR UPDATE OR DELETE ON entities
FOR EACH ROW EXECUTE FUNCTION document_grouping_source_changed();
CREATE TRIGGER document_grouping_record_changed AFTER INSERT OR UPDATE OR DELETE ON source_records
FOR EACH ROW EXECUTE FUNCTION document_grouping_source_changed();
CREATE TRIGGER document_grouping_binding_changed AFTER INSERT OR UPDATE OR DELETE ON context_source_bindings
FOR EACH ROW EXECUTE FUNCTION document_grouping_source_changed();
CREATE TRIGGER document_grouping_membership_changed AFTER INSERT OR UPDATE OR DELETE ON document_subjects
FOR EACH ROW EXECUTE FUNCTION document_grouping_source_changed();

-- One upgrade reconciliation, including old ineligible or manual/off identities.
-- Later discovery is restricted to committed invalidations, never a full poll.
INSERT INTO document_grouping_dirty(scope,material_id,entity_id)
SELECT a.id,m.material_id,NULL::text FROM context_materials m CROSS JOIN scopes a WHERE a.id IN ('personal','meenseek')
UNION
SELECT e.scope,NULL::uuid,e.id FROM entities e JOIN sources s ON s.id=e.source_id AND s.scope=e.scope WHERE s.kind='git'
UNION
SELECT scope,material_id,entity_id FROM document_grouping;
