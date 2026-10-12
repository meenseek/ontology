#[allow(
    dead_code,
    reason = "shared Core fixture helpers cover several integration test modules"
)]
#[path = "native_harness.rs"]
mod harness_fixture;
use context_core::harness::{DraftTaskRequest, SourceVersion, StoredSourceState};
use ontology::{career_workflow::CareerCommand, domain::Error, store::Store};
use serde_json::{Value, json};
use std::process::Stdio;
static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
async fn store() -> (Store, String, String) {
    let url = std::env::var("TEST_DATABASE_URL").expect("isolated DB");
    assert!(
        ontology::config::database_options(&url)
            .unwrap()
            .get_database()
            .unwrap()
            .starts_with("ontology_test_")
    );
    let store = Store::connect(&url).await.unwrap();
    store.initialize().await.unwrap();
    sqlx::query("TRUNCATE career_workflows,document_grouping,document_subjects,memory_grouping,context_source_bindings,context_projection_versions,context_material_versions,context_manual_edits,context_materials,context_apply_batches,evidence_snapshots,evidence_contents,memory_history,memories,memory_creations,subjects,curation_reviews,confirmation_history,related_materials,entity_areas,entity_topics,source_records,entities,sources,topics").execute(store.pool()).await.unwrap();
    let sha = harness_fixture::seed(&store).await;
    let id = store.context_identity().await.unwrap()["store_id"]
        .as_str()
        .unwrap()
        .to_owned();
    (store, id, sha)
}
async fn reference(store: &Store, path: &str) -> SourceVersion {
    let relative = path.strip_prefix("vault/personal/").unwrap();
    let m = store
        .read_context_material(&"personal".parse().unwrap(), relative)
        .await
        .unwrap()
        .metadata;
    SourceVersion {
        logical_path: path.into(),
        state: StoredSourceState::Live {
            material_id: m.material_id,
            revision: m.revision as u64,
            content_digest: m.content_digest,
        },
    }
}
fn request(
    sha: &str,
    target: &str,
    evidence: Vec<String>,
) -> context_core::harness::RequestEnvelope {
    let mut req = harness_fixture::envelope(sha, vec![target.into()], vec![], false);
    if let DraftTaskRequest::Write(w) = &mut req.draft {
        let statement = format!(
            "Write the exact synthetic candidate and independently review it. Caller-bound native inputs: {}",
            evidence.join(", ")
        );
        req.source = context_core::harness::RequestSource::UserLanguage {
            statements: vec![context_core::harness::UserStatement {
                identifier: "statement-0000000000000001".into(),
                text: statement.clone(),
            }],
        };
        req.decision_trace.records.retain(|r| {
            serde_json::to_value(r).unwrap()["identifier"] != "decision-0000000000000003"
        });
        w.common.task_statement =
            harness_fixture::value(statement, 3, &mut req.decision_trace.records);
    }
    req
}
fn configuration() -> context_core::harness::PolicyConfiguration {
    let mut value: Value = serde_json::from_slice(
        &std::fs::read(harness_fixture::context_fixture::configuration_path()).unwrap(),
    )
    .unwrap();
    value["documents"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|d| d["key"] == "authoring")
        .unwrap()["path"] = json!("vault/personal/writing/overview.md");
    value["company"]["shared_documents"] = json!(["foundation"]);
    for rule in value["rules"].as_array_mut().unwrap() {
        if rule["documents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d == "authoring")
        {
            rule["selector"]["owners"] = json!(["personal"]);
        }
    }
    context_core::harness::PolicyConfiguration::from_bytes(&serde_json::to_vec(&value).unwrap())
        .unwrap()
}
async fn prepare(
    store: &Store,
    view: std::path::PathBuf,
    workspace: std::path::PathBuf,
    envelope: context_core::harness::RequestEnvelope,
    run: String,
    content: String,
) -> context_core::harness::PreparedHarnessRun {
    use context_core::harness::*;
    store
        .with_native_context(view, move |source| {
            let engine = HarnessEngine::with_source(
                source.view_root(),
                &workspace,
                source.clone(),
                configuration(),
            )
            .unwrap();
            let resolved = engine.resolve_envelope(&envelope, None).unwrap();
            let caps = harness_fixture::capabilities();
            let plan =
                HarnessPlan::from_resolved(resolved, &workspace, caps.lifecycle.clone()).unwrap();
            let prepared = PreparedHarnessRun::prepare(
                &engine,
                &serde_json::to_vec(&plan).unwrap(),
                plan,
                caps,
                None,
                None,
            )
            .unwrap();
            let mut head = HarnessExecutionRecord::begin_durable(
                &engine,
                &serde_json::to_vec(&prepared).unwrap(),
                &prepared,
                &run,
            )
            .unwrap();
            while let Some(invocation) = head.ready_role_invocations.first() {
                head = HarnessExecutionRecord::advance_durable(
                    &engine,
                    &run,
                    harness_fixture::role_event(&prepared, invocation, &content),
                )
                .unwrap();
            }
            HarnessExecutionRecord::evaluate_durable(&engine, &run, None).unwrap();
            HarnessExecutionRecord::validate_durable(&engine, &run).unwrap();
            Ok(prepared)
        })
        .await
        .unwrap()
}
async fn apply(
    store: &Store,
    view: std::path::PathBuf,
    workspace: std::path::PathBuf,
    run: String,
    prepared: context_core::harness::PreparedHarnessRun,
) {
    use context_core::harness::*;
    let id = store.context_identity().await.unwrap()["store_id"]
        .as_str()
        .unwrap()
        .to_owned();
    store
        .with_native_commit(view, id, move |session| {
            let source = session.fresh_source().unwrap();
            let engine = HarnessEngine::with_source(
                source.view_root(),
                &workspace,
                source.clone(),
                configuration(),
            )
            .unwrap();
            let (_, receipt) =
                HarnessExecutionRecord::apply_durable(&engine, &run, Some(session)).unwrap();
            receipt.validate(&prepared).unwrap();
            Ok(())
        })
        .await
        .unwrap();
}
async fn write(
    store: &Store,
    sha: &str,
    target: &str,
    body: &str,
    evidence: Vec<String>,
) -> SourceVersion {
    let view = harness_fixture::view();
    let root = view.path().canonicalize().unwrap();
    let run = format!("career-test-{}", uuid::Uuid::new_v4());
    let prepared = prepare(
        store,
        root.clone(),
        root.clone(),
        request(sha, target, evidence),
        run.clone(),
        body.into(),
    )
    .await;
    apply(store, root.clone(), root, run, prepared).await;
    reference(store, target).await
}
fn fenced(record: Value) -> String {
    format!(
        "---\nexport: false\n---\n# Synthetic review\n\n```career-workflow\n{}\n```\n",
        record
    )
}
async fn call(store: &Store, value: Value) -> Result<Value, Error> {
    store
        .career_workflow(serde_json::from_value::<CareerCommand>(value).unwrap())
        .await
}
async fn cli(value: Value) -> std::process::Output {
    use tokio::io::AsyncWriteExt;
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_ontology"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("DATABASE_URL", std::env::var("TEST_DATABASE_URL").unwrap())
        .arg("career")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(value.to_string().as_bytes())
        .await
        .unwrap();
    child.wait_with_output().await.unwrap()
}
#[tokio::test]
async fn career_real_core_apply_and_fresh_process_output_verification() {
    let _guard = LOCK.lock().await;
    let (store, id, sha) = store().await;
    let outcome = reference(&store, "vault/personal/index.md").await;
    let criterion = reference(&store, "vault/personal/writing/overview.md").await;
    let initial = store
        .read_context(&"personal".parse().unwrap(), "index.md", false)
        .await
        .unwrap();
    let start = json!({"op":"start","store_id":id,"application_key":"fictional-ai-ops","outcome_key":"screening-rejected-1","outcome":outcome,"criteria":[criterion],"surfaces":["application-essay","career-description","portfolio"]});
    let (a, b) = tokio::join!(call(&store, start.clone()), call(&store, start.clone()));
    let mut p = a.unwrap();
    assert_eq!(p["id"], b.unwrap()["id"]);
    let flow = p["id"].clone();
    let mutate = |op: &str, revision: i64| json!({"op":op,"store_id":id,"id":flow,"expected_revision":revision});
    assert_eq!(
        call(&store, mutate("pause", 99)).await,
        Err(Error::Conflict)
    );
    p = call(&store, mutate("pause", 1)).await.unwrap();
    assert_eq!(p["progress"]["paused"], true);
    let resumed = cli(mutate("resume", 2)).await;
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&resumed.stdout).unwrap()["revision"],
        3
    );
    let reflection=write(&store,&sha,"vault/personal/knowledge/reflection.md",&fenced(json!({"kind":"reflection","application_key":"fictional-ai-ops","outcome_key":"screening-rejected-1","findings":[{"id":"placement","kind":"artifact-observation","statement":"The submitted sample puts the evidence after generic prose.","evidence":[outcome]},{"id":"cause","kind":"unknown","statement":"The employer did not provide the cause.","evidence":[]}]})),vec![outcome.logical_path.clone()]).await;
    let mut record = mutate("record", 3);
    record["record"] = json!(reflection);
    p = call(&store, record).await.unwrap();
    assert_eq!(p["progress"]["stage"], "review");
    let bad_review=write(&store,&sha,"vault/personal/knowledge/bad-review.md",&fenced(json!({"kind":"review","reflection":reflection,"decisions":[{"finding_id":"placement","disposition":"reject","reason":"Synthetic rejection","targets":[]},{"finding_id":"cause","disposition":"apply","reason":"guess cause","targets":[criterion.logical_path]}]})),vec![reflection.logical_path.clone(),criterion.logical_path.clone()]).await;
    let mut record = mutate("record", 4);
    record["record"] = json!(bad_review);
    assert_eq!(call(&store, record).await, Err(Error::Invalid));
    let review=write(&store,&sha,"vault/personal/knowledge/review.md",&fenced(json!({"kind":"review","reflection":reflection,"decisions":[{"finding_id":"placement","disposition":"apply","reason":"Make supported evidence visible early.","targets":[criterion.logical_path]},{"finding_id":"cause","disposition":"hold","reason":"No hiring cause was given.","targets":[],"revisit_when":"Employer feedback arrives"}]})),vec![reflection.logical_path.clone(),criterion.logical_path.clone()]).await;
    let mut record = mutate("record", 4);
    record["record"] = json!(review);
    p = call(&store, record).await.unwrap();
    assert_eq!(p["progress"]["stage"], "criteria-apply");
    assert_eq!(
        call(&store, mutate("confirm-apply", 5)).await,
        Err(Error::Conflict)
    );
    let view = harness_fixture::view();
    let root = view.path().canonicalize().unwrap();
    let run = "career-criteria-apply";
    let evidence = vec![
        outcome.logical_path.clone(),
        reflection.logical_path.clone(),
        review.logical_path.clone(),
    ];
    let prepared=prepare(&store,root.clone(),root.clone(),request(&sha,&criterion.logical_path,evidence),run.into(),"---\nexport: false\n---\n# Writing criteria\nPlace the supported problem, personal action and operational boundary before generic prose.\nDo not infer rejection causes.\n".into()).await;
    let mut bind = mutate("bind-apply", 5);
    bind["run_id"] = json!(run);
    bind["prepared_run_digest"] = json!(prepared.prepared_run_digest);
    p = call(&store, bind).await.unwrap();
    assert_eq!(p["revision"], 6);
    assert_eq!(
        call(&store, mutate("confirm-apply", 6)).await,
        Err(Error::Conflict)
    );
    let fake:String=sqlx::query_scalar("INSERT INTO context_apply_batches(apply_id,store_id,core_run_id,prepared_run_digest,candidate_digest,expected_source_versions,context_targets,core_apply_attempt_id,expected_batch_id,expected_journal_locator,state,actual_batch_id,actual_journal_locator,commit_receipt,final_core_receipt_digest) VALUES(gen_random_uuid(),$1::uuid,$2,$3,$4,'{}','[{}]','career-fake-attempt','career-fake-batch','career-fake-journal','finalized','career-fake-batch','career-fake-journal','{}',$4) RETURNING apply_id::text").bind(&id).bind(run).bind(&prepared.prepared_run_digest).bind("a".repeat(64)).fetch_one(store.pool()).await.unwrap();
    assert_eq!(
        call(&store, mutate("confirm-apply", 6)).await,
        Err(Error::Conflict),
        "finalized flag without Core bytes is not proof"
    );
    sqlx::query("UPDATE context_apply_batches SET state='pending',actual_batch_id=NULL,actual_journal_locator=NULL,commit_receipt=NULL,final_core_receipt_digest=NULL WHERE apply_id=$1::uuid").bind(&fake).execute(store.pool()).await.unwrap();
    assert_eq!(
        call(&store, mutate("confirm-apply", 6)).await,
        Err(Error::ContextPending)
    );
    assert_eq!(
        call(&store, json!({"op":"get","store_id":id,"id":flow}))
            .await
            .unwrap()["health"],
        "native-apply-pending"
    );
    assert_eq!(
        store.list_career_workflows().await.unwrap()["workflows"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    sqlx::query("DELETE FROM context_apply_batches WHERE apply_id=$1::uuid")
        .bind(&fake)
        .execute(store.pool())
        .await
        .unwrap();
    apply(&store, root.clone(), root, run.into(), prepared).await;
    let probe = cli(json!({"op":"get","store_id":id,"id":flow})).await;
    assert!(probe.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&probe.stdout).unwrap()["health"],
        "apply-confirmation-needed"
    );
    p = call(&store, mutate("confirm-apply", 6)).await.unwrap();
    assert_eq!(p["progress"]["stage"], "output-verification");
    let criteria = p["progress"]["criteria"].clone();
    let new_criterion = reference(&store, &criterion.logical_path).await;
    assert_eq!(
        new_criterion.state,
        serde_json::from_value::<SourceVersion>(criteria[0].clone())
            .unwrap()
            .state
    );
    let paths = [
        "vault/personal/knowledge/new-essay.md",
        "vault/personal/knowledge/new-description.md",
        "vault/personal/knowledge/new-portfolio.md",
    ];
    let view = harness_fixture::view();
    let root = view.path().canonicalize().unwrap();
    let mut multi = request(&sha, paths[0], vec![new_criterion.logical_path.clone()]);
    multi
        .decision_trace
        .records
        .retain(|r| serde_json::to_value(r).unwrap()["identifier"] != "decision-0000000000000006");
    if let DraftTaskRequest::Write(w) = &mut multi.draft {
        w.targets = harness_fixture::value(
            paths.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
            6,
            &mut multi.decision_trace.records,
        );
    }
    let prepared = prepare(
        &store,
        root.clone(),
        root.clone(),
        multi,
        "career-output-batch".into(),
        "# Fictional project\nProblem → personal action → verified boundary.\n".into(),
    )
    .await;
    apply(
        &store,
        root.clone(),
        root,
        "career-output-batch".into(),
        prepared,
    )
    .await;
    write(
        &store,
        &sha,
        paths[1],
        "# Corrected fictional career description\nProblem → personal action → current boundary.\n",
        vec![new_criterion.logical_path.clone()],
    )
    .await;
    let surfaces = ["application-essay", "career-description", "portfolio"];
    let mut checks = vec![];
    for (surface, path) in surfaces.into_iter().zip(paths) {
        let artifact = reference(&store, path).await;
        let review_path = format!("vault/personal/knowledge/{surface}-output-review.md");
        let independent=write(&store,&sha,&review_path,&fenced(json!({"kind":"output-review","review":review,"surface":surface,"criteria":criteria,"artifact":artifact,"result":"passed","observations":"Supported boundary is visible; no guessed cause is claimed."})),vec![artifact.logical_path.clone(),new_criterion.logical_path.clone()]).await;
        checks.push(json!({"surface":surface,"artifact":artifact,"independent_review":independent,"result":"passed"}));
    }
    let verification = write(
        &store,
        &sha,
        "vault/personal/knowledge/verification.md",
        &fenced(json!({"kind":"verification","review":review,"criteria":criteria,"checks":checks})),
        vec![],
    )
    .await;
    let missing = write(
        &store,
        &sha,
        "vault/personal/knowledge/missing-output.md",
        &fenced(
            json!({"kind":"verification","review":review,"criteria":criteria,"checks":checks[..2]}),
        ),
        vec![],
    )
    .await;
    let mut bad = mutate("record", 7);
    bad["record"] = json!(missing);
    assert_eq!(
        call(&store, bad).await,
        Err(Error::Conflict),
        "all three requested surfaces need actual review"
    );
    let mut record = mutate("record", 7);
    record["record"] = json!(verification);
    p = call(&store, record).await.unwrap();
    assert_eq!(p["progress"]["stage"], "complete");
    assert_eq!(
        store
            .read_context(&"personal".parse().unwrap(), "index.md", false)
            .await
            .unwrap(),
        initial,
        "historical input remains unchanged"
    );
    let before:Value=sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(v) ORDER BY material_id,revision) FROM context_material_versions v").fetch_one(store.pool()).await.unwrap();
    sqlx::raw_sql("DROP TABLE career_workflows; DELETE FROM ontology_migrations WHERE name='017-career-workflows.sql'").execute(store.pool()).await.unwrap();
    store.initialize().await.unwrap();
    let after:Value=sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(v) ORDER BY material_id,revision) FROM context_material_versions v").fetch_one(store.pool()).await.unwrap();
    assert_eq!(
        before, after,
        "append-only upgrade preserves originals and history"
    );
}
#[tokio::test]
async fn career_drift_rebase_strict_inputs_and_missing_migration() {
    let _guard = LOCK.lock().await;
    let (store, id, sha) = store().await;
    let outcome = write(
        &store,
        &sha,
        "vault/personal/index.md",
        &format!("# Synthetic outcome owner\n{}", "x".repeat(75000)),
        vec![],
    )
    .await;
    let criterion = reference(&store, "vault/personal/writing/overview.md").await;
    let start = json!({"op":"start","store_id":id,"application_key":"sample","outcome_key":"rejected-1","outcome":outcome,"criteria":[criterion],"surfaces":["portfolio"]});
    let p = call(&store, start.clone()).await.unwrap();
    let flow = p["id"].clone();
    let wrong =
        cli(json!({"op":"get","store_id":uuid::Uuid::new_v4().to_string(),"id":flow})).await;
    assert!(!wrong.status.success());
    assert!(wrong.stdout.is_empty());
    let extra = cli(json!({"op":"get","store_id":id,"id":flow,"applied":true})).await;
    assert!(!extra.status.success());
    let changed = write(
        &store,
        &sha,
        &criterion.logical_path,
        "# New synthetic criteria\nUpdated source.\n",
        vec![],
    )
    .await;
    let p = call(&store, json!({"op":"get","store_id":id,"id":flow}))
        .await
        .unwrap();
    assert_eq!(p["health"], "source-drift");
    assert_eq!(
        call(
            &store,
            json!({"op":"resume","store_id":id,"id":flow,"expected_revision":1})
        )
        .await,
        Err(Error::Conflict)
    );
    let p=call(&store,json!({"op":"rebase","store_id":id,"id":flow,"expected_revision":1,"outcome":outcome,"criteria":[changed]})).await.unwrap();
    assert_eq!(p["progress"]["stage"], "reflection");
    sqlx::raw_sql("DROP TABLE career_workflows; DELETE FROM ontology_migrations WHERE name='017-career-workflows.sql'").execute(store.pool()).await.unwrap();
    let out = cli(json!({"op":"list","store_id":id})).await;
    assert!(!out.status.success());
    let absent: Option<String> = sqlx::query_scalar("SELECT to_regclass('career_workflows')::text")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(absent, None, "read never migrates");
    store.initialize().await.unwrap();
}
#[tokio::test]
async fn career_api_reuses_session_csrf_and_revision_guards() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let _guard = LOCK.lock().await;
    let (store, id, _) = store().await;
    let web = tempfile::tempdir().unwrap();
    std::fs::write(
        web.path().join("index.html"),
        "<main>career workspace</main>",
    )
    .unwrap();
    let app = ontology::api::router(ontology::api::AppState::new(
        store.clone(),
        ontology::config::Config {
            address: "127.0.0.1:47831".parse().unwrap(),
            web_dist: web.path().into(),
        },
    ));
    let page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/career")
                .header("host", "127.0.0.1:47831")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page.status(), 200);
    assert_eq!(
        page.into_body().collect().await.unwrap().to_bytes(),
        "<main>career workspace</main>"
    );
    let session = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header("host", "127.0.0.1:47831")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let cookie = session.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let data: Value =
        serde_json::from_slice(&session.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let request = |method: &str, path: &str, body: String, csrf: bool| {
        let mut r = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1:47831")
            .header("origin", "http://127.0.0.1:47831")
            .header("cookie", &cookie)
            .header("content-type", "application/json");
        if csrf {
            r = r.header("x-csrf-token", data["csrf"].as_str().unwrap());
        }
        r.body(Body::from(body)).unwrap()
    };
    let outcome = reference(&store, "vault/personal/index.md").await;
    let criteria = reference(&store, "vault/personal/writing/overview.md").await;
    let start=json!({"op":"start","store_id":id,"application_key":"api-sample","outcome_key":"rejected","outcome":outcome,"criteria":[criteria],"surfaces":["portfolio"]}).to_string();
    assert_eq!(
        app.clone()
            .oneshot(request("POST", "/api/career", start.clone(), false))
            .await
            .unwrap()
            .status(),
        403
    );
    let created = app
        .clone()
        .oneshot(request("POST", "/api/career", start, true))
        .await
        .unwrap();
    assert_eq!(created.status(), 200);
    let created: Value =
        serde_json::from_slice(&created.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let list = app
        .clone()
        .oneshot(request("GET", "/api/career", String::new(), false))
        .await
        .unwrap();
    assert_eq!(list.status(), 200);
    let list: Value =
        serde_json::from_slice(&list.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(list["store_id"], id);
    assert_eq!(list["workflows"].as_array().unwrap().len(), 1);
    let stale =
        json!({"op":"pause","store_id":id,"id":created["id"],"expected_revision":9}).to_string();
    assert_eq!(
        app.clone()
            .oneshot(request("POST", "/api/career", stale, true))
            .await
            .unwrap()
            .status(),
        409
    );
    assert_eq!(
        app.clone()
            .oneshot(request("POST", "/api/career", "x".repeat(16385), true))
            .await
            .unwrap()
            .status(),
        413
    );
    let hostile = Request::builder()
        .uri("/api/career")
        .header("host", "evil.invalid")
        .header("cookie", &cookie)
        .body(Body::empty())
        .unwrap();
    assert_eq!(app.oneshot(hostile).await.unwrap().status(), 403);
}
