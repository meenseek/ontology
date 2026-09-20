-- Exact original materials are separate from accepted memories and document projections.
CREATE TABLE context_materials (
    scope text NOT NULL CHECK (scope IN ('profile','personal') OR scope ~ '^work/[a-z0-9][a-z0-9_-]{0,63}$'),
    path text NOT NULL CHECK (octet_length(path) BETWEEN 1 AND 1024 AND path !~ '(^/|/$|//|(^|/)\.{1,2}(/|$)|\\)'),
    source_root text NOT NULL CHECK (left(source_root,1) = '/' AND octet_length(source_root) <= 1024),
    source_path text NOT NULL CHECK (source_path = scope || '/' || path),
    source_digest text NOT NULL CHECK (source_digest ~ '^[0-9a-f]{64}$'),
    content_digest text NOT NULL CHECK (content_digest = encode(sha256(content),'hex')),
    content bytea NOT NULL CHECK (octet_length(content) <= 16777216),
    byte_len bigint NOT NULL CHECK (byte_len = octet_length(content)),
    restricted boolean NOT NULL,
    search_text text CHECK (NOT restricted OR search_text IS NULL),
    imported_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (scope,path)
);
CREATE INDEX context_materials_origin ON context_materials(source_root,scope,path);
