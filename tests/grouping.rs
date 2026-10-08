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

fn quoted_path(path: &Path) -> String {
    format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''"))
}

// Two actual processes wait independently; reclaiming the same source revision
// must revoke the first process even when it later succeeds or fails.
fn gated_codex(path: &Path, dir: &Path, answer: &Value, old_fails: bool) {
    std::fs::create_dir(dir).unwrap();
    let output =
        json!({"type":"item.completed","item":{"type":"agent_message","text":answer.to_string()}})
            .to_string();
    std::fs::write(dir.join("answer"), output).unwrap();
    let script = format!(
        "#!/bin/sh\ncat >/dev/null\nif mkdir {} 2>/dev/null; then\n while [ ! -f {} ]; do sleep 0.01; done\n {}\nelse\n mkdir {}\n while [ ! -f {} ]; do sleep 0.01; done\n cat {}\nfi\n",
        quoted_path(&dir.join("old-started")),
        quoted_path(&dir.join("old-release")),
        if old_fails {
            "exit 1".to_owned()
        } else {
            format!("cat {}", quoted_path(&dir.join("answer")))
        },
        quoted_path(&dir.join("new-started")),
        quoted_path(&dir.join("new-release")),
        quoted_path(&dir.join("answer"))
    );
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

async fn wait_marker(path: &Path) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !path.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fake provider reached its deterministic gate");
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

    for old_fails in [false, true] {
        let item=call(&store,json!({"op":"remember","scope":"personal","idempotency_key":format!("claim-reclaim-{old_fails}"),"memory":input()})).await;
        let id = item["id"].as_str().unwrap();
        let gates = temp.path().join(format!("claim-{old_fails}"));
        gated_codex(
            &binary,
            &gates,
            &judgment("assign", Some(subject_id), vec![], None),
            old_fails,
        );
        let old_worker = {
            let store = store.clone();
            tokio::spawn(async move { store.grouping_once().await.unwrap() })
        };
        wait_marker(&gates.join("old-started")).await;
        let old_token:String=sqlx::query_scalar("UPDATE memory_grouping SET lease_until=now()-interval '1 second' WHERE memory_id=$1 RETURNING claim_token::text").bind(id).fetch_one(store.pool()).await.unwrap();
        let new_worker = {
            let store = store.clone();
            tokio::spawn(async move { store.grouping_once().await.unwrap() })
        };
        wait_marker(&gates.join("new-started")).await;
        let before: Value =
            sqlx::query_scalar("SELECT to_jsonb(g) FROM memory_grouping g WHERE memory_id=$1")
                .bind(id)
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_ne!(before["claim_token"], old_token);
        let histories: i64 =
            sqlx::query_scalar("SELECT count(*) FROM memory_history WHERE memory_id=$1")
                .bind(id)
                .fetch_one(store.pool())
                .await
                .unwrap();
        std::fs::write(gates.join("old-release"), b"").unwrap();
        assert!(old_worker.await.unwrap());
        let after: Value =
            sqlx::query_scalar("SELECT to_jsonb(g) FROM memory_grouping g WHERE memory_id=$1")
                .bind(id)
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(
            after, before,
            "old success/error cannot change another process's claim"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM memory_history WHERE memory_id=$1")
                .bind(id)
                .fetch_one(store.pool())
                .await
                .unwrap(),
            histories
        );
        std::fs::write(gates.join("new-release"), b"").unwrap();
        assert!(new_worker.await.unwrap());
        let applied = call(&store, json!({"op":"read","scope":"personal","id":id})).await;
        assert_eq!(applied["subject_id"], subject["id"]);
        assert_eq!(applied["revision"], 2);
        assert!(
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT claim_token::text FROM memory_grouping WHERE memory_id=$1"
            )
            .bind(id)
            .fetch_one(store.pool())
            .await
            .unwrap()
            .is_none()
        );
    }
    fake_codex(
        &binary,
        Some(judgment("assign", Some(subject_id), vec![], None)),
        false,
    );
    for attempts in [2, 3] {
        let item=call(&store,json!({"op":"remember","scope":"personal","idempotency_key":format!("legacy-claim-{attempts}"),"memory":input()})).await;
        let id = item["id"].as_str().unwrap();
        sqlx::query("UPDATE memory_grouping SET state='processing',attempts=$2,lease_until=now()-interval '1 second',claim_token=NULL WHERE memory_id=$1").bind(id).bind(attempts).execute(store.pool()).await.unwrap();
        assert_eq!(store.grouping_once().await.unwrap(), attempts < 3);
        let state = call(&store, json!({"op":"read","scope":"personal","id":id})).await;
        assert_eq!(
            state["grouping"]["state"],
            if attempts < 3 { "assigned" } else { "error" }
        );
        if attempts == 3 {
            assert_eq!(state["revision"], 1);
            call(
                &store,
                json!({"op":"grouping-retry","scope":"personal","id":id}),
            )
            .await;
            assert!(store.grouping_once().await.unwrap());
        }
    }

    let waiting=call(&store,json!({"op":"remember","scope":"personal","idempotency_key":"claim-expired-in-lock-wait","memory":input()})).await;
    let id = waiting["id"].as_str().unwrap();
    let gates = temp.path().join("lease-lock-wait");
    gated_codex(
        &binary,
        &gates,
        &judgment("assign", Some(subject_id), vec![], None),
        false,
    );
    let worker = {
        let store = store.clone();
        tokio::spawn(async move { store.grouping_once().await.unwrap() })
    };
    wait_marker(&gates.join("old-started")).await;
    let mut blocking = store.pool().begin().await.unwrap();
    let blocker: i32 = sqlx::query_scalar(
        "SELECT pg_backend_pid() FROM pg_advisory_xact_lock(hashtextextended('personal',478312))",
    )
    .fetch_one(&mut *blocking)
    .await
    .unwrap();
    sqlx::query("UPDATE memory_grouping SET lease_until=clock_timestamp()+interval '2 seconds' WHERE memory_id=$1").bind(id).execute(store.pool()).await.unwrap();
    std::fs::write(gates.join("old-release"), b"").unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10),async {
        loop {
            let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_locks w JOIN pg_locks b USING(locktype,database,classid,objid,objsubid) WHERE NOT w.granted AND b.granted AND b.pid=$1)").bind(blocker).fetch_one(store.pool()).await.unwrap();
            if waiting {break;}
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        loop {
            let expired:bool=sqlx::query_scalar("SELECT lease_until<clock_timestamp() FROM memory_grouping WHERE memory_id=$1").bind(id).fetch_one(store.pool()).await.unwrap();
            if expired {break;}
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("apply transaction waits across lease expiration");
    blocking.commit().await.unwrap();
    assert!(worker.await.unwrap());
    let expired = call(&store, json!({"op":"read","scope":"personal","id":id})).await;
    assert_eq!(expired["revision"], 1);
    assert_eq!(expired["subject_id"], Value::Null);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM memory_history WHERE memory_id=$1")
            .bind(id)
            .fetch_one(store.pool())
            .await
            .unwrap(),
        1
    );
    fake_codex(
        &binary,
        Some(judgment("assign", Some(subject_id), vec![], None)),
        false,
    );
    assert!(store.grouping_once().await.unwrap());

    let failing_wait=call(&store,json!({"op":"remember","scope":"personal","idempotency_key":"claim-failure-expired-in-row-lock","memory":input()})).await;
    let id = failing_wait["id"].as_str().unwrap();
    let gates = temp.path().join("lease-error-lock-wait");
    gated_codex(
        &binary,
        &gates,
        &judgment("assign", Some(subject_id), vec![], None),
        true,
    );
    let worker = {
        let store = store.clone();
        tokio::spawn(async move { store.grouping_once().await.unwrap() })
    };
    wait_marker(&gates.join("old-started")).await;
    sqlx::query("UPDATE memory_grouping SET lease_until=clock_timestamp()+interval '2 seconds' WHERE memory_id=$1").bind(id).execute(store.pool()).await.unwrap();
    let before: Value =
        sqlx::query_scalar("SELECT to_jsonb(g) FROM memory_grouping g WHERE memory_id=$1")
            .bind(id)
            .fetch_one(store.pool())
            .await
            .unwrap();
    let mut blocking = store.pool().begin().await.unwrap();
    let blocker: i32 = sqlx::query_scalar(
        "SELECT pg_backend_pid() FROM memory_grouping WHERE memory_id=$1 FOR UPDATE",
    )
    .bind(id)
    .fetch_one(&mut *blocking)
    .await
    .unwrap();
    std::fs::write(gates.join("old-release"), b"").unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1=ANY(pg_blocking_pids(pid)))",
            )
            .bind(blocker)
            .fetch_one(store.pool())
            .await
            .unwrap();
            if waiting {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        loop {
            let expired: bool = sqlx::query_scalar(
                "SELECT lease_until<clock_timestamp() FROM memory_grouping WHERE memory_id=$1",
            )
            .bind(id)
            .fetch_one(store.pool())
            .await
            .unwrap();
            if expired {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("failure waits on an unchanged row across lease expiry");
    blocking.commit().await.unwrap();
    assert!(worker.await.unwrap());
    let after: Value =
        sqlx::query_scalar("SELECT to_jsonb(g) FROM memory_grouping g WHERE memory_id=$1")
            .bind(id)
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(
        before, after,
        "expired failure must not turn the queue into error"
    );
    fake_codex(
        &binary,
        Some(judgment("assign", Some(subject_id), vec![], None)),
        false,
    );
    assert!(store.grouping_once().await.unwrap());

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

    // The provider has read a definition snapshot before the concurrent edit.
    let definition_race=call(&store,json!({"op":"remember","scope":"personal","idempotency_key":"grouping-definition-race","memory":input()})).await;
    fake_codex(
        &binary,
        Some(judgment("assign", Some(subject_id), vec![], None)),
        true,
    );
    let capture = temp.path().join("definition-input");
    let quoted = format!("'{}'", capture.to_str().unwrap().replace('\'', "'\\''"));
    let script = std::fs::read_to_string(&binary).unwrap().replacen(
        "cat >/dev/null",
        &format!("cat >{quoted}"),
        1,
    );
    std::fs::write(&binary, script).unwrap();
    let worker = {
        let store = store.clone();
        tokio::spawn(async move { store.grouping_once().await.unwrap() })
    };
    for _ in 0..100 {
        if capture.exists() && std::fs::metadata(&capture).unwrap().len() > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let prompt = std::fs::read_to_string(&capture).unwrap();
    let captured: Value =
        serde_json::from_str(prompt.split("입력 JSON:\n").nth(1).unwrap()).unwrap();
    let prior = captured["subjects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == subject["id"])
        .unwrap();
    assert!(prior["definition"].is_null());
    assert_eq!(prior["revision"], 0);
    let defined=call(&store,json!({"op":"subject-define","scope":"personal","id":subject["id"],"revision":0,"name":"대학원 진학","definition":{"purpose":"대학원 진학 방향을 결정한다","include":"과정·연구실·지원 판단","exclude":"일반 취업·별도 영어 학습"}})).await;
    worker.await.unwrap();
    let stale = call(
        &store,
        json!({"op":"read","scope":"personal","id":definition_race["id"]}),
    )
    .await;
    assert!(stale["subject_id"].is_null());
    assert_eq!(stale["grouping"]["state"], "pending");
    assert_eq!(stale["revision"], 1);
    fake_codex(
        &binary,
        Some(judgment("assign", Some(subject_id), vec![], None)),
        false,
    );
    let script = std::fs::read_to_string(&binary).unwrap().replacen(
        "cat >/dev/null",
        &format!("cat >{quoted}"),
        1,
    );
    std::fs::write(&binary, script).unwrap();
    store.grouping_once().await.unwrap();
    let prompt = std::fs::read_to_string(&capture).unwrap();
    let captured: Value =
        serde_json::from_str(prompt.split("입력 JSON:\n").nth(1).unwrap()).unwrap();
    let current = captured["subjects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == subject["id"])
        .unwrap();
    assert_eq!(current["definition"], defined["definition"]);
    assert_eq!(current["revision"], 1);
    assert!(current.get("example_titles").is_none());
    let fresh = call(
        &store,
        json!({"op":"read","scope":"personal","id":definition_race["id"]}),
    )
    .await;
    assert_eq!(fresh["subject_id"], subject["id"]);
    assert_eq!(fresh["grouping"]["state"], "assigned");
    let unchanged = call(
        &store,
        json!({"op":"read","scope":"personal","id":after_restart["id"]}),
    )
    .await;
    assert_eq!(unchanged["subject_id"], after_restart["subject_id"]);
    assert_eq!(unchanged["revision"], after_restart["revision"]);

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

    let company_subject = call(&store,json!({"op":"subject-create","scope":"meenseek","idempotency_key":"company-preserved-purpose","name":"회사 목적"})).await;
    let company_context = call(&store,json!({"op":"curation","scope":"meenseek","command":{"action":"context","ids":[company["id"]]}})).await;
    let company_prepared = call(&store,json!({"op":"curation","scope":"meenseek","command":{"action":"prepare","idempotency_key":"company-preserved-curation","author":"writer-task","candidate":curated_candidate(&company_context["items"][0],"회사 검토 이력을 보존한다",None)}})).await;
    let company_applied = call(&store,json!({"op":"curation","scope":"meenseek","command":{"action":"apply","id":company_prepared["id"],"review":{"candidate_digest":company_prepared["candidate_digest"],"reviewer":"reviewer-task","decision":"approve","reason":"회사 합성 근거와 범위를 검토했다"}}})).await;
    let company_id = company_applied["memory_id"].as_str().unwrap();
    let original: Value = sqlx::query_scalar("SELECT document FROM memories WHERE id=$1")
        .bind(company_id)
        .fetch_one(store.pool())
        .await
        .unwrap();
    let old_pins: Value = sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(p)-'revision' ORDER BY entity_id) FROM evidence_snapshots p WHERE memory_id=$1 AND revision=1").bind(company_id).fetch_one(store.pool()).await.unwrap();
    call(&store,json!({"op":"correct","scope":"meenseek","id":company["id"],"revision":company["revision"],"memory":{"title":"회사 메모","body":"회사 원문의 새 관측"}})).await;
    let choice = json!({"op":"grouping-set","scope":"meenseek","id":company_id,"revision":1,"subject_id":company_subject["id"],"mode":"manual"});
    for (scope, target, mode, expected) in [
        (
            "personal",
            company_subject["id"].clone(),
            "manual",
            ontology::domain::Error::NotFound,
        ),
        (
            "meenseek",
            subject["id"].clone(),
            "manual",
            ontology::domain::Error::Invalid,
        ),
        (
            "meenseek",
            Value::Null,
            "auto",
            ontology::domain::Error::Invalid,
        ),
    ] {
        let mut invalid = choice.clone();
        invalid["scope"] = json!(scope);
        invalid["subject_id"] = target;
        invalid["mode"] = json!(mode);
        assert_eq!(
            store.brain(serde_json::from_value(invalid).unwrap()).await,
            Err(expected)
        );
    }
    let assigned = call(&store, choice.clone()).await;
    assert_eq!(assigned["revision"], 2);
    assert_eq!(
        assigned["grouping"],
        Value::Null,
        "company manual choice creates no automatic policy"
    );
    assert_eq!(assigned["curation"], original["curation"]);
    assert_eq!(
        assigned["evidence"][0]["current"], false,
        "stale source does not block metadata-only grouping"
    );
    let current: Value = sqlx::query_scalar("SELECT document FROM memories WHERE id=$1")
        .bind(company_id)
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(
        current, original,
        "all document fields stay byte-equivalent"
    );
    let pins: Value = sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(p)-'revision' ORDER BY entity_id) FROM evidence_snapshots p WHERE memory_id=$1 AND revision=2").bind(company_id).fetch_one(store.pool()).await.unwrap();
    assert_eq!(
        pins, old_pins,
        "previous evidence pins are copied rather than rebuilt"
    );
    assert_eq!(
        store
            .brain(serde_json::from_value(choice.clone()).unwrap())
            .await,
        Err(ontology::domain::Error::Conflict)
    );
    let mut repeat = choice;
    repeat["revision"] = assigned["revision"].clone();
    assert_eq!(
        call(&store, repeat).await["revision"],
        2,
        "same current company subject is a no-op"
    );
    let cleared=call(&store,json!({"op":"grouping-set","scope":"meenseek","id":company_id,"revision":2,"subject_id":null,"mode":"off"})).await;
    assert_eq!(cleared["revision"], 3);
    assert_eq!(cleared["curation"], original["curation"]);
    assert_eq!(call(&store,json!({"op":"grouping-set","scope":"meenseek","id":company_id,"revision":3,"subject_id":null,"mode":"off"})).await["revision"],3);
    call(
        &store,
        json!({"op":"withdraw","scope":"meenseek","id":company_id,"revision":3}),
    )
    .await;
    assert_eq!(store.brain(serde_json::from_value(json!({"op":"grouping-set","scope":"meenseek","id":company_id,"revision":4,"subject_id":null,"mode":"off"})).unwrap()).await,Err(ontology::domain::Error::Conflict));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM memory_grouping WHERE memory_id=$1")
            .bind(company_id)
            .fetch_one(store.pool())
            .await
            .unwrap(),
        0
    );

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
    model_input_and_temporary_directory_cleanup(&store, &binary, temp.path()).await;
    unsafe {
        std::env::remove_var("ONTOLOGY_CODEX_BINARY");
    }
    metadata_only_memory_changes_preserve_downstream_evidence(&store).await;
}

async fn model_input_and_temporary_directory_cleanup(store: &Store, binary: &Path, temp: &Path) {
    assert!(!store.grouping_once().await.unwrap());
    let definition = json!({
        "purpose":"password: synthetic-purpose",
        "include":"학습 sk-synthetic123456789 검토",
        "exclude":"cookie=synthetic-exclusion"
    });
    let subject = call(store, json!({"op":"subject-create","scope":"personal","idempotency_key":"redaction-defined-subject","name":"API_KEY: synthetic-subject","definition":definition})).await;
    let fallback = call(store, json!({"op":"subject-create","scope":"personal","idempotency_key":"redaction-fallback-subject","name":"합성 예시 묶음"})).await;
    call(store, json!({"op":"remember","scope":"personal","idempotency_key":"redaction-example","memory":{"title":"secret: synthetic-example","body":"합성 예시","subject_id":fallback["id"]}})).await;
    let before: Value = sqlx::query_scalar("SELECT to_jsonb(s) FROM subjects s WHERE id=$1")
        .bind(subject["id"].as_str().unwrap())
        .fetch_one(store.pool())
        .await
        .unwrap();

    for outcome in ["success", "failure", "cancel"] {
        let capture = temp.join(format!("judgment-{outcome}"));
        std::fs::create_dir(&capture).unwrap();
        let output = json!({"type":"item.completed","item":{"type":"agent_message","text":judgment("assign",subject["id"].as_str(),vec![],None).to_string()}}).to_string();
        let finish = match outcome {
            "failure" => "exit 1".to_owned(),
            "cancel" => "exec /bin/sleep 60".to_owned(),
            _ => format!("printf '%s\\n' '{output}'"),
        };
        let script = format!(
            "#!/bin/sh\ncat >{}\npwd >{}\nprintf '%s' \"$$\" >{}\ntouch {}\n{finish}\n",
            quoted_path(&capture.join("input")),
            quoted_path(&capture.join("directory")),
            quoted_path(&capture.join("pid")),
            quoted_path(&capture.join("started")),
        );
        std::fs::write(binary, script).unwrap();
        std::fs::set_permissions(binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let memory = json!({"title":"token: synthetic-title","body":"일반 내용\npassword=synthetic-body\n끝"});
        let saved = call(store, json!({"op":"remember","scope":"personal","idempotency_key":format!("redaction-record-{outcome}"),"memory":memory})).await;
        let worker = {
            let store = store.clone();
            tokio::spawn(async move { store.grouping_once().await.unwrap() })
        };
        wait_marker(&capture.join("started")).await;
        let directory = std::path::PathBuf::from(
            std::fs::read_to_string(capture.join("directory"))
                .unwrap()
                .trim(),
        );
        if outcome == "cancel" {
            assert_eq!(
                std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
            worker.abort();
            assert!(worker.await.unwrap_err().is_cancelled());
            let pid: i32 = std::fs::read_to_string(capture.join("pid"))
                .unwrap()
                .parse()
                .unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while unsafe { libc::kill(pid, 0) } == 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("cancelled fake provider was stopped");
        } else {
            assert!(worker.await.unwrap());
        }
        assert!(
            !directory.exists(),
            "judgment directory must be removed after {outcome}"
        );
        if outcome == "success" {
            let prompt = std::fs::read_to_string(capture.join("input")).unwrap();
            let input: Value =
                serde_json::from_str(prompt.split("입력 JSON:\n").nth(1).unwrap()).unwrap();
            assert_eq!(input["record"]["title"], "token: [REDACTED]");
            assert_eq!(
                input["record"]["body"],
                "일반 내용\npassword= [REDACTED]\n끝"
            );
            let candidates = input["subjects"].as_array().unwrap();
            let defined = candidates
                .iter()
                .find(|s| s["id"] == subject["id"])
                .unwrap();
            assert_eq!(defined["revision"], subject["revision"]);
            assert_eq!(defined["name"], "API_KEY: [REDACTED]");
            assert_eq!(
                defined["definition"],
                json!({"purpose":"password: [REDACTED]","include":"학습 [REDACTED] 검토","exclude":"cookie= [REDACTED]"})
            );
            let fallback = candidates
                .iter()
                .find(|s| s["id"] == fallback["id"])
                .unwrap();
            assert_eq!(fallback["name"], "합성 예시 묶음");
            assert!(fallback["definition"].is_null());
            assert_eq!(fallback["example_titles"], json!(["secret: [REDACTED]"]));
            let current = call(
                store,
                json!({"op":"read","scope":"personal","id":saved["id"]}),
            )
            .await;
            assert_eq!(current["subject_id"], subject["id"]);
            assert_eq!(current["title"], memory["title"]);
            assert_eq!(current["body"], memory["body"]);
        } else if outcome == "failure" {
            let current = call(
                store,
                json!({"op":"read","scope":"personal","id":saved["id"]}),
            )
            .await;
            assert_eq!(current["grouping"]["state"], "error");
        }
    }
    let after: Value = sqlx::query_scalar("SELECT to_jsonb(s) FROM subjects s WHERE id=$1")
        .bind(subject["id"].as_str().unwrap())
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(
        after, before,
        "redaction must not change stored definitions"
    );
}

async fn metadata_only_memory_changes_preserve_downstream_evidence(store: &Store) {
    for scope in ["personal", "meenseek"] {
        let subject = call(store, json!({"op":"subject-create","scope":scope,"idempotency_key":format!("currency-purpose-{scope}"),"name":"근거 의미 보존"})).await;
        let input = json!({"title":"근거 의미 보존 원문","body":"문서와 상태는 보존한다","grouping_preference":"off"});
        let source = call(store, json!({"op":"remember","scope":scope,"idempotency_key":format!("currency-source-{scope}"),"memory":input})).await;
        let options = call(
            store,
            json!({"op":"evidence","scope":scope,"query":"근거 의미 보존 원문"}),
        )
        .await;
        let option = options["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["entity_id"] == source["id"])
            .unwrap();
        let pin = json!({"entity_id":option["entity_id"],"source_revision":option["source_revision"],"content_digest":option["content_digest"],"generation":option["generation"]});
        let dependent_input = json!({"title":"근거 의미 보존 판단","body":"현재 원문에 근거한 판단","evidence":[pin],"grouping_preference":"off"});
        let dependent = call(store,json!({"op":"remember","scope":scope,"idempotency_key":format!("currency-dependent-{scope}"),"memory":dependent_input})).await;
        let source_grouped = call(store,json!({"op":"grouping-set","scope":scope,"id":source["id"],"revision":source["revision"],"subject_id":subject["id"],"mode":"manual"})).await;
        let read = call(
            store,
            json!({"op":"read","scope":scope,"id":dependent["id"]}),
        )
        .await;
        assert_eq!(
            read["evidence"][0]["current"], true,
            "source membership alone does not stale a memory pin"
        );
        assert_eq!(
            read["evidence"][0]["generation"], source["revision"],
            "pins are not rewritten"
        );
        // Reusing the old pin in a new save must agree with the read/graph predicate.
        call(store,json!({"op":"remember","scope":scope,"idempotency_key":format!("currency-after-group-{scope}"),"memory":dependent_input})).await;
        let options = call(
            store,
            json!({"op":"evidence","scope":scope,"query":"근거 의미 보존 판단"}),
        )
        .await;
        let option = options["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["entity_id"] == dependent["id"])
            .unwrap();
        let dependent_pin = json!({"entity_id":option["entity_id"],"source_revision":option["source_revision"],"content_digest":option["content_digest"],"generation":option["generation"]});
        let dependent_grouped = call(store,json!({"op":"grouping-set","scope":scope,"id":dependent["id"],"revision":dependent["revision"],"subject_id":subject["id"],"mode":"manual"})).await;
        let chain = call(store,json!({"op":"remember","scope":scope,"idempotency_key":format!("currency-chain-{scope}"),"memory":{"body":"같은 원문을 이어받은 판단","evidence":[dependent_pin],"grouping_preference":"off"}})).await;
        assert_eq!(
            chain["evidence"].as_array().unwrap().len(),
            2,
            "flat native basis is preserved"
        );
        assert!(
            chain["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .all(|e| e["current"] == true)
        );
        let graph = store
            .graph(ontology::graph::GraphQuery {
                scope: scope.parse().unwrap(),
                q: "근거 의미 보존".into(),
                focus: Some(chain["id"].as_str().unwrap().into()),
                limit: 3,
            })
            .await
            .unwrap();
        let links: Vec<_> = graph["links"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|l| l["source"] == chain["id"] && l["kind"] == "evidence")
            .collect();
        assert_eq!(
            links.len(),
            2,
            "the complete flat basis is visible in the graph"
        );
        assert!(links.iter().all(|l| l["current"] == true));
        let renamed = call(store,json!({"op":"correct","scope":scope,"id":dependent["id"],"revision":dependent_grouped["revision"],"memory":{"title":"바뀐 의미 제목","body":dependent["body"],"evidence":[pin],"grouping_preference":"off"}})).await;
        assert_eq!(renamed["body"], dependent["body"]);
        assert_eq!(
            call(store, json!({"op":"read","scope":scope,"id":chain["id"]})).await["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["entity_id"] == dependent["id"])
                .unwrap()["current"],
            false,
            "same body with changed semantics is stale"
        );
        let changed = call(store,json!({"op":"correct","scope":scope,"id":source["id"],"revision":source_grouped["revision"],"memory":{"title":input["title"],"body":"실제 변경한 원문","grouping_preference":"off"}})).await;
        let restored = call(store,json!({"op":"correct","scope":scope,"id":source["id"],"revision":changed["revision"],"memory":input})).await;
        assert_eq!(restored["body"], source["body"]);
        assert_eq!(
            call(
                store,
                json!({"op":"read","scope":scope,"id":dependent["id"]})
            )
            .await["evidence"][0]["current"],
            false,
            "content ABA cannot revive an old pin"
        );
        assert_eq!(store.brain(serde_json::from_value(json!({"op":"remember","scope":scope,"idempotency_key":format!("currency-reject-aba-{scope}"),"memory":dependent_input})).unwrap()).await,Err(ontology::domain::Error::Conflict));
        call(store,json!({"op":"withdraw","scope":scope,"id":source["id"],"revision":restored["revision"]})).await;
        assert_eq!(
            call(
                store,
                json!({"op":"read","scope":scope,"id":dependent["id"]})
            )
            .await["evidence"][0]["current"],
            false
        );
        let gap = call(store,json!({"op":"remember","scope":scope,"idempotency_key":format!("currency-gap-{scope}"),"memory":input})).await;
        let options = call(
            store,
            json!({"op":"evidence","scope":scope,"query":"근거 의미 보존 원문"}),
        )
        .await;
        let option = options["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["entity_id"] == gap["id"])
            .unwrap();
        let pin = json!({"entity_id":option["entity_id"],"source_revision":option["source_revision"],"content_digest":option["content_digest"],"generation":option["generation"]});
        let gap_dependent = call(store,json!({"op":"remember","scope":scope,"idempotency_key":format!("currency-gap-dependent-{scope}"),"memory":{"body":"이력 누락을 확인한다","evidence":[pin],"grouping_preference":"off"}})).await;
        let grouped=call(store,json!({"op":"grouping-set","scope":scope,"id":gap["id"],"revision":1,"subject_id":subject["id"],"mode":"manual"})).await;
        call(store,json!({"op":"grouping-set","scope":scope,"id":gap["id"],"revision":grouped["revision"],"subject_id":null,"mode":"off"})).await;
        sqlx::query("DELETE FROM memory_history WHERE memory_id=$1 AND revision=2")
            .bind(gap["id"].as_str().unwrap())
            .execute(store.pool())
            .await
            .unwrap();
        assert_eq!(
            call(
                store,
                json!({"op":"read","scope":scope,"id":gap_dependent["id"]})
            )
            .await["evidence"][0]["current"],
            false,
            "missing intermediate history fails closed"
        );

        if scope == "personal" {
            let mode_source = call(store,json!({"op":"remember","scope":scope,"idempotency_key":"currency-mode-source","memory":input})).await;
            let hash = ontology::store::digest(input["body"].as_str().unwrap().as_bytes());
            let mode_pin = json!({"entity_id":mode_source["id"],"source_revision":hash,"content_digest":hash,"generation":1});
            let mode_dependent = call(store,json!({"op":"remember","scope":scope,"idempotency_key":"currency-mode-dependent","memory":{"body":"의도만 변경한 근거","evidence":[mode_pin],"grouping_preference":"off"}})).await;
            let auto = call(store,json!({"op":"grouping-set","scope":scope,"id":mode_source["id"],"revision":1,"subject_id":null,"mode":"auto"})).await;
            assert_eq!(auto["subject_id"], mode_source["subject_id"]);
            assert_eq!(
                call(
                    store,
                    json!({"op":"read","scope":scope,"id":mode_dependent["id"]})
                )
                .await["evidence"][0]["current"],
                true,
                "mode-only grouping preserves a pin"
            );
        }
        let semantic = call(store,json!({"op":"remember","scope":scope,"idempotency_key":format!("currency-same-correct-{scope}"),"memory":input})).await;
        let hash = ontology::store::digest(input["body"].as_str().unwrap().as_bytes());
        let semantic_pin = json!({"entity_id":semantic["id"],"source_revision":hash,"content_digest":hash,"generation":1});
        let semantic_dependent = call(store,json!({"op":"remember","scope":scope,"idempotency_key":format!("currency-same-dependent-{scope}"),"memory":{"body":"같은 JSON 정정도 재확인한다","evidence":[semantic_pin],"grouping_preference":"off"}})).await;
        let same = call(
            store,
            json!({"op":"correct","scope":scope,"id":semantic["id"],"revision":1,"memory":input}),
        )
        .await;
        assert_eq!(same["body"], semantic["body"]);
        call(store,json!({"op":"grouping-set","scope":scope,"id":same["id"],"revision":same["revision"],"subject_id":subject["id"],"mode":"manual"})).await;
        assert_eq!(
            call(
                store,
                json!({"op":"read","scope":scope,"id":semantic_dependent["id"]})
            )
            .await["evidence"][0]["current"],
            false,
            "later grouping cannot revive a same-JSON correction"
        );
        let delete_source = call(store,json!({"op":"remember","scope":scope,"idempotency_key":format!("currency-delete-source-{scope}"),"memory":input})).await;
        let delete_pin = json!({"entity_id":delete_source["id"],"source_revision":hash,"content_digest":hash,"generation":1});
        let delete_dependent = call(store,json!({"op":"remember","scope":scope,"idempotency_key":format!("currency-delete-dependent-{scope}"),"memory":{"body":"목적 삭제는 원문 변경이 아니다","evidence":[delete_pin],"grouping_preference":"off"}})).await;
        call(store,json!({"op":"grouping-set","scope":scope,"id":delete_source["id"],"revision":1,"subject_id":subject["id"],"mode":"manual"})).await;
        call(
            store,
            json!({"op":"subject-delete","scope":scope,"id":subject["id"]}),
        )
        .await;
        assert_eq!(
            call(
                store,
                json!({"op":"read","scope":scope,"id":delete_dependent["id"]})
            )
            .await["evidence"][0]["current"],
            true,
            "subject deletion preserves unchanged source semantics"
        );
        assert_eq!(
            call(
                store,
                json!({"op":"read","scope":scope,"id":semantic_dependent["id"]})
            )
            .await["evidence"][0]["current"],
            false
        );
    }
}
