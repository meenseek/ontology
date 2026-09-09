CREATE TABLE scopes (id text PRIMARY KEY CHECK (id IN ('meenseek', 'personal')));
INSERT INTO scopes VALUES ('meenseek'), ('personal');
CREATE TABLE areas (id text PRIMARY KEY, label text NOT NULL);
CREATE TABLE topics (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    scope text NOT NULL REFERENCES scopes(id),
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 80),
    UNIQUE(scope, name), UNIQUE(scope, id)
);
CREATE TABLE sources (
    id text PRIMARY KEY,
    scope text NOT NULL REFERENCES scopes(id),
    repository text NOT NULL,
    path text NOT NULL,
    last_attempt_at timestamptz NOT NULL DEFAULT now(),
    last_success_at timestamptz,
    status text NOT NULL CHECK (status IN ('ok', 'missing', 'failed')),
    verified_commit text,
    failure_code text CHECK (failure_code IS NULL OR failure_code = 'git-read-failed'),
    UNIQUE(scope, repository, path), UNIQUE(scope, id)
);
CREATE TABLE entities (
    id text PRIMARY KEY,
    scope text NOT NULL REFERENCES scopes(id),
    source_id text NOT NULL,
    revision bigint NOT NULL DEFAULT 0 CHECK (revision >= 0),
    FOREIGN KEY(scope, source_id) REFERENCES sources(scope, id),
    UNIQUE(scope, id), UNIQUE(scope, source_id)
);
CREATE TABLE entity_areas (
    scope text NOT NULL CHECK (scope='meenseek'),
    entity_id text NOT NULL,
    area text NOT NULL REFERENCES areas(id),
    PRIMARY KEY(scope,entity_id,area),
    FOREIGN KEY(scope,entity_id) REFERENCES entities(scope,id)
);
CREATE TABLE entity_topics (
    scope text NOT NULL,
    entity_id text NOT NULL,
    topic_id bigint NOT NULL,
    PRIMARY KEY(scope,entity_id,topic_id),
    FOREIGN KEY(scope,entity_id) REFERENCES entities(scope,id),
    FOREIGN KEY(scope,topic_id) REFERENCES topics(scope,id)
);
CREATE TABLE source_records (
    entity_id text PRIMARY KEY,
    scope text NOT NULL,
    content text CHECK (octet_length(content) <= 65536),
    content_digest text,
    source_revision text,
    observed_at timestamptz NOT NULL DEFAULT now(),
    present boolean NOT NULL,
    absence_commit text,
    CHECK ((content IS NULL) = (content_digest IS NULL)),
    CHECK (present = (absence_commit IS NULL)),
    FOREIGN KEY(scope, entity_id) REFERENCES entities(scope, id)
);
CREATE TABLE confirmation_history (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    scope text NOT NULL,
    entity_id text NOT NULL,
    revision bigint NOT NULL,
    kind text NOT NULL CHECK (kind IN ('classification', 'link-add', 'link-remove')),
    previous jsonb NOT NULL,
    confirmed jsonb NOT NULL,
    confirmed_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY(scope, entity_id) REFERENCES entities(scope, id)
);
CREATE TABLE related_materials (
    scope text NOT NULL,
    left_id text NOT NULL,
    right_id text NOT NULL,
    confirmed_at timestamptz NOT NULL DEFAULT now(),
    CHECK (left_id < right_id),
    PRIMARY KEY(scope, left_id, right_id),
    FOREIGN KEY(scope, left_id) REFERENCES entities(scope, id),
    FOREIGN KEY(scope, right_id) REFERENCES entities(scope, id)
);
CREATE INDEX entities_scope_idx ON entities(scope, id);
CREATE INDEX source_records_scope_idx ON source_records(scope, entity_id);
CREATE INDEX history_scope_entity_idx ON confirmation_history(scope, entity_id, id DESC);
CREATE INDEX related_right_idx ON related_materials(scope, right_id);
