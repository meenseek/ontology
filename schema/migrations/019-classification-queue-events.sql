-- Durable queues remain authoritative. Empty notices only wake consumers after
-- commit; rolled-back producers, no-op updates and empty claims emit nothing.
CREATE FUNCTION grouping_queue_changed() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE active_before boolean := false; active_after boolean := false;
BEGIN
    IF TG_TABLE_NAME = 'context_apply_batches' THEN
        IF TG_OP <> 'INSERT' THEN active_before := OLD.state IN ('pending','committed'); END IF;
        IF TG_OP <> 'DELETE' THEN active_after := NEW.state IN ('pending','committed'); END IF;
        IF active_before IS NOT DISTINCT FROM active_after THEN RETURN NULL; END IF;
        -- Discovery can also be waiting on this same native recovery gate.
        PERFORM pg_notify('ontology_document_discovery','');
    ELSIF TG_TABLE_NAME = 'memories' THEN
        IF NEW.scope <> 'personal' OR NEW.status IS NOT DISTINCT FROM OLD.status THEN RETURN NULL; END IF;
    ELSE
        IF TG_OP <> 'INSERT' THEN active_before := OLD.mode='auto' AND OLD.state IN ('pending','processing'); END IF;
        IF TG_OP <> 'DELETE' THEN active_after := NEW.mode='auto' AND NEW.state IN ('pending','processing'); END IF;
        IF NOT active_before AND NOT active_after THEN RETURN NULL; END IF;
        IF TG_OP = 'UPDATE' THEN
            IF ROW(OLD.mode,OLD.state,OLD.attempts,OLD.lease_until,OLD.source_revision) IS NOT DISTINCT FROM
               ROW(NEW.mode,NEW.state,NEW.attempts,NEW.lease_until,NEW.source_revision) THEN
                IF TG_TABLE_NAME = 'memory_grouping' THEN RETURN NULL; END IF;
                IF ROW(OLD.content_digest,OLD.membership_revision) IS NOT DISTINCT FROM
                   ROW(NEW.content_digest,NEW.membership_revision) THEN RETURN NULL; END IF;
            END IF;
        END IF;
    END IF;
    PERFORM pg_notify('ontology_grouping','');
    RETURN NULL;
END $$;
CREATE TRIGGER memory_grouping_changed AFTER INSERT OR UPDATE OR DELETE ON memory_grouping
    FOR EACH ROW EXECUTE FUNCTION grouping_queue_changed();
CREATE TRIGGER document_grouping_changed AFTER INSERT OR UPDATE OR DELETE ON document_grouping
    FOR EACH ROW EXECUTE FUNCTION grouping_queue_changed();
CREATE TRIGGER memory_grouping_eligibility_changed AFTER UPDATE OF status ON memories
    FOR EACH ROW EXECUTE FUNCTION grouping_queue_changed();
CREATE TRIGGER grouping_context_state_changed AFTER INSERT OR UPDATE OF state OR DELETE ON context_apply_batches
    FOR EACH ROW EXECUTE FUNCTION grouping_queue_changed();
