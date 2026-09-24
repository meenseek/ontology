use ontology::{
    domain::{ImportedRecord, Scope, SourceKind},
    memory::BrainCommand,
    store::{Store, digest},
};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, path::Path};
use tokio::io::AsyncWriteExt;

async fn call(store: &Store, value: Value) -> Value {
    store
        .brain(serde_json::from_value::<BrainCommand>(value).unwrap())
        .await
        .unwrap()
}

fn fake_codex(path: &Path, judgment: Option<Value>, delay: bool) {
    let output = judgment.map(|value| json!({"type":"item.completed","item":{"type":"agent_message","text":value.to_string()}}).to_string());
    let script = match output {
        Some(output) => format!(
            "#!/bin/sh\ncat >/dev/null\n{}printf '%s\\n' '{}'\n",
            if delay { "sleep 1\n" } else { "" },
            output
        ),
        None => "#!/bin/sh\ncat >/dev/null\nexit 1\n".to_owned(),
    };
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

fn judgment(
    decision: &str,
    subject_id: Option<&str>,
    candidates: Vec<&str>,
    new_subject: Option<&str>,
) -> Value {
    json!({"decision":decision,"subject_id":subject_id,"candidate_ids":candidates,"new_subject":new_subject,"reason":"합성 기록의 주된 목적을 비교함"})
}

fn curated_candidate(item: &Value, body: &str, target: Option<&Value>) -> Value {
    json!({
        "source_id":item["id"],
        "basis":[item["token"]],
        "quotations":[{"entity_id":item["id"],"quote":item["body"]}],
        "reason":"합성 원문과 적용 조건을 대조했다.",
        "finding":{"kind":"knowledge","body":body,"applicability":"합성 기록의 범위에만 적용한다.","target":target}
    })
}

async fn cli(url: &str, binary: &Path, command: Value) -> Value {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_ontology"))
        .arg("brain")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("DATABASE_URL", url)
        .env("ONTOLOGY_CODEX_BINARY", binary)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(command.to_string().as_bytes())
        .await
        .unwrap();
    let output = child.wait_with_output().await.unwrap();
    assert!(
        output.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[tokio::test]
async fn personal_grouping_is_durable_and_respects_manual_choice() {
    let url = std::env::var("TEST_DATABASE_URL").expect("explicit isolated DB");
    assert!(
        ontology::config::database_options(&url)
            .unwrap()
            .get_database()
            .is_some_and(|name| name.starts_with("ontology_test_"))
    );
    let store = Store::connect(&url).await.unwrap();
    store.initialize().await.unwrap();
    sqlx::query("TRUNCATE curation_reviews,memory_grouping,evidence_snapshots,evidence_contents,memory_history,memories,memory_creations,subjects RESTART IDENTITY CASCADE")
        .execute(store.pool()).await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let binary = temp.path().join("codex");
    unsafe {
        std::env::set_var("ONTOLOGY_CODEX_BINARY", &binary);
    }
    let subject = call(&store, json!({"op":"subject-create","scope":"personal","idempotency_key":"grouping-test-subject","name":"대학원 진학"})).await;
    let subject_id = subject["id"].as_str().unwrap();

    let input = || json!({"kind":"record","title":"진학 고민","body":"석사 과정과 연구실 선택을 고민한다","subject_id":null,"evidence":[]});
    let hash = digest(b"personal-grouping-evidence");
    store
        .apply_import(&[ImportedRecord {
            source_id: format!("s_{hash}"),
            entity_id: format!("e_{hash}"),
            scope: Scope::Personal,
            repository: "/synthetic/personal-grouping".into(),
            path: "grouping-evidence.md".into(),
            kind: SourceKind::Git,
            source_revision: "a".repeat(40),
            digest: Some(digest(b"grouping evidence")),
            content: Some("grouping evidence".into()),
        }])
        .await
        .unwrap();
    let options = call(
        &store,
        json!({"op":"evidence","scope":"personal","query":"grouping-evidence.md"}),
    )
    .await;
    let e = &options["items"][0];
    let mut first_input = input();
    first_input["evidence"] = json!([{"entity_id":e["entity_id"],"source_revision":e["source_revision"],"content_digest":e["content_digest"],"generation":e["generation"]}]);
    let first = call(&store, json!({"op":"remember","scope":"personal","idempotency_key":"grouping-test-auto","memory":first_input})).await;
    assert_eq!(first["grouping"]["state"], "pending");
    assert_eq!(first["revision"], 1);
    fake_codex(
        &binary,
        Some(judgment("assign", Some(subject_id), vec![], None)),
        false,
    );
    assert!(store.grouping_once().await.unwrap());
    let assigned = call(
        &store,
        json!({"op":"read","scope":"personal","id":first["id"]}),
    )
    .await;
    assert_eq!(assigned["subject_id"], subject["id"]);
    assert_eq!(assigned["grouping"]["state"], "assigned");
    assert_eq!(assigned["revision"], 2);
    assert_eq!(call(&store, json!({"op":"evidence-read","scope":"personal","id":first["id"],"revision":2,"entity_id":e["entity_id"]})).await["content"], "grouping evidence");
    let history = call(
        &store,
        json!({"op":"history","scope":"personal","id":first["id"],"limit":10}),
    )
    .await;
    assert_eq!(history["items"].as_array().unwrap().len(), 2);

    let mut changed = input();
    changed["body"] = json!("이제 전혀 다른 분야의 목표를 검토한다");
    changed["subject_id"] = subject["id"].clone();
    let revised = call(
        &store,
        json!({"op":"correct","scope":"personal","id":first["id"],"revision":2,"memory":changed}),
    )
    .await;
    assert_eq!(revised["subject_id"], Value::Null);
    assert_eq!(revised["grouping"]["state"], "pending");
    fake_codex(
        &binary,
        Some(judgment("unmatched", None, vec![], None)),
        false,
    );
    store.grouping_once().await.unwrap();
    let unmatched = call(
        &store,
        json!({"op":"read","scope":"personal","id":first["id"]}),
    )
    .await;
    assert_eq!(unmatched["grouping"]["state"], "unmatched");
    assert_eq!(unmatched["subject_id"], Value::Null);

    let manual = call(&store, json!({"op":"remember","scope":"personal","idempotency_key":"grouping-test-manual","memory":{"title":"내 선택","body":"내가 고른 묶음","subject_id":subject_id}})).await;
    assert_eq!(manual["grouping"]["mode"], "manual");
    assert!(!store.grouping_once().await.unwrap());
    let off = call(&store, json!({"op":"remember","scope":"personal","idempotency_key":"grouping-test-off","memory":{"title":"묶지 않음","body":"분류 제외","grouping_preference":"off"}})).await;
    assert_eq!(off["grouping"]["state"], "off");
    let company = call(&store, json!({"op":"remember","scope":"meenseek","idempotency_key":"grouping-test-company","memory":{"title":"회사 메모","body":"회사 전용"}})).await;
    assert_eq!(company["grouping"], Value::Null);

    let ambiguous = call(&store, json!({"op":"remember","scope":"personal","idempotency_key":"grouping-test-suggestion","memory":input()})).await;
    fake_codex(
        &binary,
        Some(judgment("suggest", None, vec![subject_id], Some("새 주제"))),
        false,
    );
    store.grouping_once().await.unwrap();
    let suggested = call(
        &store,
        json!({"op":"read","scope":"personal","id":ambiguous["id"]}),
    )
    .await;
    assert_eq!(suggested["grouping"]["state"], "suggested");
    assert_eq!(
        suggested["grouping"]["suggestions"]["candidate_names"][subject_id],
        "대학원 진학"
    );
    assert_eq!(
        suggested["grouping"]["suggestions"]["new_subject"],
        "새 주제"
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM subjects WHERE scope='personal'")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(count, 1, "a model suggestion cannot create a subject");

    let invalid_name = call(&store, json!({"op":"remember","scope":"personal","idempotency_key":"grouping-test-invalid-name","memory":input()})).await;
    fake_codex(
        &binary,
        Some(judgment("suggest", None, vec![], Some("잘못된\n묶음"))),
        false,
    );
    store.grouping_once().await.unwrap();
    let invalid_result = call(
        &store,
        json!({"op":"read","scope":"personal","id":invalid_name["id"]}),
    )
    .await;
    assert_eq!(invalid_result["grouping"]["state"], "error");
    assert_eq!(invalid_result["grouping"]["suggestions"], json!({}));

    let failing = call(&store, json!({"op":"remember","scope":"personal","idempotency_key":"grouping-test-error","memory":input()})).await;
    fake_codex(&binary, None, false);
    store.grouping_once().await.unwrap();
    let error = call(
        &store,
        json!({"op":"read","scope":"personal","id":failing["id"]}),
    )
    .await;
    assert_eq!(error["grouping"]["state"], "error");
    call(
        &store,
        json!({"op":"grouping-retry","scope":"personal","id":failing["id"]}),
    )
    .await;
    fake_codex(
        &binary,
        Some(judgment("assign", Some(subject_id), vec![], None)),
        false,
    );
    let restarted = Store::connect(&url).await.unwrap();
    restarted.grouping_once().await.unwrap();
    let after_restart = call(
        &restarted,
        json!({"op":"read","scope":"personal","id":failing["id"]}),
    )
    .await;
    assert_eq!(after_restart["subject_id"], subject["id"]);

    let racing = call(&store, json!({"op":"remember","scope":"personal","idempotency_key":"grouping-test-race","memory":input()})).await;
    fake_codex(
        &binary,
        Some(judgment("assign", Some(subject_id), vec![], None)),
        true,
    );
    let worker = {
        let store = store.clone();
        tokio::spawn(async move { store.grouping_once().await.unwrap() })
    };
    for _ in 0..100 {
        let state = call(
            &store,
            json!({"op":"read","scope":"personal","id":racing["id"]}),
        )
        .await;
        if state["grouping"]["state"] == "processing" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let mut off_input = input();
    off_input["grouping_preference"] = json!("off");
    let changed = call(&store, json!({"op":"correct","scope":"personal","id":racing["id"],"revision":1,"memory":off_input})).await;
    assert_eq!(changed["grouping"]["state"], "off");
    worker.await.unwrap();
    let final_state = call(
        &store,
        json!({"op":"read","scope":"personal","id":racing["id"]}),
    )
    .await;
    assert_eq!(final_state["subject_id"], Value::Null);
    assert_eq!(final_state["grouping"]["state"], "off");

    let proposal = call(&store, json!({"op":"propose","scope":"personal","idempotency_key":"grouping-test-cli-accept","memory":input()})).await;
    fake_codex(
        &binary,
        Some(judgment("assign", Some(subject_id), vec![], None)),
        false,
    );
    let accepted = cli(
        &url,
        &binary,
        json!({"op":"accept","scope":"personal","id":proposal["id"],"revision":1}),
    )
    .await;
    assert_eq!(accepted["grouping"]["state"], "pending");
    let mut processed = false;
    for _ in 0..100 {
        let state = call(
            &store,
            json!({"op":"read","scope":"personal","id":proposal["id"]}),
        )
        .await;
        if state["grouping"]["state"] == "assigned" {
            processed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        processed,
        "CLI acceptance must wake the detached grouping worker"
    );
    let classified_proposal = call(&store, json!({"op":"propose","scope":"personal","idempotency_key":"grouping-test-accepted-classification","memory":input()})).await;
    store.grouping_once().await.unwrap();
    let classified = call(
        &store,
        json!({"op":"read","scope":"personal","id":classified_proposal["id"]}),
    )
    .await;
    assert_eq!(classified["grouping"]["state"], "assigned");
    let accepted = call(&store, json!({"op":"accept","scope":"personal","id":classified["id"],"revision":classified["revision"]})).await;
    assert_eq!(accepted["grouping"]["state"], "assigned");
    assert_eq!(accepted["subject_id"], subject["id"]);
    assert!(!store.grouping_once().await.unwrap());

    let context = call(&store, json!({"op":"curation","scope":"personal","command":{"action":"context","ids":[first["id"]]}})).await;
    let item = &context["items"][0];
    let prepared = call(&store, json!({"op":"curation","scope":"personal","command":{"action":"prepare","idempotency_key":"grouping-curation-create","author":"writer-task","candidate":curated_candidate(item,"합성 개인 기록에서 정리한 판단",None)}})).await;
    let applied = call(&store, json!({"op":"curation","scope":"personal","command":{"action":"apply","id":prepared["id"],"review":{"candidate_digest":prepared["candidate_digest"],"reviewer":"reviewer-task","decision":"approve","reason":"합성 독립 검토를 완료했다."}}})).await;
    let curated = call(
        &store,
        json!({"op":"read","scope":"personal","id":applied["memory_id"]}),
    )
    .await;
    assert_eq!(curated["grouping"]["state"], "pending");
    store.grouping_once().await.unwrap();
    let grouped = call(
        &store,
        json!({"op":"read","scope":"personal","id":applied["memory_id"]}),
    )
    .await;
    assert_eq!(grouped["grouping"]["state"], "assigned");

    let first_current = call(
        &store,
        json!({"op":"read","scope":"personal","id":first["id"]}),
    )
    .await;
    let mut revised_source = input();
    revised_source["body"] = json!("합성 원문의 새 관측을 정리한다.");
    call(&store, json!({"op":"correct","scope":"personal","id":first["id"],"revision":first_current["revision"],"memory":revised_source})).await;
    fake_codex(
        &binary,
        Some(judgment("unmatched", None, vec![], None)),
        false,
    );
    store.grouping_once().await.unwrap();
    let context = call(&store, json!({"op":"curation","scope":"personal","command":{"action":"context","ids":[first["id"]]}})).await;
    let item = &context["items"][0];
    let target = json!({"id":grouped["id"],"revision":grouped["revision"]});
    let prepared = call(&store, json!({"op":"curation","scope":"personal","command":{"action":"prepare","idempotency_key":"grouping-curation-update","author":"writer-task","candidate":curated_candidate(item,"새 관측에 따라 수정한 판단",Some(&target))}})).await;
    call(&store, json!({"op":"curation","scope":"personal","command":{"action":"apply","id":prepared["id"],"review":{"candidate_digest":prepared["candidate_digest"],"reviewer":"reviewer-task","decision":"approve","reason":"합성 독립 검토를 완료했다."}}})).await;
    let updated = call(
        &store,
        json!({"op":"read","scope":"personal","id":grouped["id"]}),
    )
    .await;
    assert_eq!(updated["grouping"]["state"], "pending");
    assert_eq!(updated["subject_id"], Value::Null);
    let chosen = call(&store, json!({"op":"grouping-set","scope":"personal","id":updated["id"],"revision":updated["revision"],"subject_id":subject["id"],"mode":"manual"})).await;
    assert_eq!(chosen["grouping"]["mode"], "manual");
    assert_eq!(
        chosen["curation"], updated["curation"],
        "group choice must preserve curation provenance and applicability"
    );
    assert_eq!(chosen["body"], updated["body"]);
    let evidence_id = chosen["evidence"][0]["entity_id"].clone();
    assert_eq!(call(&store, json!({"op":"evidence-read","scope":"personal","id":chosen["id"],"revision":chosen["revision"],"entity_id":evidence_id})).await["available"], true);
    let declined = call(&store, json!({"op":"grouping-set","scope":"personal","id":chosen["id"],"revision":chosen["revision"],"subject_id":null,"mode":"off"})).await;
    assert_eq!(declined["grouping"]["mode"], "off");
    assert_eq!(declined["subject_id"], Value::Null);
    assert_eq!(declined["curation"], updated["curation"]);

    let raw = call(&store, json!({"op":"remember","scope":"personal","idempotency_key":"grouping-cli-curation-source","memory":{"body":"CLI 검토용 합성 원문","grouping_preference":"off"}})).await;
    let context = call(&store, json!({"op":"curation","scope":"personal","command":{"action":"context","ids":[raw["id"]]}})).await;
    let item = &context["items"][0];
    let prepared = call(&store, json!({"op":"curation","scope":"personal","command":{"action":"prepare","idempotency_key":"grouping-cli-curation-prepare","author":"writer-task","candidate":curated_candidate(item,"CLI가 만든 합성 판단",None)}})).await;
    fake_codex(
        &binary,
        Some(judgment("assign", Some(subject_id), vec![], None)),
        false,
    );
    let applied = cli(&url, &binary, json!({"op":"curation","scope":"personal","command":{"action":"apply","id":prepared["id"],"review":{"candidate_digest":prepared["candidate_digest"],"reviewer":"reviewer-task","decision":"approve","reason":"합성 독립 검토를 완료했다."}}})).await;
    assert_eq!(applied["outcome"], "created");
    let mut curation_processed = false;
    for _ in 0..100 {
        let state = call(
            &store,
            json!({"op":"read","scope":"personal","id":applied["memory_id"]}),
        )
        .await;
        if state["grouping"]["state"] == "assigned" {
            curation_processed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        curation_processed,
        "CLI curation must wake the detached grouping worker"
    );
    unsafe {
        std::env::remove_var("ONTOLOGY_CODEX_BINARY");
    }
}
