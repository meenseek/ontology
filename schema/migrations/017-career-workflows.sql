-- Progress only. Native originals, Core decisions and apply lifecycle retain their owners.
CREATE TABLE career_workflows (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    store_id uuid NOT NULL REFERENCES context_store(store_id),
    application_key text NOT NULL CHECK (length(application_key) BETWEEN 1 AND 160),
    outcome_key text NOT NULL CHECK (length(outcome_key) BETWEEN 1 AND 160),
    revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0),
    progress jsonb NOT NULL CHECK (jsonb_typeof(progress)='object' AND octet_length(progress::text)<=65536),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE(store_id,application_key,outcome_key)
);
