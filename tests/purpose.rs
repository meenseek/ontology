use ontology::{
    context::{ContextScope, inventory},
    domain::{Error, ImportedRecord, Scope, SourceKind},
    memory::BrainCommand,
    store::{Store, digest},
};
use serde_json::{Value, json};

fn command(value: Value) -> BrainCommand {
    serde_json::from_value(value).unwrap()
}
async fn call(store: &Store, value: Value) -> Value {
    store.brain(command(value)).await.unwrap()
}
fn definition() -> Value {
    json!({"purpose":"합성 제품을 개발한다","include":"구현·시험 근거","exclude":"채용 표현"})
}
fn request(identity: Value, source: &Value, revision: i64, subject: &Value) -> Value {
    json!({"op":"document-subject-set","scope":"personal","document":{
        "identity":identity,"revision":revision,"source_revision":source["source_revision"],
        "content_digest":source["content_digest"],"subject_id":subject["id"],
        "subject_revision":subject["revision"],"reason":"합성 본문의 제품 목적과 정의를 대조했다"}})
}

#[tokio::test]
async fn purpose_decisions_preserve_sources_and_reject_stale_concurrent_judgments() {
    let url = std::env::var("TEST_DATABASE_URL").expect("explicit disposable DB");
    assert!(
        ontology::config::database_options(&url)
            .unwrap()
            .get_database()
            .unwrap()
            .starts_with("ontology_test_")
    );
    let store = Store::connect(&url).await.unwrap();
    store.initialize().await.unwrap();
    let complete_payload =
        json!({"name":"x","definition":{"purpose":"x","include":"x","exclude":"x"}});
    let serialized_name = serde_json::to_string(&complete_payload).unwrap();
    for (key, legacy_first) in [
        ("legacy-first-domain", true),
        ("complete-first-domain", false),
    ] {
        let legacy = json!({"op":"subject-create","scope":"personal","idempotency_key":key,"name":serialized_name});
        let complete = json!({"op":"subject-create","scope":"personal","idempotency_key":key,"name":"x","definition":complete_payload["definition"]});
        let (first, second) = if legacy_first {
            (legacy, complete)
        } else {
            (complete, legacy)
        };
        let created = call(&store, first.clone()).await;
        assert_eq!(
            store.brain(command(second)).await,
            Err(Error::Conflict),
            "distinct creation formats cannot share a birth key"
        );
        assert_eq!(call(&store, first).await, created);
    }
    let birth = json!({"op":"subject-create","scope":"personal","idempotency_key":"complete-purpose-birth","name":"원자적 목적","definition":definition()});
    let created = call(&store, birth.clone()).await;
    assert_eq!(created["revision"], 0);
    assert_eq!(created["definition"], definition());
    assert_eq!(call(&store, birth.clone()).await["id"], created["id"]);
    let mut different = birth.clone();
    different["definition"]["include"] = json!("다른 생성 기준");
    assert_eq!(store.brain(command(different)).await, Err(Error::Conflict));
    let mut updated_definition = definition();
    updated_definition["purpose"] = json!("다른 사람이 명시적으로 정정한 목적");
    let updated = call(&store, json!({"op":"subject-define","scope":"personal","id":created["id"],"revision":0,"name":"현재 이름","definition":updated_definition})).await;
    assert_eq!(
        call(&store, birth).await,
        updated,
        "birth retries return current state without restoring the old definition"
    );
    let history: Value = sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(h) ORDER BY revision) FROM subject_history h WHERE subject_id=$1").bind(created["id"].as_str()).fetch_one(store.pool()).await.unwrap();
    assert_eq!(history.as_array().unwrap().len(), 2);
    assert_eq!(history[0]["definition"], definition());
    assert_eq!(history[0]["action"], "create");
    let invalid = json!({"op":"subject-create","scope":"personal","idempotency_key":"invalid-complete-birth","name":"실패한 목적","definition":{"purpose":"목적","include":"","exclude":"제외"}});
    assert_eq!(store.brain(command(invalid)).await, Err(Error::Invalid));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM subjects WHERE name='실패한 목적'")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(count, 0, "invalid complete birth writes no partial subject");
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    std::fs::create_dir_all(root.join("personal/raw")).unwrap();
    let root = root.canonicalize().unwrap();
    let before = "# Synthetic source\nImplementation and regression evidence.\n";
    std::fs::write(root.join("personal/purpose.md"), before).unwrap();
    std::fs::write(root.join("personal/raw/private.md"), "private").unwrap();
    let scopes: Vec<ContextScope> = vec!["personal".parse().unwrap()];
    let manifest = inventory(&root, &scopes).unwrap();
    store
        .import_context(&root, &scopes, &manifest.inventory_digest)
        .await
        .unwrap();
    let material:String=sqlx::query_scalar("SELECT material_id::text FROM context_materials WHERE scope='personal' AND path='purpose.md'").fetch_one(store.pool()).await.unwrap();
    let identity = json!({"material_id":material,"entity_id":null});
    let read = json!({"op":"document-subject","scope":"personal","document":identity});
    let empty = call(&store, read.clone()).await;
    assert_eq!(empty["revision"], 0);
    let subject=call(&store,json!({"op":"subject-create","scope":"personal","idempotency_key":"purpose-native-fixture","name":"합성 목적"})).await;
    let define = json!({"op":"subject-define","scope":"personal","id":subject["id"],"revision":0,"name":"합성 목적","definition":definition()});
    let (one, two) = tokio::join!(
        store.brain(command(define.clone())),
        store.brain(command(define))
    );
    assert_eq!(usize::from(one.is_ok()) + usize::from(two.is_ok()), 1);
    assert!(matches!(
        one.as_ref().err().or(two.as_ref().err()),
        Some(Error::Conflict)
    ));
    let subject = one.or(two).unwrap();
    assert_eq!(subject["revision"], 1);
    let set = request(identity.clone(), &empty["current_source"], 0, &subject);
    let (one, two) = tokio::join!(
        store.brain(command(set.clone())),
        store.brain(command(set.clone()))
    );
    assert_eq!(usize::from(one.is_ok()) + usize::from(two.is_ok()), 1);
    let assigned = one.or(two).unwrap();
    assert_eq!(assigned["subject_id"], subject["id"]);
    assert_eq!(assigned["review_needed"], false);
    let store_id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
        .fetch_one(store.pool())
        .await
        .unwrap();
    ontology::context_importer::ContextReader::new()
        .import(
            &store,
            &store_id,
            &scopes[0],
            &["purpose.md".into()],
            Scope::Personal,
        )
        .await
        .unwrap();
    assert_eq!(
        call(&store, read.clone()).await["revision"],
        1,
        "binding cannot fork native membership"
    );
    let bound:String=sqlx::query_scalar("SELECT e.id FROM entities e JOIN context_source_bindings b ON b.source_id=e.source_id WHERE b.material_id=$1::uuid AND e.scope='personal'").bind(&material).fetch_one(store.pool()).await.unwrap();
    assert!(matches!(store.brain(command(json!({"op":"document-subject","scope":"personal","document":{"entity_id":bound}}))).await,Err(Error::NotFound)),"native bindings require canonical material identity");
    let before_graph = store.calls();
    let limited = store
        .graph(ontology::graph::GraphQuery {
            scope: Scope::Personal,
            q: "Synthetic source".into(),
            focus: Some(bound.clone()),
            limit: 1,
        })
        .await
        .unwrap();
    assert_eq!(
        store.calls() - before_graph,
        1,
        "purpose metadata stays in the single graph snapshot"
    );
    let projected = &limited["nodes"][0];
    assert_eq!(projected["id"], bound);
    assert_eq!(projected["material_id"], material);
    assert_eq!(projected["subject_id"], subject["id"]);
    assert_eq!(projected["subject_name"], subject["name"]);
    assert_eq!(
        projected["purpose_total"], 1,
        "membership survives a search without the subject marker"
    );
    assert_eq!(projected["classification_review_needed"], false);
    let mut repeat = set.clone();
    repeat["document"]["revision"] = json!(1);
    assert_eq!(
        call(&store, repeat.clone()).await["revision"],
        1,
        "same current decision is a no-op"
    );
    assert_eq!(
        store
            .read_context(&scopes[0], "purpose.md", false)
            .await
            .unwrap(),
        before
    );
    assert_eq!(
        store
            .context_history(&scopes[0], "purpose.md", None)
            .await
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let changed = store
        .edit_context(
            &scopes[0],
            "purpose.md",
            1,
            &digest(before.as_bytes()),
            "# Changed source\nNew implementation evidence\n",
        )
        .await
        .unwrap();
    assert!(matches!(
        store.brain(command(repeat.clone())).await,
        Err(Error::Conflict)
    ));
    let current = call(&store, read.clone()).await;
    assert_eq!(current["subject_id"], subject["id"]);
    assert_eq!(current["review_needed"], true);
    assert_eq!(
        current["current_source"]["content_digest"],
        changed.content_digest
    );
    let refreshed = call(
        &store,
        request(identity.clone(), &current["current_source"], 1, &subject),
    )
    .await;
    assert_eq!(refreshed["revision"], 2);
    let renamed=call(&store,json!({"op":"subject-define","scope":"personal","id":subject["id"],"revision":1,"name":"같은 ID의 새 이름","definition":definition()})).await;
    assert_eq!(renamed["id"], subject["id"]);
    assert_eq!(call(&store, read.clone()).await["review_needed"], true);
    assert!(matches!(
        store
            .brain(command(request(
                identity.clone(),
                &current["current_source"],
                2,
                &subject
            )))
            .await,
        Err(Error::Conflict)
    ));
    let refreshed = call(
        &store,
        request(identity.clone(), &current["current_source"], 2, &renamed),
    )
    .await;
    assert_eq!(refreshed["revision"], 3);
    let birth_retry=call(&store,json!({"op":"subject-create","scope":"personal","idempotency_key":"purpose-native-fixture","name":"합성 목적"})).await;
    assert_eq!(birth_retry["id"], subject["id"]);
    assert_eq!(birth_retry["name"], renamed["name"]);
    let history = call(
        &store,
        json!({"op":"document-subject-history","scope":"personal","document":identity,"limit":1}),
    )
    .await;
    assert_eq!(history["items"][0]["revision"], 3);
    assert_eq!(history["next_before_revision"], 3);
    let second=call(&store,json!({"op":"document-subject-history","scope":"personal","document":identity,"before_revision":3,"limit":1})).await;
    assert_eq!(second["items"][0]["subject_name"], "합성 목적");
    let mut wrong_scope = read.clone();
    wrong_scope["scope"] = json!("meenseek");
    assert!(matches!(
        store.brain(command(wrong_scope)).await,
        Err(Error::NotFound)
    ));
    ontology::context_importer::ContextReader::new()
        .import(
            &store,
            &store_id,
            &scopes[0],
            &["purpose.md".into()],
            Scope::Meenseek,
        )
        .await
        .unwrap();
    let separate = call(
        &store,
        json!({"op":"document-subject","scope":"meenseek","document":identity}),
    )
    .await;
    assert_eq!(separate["revision"], 0);
    assert!(
        separate["subject_id"].is_null(),
        "another app scope does not inherit membership"
    );
    let mut cross = request(identity.clone(), &current["current_source"], 0, &renamed);
    cross["scope"] = json!("meenseek");
    assert!(
        matches!(store.brain(command(cross)).await, Err(Error::Conflict)),
        "target group belongs to the original app scope"
    );
    let restricted:String=sqlx::query_scalar("SELECT material_id::text FROM context_materials WHERE scope='personal' AND path='raw/private.md'").fetch_one(store.pool()).await.unwrap();
    assert!(matches!(store.brain(command(json!({"op":"document-subject","scope":"personal","document":{"material_id":restricted}}))).await,Err(Error::NotFound)));

    let h = digest(b"purpose-git-fixture");
    let entity = format!("e_{h}");
    store
        .apply_import(&[ImportedRecord {
            source_id: format!("s_{h}"),
            entity_id: entity.clone(),
            scope: Scope::Personal,
            repository: "/synthetic/purpose".into(),
            path: "purpose.md".into(),
            kind: SourceKind::Git,
            source_revision: "a".repeat(40),
            digest: Some(digest(b"Git source")),
            content: Some("Git source".into()),
        }])
        .await
        .unwrap();
    let git_identity = json!({"entity_id":entity});
    let git = call(
        &store,
        json!({"op":"document-subject","scope":"personal","document":git_identity}),
    )
    .await;
    call(
        &store,
        request(git_identity.clone(), &git["current_source"], 0, &renamed),
    )
    .await;
    let graph = store
        .graph(ontology::graph::GraphQuery {
            scope: Scope::Personal,
            q: "purpose.md".into(),
            focus: Some(bound.clone()),
            limit: 1,
        })
        .await
        .unwrap();
    assert_eq!(
        graph["nodes"][0]["purpose_total"], 2,
        "full purpose count precedes search and response limit"
    );
    assert_eq!(graph["nodes"][0]["subject_name"], renamed["name"]);
    assert_eq!(graph["nodes"][0]["classification_review_needed"], false);
    let deleted = call(
        &store,
        json!({"op":"subject-delete","scope":"personal","id":subject["id"]}),
    )
    .await;
    assert_eq!(deleted["ungrouped_documents"], 2);
    let cleared = call(&store, read).await;
    assert!(cleared["subject_id"].is_null());
    assert_eq!(cleared["revision"], 4);
    let history = call(
        &store,
        json!({"op":"document-subject-history","scope":"personal","document":identity}),
    )
    .await;
    assert_eq!(
        history["items"][1]["subject_name"], renamed["name"],
        "deleted subject's reviewed name survives"
    );
    assert_eq!(
        store
            .read_context_revision(&scopes[0], "purpose.md", 1)
            .await
            .unwrap()["content"],
        before
    );
    store.initialize().await.unwrap();
    assert_eq!(
        call(
            &store,
            json!({"op":"document-subject","scope":"personal","document":git_identity})
        )
        .await["subject_id"],
        Value::Null
    );
}
