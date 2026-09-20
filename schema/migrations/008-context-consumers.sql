-- Consumer identity is metadata. Canonical bytes and Core history remain untouched.
ALTER TABLE sources DROP CONSTRAINT sources_kind_check;
ALTER TABLE sources ADD CONSTRAINT sources_kind_check CHECK (kind IN ('git','vault','context'));
ALTER TABLE sources DROP CONSTRAINT sources_failure_code_check;
ALTER TABLE sources ADD CONSTRAINT sources_failure_code_check CHECK (
 failure_code IS NULL OR (kind='git' AND failure_code='git-read-failed') OR
 (kind='vault' AND failure_code IN ('vault-read-failed','context-read-failed')) OR
 (kind='context' AND failure_code='context-read-failed'));
ALTER TABLE sources DROP CONSTRAINT sources_vault_status_check;
CREATE FUNCTION context_source_revision(store uuid, material uuid, revision bigint, deleted boolean, content_digest text, origin text, source_digest text) RETURNS text
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
 SELECT CASE WHEN origin='imported-file' AND revision=1 AND NOT deleted AND source_digest=content_digest THEN source_digest
 ELSE encode(sha256(convert_to('context-source-v1:'||store::text||':'||material::text||':'||revision::text||':'||CASE WHEN deleted THEN 'deleted' ELSE 'live' END||':'||content_digest,'UTF8')),'hex') END
$$;
CREATE TABLE context_source_bindings (
 source_id text PRIMARY KEY REFERENCES sources(id),
 material_id uuid NOT NULL REFERENCES context_materials(material_id)
);
CREATE INDEX context_source_bindings_material_idx ON context_source_bindings(material_id);
DO $$ BEGIN
 IF EXISTS (SELECT 1 FROM sources s LEFT JOIN context_materials m ON m.origin_kind='imported-file' AND m.source_root=s.repository AND m.source_path=s.path WHERE s.kind='vault' GROUP BY s.id HAVING count(m.material_id)<>1) THEN
  RAISE EXCEPTION 'Context consumer migration requires one exact imported material per Vault source';
 END IF;
END $$;
INSERT INTO context_source_bindings(source_id,material_id)
 SELECT s.id,m.material_id FROM sources s JOIN context_materials m ON m.origin_kind='imported-file' AND m.source_root=s.repository AND m.source_path=s.path WHERE s.kind='vault';
-- A statement can revise several materials. Lock all affected source IDs together,
-- in the order used by memory evidence checks, before updating any consumer.
CREATE FUNCTION context_invalidate_consumers() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 PERFORM s.id FROM sources s
 JOIN context_source_bindings b ON b.source_id=s.id
 JOIN new_context_materials n ON n.material_id=b.material_id
 JOIN old_context_materials o ON o.material_id=n.material_id
 WHERE n.revision<>o.revision ORDER BY s.id FOR UPDATE OF s;
 WITH current AS (
  SELECT b.source_id,context_source_revision(c.store_id,n.material_id,n.revision,n.deleted,n.content_digest,n.origin_kind,n.source_digest) AS token
  FROM new_context_materials n JOIN old_context_materials o USING(material_id)
  JOIN context_source_bindings b ON b.material_id=n.material_id CROSS JOIN context_store c
  WHERE n.revision<>o.revision
 )
 UPDATE sources s SET verified_revision=c.token,generation=s.generation+1
 FROM current c WHERE s.id=c.source_id AND s.verified_revision IS DISTINCT FROM c.token;
 RETURN NULL;
END $$;
CREATE TRIGGER context_invalidate_consumers AFTER UPDATE ON context_materials
 REFERENCING OLD TABLE AS old_context_materials NEW TABLE AS new_context_materials
 FOR EACH STATEMENT EXECUTE FUNCTION context_invalidate_consumers();
-- Backfill only a token that changed; preserve original revision-one generations.
WITH current AS (SELECT b.source_id,context_source_revision(c.store_id,m.material_id,m.revision,m.deleted,m.content_digest,m.origin_kind,m.source_digest) AS token FROM context_source_bindings b JOIN context_materials m USING(material_id) CROSS JOIN context_store c)
UPDATE sources s SET verified_revision=c.token,generation=s.generation+1 FROM current c WHERE s.id=c.source_id AND s.verified_revision IS DISTINCT FROM c.token;
