use meenseek_ontology::{
    domain::{Error, ImportedRecord, Scope, SourceKind},
    memory::BrainCommand,
    store::{Store, digest},
};
use serde_json::{Value, json};

async fn store() -> Store {
    let url = std::env::var("TEST_DATABASE_URL").expect("explicit isolated PostgreSQL");
    assert!(
        meenseek_ontology::config::database_options(&url)
            .expect("loopback test database")
            .get_database()
            .is_some_and(|name| name.starts_with("ontology_test_"))
    );
    let store = Store::connect(&url).await.expect("isolated database");
    store.initialize().await.expect("isolated schema");
    sqlx::query("TRUNCATE curation_reviews,evidence_snapshots,evidence_contents,memory_history,memories,memory_creations,subjects,confirmation_history,related_materials,entity_areas,entity_topics,source_records,entities,sources,topics RESTART IDENTITY CASCADE")
        .execute(store.pool()).await.expect("reset synthetic rows only");
    store
}

fn command(value: Value) -> BrainCommand {
    serde_json::from_value(value).expect("typed synthetic command")
}

async fn call(store: &Store, value: Value) -> Value {
    store.brain(command(value)).await.expect("valid operation")
}

fn reference(value: &Value) -> Value {
    json!({"entity_id":value["entity_id"],"source_revision":value["source_revision"],"content_digest":value["content_digest"],"generation":value["generation"]})
}

fn native_reference(value: &Value) -> Value {
    let hash = digest(value["body"].as_str().expect("record body").as_bytes());
    json!({"entity_id":value["id"],"source_revision":hash,"content_digest":hash,"generation":value["revision"]})
}

async fn remember(store: &Store, key: &str, body: &str, evidence: Vec<Value>) -> Value {
    call(store, json!({"op":"remember","scope":"meenseek","idempotency_key":key,"memory":{"body":body,"evidence":evidence}})).await
}

async fn historical(store: &Store, owner: &Value, revision: i64, entity: &str) -> Value {
    call(store, json!({"op":"evidence-read","scope":"meenseek","id":owner["id"],"revision":revision,"entity_id":entity})).await
}

fn source(index: usize, body: &str) -> ImportedRecord {
    let hash = digest(format!("native-evidence-source-{index}").as_bytes());
    ImportedRecord {
        source_id: format!("s_{hash}"),
        entity_id: format!("e_{hash}"),
        scope: Scope::Meenseek,
        repository: "/synthetic/native-evidence".into(),
        path: format!("source-{index}.md"),
        kind: SourceKind::Git,
        source_revision: "a".repeat(40),
        digest: Some(digest(body.as_bytes())),
        content: Some(body.into()),
    }
}

#[tokio::test]
async fn native_evidence_contract() {
    let store = store().await;

    // Plain capture remains unclassified; a native reference is evidence, not a proof of authorship.
    let plain = remember(
        &store,
        "native-plain-record",
        "인용문: 누군가 결정했다고 말했다.",
        vec![],
    )
    .await;
    assert_eq!(plain["kind"], "record");
    let before = store.calls();
    let options = call(
        &store,
        json!({"op":"evidence","scope":"meenseek","query":"누군가"}),
    )
    .await;
    assert_eq!(store.calls() - before, 1);
    assert_eq!(options["items"].as_array().unwrap().len(), 1);
    let option = &options["items"][0];
    assert_eq!(reference(option), native_reference(&plain));
    assert_eq!(option["kind"], "record");
    assert_eq!(option["source_id"], plain["id"]);
    assert_eq!(option["path"], plain["title"]);
    assert_eq!(option["repository"], "분신");
    assert!(
        call(
            &store,
            json!({"op":"evidence","scope":"personal","query":"누군가"})
        )
        .await["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let first = remember(
        &store,
        "native-first-citation",
        "검증 전 기록을 인용한다.",
        vec![reference(option)],
    )
    .await;
    assert_eq!(first["evidence"][0]["current"], true);
    assert_eq!(first["kind"], "record");
    let before = store.calls();
    let read = historical(&store, &first, 1, plain["id"].as_str().unwrap()).await;
    assert_eq!(store.calls() - before, 1);
    assert_eq!(read["content"], plain["body"]);
    assert_eq!(read["evidence"]["kind"], "record");
    assert_eq!(store.brain(command(json!({"op":"remember","scope":"personal","idempotency_key":"native-cross-scope","memory":{"body":"other scope","evidence":[native_reference(&plain)]}}))).await, Err(Error::Invalid));
    assert_eq!(store.brain(command(json!({"op":"evidence-read","scope":"personal","id":first["id"],"revision":1,"entity_id":plain["id"]}))).await, Err(Error::NotFound));

    // The generation is the native revision even when a correction leaves the body unchanged.
    let corrected = call(&store, json!({"op":"correct","scope":"meenseek","id":plain["id"],"revision":1,"memory":{"body":plain["body"]}})).await;
    assert_eq!(
        native_reference(&corrected)["content_digest"],
        native_reference(&plain)["content_digest"]
    );
    assert_eq!(
        call(
            &store,
            json!({"op":"read","scope":"meenseek","id":first["id"]})
        )
        .await["evidence"][0]["current"],
        false
    );
    assert_eq!(
        historical(&store, &first, 1, plain["id"].as_str().unwrap()).await["content"],
        plain["body"]
    );
    assert_eq!(store.brain(command(json!({"op":"remember","scope":"meenseek","idempotency_key":"native-old-generation","memory":{"body":"stale","evidence":[native_reference(&plain)]}}))).await, Err(Error::Conflict));

    // Proposed, withdrawn and out-of-window records are discoverable records, but not current evidence.
    for (key, op, extra) in [
        ("native-proposed", "propose", json!({})),
        (
            "native-future",
            "remember",
            json!({"effective_from":253_402_300_000_i64}),
        ),
        ("native-expired", "remember", json!({"effective_until":1})),
        ("native-withdrawn", "remember", json!({})),
    ] {
        let mut input = json!({"body":key});
        input
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let mut record = call(
            &store,
            json!({"op":op,"scope":"meenseek","idempotency_key":key,"memory":input}),
        )
        .await;
        if key == "native-withdrawn" {
            record = call(
                &store,
                json!({"op":"withdraw","scope":"meenseek","id":record["id"],"revision":1}),
            )
            .await;
        }
        assert!(
            call(
                &store,
                json!({"op":"evidence","scope":"meenseek","query":key})
            )
            .await["items"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(store.brain(command(json!({"op":"remember","scope":"meenseek","idempotency_key":format!("reject-{key}"),"memory":{"body":"invalid basis","evidence":[native_reference(&record)]}}))).await, Err(Error::Conflict));
    }

    // M2 -> M1 -> E is stored with every dependency, and a repeated E is deduplicated.
    let mut e = source(0, "# 근거\n\n모든 조건을 확인한다.");
    store.apply_import(&[e.clone()]).await.unwrap();
    let candidates = call(
        &store,
        json!({"op":"evidence","scope":"meenseek","query":"source-0.md"}),
    )
    .await;
    let e_ref = reference(&candidates["items"][0]);
    let m1 = remember(&store, "native-chain-first", "첫 판단", vec![e_ref.clone()]).await;
    let m2 = remember(
        &store,
        "native-chain-second",
        "두 번째 판단",
        vec![native_reference(&m1), e_ref.clone()],
    )
    .await;
    assert_eq!(m2["evidence"].as_array().unwrap().len(), 2);
    let m3 = remember(
        &store,
        "native-chain-third",
        "세 번째 판단",
        vec![native_reference(&m2)],
    )
    .await;
    assert_eq!(m3["evidence"].as_array().unwrap().len(), 3);
    for entity in [
        m1["id"].as_str().unwrap(),
        m2["id"].as_str().unwrap(),
        &e.entity_id,
    ] {
        assert_eq!(historical(&store, &m3, 1, entity).await["available"], true);
    }
    assert_eq!(store.brain(command(json!({"op":"correct","scope":"meenseek","id":m1["id"],"revision":1,"memory":{"body":"cycle","evidence":[native_reference(&m2)]}}))).await, Err(Error::Invalid));
    assert_eq!(store.brain(command(json!({"op":"correct","scope":"meenseek","id":m1["id"],"revision":1,"memory":{"body":"self cycle","evidence":[native_reference(&m1)]}}))).await, Err(Error::Invalid));
    let original_body = e.content.clone().unwrap();
    e.content = Some("# 근거\n\n조건이 달라졌다.".into());
    e.digest = Some(digest(e.content.as_ref().unwrap().as_bytes()));
    e.source_revision = "b".repeat(40);
    store.apply_import(&[e.clone()]).await.unwrap();
    for memory in [&m1, &m2, &m3] {
        let graph = call(
            &store,
            json!({"op":"search","scope":"meenseek","query":memory["title"]}),
        )
        .await;
        assert_eq!(
            graph["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|node| node["id"] == memory["id"])
                .unwrap()["supported"],
            false
        );
        assert!(
            call(
                &store,
                json!({"op":"evidence","scope":"meenseek","query":memory["title"]})
            )
            .await["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|candidate| candidate["entity_id"] != memory["id"])
        );
    }
    let current = call(
        &store,
        json!({"op":"evidence","scope":"meenseek","query":"source-0.md"}),
    )
    .await;
    assert_eq!(store.brain(command(json!({"op":"remember","scope":"meenseek","idempotency_key":"native-conflicting-union","memory":{"body":"conflicting basis","evidence":[native_reference(&m1),reference(&current["items"][0])]}}))).await, Err(Error::Conflict));
    assert_eq!(store.brain(command(json!({"op":"remember","scope":"meenseek","idempotency_key":"native-stale-transitive","memory":{"body":"stale basis","evidence":[native_reference(&m3)]}}))).await, Err(Error::Conflict));
    assert_eq!(
        historical(&store, &m3, 1, &e.entity_id).await["content"],
        original_body
    );

    // Union cardinality, rather than the number of direct input choices, owns the ten-reference cap.
    let many: Vec<_> = (1..=10)
        .map(|i| source(i, &format!("source body {i}")))
        .collect();
    store.apply_import(&many).await.unwrap();
    let mut refs = Vec::new();
    for source in &many {
        let options = call(
            &store,
            json!({"op":"evidence","scope":"meenseek","query":source.path}),
        )
        .await;
        refs.push(reference(&options["items"][0]));
    }
    let broad = remember(&store, "native-ten-source-basis", "Ten source basis", refs).await;
    assert_eq!(store.brain(command(json!({"op":"remember","scope":"meenseek","idempotency_key":"native-union-overflow","memory":{"body":"must not omit the eleventh reference","evidence":[native_reference(&broad)]}}))).await, Err(Error::Invalid));

    // All-native creation uses one native batch and one content batch regardless of 1 or 10 inputs.
    let mut native_refs = Vec::new();
    for i in 0..10 {
        let record = remember(
            &store,
            &format!("native-batch-source-{i}"),
            &format!("batch origin {i}"),
            vec![],
        )
        .await;
        native_refs.push(native_reference(&record));
    }
    let mut capture_calls = Vec::new();
    for size in [0, 1, 10] {
        let before = store.calls();
        let item = remember(
            &store,
            &format!("native-batch-owner-{size}"),
            "bounded batch",
            native_refs[..size].to_vec(),
        )
        .await;
        capture_calls.push(store.calls() - before);
        assert_eq!(item["evidence"].as_array().unwrap().len(), size);
        assert!(serde_json::to_vec(&item).unwrap().len() <= 1_048_576);
        let before = store.calls();
        call(
            &store,
            json!({"op":"read","scope":"meenseek","id":item["id"]}),
        )
        .await;
        assert_eq!(store.calls() - before, 1);
    }
    assert_eq!(
        capture_calls[1],
        capture_calls[0] + 2,
        "one native batch and one bulk content insert"
    );
    assert_eq!(
        capture_calls[2], capture_calls[1],
        "no per-reference queries"
    );
    assert_eq!(
        capture_calls,
        vec![10, 12, 12],
        "one final JSONB size query per capture"
    );

    // Forget erases snapshots attributed to that native origin, without erasing another origin's bytes.
    let shared_body = "동일한 본문을 가진 서로 다른 출처";
    let shared_source = source(99, shared_body);
    store
        .apply_import(std::slice::from_ref(&shared_source))
        .await
        .unwrap();
    let candidates = call(
        &store,
        json!({"op":"evidence","scope":"meenseek","query":"source-99.md"}),
    )
    .await;
    let origin = remember(&store, "native-forget-origin", shared_body, vec![]).await;
    let owner = remember(
        &store,
        "native-forget-owner",
        "같은 바이트의 두 출처",
        vec![
            native_reference(&origin),
            reference(&candidates["items"][0]),
        ],
    )
    .await;
    let another_origin = remember(&store, "native-other-origin", shared_body, vec![]).await;
    let another_owner = remember(
        &store,
        "native-other-owner",
        "다른 원저자의 연결",
        vec![native_reference(&another_origin)],
    )
    .await;
    for (key, basis) in [
        ("native-erased-review", &origin),
        ("native-retained-review", &another_origin),
    ] {
        sqlx::query("INSERT INTO curation_reviews(id,scope,key_digest,source_id,input_digest,candidate_digest,candidate,author) VALUES($1,'meenseek',$1,$2,'synthetic','synthetic',$3,'synthetic reviewer')")
            .bind(key).bind(basis["id"].as_str().unwrap())
            .bind(json!({"basis":[native_reference(basis)],"quotations":[{"entity_id":basis["id"],"quote":shared_body}]}))
            .execute(store.pool()).await.unwrap();
    }
    call(
        &store,
        json!({"op":"forget","scope":"meenseek","id":origin["id"],"revision":1}),
    )
    .await;
    let reviews: Vec<String> = sqlx::query_scalar("SELECT id FROM curation_reviews ORDER BY id")
        .fetch_all(store.pool())
        .await
        .unwrap();
    assert_eq!(reviews, vec!["native-retained-review"]);
    assert_eq!(
        historical(&store, &owner, 1, origin["id"].as_str().unwrap()).await["available"],
        false
    );
    assert_eq!(
        historical(&store, &owner, 1, &shared_source.entity_id).await["content"],
        shared_body
    );
    assert_eq!(
        historical(
            &store,
            &another_owner,
            1,
            another_origin["id"].as_str().unwrap()
        )
        .await["content"],
        shared_body
    );
    let current = call(
        &store,
        json!({"op":"read","scope":"meenseek","id":owner["id"]}),
    )
    .await;
    assert_eq!(
        current["evidence"].as_array().unwrap().len(),
        2,
        "downstream metadata remains"
    );
    assert_eq!(current["evidence"][0]["current"], false);
    assert_eq!(current["evidence"][1]["current"], true);
    let graph = call(
        &store,
        json!({"op":"search","scope":"meenseek","query":owner["title"]}),
    )
    .await;
    assert_eq!(
        graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == owner["id"])
            .unwrap()["supported"],
        false
    );
}
