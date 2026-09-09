static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
use meenseek_ontology::{
    domain::{Error, ImportedRecord, Scope, SourceKind},
    memory::BrainCommand,
    store::{Store, digest},
};
use serde_json::{Value, json};
async fn store() -> Store {
    let url = std::env::var("TEST_DATABASE_URL").expect("explicit isolated DB");
    assert!(
        meenseek_ontology::config::database_options(&url)
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
    sqlx::query("TRUNCATE memory_history,memories,memory_creations,subjects")
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
    let recall = call(
        &store,
        json!({"op":"recall","scope":"personal","query":"새 결정"}),
    )
    .await;
    assert_eq!(recall["items"][0]["id"], id);
    assert_eq!(
        call(
            &store,
            json!({"op":"recall","scope":"meenseek","query":"새 결정"})
        )
        .await["status"],
        "insufficient-evidence"
    );
    call(
        &store,
        json!({"op":"withdraw","scope":"personal","id":id,"revision":2}),
    )
    .await;
    assert_eq!(
        call(
            &store,
            json!({"op":"recall","scope":"personal","query":"새 결정"})
        )
        .await["status"],
        "insufficient-evidence"
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
            json!({"op":"recall","scope":"personal","query":"experiment"})
        )
        .await["status"],
        "insufficient-evidence"
    );
    call(
        &store,
        json!({"op":"accept","scope":"personal","id":proposal["id"],"revision":1}),
    )
    .await;
    assert_eq!(
        call(
            &store,
            json!({"op":"recall","scope":"personal","query":"EXPERIMENT"})
        )
        .await["status"],
        "ok"
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
            json!({"op":"recall","scope":"personal","query":"windowonly"})
        )
        .await["status"],
        "insufficient-evidence"
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
            json!({"op":"recall","scope":"personal","query":"supportedonly"})
        )
        .await["status"],
        "insufficient-evidence",
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
            json!({"op":"recall","scope":"personal","query":"supportedonly"})
        )
        .await["status"],
        "ok"
    );
    record.source_revision = "b".repeat(40);
    store
        .apply_import(&[record.clone()])
        .await
        .expect("changed revision");
    assert_eq!(
        call(
            &store,
            json!({"op":"recall","scope":"personal","query":"supportedonly"})
        )
        .await["status"],
        "insufficient-evidence"
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
            json!({"op":"recall","scope":"meenseek","query":format!("batch{size}")}),
        )
        .await;
        assert_eq!(store.calls() - before, 1);
        assert!(serde_json::to_vec(&result).expect("JSON").len() <= 65536);
    }
    let list = call(&store, json!({"op":"list","scope":"meenseek","limit":1})).await;
    assert!(list["next_after"].is_string());
    for bad in [
        json!({"op":"recall","scope":"personal","query":"x","extra":true}),
        json!({"op":"remember","scope":"personal","idempotency_key":"long-key","memory":{"kind":"arbitrary","title":"x","body":"y"}}),
    ] {
        assert!(serde_json::from_value::<BrainCommand>(bad).is_err());
    }
    assert_eq!(
        store
            .brain(cmd(
                json!({"op":"recall","scope":"personal","query":"x".repeat(121)})
            ))
            .await,
        Err(Error::Invalid)
    );
}

#[tokio::test]
async fn brain_transport_contract() {
    let _guard = TEST_LOCK.lock().await;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use meenseek_ontology::{
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
            json!({"op":"recall","scope":"personal","query":"x","body":"sensitive-marker"})
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
    let creation =
        json!({"op":"remember","scope":"personal","idempotency_key":key,"memory":input()});
    let (ok, first) = cli(&creation.to_string()).await;
    assert!(ok);
    let (ok, repeat) = cli(&creation.to_string()).await;
    assert!(ok);
    assert_eq!(first, repeat);
    let id = &first["id"];
    let (ok, recalled) =
        cli(&json!({"op":"recall","scope":"personal","query":"experiment"}).to_string()).await;
    assert!(ok);
    assert!(
        recalled["items"]
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
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_meenseek-ontology"))
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
    use meenseek_ontology::{
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
        meenseek_ontology::importer::identity(&repo, "outside.md", Scope::Personal).expect("id");
    let outside_before = store
        .detail(Scope::Personal, &outside)
        .await
        .expect("outside");
    let (_, id) =
        meenseek_ontology::importer::identity(&repo, "source-0.md", Scope::Personal).expect("id");
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
    let vault = root.join("vault");
    std::fs::create_dir_all(vault.join("personal/projects")).expect("synthetic vault");
    std::fs::write(
        vault.join("personal/projects/note.md"),
        "---\ntitle: Synthetic\nscope: personal\nexport: false\n---\n\nInitial note\n",
    )
    .expect("note");
    let mut both = config.clone();
    both["sources"].as_array_mut().expect("sources").push(json!({"kind":"vault","root":vault,"scope":"personal","vault_scope":"personal","binary":std::env::var("TEST_VAULT_BINARY").expect("actual Vault"),"paths":["projects/note.md"]}));
    write_config(&both);
    let first = refresh(&store, &config_path).await.expect("both sources");
    assert!(first.ok);
    assert_eq!(first.sources[1].provider_calls, 1);
    assert!(first.sources[1].response_bytes < 2048);
    let (_, vault_id) = meenseek_ontology::vault_importer::identity(
        &vault,
        "personal/projects/note.md",
        Scope::Personal,
    )
    .expect("vault id");
    let vault_before = store
        .detail(Scope::Personal, &vault_id)
        .await
        .expect("vault snapshot");
    std::fs::remove_file(vault.join("personal/projects/note.md")).expect("missing vault source");
    let before = store.calls();
    let failed = refresh(&store, &config_path)
        .await
        .expect("isolated partial failure");
    assert!(!failed.ok);
    assert!(failed.sources[0].ok);
    assert!(!failed.sources[1].ok);
    assert_eq!(failed.sources[1].provider_calls, 1);
    assert_eq!(store.calls() - before, 7);
    let after = store
        .detail(Scope::Personal, &vault_id)
        .await
        .expect("retained snapshot");
    assert_eq!(after["projection"], vault_before["projection"]);
    assert_eq!(after["source"]["status"], "failed");
    git(&repo, &["rm", "-q", "source-0.md"]);
    git(&repo, &["commit", "-qm", "missing"]);
    refresh(&store, &config_path)
        .await
        .expect("absence plus partial failure");
    assert_eq!(
        store.detail(Scope::Personal, &id).await.expect("missing")["source"]["status"],
        "missing"
    );
    assert_eq!(
        store
            .detail(Scope::Personal, &outside)
            .await
            .expect("outside untouched"),
        outside_before
    );
    let mut restored =
        "---\ntitle: Synthetic\nscope: personal\nexport: false\n---\n\nRestored note\n".to_owned();
    std::fs::write(vault.join("personal/projects/note.md"), &restored).expect("restored");
    refresh(&store, &config_path)
        .await
        .expect("restart-style current read");
    assert_eq!(
        store
            .detail(Scope::Personal, &vault_id)
            .await
            .expect("restored")["source"]["status"],
        "ok"
    );
    restored.push_str("latest\n");
    std::fs::write(vault.join("personal/projects/note.md"), restored).expect("latest");
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_meenseek-ontology"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env(
            "DATABASE_URL",
            std::env::var("TEST_DATABASE_URL").expect("synthetic"),
        )
        .env("ONTOLOGY_SYNC_CONFIG", &config_path)
        .arg("sync-once")
        .output()
        .await
        .expect("sync CLI");
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).expect("report");
    assert_eq!(report["ok"], true);
}
