use ontology::{
    context::{ContextScope, inventory},
    context_importer::ContextReader,
    domain::{Error, ImportedRecord, Scope, SourceKind},
    memory::BrainCommand,
    store::{Store, digest},
};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, path::Path, time::Duration};

async fn call(store: &Store, value: Value) -> Value {
    store
        .brain(serde_json::from_value::<BrainCommand>(value).unwrap())
        .await
        .unwrap()
}
fn git_record(label: &str, body: &str, revision: &str) -> ImportedRecord {
    let hash = digest(label.as_bytes());
    ImportedRecord {
        source_id: format!("s_{hash}"),
        entity_id: format!("e_{hash}"),
        scope: Scope::Personal,
        repository: "/synthetic/document-grouping".into(),
        path: format!("{label}.md"),
        kind: SourceKind::Git,
        source_revision: revision.into(),
        digest: Some(digest(body.as_bytes())),
        content: Some(body.into()),
    }
}
fn quoted(path: &Path) -> String {
    format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''"))
}
fn provider(binary: &Path, capture: &Path, answer: Value, gate: Option<&Path>, fails: bool) {
    let output =
        json!({"type":"item.completed","item":{"type":"agent_message","text":answer.to_string()}})
            .to_string();
    let wait = gate
        .map(|g| format!("while [ ! -f {} ]; do sleep 0.01; done\n", quoted(g)))
        .unwrap_or_default();
    let result = if fails {
        "exit 1\n".into()
    } else {
        format!("printf '%s\\n' '{}'\n", output)
    };
    std::fs::write(
        binary,
        format!("#!/bin/sh\ncat >{}\n{wait}{result}", quoted(capture)),
    )
    .unwrap();
    std::fs::set_permissions(binary, std::fs::Permissions::from_mode(0o700)).unwrap();
}
fn judgment(decision: &str, subject: Option<&str>, candidates: Vec<&str>) -> Value {
    json!({"decision":decision,"subject_id":subject,"candidate_ids":candidates,"new_subject":null,"reason":"합성 문서의 주된 목적을 정의와 비교함"})
}
async fn wait_capture(path: &Path) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !std::fs::metadata(path).is_ok_and(|m| m.len() > 0) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("provider received full source snapshot");
}
async fn membership(store: &Store, scope: &str, identity: &Value) -> Value {
    call(
        store,
        json!({"op":"document-subject","scope":scope,"document":identity}),
    )
    .await
}
async fn retry(store: &Store, scope: &str, identity: &Value) -> Value {
    let current = membership(store, scope, identity).await;
    call(store,json!({"op":"document-grouping-retry","scope":scope,"document":{"identity":identity,"revision":current["revision"],"source_revision":current["current_source"]["source_revision"],"content_digest":current["current_source"]["content_digest"]}})).await
}
async fn manual(store: &Store, scope: &str, identity: &Value, subject: Option<&Value>) -> Value {
    let current = membership(store, scope, identity).await;
    call(store,json!({"op":"document-subject-set","scope":scope,"document":{"identity":identity,"revision":current["revision"],"source_revision":current["current_source"]["source_revision"],"content_digest":current["current_source"]["content_digest"],"subject_id":subject.map(|s|s["id"].clone()),"subject_revision":subject.map(|s|s["revision"].clone()),"reason":"합성 수동 목적 확인"}})).await
}
async fn import_native(store: &Store, root: &Path) {
    let scopes: Vec<ContextScope> = vec!["personal".parse().unwrap()];
    let manifest = inventory(root, &scopes).unwrap();
    store
        .import_context(root, &scopes, &manifest.inventory_digest)
        .await
        .unwrap();
}
async fn material(store: &Store, path: &str) -> Value {
    let id: String = sqlx::query_scalar(
        "SELECT material_id::text FROM context_materials WHERE scope='personal' AND path=$1",
    )
    .bind(path)
    .fetch_one(store.pool())
    .await
    .unwrap();
    json!({"material_id":id})
}
async fn preserved_sources(store: &Store) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object('originals',(SELECT COALESCE(jsonb_agg(to_jsonb(m) ORDER BY material_id),'[]') FROM context_materials m),'git',(SELECT COALESCE(jsonb_agg(to_jsonb(p) ORDER BY entity_id),'[]') FROM source_records p),'memberships',(SELECT COALESCE(jsonb_agg(to_jsonb(d) ORDER BY scope,source_id),'[]') FROM document_subjects d),'history',(SELECT COALESCE(jsonb_agg(to_jsonb(h) ORDER BY scope,source_id,revision),'[]') FROM document_subject_history h))")
        .fetch_one(store.pool()).await.unwrap()
}
async fn queue(store: &Store, key: &str) -> Value {
    sqlx::query_scalar(
        "SELECT to_jsonb(g) FROM document_grouping g WHERE scope='personal' AND source_id=$1",
    )
    .bind(key)
    .fetch_one(store.pool())
    .await
    .unwrap()
}
fn paired_provider(binary: &Path, dir: &Path, answer: Value, old_fails: bool) {
    std::fs::create_dir(dir).unwrap();
    let output =
        json!({"type":"item.completed","item":{"type":"agent_message","text":answer.to_string()}})
            .to_string();
    std::fs::write(dir.join("answer"), output).unwrap();
    let first = if old_fails {
        "exit 1".into()
    } else {
        format!("cat {}", quoted(&dir.join("answer")))
    };
    std::fs::write(binary,format!("#!/bin/sh\ncat >/dev/null\nif mkdir {} 2>/dev/null; then\n while [ ! -f {} ]; do sleep 0.01; done\n {first}\nelse\n mkdir {}\n while [ ! -f {} ]; do sleep 0.01; done\n cat {}\nfi\n",quoted(&dir.join("old-started")),quoted(&dir.join("old-release")),quoted(&dir.join("new-started")),quoted(&dir.join("new-release")),quoted(&dir.join("answer")))).unwrap();
    std::fs::set_permissions(binary, std::fs::Permissions::from_mode(0o700)).unwrap();
}
async fn wait_marker(path: &Path) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("provider reached deterministic gate");
}
async fn wait_blocked(store: &Store, blocker: i32, count: i64) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE $1=ANY(pg_blocking_pids(pid))",
            )
            .bind(blocker)
            .fetch_one(store.pool())
            .await
            .unwrap();
            if waiting >= count {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("ordered transactions reached their lock wait");
}
async fn wait_expired_blocker(store: &Store, key: &str, blocker: i32) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1=ANY(pg_blocking_pids(pid)))").bind(blocker).fetch_one(store.pool()).await.unwrap();
            if waiting {break;}
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        loop {
            let expired:bool=sqlx::query_scalar("SELECT lease_until<clock_timestamp() FROM document_grouping WHERE scope='personal' AND source_id=$1").bind(key).fetch_one(store.pool()).await.unwrap();
            if expired {break;}
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("transaction waited across real lease expiration");
}

#[tokio::test]
async fn documents_discover_current_identity_and_preserve_intent_and_claims() {
    let url = std::env::var("TEST_DATABASE_URL").expect("explicit disposable PostgreSQL");
    assert!(
        ontology::config::database_options(&url)
            .unwrap()
            .get_database()
            .unwrap()
            .starts_with("ontology_test_")
    );
    let store = Store::connect(&url).await.unwrap();
    store.initialize().await.unwrap();
    sqlx::raw_sql("TRUNCATE document_grouping,document_subjects,document_subject_history,subject_history,context_source_bindings,context_projection_versions,context_material_versions,context_manual_edits,context_materials,context_apply_batches,memories,memory_creations,subjects,confirmation_history,related_materials,entity_areas,entity_topics,source_records,entities,sources,topics RESTART IDENTITY CASCADE").execute(store.pool()).await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let binary = temp.path().join("codex");
    let capture = temp.path().join("input");
    unsafe {
        std::env::set_var("ONTOLOGY_CODEX_BINARY", &binary);
    }
    let definition = json!({"purpose":"합성 연구자료 검토","include":"이 합성 목적을 주로 다루는 자료","exclude":"다른 목적의 자료"});
    let group=call(&store,json!({"op":"subject-create","scope":"personal","idempotency_key":"document-grouping-defined","name":"합성 연구","definition":definition})).await;
    let group_id = group["id"].as_str().unwrap();
    let undefined=call(&store,json!({"op":"subject-create","scope":"personal","idempotency_key":"document-grouping-undefined","name":"정의 없는 합성 그룹"})).await;
    let root = temp.path().join("vault");
    std::fs::create_dir_all(root.join("personal/notes")).unwrap();
    std::fs::write(
        root.join("personal/notes/legacy.md"),
        "# 기존 합성 원문\n기존 수동 결정을 보존한다\n",
    )
    .unwrap();
    let root = root.canonicalize().unwrap();
    import_native(&store, &root).await;
    let legacy = material(&store, "notes/legacy.md").await;
    manual(&store, "personal", &legacy, Some(&group)).await;
    let old_git = git_record("legacy-git", "기존 미분류 원문", &"a".repeat(40));
    store
        .apply_import(std::slice::from_ref(&old_git))
        .await
        .unwrap();
    let preserved = preserved_sources(&store).await;
    // Exact pre-015 upgrade fixture: preserve originals and decisions, baseline
    // both app scopes even when a company binding has not been created yet.
    sqlx::raw_sql("ALTER TABLE memory_history DROP COLUMN grouping_only; DELETE FROM ontology_migrations WHERE name='016-memory-grouping-history.sql'; DROP TABLE document_grouping; DELETE FROM ontology_migrations WHERE name='015-document-grouping.sql'").execute(store.pool()).await.unwrap();
    store.initialize().await.unwrap();
    assert_eq!(preserved_sources(&store).await, preserved);
    assert_eq!(
        membership(&store, "personal", &legacy).await["grouping"]["mode"],
        "manual"
    );
    assert_eq!(
        membership(&store, "personal", &json!({"entity_id":old_git.entity_id})).await["grouping"]["mode"],
        "off"
    );
    let store_id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
        .fetch_one(store.pool())
        .await
        .unwrap();
    ContextReader::new()
        .import(
            &store,
            &store_id,
            &"personal".parse().unwrap(),
            &["notes/legacy.md".into()],
            Scope::Meenseek,
        )
        .await
        .unwrap();
    assert_eq!(
        membership(&store, "meenseek", &legacy).await["grouping"]["mode"],
        "off"
    );
    store.discover_document_grouping().await.unwrap();
    assert!(!once(&store).await.unwrap());

    let body = format!(
        "# 신규 합성 문서\nAPI_KEY=synthetic-private\n{}",
        "전체 원문 뒤쪽도 목적 판단에 필요하다 ".repeat(500)
    );
    std::fs::write(root.join("personal/notes/new.md"), &body).unwrap();
    import_native(&store, &root).await;
    let native = material(&store, "notes/new.md").await;
    provider(
        &binary,
        &capture,
        judgment("assign", Some(group_id), vec![]),
        None,
        false,
    );
    assert!(once(&store).await.unwrap());
    let assigned = membership(&store, "personal", &native).await;
    assert_eq!(assigned["subject_id"], group["id"]);
    assert_eq!(assigned["revision"], 1);
    assert_eq!(assigned["grouping"]["state"], "assigned");
    let prompt = std::fs::read_to_string(&capture).unwrap();
    assert!(!prompt.contains("synthetic-private"));
    assert!(prompt.matches("전체 원문 뒤쪽").count() == 500);
    let snapshot = preserved_sources(&store).await;
    Store::connect(&url)
        .await
        .unwrap()
        .discover_document_grouping()
        .await
        .unwrap();
    assert!(!once(&store).await.unwrap());
    assert_eq!(preserved_sources(&store).await, snapshot);

    // Same-byte projection loss/restoration is not a new classification, and
    // cannot turn an expired-attempt error back into pending without consent.
    let native_id = native["material_id"].as_str().unwrap();
    let payload:Value=sqlx::query_scalar("SELECT payload FROM context_projection_versions p JOIN context_materials m USING(material_id,revision) WHERE material_id=$1::uuid").bind(native_id).fetch_one(store.pool()).await.unwrap();
    for state in ["assigned", "error"] {
        sqlx::query("UPDATE document_grouping SET state=$2,attempts=CASE WHEN $2='error' THEN 3 ELSE attempts END WHERE scope='personal' AND source_id=$1").bind(native_id).bind(state).execute(store.pool()).await.unwrap();
        let before: Value = sqlx::query_scalar(
            "SELECT to_jsonb(g) FROM document_grouping g WHERE scope='personal' AND source_id=$1",
        )
        .bind(native_id)
        .fetch_one(store.pool())
        .await
        .unwrap();
        sqlx::raw_sql("ALTER TABLE context_projection_versions DISABLE TRIGGER USER")
            .execute(store.pool())
            .await
            .unwrap();
        let excluded =
            json!({"status":"excluded","source_digest":payload["source_digest"],"terms":[]});
        sqlx::query("UPDATE context_projection_versions SET payload=$2,payload_digest=encode(sha256(convert_to($2::jsonb::text,'UTF8')),'hex') WHERE material_id=$1::uuid").bind(native_id).bind(excluded).execute(store.pool()).await.unwrap();
        store.discover_document_grouping().await.unwrap();
        sqlx::query("UPDATE context_projection_versions SET payload=$2,payload_digest=encode(sha256(convert_to($2::jsonb::text,'UTF8')),'hex') WHERE material_id=$1::uuid").bind(native_id).bind(&payload).execute(store.pool()).await.unwrap();
        sqlx::raw_sql("ALTER TABLE context_projection_versions ENABLE TRIGGER USER")
            .execute(store.pool())
            .await
            .unwrap();
        assert!(!once(&store).await.unwrap());
        let after: Value = sqlx::query_scalar(
            "SELECT to_jsonb(g) FROM document_grouping g WHERE scope='personal' AND source_id=$1",
        )
        .bind(native_id)
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!(
            after, before,
            "terminal eligibility restoration never resets attempts or reruns the model"
        );
    }

    // Losing eligibility is cancellation, never a fresh attempt budget.
    for attempts in [2, 3] {
        sqlx::query("UPDATE document_grouping SET state='pending',attempts=$2 WHERE scope='personal' AND source_id=$1").bind(native_id).bind(attempts).execute(store.pool()).await.unwrap();
        sqlx::raw_sql("ALTER TABLE context_projection_versions DISABLE TRIGGER USER")
            .execute(store.pool())
            .await
            .unwrap();
        let excluded =
            json!({"status":"excluded","source_digest":payload["source_digest"],"terms":[]});
        sqlx::query("UPDATE context_projection_versions SET payload=$2,payload_digest=encode(sha256(convert_to($2::jsonb::text,'UTF8')),'hex') WHERE material_id=$1::uuid").bind(native_id).bind(excluded).execute(store.pool()).await.unwrap();
        store.discover_document_grouping().await.unwrap();
        assert_eq!(queue(&store, native_id).await["state"], "ineligible");
        sqlx::query("UPDATE context_projection_versions SET payload=$2,payload_digest=encode(sha256(convert_to($2::jsonb::text,'UTF8')),'hex') WHERE material_id=$1::uuid").bind(native_id).bind(&payload).execute(store.pool()).await.unwrap();
        sqlx::raw_sql("ALTER TABLE context_projection_versions ENABLE TRIGGER USER")
            .execute(store.pool())
            .await
            .unwrap();
        store.discover_document_grouping().await.unwrap();
        let restored = queue(&store, native_id).await;
        assert_eq!(restored["attempts"], attempts);
        assert_eq!(
            restored["state"],
            if attempts < 3 { "pending" } else { "error" }
        );
        assert_eq!(store.document_grouping_once().await.unwrap(), attempts < 3);
        assert_eq!(queue(&store, native_id).await["attempts"], 3);
    }

    let git = git_record(
        "new-git",
        "# 신규 Git\nAPI_KEY=synthetic-git-private\n합성 연구자료",
        &"a".repeat(40),
    );
    store
        .apply_import(std::slice::from_ref(&git))
        .await
        .unwrap();
    assert!(once(&store).await.unwrap());
    assert!(
        !std::fs::read_to_string(&capture)
            .unwrap()
            .contains("synthetic-git-private")
    );
    let git_id = json!({"entity_id":git.entity_id});
    let git_assigned = membership(&store, "personal", &git_id).await;
    let mut revision_only = git.clone();
    revision_only.source_revision = "b".repeat(40);
    store.apply_import(&[revision_only.clone()]).await.unwrap();
    let stable = preserved_sources(&store).await;
    assert!(!once(&store).await.unwrap());
    assert_eq!(preserved_sources(&store).await, stable);
    assert_eq!(
        membership(&store, "personal", &git_id).await["subject_id"],
        git_assigned["subject_id"]
    );

    // Source change makes the old automatic assignment visibly stale. A
    // non-unique judgment must preserve its history instead of moving it.
    revision_only.content = Some("다른 목적의 새 본문".into());
    revision_only.digest = Some(digest(revision_only.content.as_ref().unwrap().as_bytes()));
    revision_only.source_revision = "c".repeat(40);
    store.apply_import(&[revision_only]).await.unwrap();
    provider(
        &binary,
        &capture,
        judgment("suggest", None, vec![group_id]),
        None,
        false,
    );
    assert!(once(&store).await.unwrap());
    let suggested = membership(&store, "personal", &git_id).await;
    assert_eq!(suggested["subject_id"], group["id"]);
    assert_eq!(suggested["revision"], 1);
    assert_eq!(suggested["review_needed"], true);
    assert_eq!(suggested["grouping"]["state"], "suggested");

    retry(&store, "personal", &git_id).await;
    provider(
        &binary,
        &capture,
        judgment("assign", Some(undefined["id"].as_str().unwrap()), vec![]),
        None,
        false,
    );
    let unchanged = preserved_sources(&store).await;
    assert!(once(&store).await.unwrap());
    assert_eq!(preserved_sources(&store).await, unchanged);
    assert_eq!(
        membership(&store, "personal", &git_id).await["grouping"]["state"],
        "error"
    );
    manual(&store, "personal", &git_id, None).await;
    // Null re-save is a membership no-op but still revokes explicit auto retry.
    retry(&store, "personal", &git_id).await;
    let before = preserved_sources(&store).await;
    manual(&store, "personal", &git_id, None).await;
    assert_eq!(preserved_sources(&store).await, before);
    assert!(!once(&store).await.unwrap());

    // Oversize input is reported, never silently shortened or sent to a model.
    let huge = git_record("huge-git", &"x".repeat(65_536), &"d".repeat(40));
    store
        .apply_import(std::slice::from_ref(&huge))
        .await
        .unwrap();
    std::fs::remove_file(&capture).unwrap();
    assert!(once(&store).await.unwrap());
    assert!(!capture.exists());
    let oversized = membership(&store, "personal", &json!({"entity_id":huge.entity_id})).await;
    assert_eq!(oversized["grouping"]["state"], "error");
    assert!(
        oversized["grouping"]["reason"]
            .as_str()
            .unwrap()
            .contains("자르지")
    );

    // Source+membership CAS rejects a stale manual retry without side effects.
    let result=store.brain(serde_json::from_value::<BrainCommand>(json!({"op":"document-grouping-retry","scope":"personal","document":{"identity":native,"revision":0,"source_revision":"0","content_digest":"0".repeat(64)}})).unwrap()).await;
    assert_eq!(result, Err(Error::Conflict));

    // A late projection is discovered when ready; a Context consumer binding
    // uses the same native UUID, never a second redacted Git-like copy.
    std::fs::write(
        root.join("personal/notes/late.md"),
        "# 지연 투영 합성 문서\n새 합성 목적 자료\n",
    )
    .unwrap();
    import_native(&store, &root).await;
    let late = material(&store, "notes/late.md").await;
    let late_id = late["material_id"].as_str().unwrap();
    sqlx::raw_sql("ALTER TABLE context_projection_versions DISABLE TRIGGER USER")
        .execute(store.pool())
        .await
        .unwrap();
    let projection: Value = sqlx::query_scalar(
        "DELETE FROM context_projection_versions WHERE material_id=$1::uuid RETURNING payload",
    )
    .bind(late_id)
    .fetch_one(store.pool())
    .await
    .unwrap();
    store.discover_document_grouping().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM document_grouping WHERE source_id=$1")
            .bind(late_id)
            .fetch_one(store.pool())
            .await
            .unwrap(),
        0
    );
    sqlx::query("INSERT INTO context_projection_versions(material_id,revision,payload,payload_digest) SELECT material_id,revision,$2,encode(sha256(convert_to($2::jsonb::text,'UTF8')),'hex') FROM context_materials WHERE material_id=$1::uuid").bind(late_id).bind(projection).execute(store.pool()).await.unwrap();
    sqlx::raw_sql("ALTER TABLE context_projection_versions ENABLE TRIGGER USER")
        .execute(store.pool())
        .await
        .unwrap();
    provider(
        &binary,
        &capture,
        judgment("assign", Some(group_id), vec![]),
        None,
        false,
    );
    let (a, b) = tokio::join!(
        store.discover_document_grouping(),
        store.discover_document_grouping()
    );
    a.unwrap();
    b.unwrap();
    assert!(once(&store).await.unwrap());
    assert!(!once(&store).await.unwrap());
    let company=call(&store,json!({"op":"subject-create","scope":"meenseek","idempotency_key":"document-grouping-company","name":"합성 회사 목적","definition":definition})).await;
    ContextReader::new()
        .import(
            &store,
            &store_id,
            &"personal".parse().unwrap(),
            &["notes/late.md".into()],
            Scope::Meenseek,
        )
        .await
        .unwrap();
    provider(
        &binary,
        &capture,
        judgment("assign", Some(company["id"].as_str().unwrap()), vec![]),
        None,
        false,
    );
    assert!(once(&store).await.unwrap());
    assert_eq!(
        membership(&store, "meenseek", &late).await["subject_id"],
        company["id"]
    );
    assert_eq!(
        membership(&store, "personal", &late).await["subject_id"],
        group["id"]
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM document_grouping WHERE source_id=$1")
            .bind(late_id)
            .fetch_one(store.pool())
            .await
            .unwrap(),
        2,
        "one canonical decision per app scope"
    );
    provider(
        &binary,
        &capture,
        judgment("assign", Some(group_id), vec![]),
        None,
        false,
    );

    // Changing any competing definition invalidates the whole decision set,
    // even if the selected definition itself did not change.
    retry(&store, "personal", &late).await;
    let gate = temp.path().join("definition-release");
    let captured = temp.path().join("definition-input");
    provider(
        &binary,
        &captured,
        judgment("assign", Some(group_id), vec![]),
        Some(&gate),
        false,
    );
    let worker = {
        let store = store.clone();
        tokio::spawn(async move { once(&store).await.unwrap() })
    };
    wait_capture(&captured).await;
    call(&store,json!({"op":"subject-define","scope":"personal","id":undefined["id"],"revision":undefined["revision"],"name":"명확해진 경쟁 목적","definition":definition})).await;
    let before = preserved_sources(&store).await;
    std::fs::write(&gate, b"").unwrap();
    assert!(worker.await.unwrap());
    assert_eq!(preserved_sources(&store).await, before);
    assert_eq!(
        membership(&store, "personal", &late).await["grouping"]["state"],
        "pending"
    );
    provider(
        &binary,
        &capture,
        judgment("assign", Some(group_id), vec![]),
        None,
        false,
    );
    assert!(once(&store).await.unwrap());

    // Canonical source drift revokes a running judgment without touching the
    // previous decision; a later valid unmatched judgment stays reviewable.
    retry(&store, "personal", &git_id).await;
    let gate = temp.path().join("source-release");
    let captured = temp.path().join("source-input");
    provider(
        &binary,
        &captured,
        judgment("assign", Some(group_id), vec![]),
        Some(&gate),
        false,
    );
    let worker = {
        let store = store.clone();
        tokio::spawn(async move { once(&store).await.unwrap() })
    };
    wait_capture(&captured).await;
    let changed_git = git_record(
        "new-git",
        "원문이 모델 처리 중 다시 변경됨",
        &"e".repeat(40),
    );
    store.apply_import(&[changed_git]).await.unwrap();
    store.discover_document_grouping().await.unwrap();
    let before = preserved_sources(&store).await;
    std::fs::write(&gate, b"").unwrap();
    assert!(worker.await.unwrap());
    assert_eq!(preserved_sources(&store).await, before);
    provider(
        &binary,
        &capture,
        judgment("unmatched", None, vec![]),
        None,
        false,
    );
    assert!(once(&store).await.unwrap());
    assert_eq!(
        membership(&store, "personal", &git_id).await["grouping"]["state"],
        "unmatched"
    );

    // A provider already reading a current snapshot cannot override manual/off.
    retry(&store, "personal", &native).await;
    let gate = temp.path().join("release");
    let gated_capture = temp.path().join("gated-input");
    provider(
        &binary,
        &gated_capture,
        judgment("assign", Some(group_id), vec![]),
        Some(&gate),
        false,
    );
    let worker = {
        let store = store.clone();
        tokio::spawn(async move { once(&store).await.unwrap() })
    };
    wait_capture(&gated_capture).await;
    manual(&store, "personal", &native, None).await;
    let before = preserved_sources(&store).await;
    std::fs::write(&gate, b"").unwrap();
    assert!(worker.await.unwrap());
    assert_eq!(preserved_sources(&store).await, before);
    assert_eq!(
        membership(&store, "personal", &native).await["grouping"]["mode"],
        "off"
    );

    // The graph must expose an explicit recheck of an otherwise current prior
    // assignment before its managed detail panel has been opened.
    retry(&store, "personal", &late).await;
    let graph = store
        .graph(ontology::graph::GraphQuery {
            scope: Scope::Personal,
            q: "지연 투영 합성".into(),
            focus: Some(format!("c_{late_id}")),
            limit: 1,
        })
        .await
        .unwrap();
    assert_eq!(graph["nodes"][0]["classification_review_needed"], true);
    provider(
        &binary,
        &capture,
        judgment("assign", Some(group_id), vec![]),
        None,
        false,
    );
    assert!(once(&store).await.unwrap());

    // Context apply/recovery excludes both the source API and the worker. A
    // temporary recovery gate does not consume the judge's retry budget.
    retry(&store, "personal", &native).await;
    let current = membership(&store, "personal", &native).await;
    let before = preserved_sources(&store).await;
    let gate_capture = temp.path().join("recovery-input");
    provider(
        &binary,
        &gate_capture,
        judgment("assign", Some(group_id), vec![]),
        None,
        false,
    );
    let mut recovery = store.pool().begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(478310003)")
        .execute(&mut *recovery)
        .await
        .unwrap();
    for command in [
        json!({"op":"document-subject","scope":"personal","document":native}),
        json!({"op":"document-subject-history","scope":"personal","document":native,"limit":10}),
        json!({"op":"document-subject-set","scope":"personal","document":{"identity":native,"revision":current["revision"],"source_revision":current["current_source"]["source_revision"],"content_digest":current["current_source"]["content_digest"],"subject_id":null,"subject_revision":null,"reason":"합성 복구 대기 검증"}}),
    ] {
        assert_eq!(
            store
                .brain(serde_json::from_value::<BrainCommand>(command).unwrap())
                .await,
            Err(Error::ContextPending)
        );
    }
    assert_eq!(
        store.discover_document_grouping().await,
        Err(Error::ContextPending)
    );
    assert_eq!(
        store.document_grouping_once().await,
        Err(Error::ContextPending)
    );
    assert!(!gate_capture.exists());
    assert_eq!(queue(&store, native_id).await["state"], "pending");
    assert_eq!(queue(&store, native_id).await["attempts"], 0);
    assert_eq!(preserved_sources(&store).await, before);
    recovery.rollback().await.unwrap();

    // Recovery beginning after the model read also prevents apply, preserves
    // intent/history, and allows a later current decision.
    let release = temp.path().join("recovery-release");
    provider(
        &binary,
        &gate_capture,
        judgment("assign", Some(group_id), vec![]),
        Some(&release),
        false,
    );
    let worker = {
        let store = store.clone();
        tokio::spawn(async move { once(&store).await })
    };
    wait_capture(&gate_capture).await;
    let mut recovery = store.pool().begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(478310003)")
        .execute(&mut *recovery)
        .await
        .unwrap();
    std::fs::write(&release, b"").unwrap();
    assert_eq!(worker.await.unwrap(), Err(Error::ContextPending));
    assert_eq!(queue(&store, native_id).await["state"], "pending");
    assert_eq!(queue(&store, native_id).await["attempts"], 0);
    assert_eq!(preserved_sources(&store).await, before);
    recovery.rollback().await.unwrap();
    provider(
        &binary,
        &capture,
        judgment("assign", Some(group_id), vec![]),
        None,
        false,
    );
    assert!(once(&store).await.unwrap());

    // Recovery can finish and introduce a new source between apply's pending
    // result and its failure bookkeeping. Give the NEW input a fresh budget.
    retry(&store, "personal", &native).await;
    let current = membership(&store, "personal", &native).await;
    sqlx::query("UPDATE document_grouping SET attempts=2 WHERE scope='personal' AND source_id=$1")
        .bind(native_id)
        .execute(store.pool())
        .await
        .unwrap();
    let release = temp.path().join("recovery-drift-release");
    let captured = temp.path().join("recovery-drift-input");
    provider(
        &binary,
        &captured,
        judgment("assign", Some(group_id), vec![]),
        Some(&release),
        false,
    );
    let worker = {
        let store = store.clone();
        tokio::spawn(async move { once(&store).await })
    };
    wait_capture(&captured).await;
    let mut first = store.pool().begin().await.unwrap();
    let first_pid: i32 = sqlx::query_scalar(
        "SELECT pg_backend_pid() FROM pg_advisory_xact_lock(hashtextextended('personal',478312))",
    )
    .fetch_one(&mut *first)
    .await
    .unwrap();
    let mut recovery = store.pool().begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(478310003)")
        .execute(&mut *recovery)
        .await
        .unwrap();
    std::fs::write(&release, b"").unwrap();
    wait_blocked(&store, first_pid, 1).await;
    let mut second = store.pool().begin().await.unwrap();
    let second_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *second)
        .await
        .unwrap();
    let second = tokio::spawn(async move {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('personal',478312))")
            .execute(&mut *second)
            .await
            .unwrap();
        second
    });
    wait_blocked(&store, first_pid, 2).await;
    first.commit().await.unwrap();
    let second = second.await.unwrap();
    wait_blocked(&store, second_pid, 1).await;
    recovery.rollback().await.unwrap();
    let changed = store
        .edit_context(
            &"personal".parse().unwrap(),
            "notes/new.md",
            current["current_source"]["source_revision"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap(),
            current["current_source"]["content_digest"]
                .as_str()
                .unwrap(),
            "# 복구 중 바뀐 합성 문서\n이것은 새 판단 입력이다.\n",
        )
        .await
        .unwrap();
    let before = preserved_sources(&store).await;
    second.commit().await.unwrap();
    assert_eq!(worker.await.unwrap(), Err(Error::ContextPending));
    let deferred = queue(&store, native_id).await;
    assert_eq!(deferred["state"], "pending");
    assert_eq!(deferred["attempts"], 0);
    assert_eq!(deferred["content_digest"], changed.content_digest);
    assert_eq!(preserved_sources(&store).await, before);
    provider(
        &binary,
        &capture,
        judgment("assign", Some(group_id), vec![]),
        None,
        false,
    );
    assert!(once(&store).await.unwrap());

    // Failed output is stale output too. No discovery runs between the source
    // revision change and failure; the failure path must check canonical data.
    retry(&store, "personal", &git_id).await;
    let release = temp.path().join("failure-source-release");
    let captured = temp.path().join("failure-source-input");
    provider(
        &binary,
        &captured,
        judgment("assign", Some(group_id), vec![]),
        Some(&release),
        true,
    );
    let worker = {
        let store = store.clone();
        tokio::spawn(async move { once(&store).await.unwrap() })
    };
    wait_capture(&captured).await;
    let revision_change = git_record(
        "new-git",
        "원문이 모델 처리 중 다시 변경됨",
        &"f".repeat(40),
    );
    store.apply_import(&[revision_change]).await.unwrap();
    let before = preserved_sources(&store).await;
    std::fs::write(&release, b"").unwrap();
    assert!(worker.await.unwrap());
    let after = queue(&store, git_id["entity_id"].as_str().unwrap()).await;
    assert_eq!(after["state"], "pending");
    assert_eq!(after["source_revision"], "f".repeat(40));
    assert_eq!(after["attempts"], 0);
    assert_eq!(preserved_sources(&store).await, before);
    provider(
        &binary,
        &capture,
        judgment("assign", Some(group_id), vec![]),
        None,
        false,
    );
    assert!(once(&store).await.unwrap());

    retry(&store, "personal", &git_id).await;
    let release = temp.path().join("failure-definition-release");
    let captured = temp.path().join("failure-definition-input");
    provider(
        &binary,
        &captured,
        judgment("assign", Some(group_id), vec![]),
        Some(&release),
        true,
    );
    let worker = {
        let store = store.clone();
        tokio::spawn(async move { once(&store).await.unwrap() })
    };
    wait_capture(&captured).await;
    let revision: i64 =
        sqlx::query_scalar("SELECT revision FROM subjects WHERE scope='personal' AND id=$1")
            .bind(undefined["id"].as_str().unwrap())
            .fetch_one(store.pool())
            .await
            .unwrap();
    call(&store,json!({"op":"subject-define","scope":"personal","id":undefined["id"],"revision":revision,"name":"처리 중 다시 명확해진 경쟁 목적","definition":definition})).await;
    let before = preserved_sources(&store).await;
    std::fs::write(&release, b"").unwrap();
    assert!(worker.await.unwrap());
    assert_eq!(
        queue(&store, git_id["entity_id"].as_str().unwrap()).await["state"],
        "pending"
    );
    assert_eq!(preserved_sources(&store).await, before);
    provider(
        &binary,
        &capture,
        judgment("assign", Some(group_id), vec![]),
        None,
        false,
    );
    assert!(once(&store).await.unwrap());

    for old_fails in [false, true] {
        let item = git_record(
            &format!("reclaimed-{old_fails}"),
            "합성 재획득 검증 문서",
            &"a".repeat(40),
        );
        store
            .apply_import(std::slice::from_ref(&item))
            .await
            .unwrap();
        store.discover_document_grouping().await.unwrap();
        let gates = temp.path().join(format!("reclaimed-{old_fails}"));
        paired_provider(
            &binary,
            &gates,
            judgment("assign", Some(group_id), vec![]),
            old_fails,
        );
        let old = {
            let store = store.clone();
            tokio::spawn(async move { store.document_grouping_once().await.unwrap() })
        };
        wait_marker(&gates.join("old-started")).await;
        let old_token:String=sqlx::query_scalar("UPDATE document_grouping SET lease_until=clock_timestamp()-interval '1 second' WHERE scope='personal' AND source_id=$1 RETURNING claim_token::text").bind(&item.entity_id).fetch_one(store.pool()).await.unwrap();
        let new = {
            let store = store.clone();
            tokio::spawn(async move { store.document_grouping_once().await.unwrap() })
        };
        wait_marker(&gates.join("new-started")).await;
        let before = queue(&store, &item.entity_id).await;
        assert_ne!(before["claim_token"], old_token);
        let originals = preserved_sources(&store).await;
        std::fs::write(gates.join("old-release"), b"").unwrap();
        assert!(old.await.unwrap());
        assert_eq!(queue(&store, &item.entity_id).await, before);
        assert_eq!(preserved_sources(&store).await, originals);
        std::fs::write(gates.join("new-release"), b"").unwrap();
        assert!(new.await.unwrap());
        let applied = membership(&store, "personal", &json!({"entity_id":item.entity_id})).await;
        assert_eq!(applied["subject_id"], group["id"]);
        assert_eq!(applied["revision"], 1);
        assert_eq!(
            queue(&store, &item.entity_id).await["claim_token"],
            Value::Null
        );
    }

    // Success waiting on the scope lock, and failure waiting on the queue row,
    // must both recheck the real clock after blocking (not transaction now()).
    for fails in [false, true] {
        let item = git_record(
            &format!("lease-expiry-{fails}"),
            "합성 실제 lease 만료 검증",
            &"a".repeat(40),
        );
        store
            .apply_import(std::slice::from_ref(&item))
            .await
            .unwrap();
        store.discover_document_grouping().await.unwrap();
        let release = temp.path().join(format!("expiry-release-{fails}"));
        let captured = temp.path().join(format!("expiry-input-{fails}"));
        provider(
            &binary,
            &captured,
            judgment("assign", Some(group_id), vec![]),
            Some(&release),
            fails,
        );
        let worker = {
            let store = store.clone();
            tokio::spawn(async move { store.document_grouping_once().await.unwrap() })
        };
        wait_capture(&captured).await;
        sqlx::query("UPDATE document_grouping SET lease_until=clock_timestamp()+interval '2 seconds' WHERE scope='personal' AND source_id=$1").bind(&item.entity_id).execute(store.pool()).await.unwrap();
        let before = queue(&store, &item.entity_id).await;
        let originals = preserved_sources(&store).await;
        let mut blocking = store.pool().begin().await.unwrap();
        let blocker: i32 = if fails {
            sqlx::query_scalar("SELECT pg_backend_pid() FROM document_grouping WHERE scope='personal' AND source_id=$1 FOR UPDATE").bind(&item.entity_id).fetch_one(&mut *blocking).await.unwrap()
        } else {
            sqlx::query_scalar("SELECT pg_backend_pid() FROM pg_advisory_xact_lock(hashtextextended('personal',478312))").fetch_one(&mut *blocking).await.unwrap()
        };
        std::fs::write(&release, b"").unwrap();
        wait_expired_blocker(&store, &item.entity_id, blocker).await;
        blocking.commit().await.unwrap();
        assert!(worker.await.unwrap());
        assert_eq!(queue(&store, &item.entity_id).await, before);
        assert_eq!(preserved_sources(&store).await, originals);
        provider(
            &binary,
            &capture,
            judgment("assign", Some(group_id), vec![]),
            None,
            false,
        );
        assert!(store.document_grouping_once().await.unwrap());
    }
}

async fn once(store: &Store) -> Result<bool, Error> {
    store.discover_document_grouping().await?;
    store.document_grouping_once().await
}
