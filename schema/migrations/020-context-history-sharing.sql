-- Owner: ontology native context storage. Target: existing, non-disposable 019
-- databases. Keep 005-019 unchanged. The temporary legacy table is removed only
-- after every original history column has been compared through the read view.
-- Current originals and indexed projections retain their existing fast paths.
SELECT pg_advisory_xact_lock(478310003);
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM context_apply_batches WHERE state IN ('pending','committed')) THEN
        RAISE EXCEPTION 'Resolve pending context effects before upgrading';
    END IF;
END $$;
LOCK TABLE context_materials, context_material_versions, context_projection_versions
    IN ACCESS EXCLUSIVE MODE;

CREATE TABLE context_content_chunks (
    material_id uuid NOT NULL REFERENCES context_materials(material_id),
    digest text NOT NULL CHECK (digest = encode(sha256(content),'hex')),
    content bytea NOT NULL CHECK (octet_length(content) BETWEEN 1 AND 1073741823),
    PRIMARY KEY (material_id,digest)
);
CREATE TABLE context_content_blobs (
    material_id uuid NOT NULL REFERENCES context_materials(material_id),
    digest text NOT NULL CHECK (digest ~ '^[0-9a-f]{64}$'),
    byte_len bigint NOT NULL CHECK (byte_len BETWEEN 0 AND 1073741823),
    chunks text[] NOT NULL CHECK (cardinality(chunks) BETWEEN 0 AND 2097152
        AND array_position(chunks,NULL) IS NULL
        AND (cardinality(chunks)=0 OR (array_ndims(chunks)=1 AND array_lower(chunks,1)=1))),
    PRIMARY KEY (material_id,digest)
);
CREATE TRIGGER context_chunk_immutable BEFORE UPDATE OR DELETE ON context_content_chunks
    FOR EACH ROW EXECUTE FUNCTION context_immutable_record();
CREATE TRIGGER context_blob_immutable BEFORE UPDATE OR DELETE ON context_content_blobs
    FOR EACH ROW EXECUTE FUNCTION context_immutable_record();

-- Byte-exact, independent chunks: no dependency on another revision. Select
-- newline anchors by their preceding bytes, so edits can resynchronize. Long
-- lines and binary data use 4096-byte pieces when sharing. A single-version
-- original stays whole: splitting unique data wastes rows and TOAST compression.
-- No UTF-8 decoding.
CREATE FUNCTION context_split_content(bytes bytea, can_share boolean DEFAULT true) RETURNS SETOF bytea
LANGUAGE plpgsql IMMUTABLE STRICT AS $$
DECLARE first_pos integer := 1; scan_pos integer; last_pos integer;
    newline_pos integer; relative_pos integer; size integer := octet_length(bytes);
BEGIN
    IF NOT can_share THEN
        IF size>0 THEN RETURN NEXT bytes; END IF;
        RETURN;
    END IF;
    WHILE first_pos <= size LOOP
        last_pos := least(first_pos+4095,size);
        scan_pos := first_pos+511;
        WHILE scan_pos <= last_pos LOOP
            relative_pos := position(decode('0a','hex') IN substring(bytes FROM scan_pos FOR last_pos-scan_pos+1));
            EXIT WHEN relative_pos=0;
            newline_pos := scan_pos+relative_pos-1;
            IF (get_byte(sha256(substring(bytes FROM greatest(1,newline_pos-63)
                    FOR least(64,newline_pos))),0) & 7)=0 THEN
                last_pos := newline_pos;
                EXIT;
            END IF;
            scan_pos := newline_pos+1;
        END LOOP;
        RETURN NEXT substring(bytes FROM first_pos FOR last_pos-first_pos+1);
        first_pos := last_pos+1;
    END LOOP;
END $$;

CREATE FUNCTION context_join_content(material uuid, parts text[]) RETURNS bytea
LANGUAGE plpgsql STABLE STRICT AS $$
DECLARE bytes bytea; present bigint;
BEGIN
    SELECT coalesce(string_agg(c.content,''::bytea ORDER BY p.position),''::bytea),count(c.digest)
        INTO bytes,present FROM unnest(parts) WITH ORDINALITY AS p(digest,position)
        LEFT JOIN context_content_chunks c ON c.material_id=material AND c.digest=p.digest;
    IF present <> cardinality(parts) THEN
        RAISE EXCEPTION 'Missing context content chunk';
    END IF;
    RETURN bytes;
END $$;
CREATE FUNCTION context_read_content(material uuid, sha text) RETURNS bytea
LANGUAGE plpgsql STABLE STRICT AS $$
DECLARE blob context_content_blobs; bytes bytea;
BEGIN
    SELECT * INTO STRICT blob FROM context_content_blobs
        WHERE material_id=material AND digest=sha;
    bytes := context_join_content(material,blob.chunks);
    IF octet_length(bytes) <> blob.byte_len OR encode(sha256(bytes),'hex') <> sha THEN
        RAISE EXCEPTION 'Corrupt context content';
    END IF;
    RETURN bytes;
END $$;
-- Cross-row validation belongs to triggers/read checks, not CHECK constraints:
-- pg_dump restores data before installing these triggers.
CREATE FUNCTION context_validate_blob() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE bytes bytea;
BEGIN
    bytes := context_join_content(NEW.material_id,NEW.chunks);
    IF octet_length(bytes) <> NEW.byte_len OR encode(sha256(bytes),'hex') <> NEW.digest THEN
        RAISE EXCEPTION 'Invalid context content manifest';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER context_blob_check BEFORE INSERT ON context_content_blobs
    FOR EACH ROW EXECUTE FUNCTION context_validate_blob();

CREATE FUNCTION context_store_content(material uuid, bytes bytea, can_share boolean DEFAULT true) RETURNS text
LANGUAGE plpgsql VOLATILE STRICT AS $$
DECLARE sha text := encode(sha256(bytes),'hex'); part bytea; part_sha text;
    parts text[] := ARRAY[]::text[];
BEGIN
    IF EXISTS (SELECT 1 FROM context_content_blobs WHERE material_id=material AND digest=sha) THEN
        IF context_read_content(material,sha) IS DISTINCT FROM bytes THEN
            RAISE EXCEPTION 'Context content digest collision';
        END IF;
        RETURN sha;
    END IF;
    FOR part IN SELECT * FROM context_split_content(bytes,can_share) LOOP
        part_sha := encode(sha256(part),'hex');
        INSERT INTO context_content_chunks VALUES(material,part_sha,part)
            ON CONFLICT(material_id,digest) DO NOTHING;
        IF NOT FOUND AND NOT EXISTS (SELECT 1 FROM context_content_chunks
                WHERE material_id=material AND digest=part_sha AND content=part) THEN
            RAISE EXCEPTION 'Context chunk digest collision';
        END IF;
        parts := array_append(parts,part_sha);
    END LOOP;
    INSERT INTO context_content_blobs VALUES(material,sha,octet_length(bytes),parts)
        ON CONFLICT(material_id,digest) DO NOTHING;
    IF context_read_content(material,sha) IS DISTINCT FROM bytes THEN
        RAISE EXCEPTION 'Context content digest collision';
    END IF;
    RETURN sha;
END $$;

ALTER TABLE context_material_versions RENAME TO context_material_versions_legacy;
CREATE TABLE context_material_version_records (
    material_id uuid NOT NULL REFERENCES context_materials(material_id),
    revision bigint NOT NULL CHECK (revision > 0),
    content_digest text NOT NULL,
    byte_len bigint NOT NULL CHECK (byte_len BETWEEN 0 AND 16777216),
    restricted boolean NOT NULL,
    search_digest text,
    deleted boolean NOT NULL,
    apply_id uuid REFERENCES context_apply_batches(apply_id),
    recorded_at timestamptz NOT NULL DEFAULT now(),
    manual_edit_id uuid UNIQUE REFERENCES context_manual_edits(edit_id),
    PRIMARY KEY (material_id,revision),
    FOREIGN KEY (material_id,content_digest) REFERENCES context_content_blobs(material_id,digest),
    FOREIGN KEY (material_id,search_digest) REFERENCES context_content_blobs(material_id,digest),
    CHECK (NOT restricted OR search_digest IS NULL),
    CHECK (NOT deleted OR (byte_len=0 AND content_digest=encode(sha256(''::bytea),'hex') AND search_digest IS NULL))
);
CREATE FUNCTION context_validate_version_record() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM context_content_blobs
            WHERE material_id=NEW.material_id AND digest=NEW.content_digest AND byte_len=NEW.byte_len) THEN
        RAISE EXCEPTION 'Invalid context version length';
    END IF;
    IF NEW.search_digest IS NOT NULL THEN
        PERFORM convert_from(context_read_content(NEW.material_id,NEW.search_digest),'UTF8');
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER context_version_record_check BEFORE INSERT ON context_material_version_records
    FOR EACH ROW EXECUTE FUNCTION context_validate_version_record();
CREATE TRIGGER context_history_immutable BEFORE UPDATE OR DELETE ON context_material_version_records
    FOR EACH ROW EXECUTE FUNCTION context_immutable_record();

INSERT INTO context_material_version_records
    SELECT material_id,revision,context_store_content(material_id,content,can_share),byte_len,restricted,
        CASE WHEN search_text IS NULL THEN NULL ELSE context_store_content(material_id,convert_to(search_text,'UTF8'),can_share) END,
        deleted,apply_id,recorded_at,manual_edit_id
    FROM context_material_versions_legacy
    JOIN (SELECT material_id,count(DISTINCT content_digest)>1 AS can_share
        FROM context_material_versions_legacy GROUP BY material_id) sharing USING(material_id)
    ORDER BY material_id,revision;
CREATE VIEW context_material_versions AS
    SELECT material_id,revision,context_read_content(material_id,content_digest) AS content,
        content_digest,byte_len,restricted,
        convert_from(context_read_content(material_id,search_digest),'UTF8') AS search_text,
        deleted,apply_id,recorded_at,manual_edit_id
    FROM context_material_version_records;

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
    INSERT INTO context_material_version_records
        VALUES(NEW.material_id,NEW.revision,context_store_content(NEW.material_id,NEW.content,NEW.revision>1),NEW.byte_len,NEW.restricted,
            CASE WHEN NEW.search_text IS NULL THEN NULL ELSE context_store_content(NEW.material_id,convert_to(NEW.search_text,'UTF8'),NEW.revision>1) END,
            NEW.deleted,version_apply_id,CASE WHEN TG_OP='INSERT' THEN NEW.created_at ELSE clock_timestamp() END,version_manual_edit_id);
    RETURN NULL;
END $$;

DO $$ BEGIN
    IF EXISTS (
        SELECT 1 FROM context_material_versions_legacy old
        FULL JOIN context_material_versions new USING(material_id,revision)
        WHERE to_jsonb(old) IS DISTINCT FROM to_jsonb(new)
    ) THEN
        RAISE EXCEPTION 'Context history preservation check failed';
    END IF;
END $$;
ALTER TABLE context_projection_versions
    DROP CONSTRAINT context_projection_versions_material_id_revision_fkey,
    ADD FOREIGN KEY (material_id,revision) REFERENCES context_material_version_records(material_id,revision);
-- Drop the old heap/TOAST after equality checks; DROP COLUMN alone would retain
-- the old physical payload. Transaction rollback restores the entire old layout.
DROP TABLE context_material_versions_legacy;
