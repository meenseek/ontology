use ontology::store::Store;

/// Reproduce the pre-020 layout only in a disposable test DB, preserving all
/// synthetic history before an older migration fixture creates its old state.
pub async fn legacy_history(store: &Store) {
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert!(database.starts_with("ontology_test_"));
    let mut tx = store.pool().begin().await.unwrap();
    sqlx::raw_sql("CREATE TEMP TABLE context_test_legacy_rows ON COMMIT DROP AS SELECT * FROM context_material_versions; ALTER TABLE context_projection_versions DROP CONSTRAINT context_projection_versions_material_id_revision_fkey; DROP VIEW context_material_versions; DROP TABLE context_material_version_records; DROP TABLE context_content_blobs; DROP TABLE context_content_chunks; DROP FUNCTION context_split_content(bytea,boolean); DROP FUNCTION context_join_content(uuid,text[]); DROP FUNCTION context_read_content(uuid,text); DROP FUNCTION context_store_content(uuid,bytea,boolean); DROP FUNCTION context_validate_blob(); DROP FUNCTION context_validate_version_record(); DELETE FROM ontology_migrations WHERE name='020-context-history-sharing.sql';")
        .execute(&mut *tx).await.unwrap();
    let migration = include_str!("../../schema/migrations/006-context-history.sql");
    let start = migration
        .find("CREATE TABLE context_material_versions (")
        .unwrap();
    let end = migration[start..]
        .find("INSERT INTO context_material_versions")
        .unwrap()
        + start;
    sqlx::raw_sql(&migration[start..end])
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::raw_sql("ALTER TABLE context_material_versions ADD COLUMN manual_edit_id uuid UNIQUE REFERENCES context_manual_edits(edit_id); INSERT INTO context_material_versions SELECT * FROM context_test_legacy_rows; CREATE TRIGGER context_history_immutable BEFORE UPDATE OR DELETE ON context_material_versions FOR EACH ROW EXECUTE FUNCTION context_immutable_record(); ALTER TABLE context_projection_versions ADD FOREIGN KEY(material_id,revision) REFERENCES context_material_versions(material_id,revision);")
        .execute(&mut *tx).await.unwrap();
    let migration = include_str!("../../schema/migrations/009-context-manual-edits.sql");
    let start = migration
        .find("CREATE OR REPLACE FUNCTION context_record_version()")
        .unwrap();
    let end = migration[start..]
        .find("CREATE FUNCTION context_manual_edit_complete()")
        .unwrap()
        + start;
    sqlx::raw_sql(&migration[start..end])
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}
