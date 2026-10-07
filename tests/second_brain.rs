static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
use ontology::{
    domain::{Error, ImportedRecord, Scope, SourceKind},
    memory::BrainCommand,
    store::{Store, digest},
};
use serde_json::{Value, json};
async fn store() -> Store {
    let url = std::env::var("TEST_DATABASE_URL").expect("explicit isolated DB");
    assert!(
        ontology::config::database_options(&url)
            .expect("loopback")
            .get_database()
            .is_some_and(|v| v.starts_with("ontology_test_"))
    );
    let store = Store::connect(&url).await.expect("synthetic DB");
    store.initialize().await.expect("migrate synthetic DB");
    store
}
fn cmd(v: Value) -> BrainCommand {
    serde_json::from_value(v).expect("typed fixture command")
}
fn input() -> Value {
    json!({"kind":"decision","title":"합성 프로젝트 결정","body":"한국어 실험 English experiment","evidence":[]})
}
async fn call(store: &Store, v: Value) -> Value {
    store
        .brain(cmd(v))
        .await
        .expect("valid synthetic operation")
}
#[tokio::test]
async fn native_memory_contract() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    sqlx::query("TRUNCATE document_subjects,memory_grouping,evidence_snapshots,evidence_contents,memory_history,memories,memory_creations,subjects")
        .execute(store.pool())
        .await
        .expect("reset synthetic memories only");
    let subject=call(&store,json!({"op":"subject-create","scope":"personal","idempotency_key":"subject-key","name":"합성 프로젝트"})).await;
    let again=call(&store,json!({"op":"subject-create","scope":"personal","idempotency_key":"subject-key","name":"합성 프로젝트"})).await;
    assert_eq!(subject, again);
    let mut memory = input();
    memory["subject_id"] = subject["id"].clone();
    let create =
        json!({"op":"remember","scope":"personal","idempotency_key":"test-create","memory":memory});
    let (first, second) = tokio::join!(
        store.brain(cmd(create.clone())),
        store.brain(cmd(create.clone()))
    );
    let first = first.expect("first concurrent creation");
    assert_eq!(first, second.expect("exact replay"));
    let id = first["id"].as_str().expect("id");
    assert_eq!(first["origin"], "user");
    assert_eq!(first["revision"], 1);
    assert_eq!(
        store
            .brain(cmd(json!({"op":"read","scope":"meenseek","id":id})))
            .await,
        Err(Error::NotFound)
    );
    let mut different = create.clone();
    different["memory"]["body"] = json!("different");
    assert_eq!(store.brain(cmd(different)).await, Err(Error::Conflict));
    let history = call(&store, json!({"op":"history","scope":"personal","id":id})).await;
    assert_eq!(history["items"].as_array().expect("history").len(), 1);
    let mut changed = memory.clone();
    changed["body"] = json!("새 결정 replaced experiment");
    let correction =
        json!({"op":"correct","scope":"personal","id":id,"revision":1,"memory":changed});
    let (a, b) = tokio::join!(
        store.brain(cmd(correction.clone())),
        store.brain(cmd(correction))
    );
    assert!(matches!(
        (a, b),
        (Ok(_), Err(Error::Conflict)) | (Err(Error::Conflict), Ok(_))
    ));
    let current = call(&store, json!({"op":"read","scope":"personal","id":id})).await;
    assert_eq!(current["revision"], 2);
    assert_eq!(current["body"], changed["body"]);
    assert!(current.get("history").is_none());
    let search = call(
        &store,
        json!({"op":"search","scope":"personal","query":"새 결정"}),
    )
    .await;
    assert_eq!(search["nodes"][0]["id"], id);
    assert_eq!(
        call(
            &store,
            json!({"op":"search","scope":"meenseek","query":"새 결정"})
        )
        .await["matched"],
        0
    );
    call(
        &store,
        json!({"op":"withdraw","scope":"personal","id":id,"revision":2}),
    )
    .await;
    assert_eq!(
        call(
            &store,
            json!({"op":"search","scope":"personal","query":"새 결정"})
        )
        .await["nodes"][0]["status"],
        "withdrawn"
    );
    assert_eq!(
        call(&store, json!({"op":"history","scope":"personal","id":id})).await["items"]
            .as_array()
            .expect("history")
            .len(),
        3
    );
    call(
        &store,
        json!({"op":"forget","scope":"personal","id":id,"revision":3}),
    )
    .await;
    call(
        &store,
        json!({"op":"forget","scope":"personal","id":id,"revision":3}),
    )
    .await;
    assert_eq!(store.brain(cmd(create)).await, Err(Error::Gone));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM memory_history WHERE memory_id=$1")
        .bind(id)
        .fetch_one(store.pool())
        .await
        .expect("erased history");
    assert_eq!(count, 0);
    let tombstone: Value =
        sqlx::query_scalar("SELECT to_jsonb(c) FROM memory_creations c WHERE memory_id=$1")
            .bind(id)
            .fetch_one(store.pool())
            .await
            .expect("minimal tombstone");
    assert_eq!(tombstone.as_object().expect("object").len(), 4);
    assert!(!tombstone.to_string().contains("test-create"));
    let proposal=call(&store,json!({"op":"propose","scope":"personal","idempotency_key":"proposal-key","memory":input()})).await;
    assert_eq!(proposal["status"], "proposed");
    assert_eq!(proposal["origin"], "assistant");
    assert_eq!(
        call(
            &store,
            json!({"op":"search","scope":"personal","query":"experiment"})
        )
        .await["nodes"][0]["status"],
        "proposed"
    );
    call(
        &store,
        json!({"op":"accept","scope":"personal","id":proposal["id"],"revision":1}),
    )
    .await;
    assert_eq!(
        call(
            &store,
            json!({"op":"search","scope":"personal","query":"EXPERIMENT"})
        )
        .await["nodes"][0]["status"],
        "accepted"
    );
    for (key, from, until) in [
        ("future-key", Some(253402300000i64), None),
        ("expired-key", None, Some(1)),
    ] {
        let mut m = input();
        m["title"] = json!("windowonly");
        m["effective_from"] = json!(from);
        m["effective_until"] = json!(until);
        call(
            &store,
            json!({"op":"remember","scope":"personal","idempotency_key":key,"memory":m}),
        )
        .await;
    }
    assert_eq!(
        call(
            &store,
            json!({"op":"search","scope":"personal","query":"windowonly"})
        )
        .await["matched"],
        2
    );
    let hash = digest(b"brain-source");
    let mut record = ImportedRecord {
        source_id: format!("s_{hash}"),
        entity_id: format!("e_{hash}"),
        scope: Scope::Personal,
        repository: "/synthetic/brain".into(),
        path: "brain.md".into(),
        kind: SourceKind::Git,
        source_revision: "a".repeat(40),
        digest: Some(digest(b"source")),
        content: Some("source".into()),
    };
    store
        .apply_import(&[record.clone()])
        .await
        .expect("evidence source");
    let options = call(
        &store,
        json!({"op":"evidence","scope":"personal","query":"brain.md"}),
    )
    .await;
    let e = &options["items"][0];
    let evidence = json!({"entity_id":e["entity_id"],"source_revision":e["source_revision"],"content_digest":e["content_digest"],"generation":e["generation"]});
    let mut supported = input();
    supported["title"] = json!("supportedonly");
    supported["evidence"] = json!([evidence]);
    let supported_create = json!({"op":"remember","scope":"personal","idempotency_key":"evidence-key","memory":supported});
    let supported_item = call(&store, supported_create).await;
    assert_eq!(supported_item["evidence"][0]["current"], true);
    let mut wrong_scope = json!({"op":"remember","scope":"meenseek","idempotency_key":"wrong-scope","memory":supported});
    wrong_scope["memory"]["subject_id"] = Value::Null;
    assert_eq!(store.brain(cmd(wrong_scope)).await, Err(Error::Invalid));
    store
        .mark_failed(&[record.source_id.clone()], SourceKind::Git)
        .await
        .expect("failure");
    assert_eq!(
        call(
            &store,
            json!({"op":"read","scope":"personal","id":supported_item["id"]})
        )
        .await["evidence"][0]["current"],
        false
    );
    store
        .apply_import(&[record.clone()])
        .await
        .expect("recover same source");
    assert_eq!(
        call(
            &store,
            json!({"op":"search","scope":"personal","query":"supportedonly"})
        )
        .await["nodes"][0]["supported"],
        false,
        "recovery requires explicit evidence reconfirmation"
    );
    let options = call(
        &store,
        json!({"op":"evidence","scope":"personal","query":"brain.md"}),
    )
    .await;
    supported["evidence"][0]["generation"] = options["items"][0]["generation"].clone();
    call(&store,json!({"op":"correct","scope":"personal","id":supported_item["id"],"revision":1,"memory":supported})).await;
    assert_eq!(
        call(
            &store,
            json!({"op":"search","scope":"personal","query":"supportedonly"})
        )
        .await["nodes"][0]["supported"],
        true
    );
    record.source_revision = "b".repeat(40);
    store
        .apply_import(&[record.clone()])
        .await
        .expect("changed revision");
    assert_eq!(
        call(
            &store,
            json!({"op":"search","scope":"personal","query":"supportedonly"})
        )
        .await["nodes"][0]["supported"],
        false
    );
    record.content = None;
    record.digest = None;
    store.apply_import(&[record]).await.expect("missing source");
    assert!(
        call(
            &store,
            json!({"op":"evidence","scope":"personal","query":"brain.md"})
        )
        .await["items"]
            .as_array()
            .expect("options")
            .is_empty()
    );
    // Different cardinalities retain one set-query for retrieval; output is bounded under large bodies.
    for size in [0, 1, 20] {
        for i in 0..size {
            let mut m = input();
            m["body"] = json!(format!("batch{size} {}", "x".repeat(8100)));
            call(&store,json!({"op":"remember","scope":"meenseek","idempotency_key":format!("batch-{size}-{i}"),"memory":m})).await;
        }
        let before = store.calls();
        let result = call(
            &store,
            json!({"op":"search","scope":"meenseek","query":format!("batch{size}")}),
        )
        .await;
        assert_eq!(store.calls() - before, 1);
        assert!(serde_json::to_vec(&result).expect("JSON").len() <= 65536);
    }
    let list = call(&store, json!({"op":"list","scope":"meenseek","limit":1})).await;
    assert!(list["next_after"].is_string());
    for bad in [
        json!({"op":"search","scope":"personal","query":"x","extra":true}),
        json!({"op":"remember","scope":"personal","idempotency_key":"long-key","memory":{"kind":"arbitrary","title":"x","body":"y"}}),
    ] {
        assert!(serde_json::from_value::<BrainCommand>(bad).is_err());
    }
    assert_eq!(
        store
            .brain(cmd(
                json!({"op":"search","scope":"personal","query":"x".repeat(121)})
            ))
            .await,
        Err(Error::Invalid)
    );
}

#[tokio::test]
async fn detail_and_subject_queries_stay_bounded_across_cardinalities() {
    use ontology::domain::{Classification, LinkChange, MAX_RESPONSE_BYTES};
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    for size in [0, 1, 20] {
        sqlx::query("TRUNCATE document_subjects,evidence_snapshots,evidence_contents,memory_history,memories,memory_creations,subjects,confirmation_history,related_materials,entity_areas,entity_topics,source_records,entities,sources,topics RESTART IDENTITY CASCADE")
            .execute(store.pool()).await.expect("reset isolated query fixture");
        let attached = if size == 20 { 10 } else { 0 };
        let records: Vec<_> = (0..=attached)
            .map(|i| {
                let hash = digest(format!("detail-bounds-{size}-{i}").as_bytes());
                ImportedRecord {
                    source_id: format!("s_{hash}"),
                    entity_id: format!("e_{hash}"),
                    scope: Scope::Personal,
                    repository: "/synthetic/detail-bounds".into(),
                    path: format!("document-{i}.md"),
                    kind: SourceKind::Git,
                    source_revision: "a".repeat(40),
                    digest: Some(digest(b"synthetic source")),
                    content: Some("synthetic source".into()),
                }
            })
            .collect();
        store
            .apply_import(&records)
            .await
            .expect("synthetic documents");
        let document_id = &records[0].entity_id;
        if attached > 0 {
            store
                .classify(
                    Scope::Personal,
                    document_id,
                    Classification {
                        revision: 0,
                        areas: vec![],
                        topics: (0..attached).map(|i| format!("topic-{i}")).collect(),
                    },
                )
                .await
                .expect("synthetic topics");
            for (i, record) in records.iter().skip(1).enumerate() {
                store
                    .link(
                        Scope::Personal,
                        document_id,
                        LinkChange {
                            revision: 1 + i as i64,
                            target_id: record.entity_id.clone(),
                            remove: false,
                        },
                    )
                    .await
                    .expect("synthetic relation");
            }
        }
        let options = call(
            &store,
            json!({"op":"evidence","scope":"personal","query":"document-","limit":20}),
        )
        .await;
        let evidence: Vec<_> = options["items"].as_array().expect("evidence options").iter().take(attached)
            .map(|e| json!({"entity_id":e["entity_id"],"source_revision":e["source_revision"],"content_digest":e["content_digest"],"generation":e["generation"]})).collect();
        assert_eq!(evidence.len(), attached);
        let mut ids = Vec::new();
        for i in 0..size {
            let subject = call(&store, json!({"op":"subject-create","scope":"personal","idempotency_key":format!("subject-{i}"),"name":format!("subject-{i}")})).await;
            let mut memory = input();
            memory["subject_id"] = subject["id"].clone();
            memory["body"] = json!("x".repeat(8192));
            memory["evidence"] = json!(evidence);
            let created = call(&store, json!({"op":"remember","scope":"personal","idempotency_key":format!("memory-{i}"),"memory":memory})).await;
            ids.push(created["id"].as_str().expect("memory id").to_owned());
        }
        let before = store.calls();
        let subjects = call(
            &store,
            json!({"op":"subjects","scope":"personal","limit":20}),
        )
        .await;
        assert_eq!(store.calls() - before, 1, "subjects with {size} memories");
        assert_eq!(subjects["items"].as_array().expect("subjects").len(), size);
        assert!(serde_json::to_vec(&subjects).expect("subjects JSON").len() <= MAX_RESPONSE_BYTES);
        for id in &ids {
            let before = store.calls();
            let memory = store
                .memory_detail(Scope::Personal, id)
                .await
                .expect("memory detail");
            assert_eq!(
                store.calls() - before,
                1,
                "memory detail with {attached} evidence among {size} memories"
            );
            assert_eq!(
                memory["evidence"].as_array().expect("evidence").len(),
                attached
            );
            assert!(memory["subject_name"].is_string());
            assert_eq!(memory["body"].as_str().expect("body").len(), 8192);
            assert!(serde_json::to_vec(&memory).expect("memory JSON").len() <= MAX_RESPONSE_BYTES);
        }
        let before = store.calls();
        let document = store
            .detail(Scope::Personal, document_id)
            .await
            .expect("document detail");
        assert_eq!(
            store.calls() - before,
            1,
            "document detail with {attached} relations and topics"
        );
        assert_eq!(
            document["related"].as_array().expect("related").len(),
            attached
        );
        assert_eq!(
            document["topics"].as_array().expect("topics").len(),
            attached
        );
        assert!(serde_json::to_vec(&document).expect("document JSON").len() <= MAX_RESPONSE_BYTES);
        for (id, expected, count) in [
            ("invalid".to_owned(), Error::Invalid, 0),
            (
                "m_00000000-0000-0000-0000-000000000000".to_owned(),
                Error::NotFound,
                1,
            ),
        ] {
            let before = store.calls();
            assert_eq!(
                store.memory_detail(Scope::Personal, &id).await,
                Err(expected)
            );
            assert_eq!(store.calls() - before, count);
        }
        for (scope, id, expected, count) in [
            (Scope::Personal, "invalid".to_owned(), Error::Invalid, 0),
            (
                Scope::Personal,
                format!("e_{}", "0".repeat(64)),
                Error::NotFound,
                1,
            ),
            (Scope::Meenseek, document_id.clone(), Error::NotFound, 1),
        ] {
            let before = store.calls();
            assert_eq!(store.detail(scope, &id).await, Err(expected));
            assert_eq!(store.calls() - before, count);
        }
        let before = store.calls();
        assert_eq!(
            store
                .brain(cmd(
                    json!({"op":"subjects","scope":"personal","after":"invalid"})
                ))
                .await,
            Err(Error::Invalid)
        );
        assert_eq!(store.calls() - before, 0);
    }
}

#[tokio::test]
async fn brain_transport_contract() {
    let _guard = TEST_LOCK.lock().await;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use ontology::{
        api::{AppState, router},
        config::Config,
    };
    use tower::ServiceExt;
    let store = store().await;
    let app = router(AppState::new(
        store.clone(),
        Config {
            address: "127.0.0.1:47831".parse().expect("loopback"),
            web_dist: "web/dist".into(),
        },
    ));
    let session = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header("host", "127.0.0.1:47831")
                .body(Body::empty())
                .expect("session"),
        )
        .await
        .expect("response");
    let cookie = session.headers()["set-cookie"]
        .to_str()
        .expect("cookie")
        .split(';')
        .next()
        .expect("pair")
        .to_owned();
    let session: Value = serde_json::from_slice(
        &session
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes(),
    )
    .expect("JSON");
    for (origin, token, body, status) in [
        (
            "http://evil.invalid",
            session["csrf"].as_str().expect("csrf"),
            json!({"op":"subjects","scope":"personal"}).to_string(),
            403,
        ),
        (
            "http://127.0.0.1:47831",
            "wrong",
            json!({"op":"subjects","scope":"personal"}).to_string(),
            403,
        ),
        (
            "http://127.0.0.1:47831",
            session["csrf"].as_str().expect("csrf"),
            json!({"op":"search","scope":"personal","query":"x","body":"sensitive-marker"})
                .to_string(),
            400,
        ),
        (
            "http://127.0.0.1:47831",
            session["csrf"].as_str().expect("csrf"),
            "sensitive-marker".repeat(2000),
            400,
        ),
        (
            "http://127.0.0.1:47831",
            session["csrf"].as_str().expect("csrf"),
            json!({"op":"subjects","scope":"personal"}).to_string(),
            200,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/brain")
                    .header("host", "127.0.0.1:47831")
                    .header("origin", origin)
                    .header("cookie", &cookie)
                    .header("x-csrf-token", token)
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status().as_u16(), status);
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        assert!(!String::from_utf8_lossy(&bytes).contains("sensitive-marker"));
    }
    let key = format!("cli-{}", uuid::Uuid::new_v4());
    let mut cli_input = input();
    cli_input["grouping_preference"] = json!("off");
    let creation =
        json!({"op":"remember","scope":"personal","idempotency_key":key,"memory":cli_input});
    let (ok, first) = cli(&creation.to_string()).await;
    assert!(ok);
    let (ok, repeat) = cli(&creation.to_string()).await;
    assert!(ok);
    assert_eq!(first, repeat);
    let id = &first["id"];
    let (ok, recalled) =
        cli(&json!({"op":"search","scope":"personal","query":"experiment"}).to_string()).await;
    assert!(ok);
    assert!(
        recalled["nodes"]
            .as_array()
            .expect("items")
            .iter()
            .any(|m| m["id"] == *id)
    );
    let (ok, _) =
        cli(&json!({"op":"forget","scope":"personal","id":id,"revision":1}).to_string()).await;
    assert!(ok);
    let (ok, erased) = cli(&creation.to_string()).await;
    assert!(!ok);
    assert!(
        erased["error"]
            .as_str()
            .expect("error")
            .contains("forgotten")
    );
    for bad in [
        "not-json sensitive-marker".into(),
        "sensitive-marker".repeat(2000),
        "{\"op\":\"subjects\",\"scope\":\"personal\",\"scope\":\"meenseek\"}".into(),
    ] {
        let (ok, error) = cli(&bad).await;
        assert!(!ok);
        assert!(!error.to_string().contains("sensitive-marker"));
    }
}
async fn cli(body: &str) -> (bool, Value) {
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_ontology"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env(
            "DATABASE_URL",
            std::env::var("TEST_DATABASE_URL").expect("synthetic URL"),
        )
        .arg("brain")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("candidate CLI");
    let mut stdin = child.stdin.take().expect("piped stdin");
    let _ = stdin.write_all(body.as_bytes()).await;
    drop(stdin);
    let output = child.wait_with_output().await.expect("CLI result");
    assert!(output.stderr.is_empty(), "brain failures use bounded JSON");
    assert!(output.stdout.len() <= 1_048_576);
    (
        output.status.success(),
        serde_json::from_slice(&output.stdout).expect("JSON stdout"),
    )
}

fn git(repo: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("/usr/bin/git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("synthetic Git");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("UTF8")
        .trim()
        .to_owned()
}
#[tokio::test]
async fn sync_contract() {
    let _guard = TEST_LOCK.lock().await;
    use ontology::{
        importer::GitReader,
        sync::{SyncConfig, refresh},
    };
    let store = store().await;
    let temp = tempfile::tempdir().expect("isolated sources");
    let root = temp.path().canonicalize().expect("canonical");
    let repo = root.join("git");
    std::fs::create_dir(&repo).expect("repo");
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "test@example.invalid"]);
    git(&repo, &["config", "user.name", "Synthetic"]);
    for i in 0..20 {
        std::fs::write(
            repo.join(format!("source-{i}.md")),
            format!("# Synthetic source {i}"),
        )
        .expect("fixture");
    }
    std::fs::write(repo.join("outside.md"), "outside source").expect("fixture");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "initial"]);
    let config_path = root.join("sync.json");
    let config = json!({"interval_seconds":15,"sources":[{"kind":"git","root":repo,"scope":"personal","ref":"HEAD","paths":["source-0.md"]}]});
    let write_config = |v: &Value| {
        std::fs::write(
            &config_path,
            serde_json::to_vec(v).expect("serialize fixture"),
        )
        .expect("config")
    };
    for size in [1, 20] {
        let mut c = config.clone();
        c["sources"][0]["paths"] = json!(
            (0..size)
                .map(|i| format!("source-{i}.md"))
                .collect::<Vec<_>>()
        );
        write_config(&c);
        let before = store.calls();
        let report = refresh(&store, &config_path).await.expect("shared refresh");
        assert!(report.ok);
        assert_eq!(store.calls() - before, 6);
        assert_eq!(report.sources[0].provider_calls, 4 + 2 * size as u64);
        assert!(report.sources[0].response_bytes <= (size * 2048 + 4096) as u64);
    }
    write_config(&config);
    let reader = GitReader::new(vec![repo.clone()]).expect("reader");
    let commit = git(&repo, &["rev-parse", "HEAD"]);
    reader
        .import(
            &store,
            &repo,
            &commit,
            &["outside.md".into()],
            Scope::Personal,
        )
        .await
        .expect("outside config imported manually");
    let (_, outside) =
        ontology::importer::identity(&repo, "outside.md", Scope::Personal).expect("id");
    let outside_before = store
        .detail(Scope::Personal, &outside)
        .await
        .expect("outside");
    let (_, id) = ontology::importer::identity(&repo, "source-0.md", Scope::Personal).expect("id");
    let options = call(
        &store,
        json!({"op":"evidence","scope":"personal","query":"source-0.md"}),
    )
    .await;
    let e = options["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|v| v["entity_id"] == id)
        .expect("own evidence");
    let mut m = input();
    m["evidence"] = json!([{"entity_id":e["entity_id"],"source_revision":e["source_revision"],"content_digest":e["content_digest"],"generation":e["generation"]}]);
    let item=call(&store,json!({"op":"remember","scope":"personal","idempotency_key":format!("sync-{}",uuid::Uuid::new_v4()),"memory":m})).await;
    refresh(&store, &config_path)
        .await
        .expect("repeat same state");
    assert_eq!(
        call(
            &store,
            json!({"op":"read","scope":"personal","id":item["id"]})
        )
        .await["evidence"][0]["current"],
        true
    );
    let lock = store
        .lock_import()
        .await
        .expect("hold an earlier observation");
    let before = reader.calls();
    assert_eq!(
        reader
            .import(
                &store,
                &repo,
                &commit,
                &["source-0.md".into()],
                Scope::Personal
            )
            .await,
        Err(Error::Conflict)
    );
    assert_eq!(reader.calls(), before);
    assert!(matches!(
        refresh(&store, &config_path).await,
        Err(Error::Conflict)
    ));
    std::fs::write(repo.join("source-0.md"), "changed current source")
        .expect("change while another import owns lock");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "changed"]);
    store.finish_import(lock).await.expect("release");
    // A fresh Store models restart: resolve the current HEAD, without replaying an earlier snapshot.
    let restarted = Store::connect(&std::env::var("TEST_DATABASE_URL").expect("synthetic"))
        .await
        .expect("restart");
    refresh(&restarted, &config_path).await.expect("catch up");
    assert_eq!(
        store
            .detail(Scope::Personal, &id)
            .await
            .expect("new source")["projection"]["content"],
        "changed current source"
    );
    assert_eq!(
        call(
            &store,
            json!({"op":"read","scope":"personal","id":item["id"]})
        )
        .await["evidence"][0]["current"],
        false
    );
    assert_eq!(
        call(
            &store,
            json!({"op":"history","scope":"personal","id":item["id"]})
        )
        .await["items"]
            .as_array()
            .expect("history")
            .len(),
        1
    );
    for invalid in [
        json!({"interval_seconds":0,"sources":[]}),
        json!({"interval_seconds":15,"sources":[config["sources"][0],{"kind":"git","root":repo,"scope":"personal","ref":"HEAD","paths":["*.md"]}]}),
        json!({"interval_seconds":15,"sources":[config["sources"][0],config["sources"][0]]}),
    ] {
        write_config(&invalid);
        let before = store.calls();
        assert!(refresh(&store, &config_path).await.is_err());
        assert_eq!(
            store.calls(),
            before,
            "invalid configuration makes no provider/storage observations"
        );
    }
    std::fs::write(&config_path, "x".repeat(32769)).expect("oversized");
    assert!(SyncConfig::load(&config_path).is_err());
    // Mixed Git and Context sources use the same guarded transaction and isolate errors.
    let vault = root.join("vault");
    std::fs::create_dir_all(vault.join("personal/projects")).expect("fixture");
    std::fs::write(
        vault.join("personal/projects/note.md"),
        "---\ntitle: Synthetic\nscope: personal\nexport: false\n---\n\nInitial note\n",
    )
    .expect("fixture");
    let scopes = vec!["personal".parse().expect("scope")];
    let inventory = ontology::context::inventory(&vault, &scopes).expect("inventory");
    store
        .import_context(&vault, &scopes, &inventory.inventory_digest)
        .await
        .expect("canonical import");
    let store_id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
        .fetch_one(store.pool())
        .await
        .expect("identity");
    let mut both = config.clone();
    both["sources"].as_array_mut().expect("sources").push(json!({"kind":"context","store_id":store_id,"context_scope":"personal","scope":"personal","paths":["projects/note.md"]}));
    write_config(&both);
    let first = refresh(&store, &config_path).await.expect("mixed");
    assert!(first.ok);
    assert_eq!(first.sources[1].provider_calls, 1);
    std::fs::remove_dir_all(&vault).expect("remove original transport");
    assert!(
        refresh(&store, &config_path)
            .await
            .expect("canonical only")
            .ok
    );
    both["sources"][1]["paths"] = json!(["projects/note.md", "missing.md"]);
    write_config(&both);
    let failed = refresh(&store, &config_path)
        .await
        .expect("isolated missing exact material");
    assert!(failed.sources[0].ok);
    assert!(!failed.sources[1].ok);
    assert_eq!(
        store
            .detail(Scope::Personal, &outside)
            .await
            .expect("outside"),
        outside_before
    );
}

#[tokio::test]
async fn capture_search_and_historical_evidence_contract() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    sqlx::query("TRUNCATE document_subjects,evidence_snapshots,evidence_contents,memory_history,memories,memory_creations,subjects,confirmation_history,related_materials,entity_areas,entity_topics,source_records,entities,sources,topics RESTART IDENTITY CASCADE").execute(store.pool()).await.unwrap();
    let content_only = json!({"op":"remember","scope":"meenseek","idempotency_key":"body-only-record","memory":{"body":"  관찰한 내용\n\n아직 판단하지 않았다."}});
    let record = call(&store, content_only.clone()).await;
    assert_eq!(record["kind"], "record");
    assert_eq!(record["title_from_body"], true);
    assert_eq!(record["title"], "관찰한 내용");
    assert_eq!(record["body"], content_only["memory"]["body"]);
    assert!(record["subject_id"].is_null());
    assert_eq!(call(&store, content_only.clone()).await, record);
    let mut explicit = content_only;
    explicit["memory"]["kind"] = json!("record");
    explicit["memory"]["title"] = record["title"].clone();
    assert_eq!(
        store.brain(cmd(explicit)).await,
        Err(Error::Conflict),
        "a manual title has different future editing behavior from an automatic title"
    );
    let retry = json!({"op":"remember","scope":"meenseek","idempotency_key":"body-only-record","memory":{"kind":"record","title":"  ","body":"  관찰한 내용\n\n아직 판단하지 않았다."}});
    assert_eq!(
        call(&store, retry).await,
        record,
        "same automatic intent normalizes consistently"
    );
    let manual = call(&store,json!({"op":"remember","scope":"meenseek","idempotency_key":"legacy-manual-key","memory":{"kind":"fact","title":"Manual","body":"Body"}})).await;
    let stored_digest: String =
        sqlx::query_scalar("SELECT payload_digest FROM memory_creations WHERE memory_id=$1")
            .bind(manual["id"].as_str().unwrap())
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(stored_digest,digest(br#"[false,{"kind":"fact","title":"Manual","body":"Body","subject_id":null,"effective_from":null,"effective_until":null,"evidence":[]}]"#),"explicit title keeps the previously deployed request digest");
    call(&store, json!({"op":"correct","scope":"meenseek","id":record["id"],"revision":1,"memory":{"body":"결론을 내릴 근거가 부족하다."}})).await;
    call(
        &store,
        json!({"op":"withdraw","scope":"meenseek","id":record["id"],"revision":2}),
    )
    .await;
    let result = call(
        &store,
        json!({"op":"search","scope":"meenseek","query":"관찰한 내용"}),
    )
    .await;
    assert_eq!(result["nodes"].as_array().unwrap().len(), 1);
    let found = &result["nodes"][0];
    assert_eq!(found["id"], record["id"]);
    assert_eq!(found["revision"], "3");
    assert_eq!(found["label"], "결론을 내릴 근거가 부족하다.");
    assert_eq!(found["status"], "withdrawn");
    assert_eq!(found["historical_match"], true);
    assert_eq!(found["matched_revision"], "1");
    assert_eq!(
        call(
            &store,
            json!({"op":"search","scope":"personal","query":"관찰한 내용"})
        )
        .await["matched"],
        0
    );

    let source = |scope: Scope| {
        let hash = digest(format!("evidence-lifecycle-{}", scope.as_str()).as_bytes());
        ImportedRecord {
            source_id: format!("s_{hash}"),
            entity_id: format!("e_{hash}"),
            scope,
            repository: "/synthetic/preserved".into(),
            path: "policy.md".into(),
            kind: SourceKind::Git,
            source_revision: "a".repeat(40),
            digest: Some(digest(b"# Original\n\nOnly if all conditions apply.")),
            content: Some("# Original\n\nOnly if all conditions apply.".into()),
        }
    };
    let mut original = source(Scope::Meenseek);
    let personal = source(Scope::Personal);
    store
        .apply_import(&[original.clone(), personal])
        .await
        .unwrap();
    let mut ids = Vec::new();
    for (scope, key) in [
        ("meenseek", "first-evidence"),
        ("meenseek", "second-evidence"),
        ("personal", "private-evidence"),
    ] {
        let options = call(
            &store,
            json!({"op":"evidence","scope":scope,"query":"policy.md"}),
        )
        .await;
        let e = &options["items"][0];
        let reference = json!({"entity_id":e["entity_id"],"source_revision":e["source_revision"],"content_digest":e["content_digest"],"generation":e["generation"]});
        let memory = call(&store,json!({"op":"propose","scope":scope,"idempotency_key":key,"memory":{"body":"조건을 모두 확인하고 실행한다.","evidence":[reference]}})).await;
        ids.push(memory["id"].as_str().unwrap().to_owned());
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM evidence_contents")
            .fetch_one(store.pool())
            .await
            .unwrap(),
        2,
        "identical bytes deduplicate only inside each scope"
    );
    for (i, body, title) in [
        (0, "# 회의\n\n본문", "회의"),
        (1, "# **회의** `결과`\n\n본문", "회의 결과"),
        (
            2,
            "# [운영 정책](https://example.invalid)\n\n본문",
            "운영 정책",
        ),
    ] {
        let created = call(&store,json!({"op":"remember","scope":"meenseek","idempotency_key":format!("markdown-title-{i}"),"memory":{"body":body}})).await;
        assert_eq!(created["title"], title);
        assert_eq!(created["body"], body);
    }
    let old_body = original.content.clone().unwrap();
    call(
        &store,
        json!({"op":"accept","scope":"meenseek","id":ids[0],"revision":1}),
    )
    .await;
    original.content = Some("# Changed\n\nDo not proceed.".into());
    original.digest = Some(digest(original.content.as_ref().unwrap().as_bytes()));
    original.source_revision = "b".repeat(40);
    store.apply_import(&[original.clone()]).await.unwrap();

    call(
        &store,
        json!({"op":"withdraw","scope":"meenseek","id":ids[0],"revision":2}),
    )
    .await;
    for revision in 1..=3 {
        let before = store.calls();
        let read = call(&store,json!({"op":"evidence-read","scope":"meenseek","id":ids[0],"revision":revision,"entity_id":original.entity_id})).await;
        assert_eq!(store.calls() - before, 1);
        assert_eq!(read["content"], old_body);
        assert_eq!(read["evidence"]["source_revision"], "a".repeat(40));
        assert_eq!(read["available"], true);
    }
    assert_eq!(store.brain(cmd(json!({"op":"evidence-read","scope":"personal","id":ids[0],"revision":1,"entity_id":original.entity_id}))).await,Err(Error::NotFound));
    let found = call(
        &store,
        json!({"op":"search","scope":"meenseek","query":"조건을"}),
    )
    .await;
    assert!(
        found["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|node| node["supported"] == false)
    );
    original.content = None;
    original.digest = None;
    store.apply_import(&[original.clone()]).await.unwrap();
    assert_eq!(call(&store,json!({"op":"evidence-read","scope":"meenseek","id":ids[0],"revision":3,"entity_id":original.entity_id})).await["content"],old_body);
    call(
        &store,
        json!({"op":"forget","scope":"meenseek","id":ids[0],"revision":3}),
    )
    .await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM evidence_contents WHERE scope='meenseek'"
        )
        .fetch_one(store.pool())
        .await
        .unwrap(),
        1
    );
    call(
        &store,
        json!({"op":"forget","scope":"meenseek","id":ids[1],"revision":1}),
    )
    .await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM evidence_contents WHERE scope='meenseek'"
        )
        .fetch_one(store.pool())
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM evidence_contents WHERE scope='personal'"
        )
        .fetch_one(store.pool())
        .await
        .unwrap(),
        1
    );

    // Creating another reference concurrently with removal of the last owner cannot lose the body.
    let original = source(Scope::Meenseek);
    store
        .apply_import(std::slice::from_ref(&original))
        .await
        .unwrap();
    let e = &call(&store, json!({"op":"evidence","scope":"meenseek"})).await["items"][0];
    let memory = json!({"body":"Concurrent evidence","evidence":[{"entity_id":e["entity_id"],"source_revision":e["source_revision"],"content_digest":e["content_digest"],"generation":e["generation"]}]});
    let first = call(
        &store,
        json!({"op":"remember","scope":"meenseek","idempotency_key":"race-first","memory":memory}),
    )
    .await;
    let (forget, create) = tokio::join!(
        store.brain(cmd(json!({"op":"forget","scope":"meenseek","id":first["id"],"revision":1}))),
        store.brain(cmd(json!({"op":"remember","scope":"meenseek","idempotency_key":"race-second","memory":memory})))
    );
    forget.unwrap();
    let created = create.unwrap();
    assert_eq!(call(&store,json!({"op":"evidence-read","scope":"meenseek","id":created["id"],"revision":1,"entity_id":original.entity_id})).await["content"],old_body);
    let search = call(&store, json!({"op":"search","scope":"meenseek","query":""})).await;
    assert!(
        search["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["kind"] == "document")
    );
    assert!(
        search["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["kind"] == "memory")
    );
}

#[tokio::test]
async fn evidence_migration_preserves_existing_records() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    sqlx::query("TRUNCATE document_subjects,evidence_snapshots,evidence_contents,memory_history,memories,memory_creations,subjects,confirmation_history,related_materials,entity_areas,entity_topics,source_records,entities,sources,topics RESTART IDENTITY CASCADE").execute(store.pool()).await.unwrap();
    let hash = digest(b"migration-evidence");
    let mut source = ImportedRecord {
        source_id: format!("s_{hash}"),
        entity_id: format!("e_{hash}"),
        scope: Scope::Meenseek,
        repository: "/synthetic/migration".into(),
        path: "policy.md".into(),
        kind: SourceKind::Git,
        source_revision: "a".repeat(40),
        digest: Some(digest(b"Original policy")),
        content: Some("Original policy".into()),
    };
    store.apply_import(&[source.clone()]).await.unwrap();
    let e = &call(&store, json!({"op":"evidence","scope":"meenseek"})).await["items"][0];
    let evidence = json!({"entity_id":e["entity_id"],"source_revision":e["source_revision"],"content_digest":e["content_digest"],"generation":e["generation"]});
    let m = call(&store,json!({"op":"propose","scope":"meenseek","idempotency_key":"migration-memory","memory":{"body":"First decision","evidence":[evidence]}})).await;
    source.content = Some("Changed policy".into());
    source.digest = Some(digest(b"Changed policy"));
    source.source_revision = "b".repeat(40);
    store.apply_import(&[source.clone()]).await.unwrap();
    let e = &call(&store, json!({"op":"evidence","scope":"meenseek"})).await["items"][0];
    let evidence = json!({"entity_id":e["entity_id"],"source_revision":e["source_revision"],"content_digest":e["content_digest"],"generation":e["generation"]});
    call(&store,json!({"op":"correct","scope":"meenseek","id":m["id"],"revision":1,"memory":{"body":"Corrected decision","evidence":[evidence]}})).await;
    for i in 0..3 {
        call(&store,json!({"op":"remember","scope":"meenseek","idempotency_key":format!("migration-plain-{i}"),"memory":{"body":"No classification required"}})).await;
    }
    async fn existing(store: &Store) -> Value {
        sqlx::query_scalar("SELECT jsonb_build_object('memories',(SELECT jsonb_agg(to_jsonb(m) ORDER BY id) FROM memories m),'history',(SELECT jsonb_agg(to_jsonb(h)-'grouping_only' ORDER BY memory_id,revision) FROM memory_history h),'creations',(SELECT jsonb_agg(to_jsonb(c) ORDER BY memory_id) FROM memory_creations c),'subjects',(SELECT jsonb_agg(to_jsonb(s)-'revision'-'definition' ORDER BY id) FROM subjects s),'sources',(SELECT jsonb_agg(to_jsonb(s) ORDER BY id) FROM sources s),'records',(SELECT jsonb_agg(to_jsonb(r)-'reference_paths' ORDER BY entity_id) FROM source_records r),'entities',(SELECT jsonb_agg(to_jsonb(e) ORDER BY id) FROM entities e))").fetch_one(store.pool()).await.unwrap()
    }
    let before = existing(&store).await;
    // Recreate the actual pre-upgrade schema, retaining every existing row, timestamp and digest.
    sqlx::raw_sql("ALTER TABLE memory_history DROP COLUMN grouping_only; DELETE FROM ontology_migrations WHERE name='016-memory-grouping-history.sql'; DROP TABLE document_grouping; DELETE FROM ontology_migrations WHERE name='015-document-grouping.sql'; DROP TABLE document_subjects; DROP TABLE document_subject_history; DROP TABLE subject_history; ALTER TABLE subjects DROP COLUMN definition, DROP COLUMN revision; ALTER TABLE source_records DROP COLUMN reference_paths; DELETE FROM ontology_migrations WHERE name='013-knowledge-taxonomy.sql'; DROP FUNCTION context_projection_refresh_guard() CASCADE; DELETE FROM ontology_migrations WHERE name='012-context-projection-refresh.sql'; DELETE FROM ontology_migrations WHERE name='014-grouping-claim-ownership.sql'; DROP TABLE memory_grouping; DELETE FROM ontology_migrations WHERE name='011-personal-memory-grouping.sql'; DROP TRIGGER context_invalidate_consumers ON context_materials; DROP TABLE context_source_bindings; DROP FUNCTION context_invalidate_consumers(); DROP FUNCTION context_source_revision(uuid,uuid,bigint,boolean,text,text,text); ALTER TABLE sources DROP CONSTRAINT sources_kind_check; ALTER TABLE sources ADD CONSTRAINT sources_kind_check CHECK(kind IN ('git','vault')); ALTER TABLE sources DROP CONSTRAINT sources_failure_code_check; ALTER TABLE sources ADD CONSTRAINT sources_failure_code_check CHECK(failure_code IS NULL OR (kind='git' AND failure_code='git-read-failed') OR (kind='vault' AND failure_code='vault-read-failed')); ALTER TABLE sources ADD CONSTRAINT sources_vault_status_check CHECK(kind<>'vault' OR status<>'missing'); DELETE FROM ontology_migrations WHERE name IN ('008-context-consumers.sql','009-context-manual-edits.sql','010-profile-manual-edits.sql'); DROP TABLE context_projection_versions; DROP FUNCTION context_projection_validate(); DROP TABLE context_material_versions; ALTER TABLE context_materials DROP COLUMN last_manual_edit_id; DROP TABLE context_manual_edits; DROP FUNCTION context_manual_edit_complete(); DROP TABLE context_materials; DROP TABLE context_apply_batches; DROP FUNCTION context_core_contract_immutable(); DROP TABLE context_store; DROP FUNCTION context_record_version(); DROP FUNCTION context_material_revision(); DROP FUNCTION context_immutable_record(); DROP FUNCTION context_exclusive_gate(); DROP TABLE curation_reviews; DROP TABLE evidence_snapshots; DROP TABLE evidence_contents; DELETE FROM ontology_migrations WHERE name IN ('003-evidence-snapshots.sql','004-curation-reviews.sql','005-context-materials.sql','006-context-history.sql','007-context-native.sql');").execute(store.pool()).await.unwrap();
    sqlx::query("CREATE TABLE evidence_snapshots (failure_fixture boolean)")
        .execute(store.pool())
        .await
        .unwrap();
    assert_eq!(store.initialize().await, Err(Error::Baseline));
    assert_eq!(existing(&store).await, before);
    assert_eq!(
        sqlx::query_scalar::<_, Option<String>>("SELECT to_regclass('evidence_contents')::text")
            .fetch_one(store.pool())
            .await
            .unwrap(),
        None,
        "failed append rolls back preceding CREATE and migration marker"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM ontology_migrations")
            .fetch_one(store.pool())
            .await
            .unwrap(),
        2
    );
    sqlx::query("DROP TABLE evidence_snapshots")
        .execute(store.pool())
        .await
        .unwrap();
    store.initialize().await.unwrap();
    assert_eq!(existing(&store).await, before);
    assert_eq!(call(&store,json!({"op":"evidence-read","scope":"meenseek","id":m["id"],"revision":1,"entity_id":source.entity_id})).await["available"],false,"missing old bytes cannot be replaced by today's source");
    assert_eq!(call(&store,json!({"op":"evidence-read","scope":"meenseek","id":m["id"],"revision":2,"entity_id":source.entity_id})).await["content"],"Changed policy");
    store.initialize().await.unwrap();
    assert_eq!(existing(&store).await, before);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM evidence_snapshots")
            .fetch_one(store.pool())
            .await
            .unwrap(),
        1
    );
    call(
        &store,
        json!({"op":"accept","scope":"meenseek","id":m["id"],"revision":2}),
    )
    .await;
    assert_eq!(call(&store,json!({"op":"evidence-read","scope":"meenseek","id":m["id"],"revision":3,"entity_id":source.entity_id})).await["content"],"Changed policy");
}

#[tokio::test]
async fn subject_delete_keeps_records_and_bounds_database_calls() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    sqlx::query("TRUNCATE document_subjects,memory_grouping,evidence_snapshots,evidence_contents,memory_history,memories,memory_creations,subjects")
        .execute(store.pool()).await.expect("reset synthetic memories only");

    let empty = call(&store, json!({"op":"subject-create","scope":"personal","idempotency_key":"empty-delete","name":"빈 합성 묶음"})).await;
    let empty_id = empty["id"].as_str().unwrap();
    let before_empty = store.calls();
    let removed = call(
        &store,
        json!({"op":"subject-delete","scope":"personal","id":empty_id}),
    )
    .await;
    assert_eq!(removed["ungrouped"], 0);
    assert_eq!(
        store.calls() - before_empty,
        7,
        "empty deletion has a fixed call bound including document assignments and subject history"
    );

    let subject = call(&store, json!({"op":"subject-create","scope":"personal","idempotency_key":"linked-delete","name":"합성 묶음"})).await;
    let id = subject["id"].as_str().unwrap();
    assert_eq!(
        store
            .brain(cmd(
                json!({"op":"subject-delete","scope":"meenseek","id":id})
            ))
            .await,
        Err(Error::NotFound)
    );
    let source_hash = digest(b"subject-delete-evidence");
    let source = ImportedRecord {
        source_id: format!("s_{source_hash}"),
        entity_id: format!("e_{source_hash}"),
        scope: Scope::Personal,
        repository: "/synthetic/subject-delete".into(),
        path: "subject-delete-evidence.md".into(),
        kind: SourceKind::Git,
        source_revision: "a".repeat(40),
        digest: Some(digest(b"preserved evidence")),
        content: Some("preserved evidence".into()),
    };
    store
        .apply_import(std::slice::from_ref(&source))
        .await
        .expect("synthetic evidence source");
    let options = call(
        &store,
        json!({"op":"evidence","scope":"personal","query":"subject-delete-evidence.md"}),
    )
    .await;
    let candidate = &options["items"][0];
    let evidence = json!({"entity_id":candidate["entity_id"],"source_revision":candidate["source_revision"],"content_digest":candidate["content_digest"],"generation":candidate["generation"]});
    let suggestion = call(&store, json!({"op":"remember","scope":"personal","idempotency_key":"suggestion-memory","memory":{"title":"미분류 기록","body":"후보 묶음을 기다림"}})).await;
    sqlx::query("UPDATE memory_grouping SET state='suggested',suggestions=jsonb_build_object('candidate_ids',jsonb_build_array($2::text),'candidate_names',jsonb_build_object($2::text,'합성 묶음')) WHERE memory_id=$1")
        .bind(suggestion["id"].as_str().unwrap()).bind(id).execute(store.pool()).await.unwrap();
    let mut memories = Vec::new();
    for index in 0..20 {
        let linked = if index == 1 {
            vec![evidence.clone()]
        } else {
            vec![]
        };
        memories.push(call(&store, json!({"op":"remember","scope":"personal","idempotency_key":format!("keep-memory-{index}"),"memory":{"title":format!("보존할 기록 {index}"),"body":"묶음만 삭제한다","subject_id":id,"evidence":linked}})).await);
    }
    let withdrawn = call(
        &store,
        json!({"op":"withdraw","scope":"personal","id":memories[0]["id"],"revision":1}),
    )
    .await;
    assert_eq!(withdrawn["status"], "withdrawn");
    let before_linked = store.calls();
    let removed = call(
        &store,
        json!({"op":"subject-delete","scope":"personal","id":id}),
    )
    .await;
    assert_eq!(removed["ungrouped"], 20);
    assert_eq!(
        store.calls() - before_linked,
        10,
        "linked deletion batches history, evidence and grouping regardless of record count"
    );
    let before_missing = store.calls();
    assert_eq!(
        store
            .brain(cmd(
                json!({"op":"subject-delete","scope":"personal","id":id})
            ))
            .await,
        Err(Error::NotFound)
    );
    assert_eq!(
        store.calls() - before_missing,
        2,
        "missing subject stops after the locked lookup"
    );
    for (index, memory) in memories.iter().enumerate() {
        let kept = call(
            &store,
            json!({"op":"read","scope":"personal","id":memory["id"]}),
        )
        .await;
        assert_eq!(kept["body"], memory["body"]);
        assert!(kept["subject_id"].is_null());
        assert_eq!(kept["grouping"]["mode"], "off");
        assert_eq!(kept["revision"], if index == 0 { 3 } else { 2 });
        assert_eq!(
            kept["status"],
            if index == 0 { "withdrawn" } else { "accepted" }
        );
    }
    let history = call(
        &store,
        json!({"op":"history","scope":"personal","id":memories[0]["id"]}),
    )
    .await;
    assert_eq!(history["items"].as_array().unwrap().len(), 3);
    assert_eq!(history["items"][0]["subject_id"], Value::Null);
    assert_eq!(history["items"][1]["subject_id"], subject["id"]);
    let refreshed_suggestion = call(
        &store,
        json!({"op":"read","scope":"personal","id":suggestion["id"]}),
    )
    .await;
    assert_eq!(refreshed_suggestion["grouping"]["state"], "pending");
    assert_eq!(refreshed_suggestion["grouping"]["suggestions"], json!({}));
    let pinned = call(&store, json!({"op":"evidence-read","scope":"personal","id":memories[1]["id"],"revision":2,"entity_id":source.entity_id})).await;
    assert_eq!(pinned["content"], "preserved evidence");
    let graph = call(
        &store,
        json!({"op":"search","scope":"personal","query":"합성 묶음"}),
    )
    .await;
    assert!(
        !graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["id"] == id)
    );
    let company_subject = call(&store, json!({"op":"subject-create","scope":"meenseek","idempotency_key":"company-delete","name":"회사 합성 묶음"})).await;
    let company_memory = call(&store, json!({"op":"remember","scope":"meenseek","idempotency_key":"company-memory","memory":{"title":"회사 기록","body":"그룹만 삭제","subject_id":company_subject["id"]}})).await;
    let before_company = store.calls();
    assert_eq!(
        call(
            &store,
            json!({"op":"subject-delete","scope":"meenseek","id":company_subject["id"]})
        )
        .await["ungrouped"],
        1
    );
    assert_eq!(
        store.calls() - before_company,
        8,
        "company deletion includes document assignments and subject history, but skips personal grouping state"
    );
    let company_kept = call(
        &store,
        json!({"op":"read","scope":"meenseek","id":company_memory["id"]}),
    )
    .await;
    assert!(company_kept["subject_id"].is_null());
    assert_eq!(company_kept["body"], company_memory["body"]);
}
