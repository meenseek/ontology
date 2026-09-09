-- Preserve the applied baseline and every existing Git identity and confirmation.
ALTER TABLE sources ADD COLUMN kind text NOT NULL DEFAULT 'git';
ALTER TABLE sources ALTER COLUMN kind DROP DEFAULT;
ALTER TABLE sources ADD CONSTRAINT sources_kind_check CHECK (kind IN ('git', 'vault'));
ALTER TABLE sources DROP CONSTRAINT sources_scope_repository_path_key;
ALTER TABLE sources ADD CONSTRAINT sources_scope_kind_repository_path_key UNIQUE(scope, kind, repository, path);
ALTER TABLE sources RENAME COLUMN verified_commit TO verified_revision;
ALTER TABLE source_records RENAME COLUMN absence_commit TO absence_revision;
ALTER TABLE sources DROP CONSTRAINT sources_failure_code_check;
ALTER TABLE sources ADD CONSTRAINT sources_failure_code_check CHECK (
    failure_code IS NULL OR
    (kind = 'git' AND failure_code = 'git-read-failed') OR
    (kind = 'vault' AND failure_code = 'vault-read-failed')
);
ALTER TABLE sources ADD CONSTRAINT sources_vault_status_check CHECK (kind <> 'vault' OR status <> 'missing');
