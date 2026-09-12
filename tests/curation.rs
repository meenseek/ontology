use meenseek_ontology::{
    domain::{Error, ImportedRecord, Scope, SourceKind},
    memory::BrainCommand,
    store::{Store, digest},
};
use serde_json::{Value, json};

async fn call(store: &Store, value: Value) -> Result<Value, Error> {
    store
        .brain(serde_json::from_value::<BrainCommand>(value).expect("typed command"))
        .await
}
fn command(action: &str, values: Value) -> Value {
    let mut detail = json!({"action":action});
    detail
        .as_object_mut()
        .unwrap()
        .extend(values.as_object().unwrap().clone());
    json!({"op":"curation","scope":"meenseek","command":detail})
}
fn candidate(items: &[Value], body: Option<&str>) -> Value {
    json!({"source_id":items[0]["id"],"basis":items.iter().map(|i|i["token"].clone()).collect::<Vec<_>>(),
        "quotations":items.iter().map(|i|json!({"entity_id":i["id"],"quote":i["body"]})).collect::<Vec<_>>(),
        "reason":"합성 원문과 적용 조건을 대조했다.",
        "finding":body.map_or(json!({"kind":"no-change"}),|body|json!({"kind":"knowledge","body":body,"applicability":"합성 실험의 같은 대상과 관측 기간에서만 적용한다."}))})
}
fn review(prepared: &Value, reviewer: &str, decision: &str) -> Value {
    command(
        "apply",
        json!({"id":prepared["id"],"review":{"candidate_digest":prepared["candidate_digest"],"reviewer":reviewer,"decision":decision,"reason":"합성 독립 검토: 원문과 조건의 대응을 확인했다."}}),
    )
}

#[tokio::test]
async fn curation_lifecycle_and_call_bounds() {
    let url = std::env::var("TEST_DATABASE_URL").expect("isolated DB");
    assert!(
        meenseek_ontology::config::database_options(&url)
            .unwrap()
            .get_database()
            .unwrap()
            .starts_with("ontology_test_")
    );
    let store = Store::connect(&url).await.unwrap();
    store.initialize().await.unwrap();
    sqlx::raw_sql("TRUNCATE curation_reviews,evidence_snapshots,evidence_contents,memory_history,memories,memory_creations,subjects,confirmation_history,related_materials,entity_areas,entity_topics,source_records,entities,sources,topics RESTART IDENTITY CASCADE;").execute(store.pool()).await.unwrap();
    let before = store.calls();
    assert_eq!(
        call(&store, command("pending", json!({}))).await.unwrap()["items"],
        json!([])
    );
    assert_eq!(store.calls() - before, 1);
    let mut raw = Vec::new();
    for i in 0..10 {
        raw.push(call(&store,json!({"op":"remember","scope":"meenseek","idempotency_key":format!("raw-curation-{i}"),"memory":{"body":format!("합성 관측 {i}: 방문 100, 복사 12. 같은 기간에서 측정했다.")}})).await.unwrap());
    }
    // A blocked first page must not starve later sources; cursors are stable IDs.
    let mut paged_ids = Vec::new();
    let mut after = Value::Null;
    loop {
        let before = store.calls();
        let page = call(&store, command("pending", json!({"limit":3,"after":after})))
            .await
            .unwrap();
        assert_eq!(store.calls() - before, 1);
        paged_ids.extend(
            page["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["id"].clone()),
        );
        after = page["next_after"].clone();
        if after.is_null() {
            break;
        }
    }
    let mut expected_ids = raw.iter().map(|v| v["id"].clone()).collect::<Vec<_>>();
    expected_ids.sort_by_key(|v| v.as_str().unwrap().to_owned());
    assert_eq!(paged_ids, expected_ids);
    assert_eq!(
        call(&store, command("pending", json!({"after":"bad-id"}))).await,
        Err(Error::Invalid)
    );
    for size in [1, 10] {
        let ids = raw[..size]
            .iter()
            .map(|v| v["id"].clone())
            .collect::<Vec<_>>();
        let before = store.calls();
        let context = call(&store, command("context", json!({"ids":ids})))
            .await
            .unwrap();
        assert_eq!(
            store.calls() - before,
            1,
            "one snapshot for full current bodies"
        );
        let items = context["items"].as_array().unwrap();
        assert_eq!(items.len(), size);
        assert!(serde_json::to_vec(&context).unwrap().len() < size * 2000 + 512);
        let prepare = command(
            "prepare",
            json!({"idempotency_key":format!("candidate-bounded-{size}"),"author":"writer-task","candidate":candidate(items,Some("합성 원값은 방문 100과 복사 12다. 효과가 입증되었다는 뜻은 아니다."))}),
        );
        let before = store.calls();
        let prepared = call(&store, prepare.clone()).await.unwrap();
        assert_eq!(
            store.calls() - before,
            9,
            "prepare is bounded for 1/10 native sources"
        );
        assert_eq!(
            call(&store, prepare).await.unwrap(),
            prepared,
            "same prepare is idempotent"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM evidence_contents")
                .fetch_one(store.pool())
                .await
                .unwrap(),
            if size == 1 { 0 } else { 1 },
            "prepare does not pin unused content"
        );
        assert_eq!(
            call(&store, review(&prepared, "writer-task", "approve")).await,
            Err(Error::Conflict)
        );
        let mut wrong = review(&prepared, "reviewer-task", "approve");
        wrong["command"]["review"]["candidate_digest"] = json!("0".repeat(64));
        assert_eq!(call(&store, wrong).await, Err(Error::Conflict));
        let approve = review(&prepared, "reviewer-task", "approve");
        let before = store.calls();
        let applied = call(&store, approve.clone()).await.unwrap();
        assert_eq!(
            store.calls() - before,
            14,
            "apply is atomic and bounded for 1/10 sources"
        );
        assert_eq!(applied["outcome"], "created");
        assert_eq!(
            call(&store, approve).await.unwrap(),
            applied,
            "retry never duplicates a record"
        );
        let memory = call(
            &store,
            json!({"op":"read","scope":"meenseek","id":applied["memory_id"]}),
        )
        .await
        .unwrap();
        assert_eq!(memory["status"], "accepted");
        assert_eq!(memory["curation"]["review_id"], prepared["id"]);
        assert_eq!(memory["evidence"].as_array().unwrap().len(), size);
        assert_eq!(memory["origin"], "assistant");
        let mut cross = command("read", json!({"id":prepared["id"]}));
        cross["scope"] = json!("personal");
        assert_eq!(call(&store, cross).await, Err(Error::NotFound));
        // Retrying a distinct proposal against exactly the same inspected inputs cannot double-apply.
        let duplicate=call(&store,command("prepare",json!({"idempotency_key":format!("candidate-duplicate-{size}"),"author":"writer-task","candidate":candidate(items,Some("다른 표현으로 만든 같은 합성 결과"))}))).await.unwrap();
        assert_eq!(
            call(&store, review(&duplicate, "reviewer-task", "approve")).await,
            Err(Error::Conflict)
        );
    }
    let pending = call(&store, command("pending", json!({}))).await.unwrap();
    assert!(
        !pending["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"] == raw[0]["id"] && i["token"]["generation"] == 1)
    );
    // Freshness is rechecked after independent review and before any write.
    let one = call(&store, command("context", json!({"ids":[raw[9]["id"]]})))
        .await
        .unwrap();
    let pending=call(&store,command("prepare",json!({"idempotency_key":"candidate-will-stale","author":"writer-task","candidate":candidate(one["items"].as_array().unwrap(),Some("검토 후 입력 변경을 검증한다."))}))).await.unwrap();
    call(&store,json!({"op":"correct","scope":"meenseek","id":raw[9]["id"],"revision":1,"memory":{"body":"정정: 원래 관측의 분모를 잘못 읽었다."}})).await.unwrap();
    assert_eq!(
        call(&store, review(&pending, "reviewer-task", "approve")).await,
        Err(Error::Conflict)
    );
    assert!(
        call(&store, command("read", json!({"id":pending["id"]})))
            .await
            .unwrap()["result"]
            .is_null()
    );
    let current = call(&store, command("context", json!({"ids":[raw[9]["id"]]})))
        .await
        .unwrap();
    let mut invalid = candidate(current["items"].as_array().unwrap(), None);
    invalid["quotations"][0]["quote"] = json!("원문에 없는 인용");
    assert_eq!(call(&store,command("prepare",json!({"idempotency_key":"candidate-invalid-quote","author":"writer-task","candidate":invalid}))).await,Err(Error::Invalid));
    let unchanged=call(&store,command("prepare",json!({"idempotency_key":"candidate-no-knowledge","author":"writer-task","candidate":candidate(current["items"].as_array().unwrap(),None)}))).await.unwrap();
    let before_count: i64 = sqlx::query_scalar("SELECT count(*) FROM memories")
        .fetch_one(store.pool())
        .await
        .unwrap();
    // Failure at the receipt write rolls back the whole mutation, then an exact retry succeeds.
    sqlx::raw_sql("CREATE FUNCTION fail_curation_apply() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic apply failure'; END $$; CREATE TRIGGER fail_curation_apply BEFORE UPDATE ON curation_reviews FOR EACH ROW EXECUTE FUNCTION fail_curation_apply();").execute(store.pool()).await.unwrap();
    assert_eq!(
        call(&store, review(&unchanged, "reviewer-task", "approve")).await,
        Err(Error::Storage)
    );
    sqlx::raw_sql("DROP TRIGGER fail_curation_apply ON curation_reviews; DROP FUNCTION fail_curation_apply();").execute(store.pool()).await.unwrap();
    assert_eq!(
        call(&store, review(&unchanged, "reviewer-task", "approve"))
            .await
            .unwrap()["outcome"],
        "no-change"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM memories")
            .fetch_one(store.pool())
            .await
            .unwrap(),
        before_count,
        "no-change is not fake knowledge"
    );
    let before = store.calls();
    let pending = call(&store, command("pending", json!({}))).await.unwrap();
    assert_eq!(store.calls() - before, 1);
    assert!(
        !pending["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"] == raw[9]["id"])
    );
    // Original source bodies survive curation; forgetting native input removes its copied receipts.
    assert_eq!(
        call(
            &store,
            json!({"op":"read","scope":"meenseek","id":raw[0]["id"]})
        )
        .await
        .unwrap()["body"],
        raw[0]["body"]
    );
    call(
        &store,
        json!({"op":"forget","scope":"meenseek","id":raw[9]["id"],"revision":2}),
    )
    .await
    .unwrap();
    assert_eq!(
        call(&store, command("read", json!({"id":unchanged["id"]}))).await,
        Err(Error::NotFound)
    );
    // Empty imported documents can be examined without fabricating a quotation.
    let empty_id = format!("e_{}", digest(b"empty-curation-source"));
    store
        .apply_import(&[ImportedRecord {
            source_id: format!("s_{}", digest(b"empty-curation-source")),
            entity_id: empty_id.clone(),
            scope: Scope::Meenseek,
            repository: "/synthetic/curation".into(),
            path: "empty.md".into(),
            kind: SourceKind::Git,
            source_revision: "a".repeat(40),
            digest: Some(digest(b"")),
            content: Some(String::new()),
        }])
        .await
        .unwrap();
    let empty = call(&store, command("context", json!({"ids":[empty_id]})))
        .await
        .unwrap();
    let mut empty_candidate = candidate(empty["items"].as_array().unwrap(), None);
    empty_candidate["quotations"] = json!([]);
    let prepared=call(&store,command("prepare",json!({"idempotency_key":"empty-source-no-change","author":"writer-task","candidate":empty_candidate}))).await.unwrap();
    call(&store, review(&prepared, "reviewer-task", "approve"))
        .await
        .unwrap();
    assert!(
        !call(&store, command("pending", json!({}))).await.unwrap()["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"] == empty_id)
    );

    // A source with ten citations cannot produce an eleventh evidence reference,
    // but a genuine no-new-knowledge review does not need to create those snapshots.
    let mut ten = Vec::new();
    for i in 0..10 {
        let note=call(&store,json!({"op":"remember","scope":"meenseek","idempotency_key":format!("ten-basis-{i}"),"memory":{"body":format!("근거 상한 합성값 {i}")}})).await.unwrap();
        ten.push(note["id"].clone());
    }
    let ten = call(&store, command("context", json!({"ids":ten})))
        .await
        .unwrap();
    let refs = ten["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["token"].clone())
        .collect::<Vec<_>>();
    let rich=call(&store,json!({"op":"remember","scope":"meenseek","idempotency_key":"raw-ten-citations","memory":{"kind":"idea","body":"열 근거를 연결한 가설이다.","effective_until":4102444800i64,"evidence":refs}})).await.unwrap();
    let rich_context = call(&store, command("context", json!({"ids":[rich["id"]]})))
        .await
        .unwrap();
    assert_eq!(rich_context["items"][0]["semantics"]["kind"], "idea");
    assert_eq!(
        rich_context["items"][0]["semantics"]["effective_until"],
        4102444800i64
    );
    assert_eq!(
        rich_context["items"][0]["dependencies"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    let limited=call(&store,command("prepare",json!({"idempotency_key":"too-many-citations-reviewed","author":"writer-task","candidate":candidate(rich_context["items"].as_array().unwrap(),None)}))).await.unwrap();
    call(&store, review(&limited, "reviewer-task", "approve"))
        .await
        .unwrap();

    // A curation update keeps identity/history and never overwrites a raw or manually edited record.
    let source=call(&store,json!({"op":"remember","scope":"meenseek","idempotency_key":"update-learning-source","memory":{"body":"관측: 실행 시간 30분, 첫 검토 통과 여부 미관측."}})).await.unwrap();
    let context = call(&store, command("context", json!({"ids":[source["id"]]})))
        .await
        .unwrap();
    let prepared=call(&store,command("prepare",json!({"idempotency_key":"learning-create","author":"writer-task","candidate":candidate(context["items"].as_array().unwrap(),Some("실행 시간은 30분이다. 품질 개선은 아직 판단할 수 없다."))}))).await.unwrap();
    let record_count: i64 = sqlx::query_scalar("SELECT count(*) FROM memories")
        .fetch_one(store.pool())
        .await
        .unwrap();
    sqlx::raw_sql("CREATE FUNCTION fail_curation_apply() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic apply failure'; END $$; CREATE TRIGGER fail_curation_apply BEFORE UPDATE ON curation_reviews FOR EACH ROW EXECUTE FUNCTION fail_curation_apply();").execute(store.pool()).await.unwrap();
    assert_eq!(
        call(&store, review(&prepared, "reviewer-task", "approve")).await,
        Err(Error::Storage)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM memories")
            .fetch_one(store.pool())
            .await
            .unwrap(),
        record_count,
        "late failure rolls back new knowledge and history"
    );
    sqlx::raw_sql("DROP TRIGGER fail_curation_apply ON curation_reviews; DROP FUNCTION fail_curation_apply();").execute(store.pool()).await.unwrap();
    let created = call(&store, review(&prepared, "reviewer-task", "approve"))
        .await
        .unwrap();
    call(&store,json!({"op":"correct","scope":"meenseek","id":source["id"],"revision":1,"memory":{"body":"정정 관측: 실행 시간 45분, 첫 검토는 통과하지 못했다."}})).await.unwrap();
    let stale = call(
        &store,
        json!({"op":"read","scope":"meenseek","id":created["memory_id"]}),
    )
    .await
    .unwrap();
    assert_eq!(stale["evidence"][0]["current"], false);
    let context = call(&store, command("context", json!({"ids":[source["id"]]})))
        .await
        .unwrap();
    let mut update = candidate(
        context["items"].as_array().unwrap(),
        Some("정정된 실행 시간은 45분이며 첫 검토를 통과하지 못했다."),
    );
    update["finding"]["target"] = json!({"id":created["memory_id"],"revision":1});
    let updated = call(
        &store,
        command(
            "prepare",
            json!({"idempotency_key":"learning-update","author":"writer-task","candidate":update}),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        call(&store, review(&updated, "reviewer-task", "approve"))
            .await
            .unwrap()["outcome"],
        "updated"
    );
    let current = call(
        &store,
        json!({"op":"read","scope":"meenseek","id":created["memory_id"]}),
    )
    .await
    .unwrap();
    assert_eq!(current["revision"], 2);
    assert_eq!(current["evidence"][0]["current"], true);
    let history = call(
        &store,
        json!({"op":"history","scope":"meenseek","id":created["memory_id"]}),
    )
    .await
    .unwrap();
    assert_eq!(history["items"].as_array().unwrap().len(), 2);
    call(
        &store,
        json!({"op":"forget","scope":"meenseek","id":created["memory_id"],"revision":2}),
    )
    .await
    .unwrap();
    let erased = call(&store, command("read", json!({"id":updated["id"]})))
        .await
        .unwrap();
    assert_eq!(erased["result"]["outcome"], "forgotten");
    assert!(erased["candidate"]["finding"].is_null());
    assert!(erased["review"]["reason"].is_null());
    assert_eq!(
        call(&store, review(&updated, "reviewer-task", "approve")).await,
        Err(Error::Gone)
    );
    assert!(
        !call(&store, command("pending", json!({}))).await.unwrap()["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"] == source["id"]),
        "deleting output does not recreate it from unchanged input"
    );
    pending_work_digest_contract(&store).await;
    document_size_limit_contract(&store).await;
}

async fn pending_item(store: &Store, id: &Value) -> Value {
    let mut after = Value::Null;
    loop {
        let before = store.calls();
        let page = call(store, command("pending", json!({"after":after})))
            .await
            .unwrap();
        assert_eq!(
            store.calls() - before,
            1,
            "work digest uses the existing pending query"
        );
        if let Some(item) = page["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == *id)
        {
            return item.clone();
        }
        after = page["next_after"].clone();
        assert!(!after.is_null(), "synthetic source must remain pending");
    }
}

async fn pending_work_digest_contract(store: &Store) {
    let a = call(store,json!({"op":"remember","scope":"meenseek","idempotency_key":"work-digest-source-a","memory":{"body":"합성 원문 A는 그대로 유지한다."}})).await.unwrap();
    let b = call(store,json!({"op":"remember","scope":"meenseek","idempotency_key":"work-digest-source-b","memory":{"body":"합성 비교 근거 B 첫 관측"}})).await.unwrap();
    let first = pending_item(store, &a["id"]).await;
    assert_eq!(first["work_digest"].as_str().unwrap().len(), 64);
    let context = call(store, command("context", json!({"ids":[a["id"]]})))
        .await
        .unwrap();
    let draft = candidate(
        context["items"].as_array().unwrap(),
        Some("합성 최초 정리: A만 참고했다."),
    );
    let prepared = call(store,command("prepare",json!({"idempotency_key":"work-digest-first-review","author":"writer-task","candidate":draft}))).await.unwrap();
    assert_eq!(
        pending_item(store, &a["id"]).await["work_digest"],
        first["work_digest"],
        "prepare cannot create a new queue identity"
    );
    let first_result = call(store, review(&prepared, "reviewer-task", "approve"))
        .await
        .unwrap();
    // A later update expands the basis. Its stale state must supersede the still-current A-only receipt.
    let context = call(store, command("context", json!({"ids":[a["id"],b["id"]]})))
        .await
        .unwrap();
    let mut expanded = candidate(
        context["items"].as_array().unwrap(),
        Some("합성 후속 정리: A와 B의 조건을 함께 참고했다."),
    );
    expanded["source_id"] = a["id"].clone();
    expanded["finding"]["target"] = json!({"id":first_result["memory_id"],"revision":1});
    let latest=call(store,command("prepare",json!({"idempotency_key":"work-digest-expanded-basis","author":"writer-task","candidate":expanded}))).await.unwrap();
    let updated = call(store, review(&latest, "reviewer-task", "approve"))
        .await
        .unwrap();
    assert_eq!(updated["memory_id"], first_result["memory_id"]);
    let mut previous = first["work_digest"].clone();
    for revision in [1, 2] {
        call(store,json!({"op":"correct","scope":"meenseek","id":b["id"],"revision":revision,"memory":{"body":format!("합성 비교 근거 B 정정 {revision}")}})).await.unwrap();
        assert_eq!(
            call(store, command("read", json!({"id":prepared["id"]})))
                .await
                .unwrap()["current"],
            true,
            "old A-only receipt still matches"
        );
        assert_eq!(
            call(store, command("read", json!({"id":latest["id"]})))
                .await
                .unwrap()["current"],
            false,
            "latest expanded basis needs review"
        );
        let changed = pending_item(store, &a["id"]).await;
        assert_eq!(
            changed["token"], first["token"],
            "A's original token is unchanged"
        );
        assert_ne!(
            changed["work_digest"], previous,
            "each dependency change changes the work identity"
        );
        let context = call(store, command("context", json!({"ids":[a["id"],b["id"]]})))
            .await
            .unwrap();
        let mut draft = candidate(context["items"].as_array().unwrap(), None);
        draft["source_id"] = a["id"].clone();
        let prepared = call(store,command("prepare",json!({"idempotency_key":format!("work-digest-retry-{revision}"),"author":"writer-task","candidate":draft}))).await.unwrap();
        assert_eq!(
            pending_item(store, &a["id"]).await["work_digest"],
            changed["work_digest"]
        );
        call(store, review(&prepared, "reviewer-task", "reject"))
            .await
            .unwrap();
        assert_eq!(
            pending_item(store, &a["id"]).await["work_digest"],
            changed["work_digest"],
            "rejection cannot bypass a paused queue entry"
        );
        previous = changed["work_digest"].clone();
    }
}

fn native_token(item: &Value) -> Value {
    let hash = digest(item["body"].as_str().unwrap().as_bytes());
    json!({"entity_id":item["id"],"source_revision":hash,"content_digest":hash,"generation":item["revision"]})
}

async fn memory_state(store: &Store) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object('memories',(SELECT jsonb_agg(to_jsonb(m) ORDER BY id) FROM memories m),'history',(SELECT jsonb_agg(to_jsonb(h) ORDER BY memory_id,revision) FROM memory_history h),'creations',(SELECT jsonb_agg(to_jsonb(c) ORDER BY scope,key_digest) FROM memory_creations c),'contents',(SELECT jsonb_agg(to_jsonb(c) ORDER BY scope,digest) FROM evidence_contents c),'snapshots',(SELECT jsonb_agg(to_jsonb(s) ORDER BY scope,memory_id,revision,entity_id) FROM evidence_snapshots s),'reviews',(SELECT jsonb_agg(to_jsonb(r) ORDER BY id) FROM curation_reviews r))")
        .fetch_one(store.pool()).await.unwrap()
}

async fn document_size_limit_contract(store: &Store) {
    let mut raw = call(store,json!({"op":"remember","scope":"meenseek","idempotency_key":"size-chain-origin","memory":{"title":"size chain","body":"synthetic chain origin"}})).await.unwrap();
    // Four accepted curation records contribute escaped applicability text through a native chain.
    // Every input is individually valid and the flattened union stays below ten references.
    for index in 0..4 {
        let mut ids = vec![raw["id"].clone()];
        ids.extend(
            raw["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["entity_id"].clone()),
        );
        let context = call(store, command("context", json!({"ids":ids})))
            .await
            .unwrap();
        let mut draft = candidate(
            context["items"].as_array().unwrap(),
            Some("synthetic derived chain record"),
        );
        draft["source_id"] = raw["id"].clone();
        draft["finding"]["applicability"] = json!("\\".repeat(2048));
        let prepared = call(store,command("prepare",json!({"idempotency_key":format!("size-chain-curation-{index}"),"author":"writer-task","candidate":draft}))).await.unwrap();
        let applied = call(store, review(&prepared, "reviewer-task", "approve"))
            .await
            .unwrap();
        let derived = call(
            store,
            json!({"op":"read","scope":"meenseek","id":applied["memory_id"]}),
        )
        .await
        .unwrap();
        raw = call(store,json!({"op":"remember","scope":"meenseek","idempotency_key":format!("size-chain-raw-{index}"),"memory":{"title":"size chain","body":format!("synthetic next chain source {index}"),"evidence":[native_token(&derived)]}})).await.unwrap();
    }
    assert_eq!(raw["evidence"].as_array().unwrap().len(), 8);
    let room: i32 = sqlx::query_scalar("SELECT 24576-octet_length((document || jsonb_build_object('body',''))::text) FROM memories WHERE id=$1")
        .bind(raw["id"].as_str().unwrap()).fetch_one(store.pool()).await.unwrap();
    assert!(
        (1..8192).contains(&room),
        "native semantics consume enough of the document limit"
    );
    let refs = raw["evidence"].as_array().unwrap().iter().map(|e| json!({"entity_id":e["entity_id"],"source_revision":e["source_revision"],"content_digest":e["content_digest"],"generation":e["generation"]})).collect::<Vec<_>>();
    let mut input = json!({"title":"size chain","body":"x".repeat(room as usize),"evidence":refs});
    let boundary = call(store,json!({"op":"remember","scope":"meenseek","idempotency_key":"size-exact-boundary","memory":input})).await.unwrap();
    let bytes: i32 =
        sqlx::query_scalar("SELECT octet_length(document::text) FROM memories WHERE id=$1")
            .bind(boundary["id"].as_str().unwrap())
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(
        bytes, 24576,
        "the exact PostgreSQL JSONB limit remains accepted"
    );
    input["body"] = json!("x".repeat(room as usize + 1));
    let before = memory_state(store).await;
    for operation in [
        json!({"op":"remember","scope":"meenseek","idempotency_key":"size-over-boundary","memory":input}),
        json!({"op":"correct","scope":"meenseek","id":raw["id"],"revision":1,"memory":input}),
    ] {
        assert_eq!(call(store, operation).await, Err(Error::Limit));
        assert_eq!(
            memory_state(store).await,
            before,
            "size rejection preserves original, history, creation key and evidence snapshots"
        );
    }

    let mut ids = vec![raw["id"].clone()];
    ids.extend(
        raw["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["entity_id"].clone()),
    );
    let context = call(store, command("context", json!({"ids":ids})))
        .await
        .unwrap();
    let mut draft = candidate(
        context["items"].as_array().unwrap(),
        Some("synthetic final size test"),
    );
    draft["source_id"] = raw["id"].clone();
    draft["finding"]["applicability"] = json!("synthetic scope only");
    let prepared = call(store,command("prepare",json!({"idempotency_key":"size-small-metadata","author":"writer-task","candidate":draft}))).await.unwrap();
    draft["finding"]["applicability"] = json!("\\".repeat(2048));
    let before = memory_state(store).await;
    assert_eq!(call(store,command("prepare",json!({"idempotency_key":"size-large-metadata","author":"writer-task","candidate":draft}))).await,Err(Error::Limit),"curation metadata is included during prepare");
    assert_eq!(
        memory_state(store).await,
        before,
        "failed prepare leaves no receipt or snapshots"
    );

    // Simulate a pending receipt saved by the version that did not check final document size.
    // This deliberately changes only synthetic test data, then exercises the public apply boundary.
    let mut legacy_candidate = prepared["candidate"].clone();
    legacy_candidate["finding"]["applicability"] = draft["finding"]["applicability"].clone();
    let legacy_digest = digest(&serde_json::to_vec(&legacy_candidate).unwrap());
    sqlx::query("UPDATE curation_reviews SET candidate=$2,candidate_digest=$3 WHERE id=$1")
        .bind(prepared["id"].as_str().unwrap())
        .bind(&legacy_candidate)
        .bind(&legacy_digest)
        .execute(store.pool())
        .await
        .unwrap();
    let mut legacy = prepared.clone();
    legacy["candidate_digest"] = json!(legacy_digest);
    let before = memory_state(store).await;
    assert_eq!(
        call(store, review(&legacy, "reviewer-task", "approve")).await,
        Err(Error::Limit),
        "apply rechecks the complete document independently"
    );
    assert_eq!(
        memory_state(store).await,
        before,
        "failed apply leaves original, pending receipt and snapshots unchanged"
    );
    sqlx::query("UPDATE curation_reviews SET candidate=$2,candidate_digest=$3 WHERE id=$1")
        .bind(prepared["id"].as_str().unwrap())
        .bind(&prepared["candidate"])
        .bind(prepared["candidate_digest"].as_str().unwrap())
        .execute(store.pool())
        .await
        .unwrap();
    assert_eq!(
        call(store, review(&prepared, "reviewer-task", "approve"))
            .await
            .unwrap()["outcome"],
        "created",
        "the smaller candidate can still apply"
    );
}
