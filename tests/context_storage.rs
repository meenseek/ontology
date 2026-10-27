#[path = "support/context_history.rs"]
mod context_history;

use ontology::{
    context::{ContextScope, inventory},
    domain::Error,
    store::{Store, digest},
};
use serde_json::Value;
use sqlx::Row;
use std::{collections::BTreeMap, time::Instant};

static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn store(legacy: bool) -> Store {
    let url = std::env::var("TEST_DATABASE_URL").expect("owned test DB");
    assert!(
        ontology::config::database_options(&url)
            .unwrap()
            .get_database()
            .is_some_and(|name| name.starts_with("ontology_test_"))
    );
    let store = Store::connect(&url).await.unwrap();
    sqlx::raw_sql("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
        .execute(store.pool())
        .await
        .unwrap();
    store.initialize().await.unwrap();
    if legacy {
        context_history::legacy_history(&store).await;
    }
    store
}

async fn snapshot(store: &Store, tables: &[String]) -> BTreeMap<String, Value> {
    let mut values = BTreeMap::new();
    for table in tables {
        assert!(
            table
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        );
        let sql = format!(
            "SELECT jsonb_build_array(count(*),encode(sha256(convert_to(coalesce(string_agg(to_jsonb(t)::text,E'\\n' ORDER BY to_jsonb(t)::text),''),'UTF8')),'hex')) FROM \"{table}\" t"
        );
        values.insert(
            table.clone(),
            // The only interpolated identifier passed the ASCII allowlist above.
            sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
                .fetch_one(store.pool())
                .await
                .unwrap(),
        );
    }
    values
}

async fn tables(store: &Store) -> Vec<String> {
    sqlx::query_scalar("SELECT tablename::text FROM pg_tables WHERE schemaname='public' AND tablename<>'ontology_migrations' ORDER BY tablename")
        .fetch_all(store.pool()).await.unwrap()
}

async fn seed(store: &Store, body: &[u8]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("personal/raw")).unwrap();
    for (name, bytes) in [
        ("note.md", body),
        ("empty.md", b"".as_slice()),
        ("raw/binary", b"\0\xff\r\n\xef\xbb\xbf".as_slice()),
    ] {
        std::fs::write(dir.path().join("personal").join(name), bytes).unwrap();
    }
    let scope: ContextScope = "personal".parse().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let inv = inventory(&root, std::slice::from_ref(&scope)).unwrap();
    store
        .import_context(&root, &[scope], &inv.inventory_digest)
        .await
        .unwrap();
    dir
}

fn body() -> String {
    let mut text = "\u{feff}---\r\ntitle: History\r\n---\r\n# 원문\r\n".to_string();
    for i in 0..1600 {
        text.push_str(&format!(
            "{i:04} {} 한글🙂\r\n",
            digest(format!("synthetic-{i}").as_bytes())
        ));
    }
    text
}

fn noisy_bytes() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(16 * 1024 * 1024);
    let mut state = 0x123456789abcdef0_u64;
    for _ in 0..(16 * 1024 * 1024 / 8) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        bytes.extend_from_slice(&state.to_le_bytes());
    }
    bytes
}

#[tokio::test]
async fn single_revision_binary_does_not_pay_fragmentation_overhead() {
    let _guard = LOCK.lock().await;
    let store = store(true).await;
    let bytes = noisy_bytes();
    let sha = digest(&bytes);
    sqlx::query("INSERT INTO context_materials(scope,path,source_root,source_path,source_digest,content_digest,content,byte_len,restricted,search_text,imported_at) VALUES('personal','raw/large.bin','/synthetic','personal/raw/large.bin',$1,$1,$2,$3,true,NULL,now())")
        .bind(&sha).bind(&bytes).bind(bytes.len() as i64).execute(store.pool()).await.unwrap();
    let before: i64 =
        sqlx::query_scalar("SELECT pg_total_relation_size('context_material_versions')::bigint")
            .fetch_one(store.pool())
            .await
            .unwrap();
    store.initialize().await.unwrap();
    let after: i64 = sqlx::query_scalar("SELECT (pg_total_relation_size('context_material_version_records')+pg_total_relation_size('context_content_blobs')+pg_total_relation_size('context_content_chunks'))::bigint")
        .fetch_one(store.pool()).await.unwrap();
    assert!(
        after * 100 <= before * 110,
        "single-revision binary storage expanded: {before} -> {after}"
    );
    let restored: Vec<u8> = sqlx::query_scalar("SELECT content FROM context_material_versions")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(restored, bytes);
    eprintln!("single-version binary: old_total={before}, shared_total={after}");
}

#[tokio::test]
async fn history_upgrade_preserves_all_old_values_rolls_back_and_shares_small_edits() {
    let _guard = LOCK.lock().await;
    let store = store(true).await;
    let original = body();
    let _dir = seed(&store, original.as_bytes()).await;
    let scope: ContextScope = "personal".parse().unwrap();
    let mut previous = original.clone();
    for revision in 1..=12 {
        let next = format!("{}\r\n새 줄 {revision} 🙂\r\n", previous);
        store
            .edit_context(
                &scope,
                "note.md",
                revision,
                &digest(previous.as_bytes()),
                &next,
            )
            .await
            .unwrap();
        previous = next;
    }
    // An old tombstone and its import history remain readable at the SQL boundary.
    sqlx::raw_sql("WITH i AS (SELECT gen_random_uuid() id) INSERT INTO context_apply_batches(apply_id,store_id,core_run_id,prepared_run_digest,candidate_digest,expected_source_versions,context_targets,core_apply_attempt_id,expected_batch_id,expected_journal_locator,state) SELECT i.id,s.store_id,'synthetic-history',repeat('a',64),repeat('b',64),'{}','[{}]',i.id::text,i.id::text,i.id::text,'pending' FROM i CROSS JOIN context_store s; UPDATE context_materials SET content=''::bytea,content_digest=encode(sha256(''::bytea),'hex'),byte_len=0,search_text=NULL,deleted=true,last_apply_id=(SELECT apply_id FROM context_apply_batches) WHERE path='empty.md'; UPDATE context_apply_batches SET state='finalized',actual_batch_id=expected_batch_id,actual_journal_locator=expected_journal_locator,commit_receipt='{\"synthetic_fixture\":true}',final_core_receipt_digest=repeat('c',64);")
        .execute(store.pool()).await.unwrap();
    let names = tables(&store).await;
    let before = snapshot(&store, &names).await;
    let physical_before: i64 =
        sqlx::query_scalar("SELECT pg_total_relation_size('context_material_versions')::bigint")
            .fetch_one(store.pool())
            .await
            .unwrap();
    // Fail after backfill and the equality check, at removal of the old heap.
    sqlx::raw_sql("CREATE FUNCTION history_test_abort_drop() RETURNS event_trigger LANGUAGE plpgsql AS $$ BEGIN IF to_regclass('context_material_versions_legacy') IS NOT NULL THEN IF NOT EXISTS(SELECT 1 FROM context_content_blobs) THEN RAISE EXCEPTION 'fixture did not reach backfill'; END IF; RAISE EXCEPTION 'injected failure after backfill'; END IF; END $$; CREATE EVENT TRIGGER history_test_abort ON ddl_command_start WHEN TAG IN ('DROP TABLE') EXECUTE FUNCTION history_test_abort_drop();")
        .execute(store.pool()).await.unwrap();
    assert_eq!(store.initialize().await, Err(Error::Baseline));
    assert_eq!(snapshot(&store, &names).await, before);
    let rolled_back: bool = sqlx::query_scalar("SELECT to_regclass('context_content_chunks') IS NULL AND to_regclass('context_material_version_records') IS NULL AND NOT EXISTS(SELECT 1 FROM ontology_migrations WHERE name='020-context-history-sharing.sql')")
        .fetch_one(store.pool()).await.unwrap();
    assert!(rolled_back);
    sqlx::raw_sql(
        "DROP EVENT TRIGGER history_test_abort; DROP FUNCTION history_test_abort_drop();",
    )
    .execute(store.pool())
    .await
    .unwrap();
    let started = Instant::now();
    store.initialize().await.unwrap();
    assert_eq!(snapshot(&store, &names).await, before);
    let physical_after: i64 = sqlx::query_scalar("SELECT (pg_total_relation_size('context_material_version_records')+pg_total_relation_size('context_content_blobs')+pg_total_relation_size('context_content_chunks'))::bigint")
        .fetch_one(store.pool()).await.unwrap();
    assert!(
        physical_after * 100 < physical_before * 70,
        "{physical_before} -> {physical_after}"
    );
    eprintln!(
        "history fixture: old_total={physical_before}, shared_total={physical_after}, upgrade_ms={}",
        started.elapsed().as_millis()
    );
    let all_names = tables(&store).await;
    let all_before = snapshot(&store, &all_names).await;
    store.initialize().await.unwrap();
    assert_eq!(snapshot(&store, &all_names).await, all_before);
    assert_eq!(snapshot(&store, &names).await, before);
    let timeout: String = sqlx::query_scalar("SHOW statement_timeout")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(timeout, "5s");
    let getter: String = sqlx::query_scalar(
        "SELECT pg_get_functiondef('context_read_content(uuid,text)'::regprocedure)",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    sqlx::query("CREATE SEQUENCE history_test_reads")
        .execute(store.pool())
        .await
        .unwrap();
    // This DDL comes from the migration installed in this owned synthetic DB;
    // the replacement adds only a literal counter to its function body.
    sqlx::raw_sql(sqlx::AssertSqlSafe(getter.replacen(
        "BEGIN",
        "BEGIN PERFORM nextval('history_test_reads');",
        1,
    )))
    .execute(store.pool())
    .await
    .unwrap();
    let restored = store
        .read_context_revision(&scope, "note.md", 1)
        .await
        .unwrap();
    assert_eq!(restored["content"].as_str().unwrap(), original);
    let reads: i64 = sqlx::query_scalar("SELECT last_value FROM history_test_reads")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(
        reads, 1,
        "the public history read must reconstruct its body once"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(getter))
        .execute(store.pool())
        .await
        .unwrap();
    sqlx::query("DROP SEQUENCE history_test_reads")
        .execute(store.pool())
        .await
        .unwrap();
    // Metadata-only history queries must not need content reconstruction at all.
    // Roll back the failing getter so later test binaries keep the real function,
    // regardless of which storage test acquired LOCK last.
    let mut tx = store.pool().begin().await.unwrap();
    sqlx::raw_sql("CREATE OR REPLACE FUNCTION context_read_content(material uuid,sha text) RETURNS bytea LANGUAGE plpgsql STABLE STRICT AS $$ BEGIN RAISE EXCEPTION 'metadata query reconstructed a body'; END $$;")
        .execute(&mut *tx).await.unwrap();
    let listed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context_material_versions WHERE revision>0 AND byte_len>=0",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(listed, 16);
    assert_eq!(
        store
            .read_context_revision(&scope, "note.md", 1)
            .await
            .unwrap()["content"]
            .as_str()
            .unwrap(),
        original
    );
}

#[tokio::test]
async fn shared_history_keeps_exact_boundaries_and_atomic_new_versions() {
    let _guard = LOCK.lock().await;
    let store = store(false).await;
    let original = body();
    let _dir = seed(&store, original.as_bytes()).await;
    let material: String =
        sqlx::query_scalar("SELECT material_id::text FROM context_materials WHERE path='note.md'")
            .fetch_one(store.pool())
            .await
            .unwrap();
    let mut variants = vec![
        Vec::new(),
        vec![0; 511],
        vec![255; 512],
        vec![b'x'; 4096],
        vec![b'y'; 4097],
        original.as_bytes().to_vec(),
    ];
    variants.push(
        original
            .replacen("0100 ", "新しい行🙂\r\n0100 ", 1)
            .into_bytes(),
    );
    variants.push(original.replacen("0200 ", "", 1).into_bytes());
    variants.push((0..16 * 1024 * 1024).map(|i| (i % 256) as u8).collect());
    variants.push(noisy_bytes());
    for bytes in &variants {
        let start = Instant::now();
        let sha: String = sqlx::query_scalar("SELECT context_store_content($1::uuid,$2)")
            .bind(&material)
            .bind(bytes)
            .fetch_one(store.pool())
            .await
            .unwrap();
        let restored: Vec<u8> = sqlx::query_scalar("SELECT context_read_content($1::uuid,$2)")
            .bind(&material)
            .bind(&sha)
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(&restored, bytes);
        assert_eq!(sha, digest(bytes));
        eprintln!(
            "history boundary bytes={} put_and_get_ms={}",
            bytes.len(),
            start.elapsed().as_millis()
        );
    }
    let missing = sqlx::query(
        "INSERT INTO context_content_blobs VALUES($1::uuid,repeat('a',64),1,ARRAY[repeat('b',64)])",
    )
    .bind(&material)
    .execute(store.pool())
    .await;
    assert!(missing.is_err());
    for statement in [
        "UPDATE context_content_chunks SET content=content",
        "DELETE FROM context_content_chunks",
        "UPDATE context_content_blobs SET byte_len=byte_len",
        "DELETE FROM context_content_blobs",
        "UPDATE context_material_versions SET recorded_at=now()",
        "DELETE FROM context_material_versions",
    ] {
        assert!(
            sqlx::query(statement).execute(store.pool()).await.is_err(),
            "{statement}"
        );
    }
    let scope: ContextScope = "personal".parse().unwrap();
    let names = tables(&store).await;
    let before = snapshot(&store, &names).await;
    sqlx::raw_sql("CREATE FUNCTION history_test_reject_version() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected version failure'; END $$; CREATE TRIGGER history_test_reject BEFORE INSERT ON context_material_version_records FOR EACH ROW EXECUTE FUNCTION history_test_reject_version();")
        .execute(store.pool()).await.unwrap();
    assert_eq!(
        store
            .edit_context(
                &scope,
                "note.md",
                1,
                &digest(original.as_bytes()),
                "# Changed\nUnshared bytes"
            )
            .await,
        Err(Error::Storage)
    );
    assert_eq!(snapshot(&store, &names).await, before);
    sqlx::raw_sql("DROP TRIGGER history_test_reject ON context_material_version_records; DROP FUNCTION history_test_reject_version();")
        .execute(store.pool()).await.unwrap();
    let next = original.replacen("0300 ", "0300 replaced ", 1);
    store
        .edit_context(&scope, "note.md", 1, &digest(original.as_bytes()), &next)
        .await
        .unwrap();
    let rows = sqlx::query("SELECT revision,content,content_digest,byte_len FROM context_material_versions WHERE material_id=$1::uuid ORDER BY revision")
        .bind(&material).fetch_all(store.pool()).await.unwrap();
    assert_eq!(rows.len(), 2);
    for (row, bytes) in rows.iter().zip([original.as_bytes(), next.as_bytes()]) {
        assert_eq!(row.get::<Vec<u8>, _>("content"), bytes);
        assert_eq!(row.get::<String, _>("content_digest"), digest(bytes));
        assert_eq!(row.get::<i64, _>("byte_len"), bytes.len() as i64);
    }
    // Equal byte sequences in a different material never share private references.
    let empty_material: String =
        sqlx::query_scalar("SELECT material_id::text FROM context_materials WHERE path='empty.md'")
            .fetch_one(store.pool())
            .await
            .unwrap();
    let other: String = sqlx::query_scalar("SELECT context_store_content($1::uuid,$2)")
        .bind(empty_material)
        .bind(original.as_bytes())
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(other, digest(original.as_bytes()));
    assert!(
        sqlx::query("SELECT context_read_content($1::uuid,repeat('f',64))")
            .bind(&material)
            .execute(store.pool())
            .await
            .is_err()
    );
}
