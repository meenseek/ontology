use ontology::{
    domain::{ImportedRecord, Scope, SourceKind},
    memory::BrainCommand,
    store::{Store, digest},
};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, path::Path};

async fn call(store: &Store, value: Value) -> Value {
    store
        .brain(serde_json::from_value::<BrainCommand>(value).unwrap())
        .await
        .unwrap()
}

fn provider(binary: &Path, capture: &Path) {
    let output = json!({"type":"item.completed","item":{"type":"agent_message","text":json!({"decision":"unmatched","subject_id":null,"candidate_ids":[],"new_subject":null,"reason":"합성 한도 검증"}).to_string()}}).to_string();
    let capture = capture.to_str().unwrap().replace('\'', "'\\''");
    std::fs::write(
        binary,
        format!("#!/bin/sh\ncat >'{capture}'\nprintf '%s\\n' '{output}'\n"),
    )
    .unwrap();
    std::fs::set_permissions(binary, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[tokio::test]
async fn expanded_judgment_input_never_calls_memory_or_document_provider() {
    let url = std::env::var("TEST_DATABASE_URL").expect("explicit disposable PostgreSQL");
    assert!(
        ontology::config::database_options(&url)
            .unwrap()
            .get_database()
            .is_some_and(|name| name.starts_with("ontology_test_"))
    );
    let store = Store::connect(&url).await.unwrap();
    store.initialize().await.unwrap();
    sqlx::raw_sql("TRUNCATE document_grouping,document_subjects,document_subject_history,subject_history,context_source_bindings,context_projection_versions,context_material_versions,context_manual_edits,context_materials,context_apply_batches,memories,memory_creations,subjects,confirmation_history,related_materials,entity_areas,entity_topics,source_records,entities,sources,topics RESTART IDENTITY CASCADE")
        .execute(store.pool()).await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let binary = temp.path().join("codex");
    unsafe {
        std::env::set_var("ONTOLOGY_CODEX_BINARY", &binary);
    }

    // Each definition is accepted through the public storage contract. Its
    // short synthetic assignments grow only in the outgoing redacted copy.
    let definition = json!({"purpose":"token:x\n".repeat(128),"include":"token:x\n".repeat(256),"exclude":"token:x\n".repeat(256)});
    let mut subjects = Vec::new();
    for i in 0..10 {
        let subject = call(&store, json!({"op":"subject-create","scope":"personal","idempotency_key":format!("expanded-definition-{i}"),"name":format!("합성 목적 {i}"),"definition":definition})).await;
        subjects.push(json!({"id":subject["id"],"name":subject["name"],"revision":subject["revision"],"definition":subject["definition"]}));
    }
    let memory = json!({"title":"합성 한도","body":"일반 자료"});
    let original_input = json!({"record":memory,"subjects":subjects});
    assert!(original_input.to_string().len() < 65_536);
    let definitions_before: Value =
        sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(s) ORDER BY id) FROM subjects s")
            .fetch_one(store.pool())
            .await
            .unwrap();

    let saved = call(&store, json!({"op":"remember","scope":"personal","idempotency_key":"expanded-memory-input","memory":memory})).await;
    let memory_before: Value = sqlx::query_scalar("SELECT document FROM memories WHERE id=$1")
        .bind(saved["id"].as_str().unwrap())
        .fetch_one(store.pool())
        .await
        .unwrap();
    let memory_capture = temp.path().join("memory-input");
    provider(&binary, &memory_capture);
    assert!(store.grouping_once().await.unwrap());

    let hash = digest(b"expanded-document-input");
    let body = "# 합성 한도\n일반 자료";
    let source = ImportedRecord {
        source_id: format!("s_{hash}"),
        entity_id: format!("e_{hash}"),
        scope: Scope::Personal,
        repository: "/synthetic/grouping-limits".into(),
        path: "expanded.md".into(),
        kind: SourceKind::Git,
        source_revision: "a".repeat(40),
        digest: Some(digest(body.as_bytes())),
        content: Some(body.into()),
    };
    store
        .apply_import(std::slice::from_ref(&source))
        .await
        .unwrap();
    let identity = json!({"entity_id":source.entity_id});
    let current = call(
        &store,
        json!({"op":"document-subject","scope":"personal","document":identity}),
    )
    .await;
    let assigned = call(&store, json!({"op":"document-subject-set","scope":"personal","document":{"identity":identity,"revision":current["revision"],"source_revision":current["current_source"]["source_revision"],"content_digest":current["current_source"]["content_digest"],"subject_id":subjects[0]["id"],"subject_revision":subjects[0]["revision"],"reason":"합성 소속 보존 검증"}})).await;
    let history_before = call(
        &store,
        json!({"op":"document-subject-history","scope":"personal","document":identity}),
    )
    .await;
    call(&store, json!({"op":"document-grouping-retry","scope":"personal","document":{"identity":identity,"revision":assigned["revision"],"source_revision":assigned["current_source"]["source_revision"],"content_digest":assigned["current_source"]["content_digest"]}})).await;
    let document_capture = temp.path().join("document-input");
    provider(&binary, &document_capture);
    assert!(store.document_grouping_once().await.unwrap());

    // If the regression returns, prove that the actual process received an
    // oversized JSON, then fail on its invocation rather than its answer.
    for capture in [&memory_capture, &document_capture] {
        if capture.exists() {
            let prompt = std::fs::read_to_string(capture).unwrap();
            let input: Value =
                serde_json::from_str(prompt.split("입력 JSON:\n").nth(1).unwrap()).unwrap();
            assert!(input.to_string().len() > 65_536);
        }
    }
    assert!(
        !memory_capture.exists() && !document_capture.exists(),
        "oversized redacted JSON reached a provider: memory={}, document={}",
        memory_capture.exists(),
        document_capture.exists()
    );
    let memory_after = call(
        &store,
        json!({"op":"read","scope":"personal","id":saved["id"]}),
    )
    .await;
    assert_eq!(memory_after["grouping"]["state"], "error");
    assert_eq!(memory_after["revision"], saved["revision"]);
    let stored: Value = sqlx::query_scalar("SELECT document FROM memories WHERE id=$1")
        .bind(saved["id"].as_str().unwrap())
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(stored, memory_before);
    let document_after = call(
        &store,
        json!({"op":"document-subject","scope":"personal","document":identity}),
    )
    .await;
    assert_eq!(document_after["grouping"]["state"], "error");
    assert!(
        document_after["grouping"]["reason"]
            .as_str()
            .unwrap()
            .contains("수동 검토")
    );
    assert_eq!(document_after["subject_id"], assigned["subject_id"]);
    assert_eq!(document_after["revision"], assigned["revision"]);
    assert_eq!(
        call(
            &store,
            json!({"op":"document-subject-history","scope":"personal","document":identity})
        )
        .await,
        history_before
    );
    let content: String =
        sqlx::query_scalar("SELECT content FROM source_records WHERE entity_id=$1")
            .bind(&source.entity_id)
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(content, body);
    let definitions_after: Value =
        sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(s) ORDER BY id) FROM subjects s")
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(definitions_after, definitions_before);
    unsafe {
        std::env::remove_var("ONTOLOGY_CODEX_BINARY");
    }
}
