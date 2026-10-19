use ontology::{
    domain::{ImportedRecord, Scope, SourceKind},
    memory::BrainCommand,
    store::{Store, digest},
};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, time::Duration};

async fn call(store: &Store, value: Value) -> Value {
    store
        .brain(serde_json::from_value::<BrainCommand>(value).unwrap())
        .await
        .unwrap()
}

async fn listener_pid(store: &Store) -> i32 {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let pid = sqlx::query_scalar::<_, i32>("SELECT pid FROM pg_stat_activity WHERE application_name='classification_queue_events_test' AND query LIKE 'LISTEN %' AND state='idle' LIMIT 1")
                .fetch_optional(store.pool()).await.unwrap();
            if let Some(pid) = pid { return pid; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("classification listener registered before drain")
}

async fn settled(worker: &Store) -> u64 {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let before = worker.calls();
            tokio::time::sleep(Duration::from_millis(100)).await;
            if before >= 8 && before == worker.calls() {
                return before;
            }
        }
    })
    .await
    .expect("both queue workers finished startup/notifications")
}

async fn states(store: &Store, memory: &str, document: &str) -> (String, String) {
    sqlx::query_as("SELECT (SELECT state FROM memory_grouping WHERE memory_id=$1),(SELECT state FROM document_grouping WHERE source_id=$2 AND scope='personal')")
        .bind(memory).bind(document).fetch_one(store.pool()).await.unwrap()
}

async fn wait_states(store: &Store, memory: &str, document: &str, state: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while states(store, memory, document).await != (state.into(), state.into()) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("both durable queues progressed from committed work")
}

async fn pending(store: &Store, memory: &str, document: &str) {
    let mut tx = store.pool().begin().await.unwrap();
    sqlx::query("UPDATE memory_grouping SET mode='auto',state='pending',attempts=0,claim_token=NULL,lease_until=NULL WHERE memory_id=$1")
        .bind(memory).execute(&mut *tx).await.unwrap();
    sqlx::query("UPDATE document_grouping SET mode='auto',state='pending',attempts=0,claim_token=NULL,lease_until=NULL WHERE scope='personal' AND source_id=$1")
        .bind(document).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
}

async fn wait_blocked(store: &Store, blocker: i32, count: i64) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let n: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE $1=ANY(pg_blocking_pids(pid))",
            )
            .bind(blocker)
            .fetch_one(store.pool())
            .await
            .unwrap();
            if n >= count {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("workers reached actual producer lock waits");
}

#[tokio::test]
async fn classification_queues_wait_for_commits_and_real_lease_deadlines() {
    let url = std::env::var("TEST_DATABASE_URL").expect("explicit isolated DB");
    assert!(
        ontology::config::database_options(&url)
            .unwrap()
            .get_database()
            .is_some_and(|n| n.starts_with("ontology_test_"))
    );
    let store = Store::connect(&url).await.unwrap();
    store.initialize().await.unwrap();
    sqlx::query("TRUNCATE curation_reviews,memory_grouping,document_grouping,document_grouping_dirty,evidence_snapshots,evidence_contents,memory_history,memories,memory_creations,subjects RESTART IDENTITY CASCADE")
        .execute(store.pool()).await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let binary = temp.path().join("codex");
    let answer = json!({"type":"item.completed","item":{"type":"agent_message","text":json!({"decision":"unmatched","subject_id":null,"candidate_ids":[],"new_subject":null,"reason":"합성 분류 알림 검증"}).to_string()}});
    let started = temp.path().join("started");
    let release = temp.path().join("release");
    let quote = |p: &std::path::Path| format!("'{}'", p.to_str().unwrap().replace('\'', "'\\''"));
    std::fs::write(
        &binary,
        format!("#!/bin/sh\ncat >/dev/null\ntouch {}\nwhile [ ! -f {} ]; do sleep 0.01; done\nprintf '%s\\n' '{}'\n", quote(&started), quote(&release), answer),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    unsafe {
        std::env::set_var("ONTOLOGY_CODEX_BINARY", &binary);
    }
    let sep = if url.contains('?') { '&' } else { '?' };
    let worker_store = Store::connect(&format!(
        "{url}{sep}application_name=classification_queue_events_test"
    ))
    .await
    .unwrap();
    let mut worker = tokio::spawn(ontology::grouping::run_loop(worker_store.clone()));
    listener_pid(&store).await;
    let idle = settled(&worker_store).await;
    tokio::time::sleep(Duration::from_secs(31)).await;
    assert_eq!(
        worker_store.calls(),
        idle,
        "both healthy empty queues issue zero SQL over31s"
    );

    let memory = call(&store,json!({"op":"remember","scope":"personal","idempotency_key":"queue-event-memory","memory":{"kind":"record","title":"합성 알림","body":"합성 분류 자료","subject_id":null,"evidence":[]}})).await;
    let memory = memory["id"].as_str().unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !started.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("memory provider reached its deterministic gate");
    let hash = digest(b"queue-event-document");
    let document = format!("e_{hash}");
    store
        .apply_import(&[ImportedRecord {
            source_id: format!("s_{hash}"),
            entity_id: document.clone(),
            scope: Scope::Personal,
            repository: "/synthetic/classification-events".into(),
            path: "queue.md".into(),
            kind: SourceKind::Git,
            source_revision: "a".repeat(40),
            digest: Some(digest(b"synthetic queue document")),
            content: Some("synthetic queue document".into()),
        }])
        .await
        .unwrap();
    store.discover_document_grouping().await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while states(&store, memory, &document).await != ("processing".into(), "unmatched".into()) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("document worker progresses while memory's provider is blocked");
    std::fs::write(&release, b"release").unwrap();
    wait_states(&store, memory, &document, "unmatched").await;
    settled(&worker_store).await;

    // Rolled-back queue writes produce neither notices nor model work.
    let before = worker_store.calls();
    let mut tx = store.pool().begin().await.unwrap();
    sqlx::query("UPDATE memory_grouping SET state='pending',attempts=0 WHERE memory_id=$1")
        .bind(memory)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE document_grouping SET state='pending',attempts=0 WHERE source_id=$1")
        .bind(&document)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(worker_store.calls(), before);
    assert_eq!(
        states(&store, memory, &document).await,
        ("unmatched".into(), "unmatched".into())
    );

    // No-op queue updates and zero-row claim cleanup cannot self-notify forever.
    sqlx::query("UPDATE memory_grouping SET updated_at=now() WHERE memory_id=$1")
        .bind(memory)
        .execute(store.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE document_grouping SET updated_at=now() WHERE source_id=$1")
        .bind(&document)
        .execute(store.pool())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(worker_store.calls(), before);
    pending(&store, memory, &document).await;
    wait_states(&store, memory, &document, "unmatched").await;

    // Already committed pending rows must survive a locking producer's rollback.
    worker.abort();
    let _ = worker.await;
    pending(&store, memory, &document).await;
    let mut locked = store.pool().begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *locked)
        .await
        .unwrap();
    sqlx::query("SELECT memory_id FROM memory_grouping WHERE memory_id=$1 FOR UPDATE")
        .bind(memory)
        .execute(&mut *locked)
        .await
        .unwrap();
    sqlx::query("SELECT source_id FROM document_grouping WHERE source_id=$1 FOR UPDATE")
        .bind(&document)
        .execute(&mut *locked)
        .await
        .unwrap();
    worker = tokio::spawn(ontology::grouping::run_loop(worker_store.clone()));
    listener_pid(&store).await;
    wait_blocked(&store, pid, 2).await;
    locked.rollback().await.unwrap();
    wait_states(&store, memory, &document, "unmatched").await;

    // An exclusive gate released by rollback emits no notice either.
    worker.abort();
    let _ = worker.await;
    pending(&store, memory, &document).await;
    let mut gate = store.pool().begin().await.unwrap();
    let pid: i32 =
        sqlx::query_scalar("SELECT pg_backend_pid() FROM pg_advisory_xact_lock(478310003)")
            .fetch_one(&mut *gate)
            .await
            .unwrap();
    worker = tokio::spawn(ontology::grouping::run_loop(worker_store.clone()));
    listener_pid(&store).await;
    wait_blocked(&store, pid, 2).await;
    gate.rollback().await.unwrap();
    wait_states(&store, memory, &document, "unmatched").await;
    settled(&worker_store).await;

    // A committed pending native batch parks workers until its state changes.
    let before_pending = worker_store.calls();
    sqlx::query("INSERT INTO context_apply_batches(apply_id,store_id,core_run_id,prepared_run_digest,candidate_digest,expected_source_versions,context_targets,core_apply_attempt_id,expected_batch_id,expected_journal_locator,state) SELECT gen_random_uuid(),store_id,'synthetic-queue',repeat('a',64),repeat('b',64),'{}','[{}]','synthetic-queue','synthetic-queue','synthetic-queue','pending' FROM context_store").execute(store.pool()).await.unwrap();
    pending(&store, memory, &document).await;
    sqlx::query("SELECT document_grouping_invalidate('personal',NULL,$1)")
        .bind(&document)
        .execute(store.pool())
        .await
        .unwrap();
    let discovery_store = Store::connect(&url).await.unwrap();
    let discovery = tokio::spawn(ontology::document_grouping::run_discovery_loop(
        discovery_store.clone(),
    ));
    tokio::time::timeout(Duration::from_secs(10), async {
        while discovery_store.calls() < 3 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let discovery_blocked = discovery_store.calls();
    tokio::time::timeout(Duration::from_secs(10), async {
        while worker_store.calls() < before_pending + 6 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let blocked = settled(&worker_store).await;
    // Span the old5s recovery retry to distinguish an event wait from polling.
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(
        worker_store.calls(),
        blocked,
        "native recovery waits for its own event"
    );
    assert_eq!(
        discovery_store.calls(),
        discovery_blocked,
        "discovery also parks on committed native recovery"
    );

    assert_eq!(
        states(&store, memory, &document).await,
        ("pending".into(), "pending".into())
    );
    sqlx::query(
        "UPDATE context_apply_batches SET state='aborted' WHERE core_run_id='synthetic-queue'",
    )
    .execute(store.pool())
    .await
    .unwrap();
    wait_states(&store, memory, &document, "unmatched").await;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM document_grouping_dirty WHERE scope='personal' AND source_id=$1")
                .bind(&document).fetch_one(store.pool()).await.unwrap();
            if remaining == 0 { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("native state event resumes discovery's durable backlog");
    discovery_store.pool().close().await;
    tokio::time::timeout(Duration::from_secs(10), discovery)
        .await
        .unwrap()
        .unwrap();

    // The exact lease deadline, without another write, recovers abandoned work.
    for attempts in [1, 3] {
        let mut tx = store.pool().begin().await.unwrap();
        sqlx::query("UPDATE memory_grouping SET state='processing',claim_token=gen_random_uuid(),lease_until=clock_timestamp()+interval '1 second',attempts=$2 WHERE memory_id=$1").bind(memory).bind(attempts).execute(&mut *tx).await.unwrap();
        sqlx::query("UPDATE document_grouping SET state='processing',claim_token=gen_random_uuid(),lease_until=clock_timestamp()+interval '1 second',attempts=$2 WHERE source_id=$1").bind(&document).bind(attempts).execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        settled(&worker_store).await;
        assert_eq!(
            states(&store, memory, &document).await,
            ("processing".into(), "processing".into())
        );
        wait_states(
            &store,
            memory,
            &document,
            if attempts == 3 { "error" } else { "unmatched" },
        )
        .await;
    }
    // A new off/manual decision cancels a previously armed deadline.
    sqlx::query("UPDATE memory_grouping SET state='processing',claim_token=gen_random_uuid(),lease_until=clock_timestamp()+interval '1 second' WHERE memory_id=$1").bind(memory).execute(store.pool()).await.unwrap();
    sqlx::query("UPDATE document_grouping SET state='processing',claim_token=gen_random_uuid(),lease_until=clock_timestamp()+interval '1 second' WHERE source_id=$1").bind(&document).execute(store.pool()).await.unwrap();
    settled(&worker_store).await;
    sqlx::query("UPDATE memory_grouping SET mode='off',state='off',claim_token=NULL,lease_until=NULL WHERE memory_id=$1").bind(memory).execute(store.pool()).await.unwrap();
    sqlx::query("UPDATE document_grouping SET mode='off',state='off',claim_token=NULL,lease_until=NULL WHERE source_id=$1").bind(&document).execute(store.pool()).await.unwrap();
    let cancelled = settled(&worker_store).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(
        worker_store.calls(),
        cancelled,
        "cancelled lease causes no future query"
    );

    // Reconnection drains durable work; restart also catches notices sent offline.
    let old_listener = listener_pid(&store).await;
    sqlx::query("SELECT pg_terminate_backend($1)")
        .bind(old_listener)
        .execute(store.pool())
        .await
        .unwrap();
    pending(&store, memory, &document).await;
    wait_states(&store, memory, &document, "unmatched").await;
    assert_ne!(listener_pid(&store).await, old_listener);
    worker.abort();
    let _ = worker.await;
    pending(&store, memory, &document).await;
    worker = tokio::spawn(ontology::grouping::run_loop(worker_store.clone()));
    wait_states(&store, memory, &document, "unmatched").await;
    settled(&worker_store).await;
    worker_store.pool().close().await;
    tokio::time::timeout(Duration::from_secs(10), worker)
        .await
        .expect("closed listener ends both owned workers")
        .unwrap();
    unsafe {
        std::env::remove_var("ONTOLOGY_CODEX_BINARY");
    }
}
