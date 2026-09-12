-- Processing receipts are not knowledge, a job queue, or evidence of universal truth.
-- Keep deployed history and user records intact; only new curation writes own these rows.
CREATE TABLE curation_reviews (
    id text PRIMARY KEY,
    scope text NOT NULL REFERENCES scopes(id),
    key_digest text NOT NULL,
    source_id text NOT NULL,
    input_digest text NOT NULL,
    candidate_digest text NOT NULL,
    candidate jsonb NOT NULL CHECK (octet_length(candidate::text) <= 24576),
    author text NOT NULL,
    review jsonb,
    result jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    reviewed_at timestamptz,
    UNIQUE(scope,id), UNIQUE(scope,key_digest),
    CHECK ((review IS NULL) = (result IS NULL)),
    CHECK ((review IS NULL) = (reviewed_at IS NULL))
);
CREATE INDEX curation_reviews_source_idx ON curation_reviews(scope,source_id);
CREATE UNIQUE INDEX curation_reviews_applied_idx
ON curation_reviews(scope,source_id,input_digest)
WHERE result->>'outcome' IN ('created','updated','no-change','forgotten');
