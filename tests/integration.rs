static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
use meenseek_ontology::store::digest;
use meenseek_ontology::{
    domain::{
        Classification, Error, ImportedRecord, LinkChange, MAX_RESPONSE_BYTES, Scope, SourceKind,
    },
    store::Store,
};
use serde_json::Value;

fn record(index: usize, scope: Scope) -> ImportedRecord {
    let hash = digest(format!("{}:{index}", scope.as_str()).as_bytes());
    ImportedRecord {
        source_id: format!("s_{hash}"),
        entity_id: format!("e_{hash}"),
        scope,
        repository: "/synthetic/repository".into(),
        path: format!("document-{index:03}.md"),
        kind: SourceKind::Git,
        source_revision: "a".repeat(40),
        digest: Some(digest("한국어 자료 <script>alert(1)</script>".as_bytes())),
        content: Some("한국어 자료 <script>alert(1)</script>".into()),
    }
}
async fn store() -> Store {
    let url = std::env::var("TEST_DATABASE_URL").expect(
        "TEST_DATABASE_URL must point to an explicitly created temporary PostgreSQL database",
    );
    let options =
        meenseek_ontology::config::database_options(&url).expect("test URL must be loopback");
    assert!(
        options
            .get_database()
            .is_some_and(|name| name.starts_with("ontology_test_")),
        "tests require ontology_test_ database prefix"
    );
    let store = Store::connect(&url).await.expect("connect test database");
    store
        .initialize()
        .await
        .expect("initialize empty test database");
    store
}

#[tokio::test]
async fn database_contract() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    store
        .initialize()
        .await
        .expect("same baseline is idempotent");
    assert_eq!(
        store.initialize_baseline("-- changed baseline").await,
        Err(Error::Baseline)
    );
    // The test database is created by the verification runner; only its own rows are reset.
    sqlx::query("TRUNCATE confirmation_history,related_materials,entity_areas,entity_topics,source_records,entities,sources,topics RESTART IDENTITY CASCADE").execute(store.pool()).await.expect("reset test fixture");
    for size in [0, 1, 100] {
        let records = (0..size)
            .map(|i| record(i, Scope::Meenseek))
            .collect::<Vec<_>>();
        let before = store.calls();
        store
            .apply_import(&records)
            .await
            .expect("import fixture batch");
        assert_eq!(
            store.calls() - before,
            if size == 0 { 0 } else { 3 },
            "batch import includes begin, one statement, commit"
        );
        let before = store.calls();
        let result = store
            .list(Scope::Meenseek, "한국어", false, None)
            .await
            .expect("search bounded result");
        assert_eq!(
            store.calls() - before,
            1,
            "list must issue one set query regardless of size"
        );
        assert_eq!(result["items"].as_array().expect("items").len(), size);
        assert!(serde_json::to_vec(&result).expect("serialize").len() < MAX_RESPONSE_BYTES);
    }
    let personal = record(0, Scope::Personal);
    store
        .apply_import(std::slice::from_ref(&personal))
        .await
        .expect("synthetic personal fixture");
    let first = record(0, Scope::Meenseek);
    let second = record(1, Scope::Meenseek);
    assert_eq!(
        store.detail(Scope::Meenseek, &personal.entity_id).await,
        Err(Error::NotFound)
    );
    let list = store
        .list(Scope::Personal, "", false, None)
        .await
        .expect("personal list");
    assert_eq!(list["total"], 1);
    assert_eq!(list["items"][0]["id"], personal.entity_id);
    let invalid = sqlx::query("UPDATE entities SET source_id=$1 WHERE id=$2")
        .bind(&personal.source_id)
        .bind(&first.entity_id)
        .execute(store.pool())
        .await;
    assert!(
        invalid.is_err(),
        "composite scope foreign key must reject cross-scope source"
    );
    assert!(
        sqlx::query(
            "INSERT INTO entity_areas(scope,entity_id,area) VALUES('meenseek',$1,'made-up')"
        )
        .bind(&first.entity_id)
        .execute(store.pool())
        .await
        .is_err()
    );
    let before = store.calls();
    store
        .classify(
            Scope::Meenseek,
            &first.entity_id,
            Classification {
                revision: 0,
                areas: vec!["strategy-portfolio".into(), "market-customer".into()],
                topics: vec!["구조".into(), "시장".into()],
            },
        )
        .await
        .expect("confirm classification");
    assert_eq!(store.calls() - before, 8);
    assert_eq!(
        store
            .classify(
                Scope::Meenseek,
                &first.entity_id,
                Classification {
                    revision: 0,
                    areas: vec![],
                    topics: vec![]
                }
            )
            .await,
        Err(Error::Conflict)
    );
    store
        .classify(
            Scope::Meenseek,
            &first.entity_id,
            Classification {
                revision: 1,
                areas: vec!["business-operations".into()],
                topics: vec!["정정한 주제".into()],
            },
        )
        .await
        .expect("correct classification");
    assert_eq!(
        store
            .classify(
                Scope::Personal,
                &personal.entity_id,
                Classification {
                    revision: 0,
                    areas: vec!["strategy-portfolio".into()],
                    topics: vec![]
                }
            )
            .await,
        Err(Error::Invalid)
    );
    assert_eq!(
        store
            .link(
                Scope::Meenseek,
                &first.entity_id,
                LinkChange {
                    revision: 2,
                    target_id: personal.entity_id.clone(),
                    remove: false
                }
            )
            .await,
        Err(Error::NotFound)
    );
    store
        .link(
            Scope::Meenseek,
            &first.entity_id,
            LinkChange {
                revision: 2,
                target_id: second.entity_id.clone(),
                remove: false,
            },
        )
        .await
        .expect("link same scope");
    let before = store.calls();
    let detail = store
        .detail(Scope::Meenseek, &first.entity_id)
        .await
        .expect("detail");
    assert_eq!(
        store.calls() - before,
        1,
        "detail and related/history must be one query"
    );
    assert_eq!(detail["related"].as_array().expect("related").len(), 1);
    assert_eq!(detail["history"].as_array().expect("history").len(), 3);
    assert_eq!(
        detail["history"][1]["previous"]["areas"],
        serde_json::json!(["market-customer", "strategy-portfolio"])
    );
    assert_eq!(
        detail["history"][1]["confirmed"]["areas"][0],
        "business-operations"
    );
    assert!(serde_json::to_vec(&detail).expect("serialize").len() < MAX_RESPONSE_BYTES);
    store
        .apply_import(std::slice::from_ref(&first))
        .await
        .expect("idempotent reimport");
    let detail = store
        .detail(Scope::Meenseek, &first.entity_id)
        .await
        .expect("detail after import");
    assert_eq!(detail["revision"], 3);
    assert_eq!(detail["topics"][0], "정정한 주제");
    let mut invalid_record = record(101, Scope::Meenseek);
    invalid_record.content = Some("x".repeat(65_537));
    assert!(
        store
            .apply_import(&[record(100, Scope::Meenseek), invalid_record])
            .await
            .is_err()
    );
    assert_eq!(
        store
            .detail(Scope::Meenseek, &record(100, Scope::Meenseek).entity_id)
            .await,
        Err(Error::NotFound),
        "entire batch rolls back"
    );
    let counts: Value = store
        .list(Scope::Meenseek, "", true, None)
        .await
        .expect("unclassified filter");
    assert_eq!(counts["total"], 99);
}

fn git(repo: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("/usr/bin/git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("run fixture git");
    assert!(output.status.success(), "fixture git failed");
    String::from_utf8(output.stdout)
        .expect("git output")
        .trim()
        .into()
}
#[tokio::test]
async fn git_import_contract() {
    let _guard = TEST_LOCK.lock().await;
    use meenseek_ontology::importer::{GitReader, identity};
    let store = store().await;
    let temp = tempfile::tempdir().expect("temporary repository");
    let repo = temp
        .path()
        .canonicalize()
        .expect("canonical temporary repository");
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "test@example.invalid"]);
    git(&repo, &["config", "user.name", "Synthetic Test"]);
    for i in 0..100 {
        std::fs::write(
            repo.join(format!("source-{i}.md")),
            format!("# 합성 자료 {i}\n원문 <img src=x onerror=alert(1)>"),
        )
        .expect("write synthetic document");
    }
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "Synthetic fixtures"]);
    let first_commit = git(&repo, &["rev-parse", "HEAD"]);
    let reader = GitReader::new(vec![repo.clone()]).expect("explicit allow boundary");
    for size in [1, 100] {
        let paths = (0..size)
            .map(|i| format!("source-{i}.md"))
            .collect::<Vec<_>>();
        let before_git = reader.calls();
        let before_db = store.calls();
        reader
            .import(&store, &repo, &first_commit, &paths, Scope::Personal)
            .await
            .expect("import pinned commit");
        assert_eq!(reader.calls() - before_git, 3 + 2 * size as u64);
        assert_eq!(store.calls() - before_db, 6);
    }
    let paths: Vec<String> = vec!["source-0.md".into()];
    let (_, id) = identity(&repo, &paths[0], Scope::Personal).expect("stable identity");
    store
        .classify(
            Scope::Personal,
            &id,
            Classification {
                revision: 0,
                areas: vec![],
                topics: vec!["개인 합성 주제".into()],
            },
        )
        .await
        .expect("personal topic");
    reader
        .import(&store, &repo, &first_commit, &paths, Scope::Personal)
        .await
        .expect("same revision reimport");
    assert_eq!(
        store
            .detail(Scope::Personal, &id)
            .await
            .expect("projection")["revision"],
        1
    );
    std::fs::write(repo.join(&paths[0]), "# 새로운 커밋").expect("change synthetic document");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "New version"]);
    let second_commit = git(&repo, &["rev-parse", "HEAD"]);
    reader
        .import(&store, &repo, &second_commit, &paths, Scope::Personal)
        .await
        .expect("new revision");
    let detail = store
        .detail(Scope::Personal, &id)
        .await
        .expect("new detail");
    assert_eq!(detail["projection"]["source_revision"], second_commit);
    assert_eq!(detail["projection"]["content"], "# 새로운 커밋");
    assert_eq!(detail["topics"][0], "개인 합성 주제");
    let success = detail["source"]["last_success_at"].clone();
    let invalid_commit = "f".repeat(40);
    assert!(
        reader
            .import(&store, &repo, &invalid_commit, &paths, Scope::Personal)
            .await
            .is_err()
    );
    let failed = store
        .detail(Scope::Personal, &id)
        .await
        .expect("failure keeps projection");
    assert_eq!(failed["source"]["status"], "failed");
    assert_eq!(failed["source"]["last_success_at"], success);
    assert_eq!(failed["projection"], detail["projection"]);
    git(&repo, &["rm", "-q", "source-0.md"]);
    git(&repo, &["commit", "-qm", "Remove synthetic source"]);
    let missing_commit = git(&repo, &["rev-parse", "HEAD"]);
    let before = reader.calls();
    reader
        .import(&store, &repo, &missing_commit, &paths, Scope::Personal)
        .await
        .expect("proved absence");
    assert_eq!(reader.calls() - before, 4);
    let absent = store
        .detail(Scope::Personal, &id)
        .await
        .expect("missing detail");
    assert_eq!(absent["projection"]["present"], false);
    assert_eq!(absent["projection"]["absence_revision"], missing_commit);
    assert_eq!(absent["projection"]["content"], "# 새로운 커밋");
    assert_eq!(absent["topics"][0], "개인 합성 주제");
    for bad in [
        "../escape.md",
        "/absolute.md",
        "source-1.exe",
        ":(glob)*.md",
    ] {
        assert!(
            reader
                .read(&repo, &first_commit, &[bad.into()], Scope::Personal)
                .await
                .is_err()
        );
    }
    assert!(
        reader
            .read(
                &repo,
                &first_commit,
                &["source-1.md".into(), "source-1.md".into()],
                Scope::Personal
            )
            .await
            .is_err()
    );
    std::os::unix::fs::symlink("source-1.md", repo.join("link.md")).expect("symlink fixture");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "Symlink fixture"]);
    let bad_commit = git(&repo, &["rev-parse", "HEAD"]);
    assert!(
        reader
            .read(&repo, &bad_commit, &["link.md".into()], Scope::Personal)
            .await
            .is_err()
    );
    std::fs::remove_file(repo.join("link.md"))
        .expect("remove worktree link to exercise tree mode check");
    let (_, second_id) = identity(&repo, "source-1.md", Scope::Personal).expect("second identity");
    let previous = store
        .detail(Scope::Personal, &second_id)
        .await
        .expect("previous projection");
    assert!(
        reader
            .import(
                &store,
                &repo,
                &bad_commit,
                &["source-1.md".into(), "link.md".into()],
                Scope::Personal
            )
            .await
            .is_err()
    );
    let after = store
        .detail(Scope::Personal, &second_id)
        .await
        .expect("rollback projection");
    assert_eq!(after["projection"], previous["projection"]);
    assert_eq!(after["source"]["status"], "failed");
}

#[tokio::test]
async fn api_protection_contract() {
    let _guard = TEST_LOCK.lock().await;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use meenseek_ontology::{
        api::{AppState, router},
        config::Config,
    };
    use serde_json::json;
    use tower::ServiceExt;
    let store = store().await;
    let company = record(200, Scope::Meenseek);
    let personal = record(200, Scope::Personal);
    store
        .apply_import(&[company.clone(), personal.clone()])
        .await
        .expect("API fixtures");
    let app = router(AppState::new(
        store.clone(),
        Config {
            address: "127.0.0.1:47831".parse().expect("loopback address"),
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
                .expect("request"),
        )
        .await
        .expect("session response");
    let cookie = session.headers()["set-cookie"]
        .to_str()
        .expect("cookie")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_owned();
    let bytes = session
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let session: Value = serde_json::from_slice(&bytes).expect("session JSON");
    let csrf = session["csrf"].as_str().expect("CSRF");
    let revision = store
        .detail(Scope::Meenseek, &company.entity_id)
        .await
        .expect("current API fixture")["revision"]
        .as_i64()
        .expect("revision");
    let valid=json!({"revision":revision,"areas":["market-customer","growth-customer"],"topics":["가격","실험"]}).to_string();
    let url = format!(
        "/api/records/{}/classification?scope=meenseek",
        company.entity_id
    );
    for (host, origin, has_cookie, has_csrf) in [
        ("evil.invalid", "http://127.0.0.1:47831", true, true),
        ("127.0.0.1:47831", "http://evil.invalid", true, true),
        ("127.0.0.1:47831", "", true, true),
        ("127.0.0.1:47831", "http://127.0.0.1:47831", false, true),
        ("127.0.0.1:47831", "http://127.0.0.1:47831", true, false),
    ] {
        let mut builder = Request::builder()
            .method("POST")
            .uri(&url)
            .header("host", host)
            .header("content-type", "application/json");
        if !origin.is_empty() {
            builder = builder.header("origin", origin)
        }
        if has_cookie {
            builder = builder.header("cookie", &cookie)
        }
        if has_csrf {
            builder = builder.header("x-csrf-token", csrf)
        }
        let response = app
            .clone()
            .oneshot(
                builder
                    .body(Body::from(valid.clone()))
                    .expect("hostile request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    for path in [
        "/api/records".to_owned(),
        "/api/records?scope=other".to_owned(),
        format!("/api/records/{}?scope=meenseek", personal.entity_id),
        format!("/api/records?scope=meenseek&q={}", "x".repeat(121)),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(&path)
                    .header("host", "127.0.0.1:47831")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .expect("scoped request"),
            )
            .await
            .expect("response");
        assert!(matches!(
            response.status(),
            StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND
        ));
    }
    for body in [
        "{not-json}".to_owned(),
        json!({"revision":0,"areas":["wrong"],"topics":[]}).to_string(),
        "x".repeat(16_385),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(&url)
                    .header("host", "127.0.0.1:47831")
                    .header("origin", "http://127.0.0.1:47831")
                    .header("cookie", &cookie)
                    .header("x-csrf-token", csrf)
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .expect("invalid request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let error = response
            .into_body()
            .collect()
            .await
            .expect("error body")
            .to_bytes();
        assert!(!String::from_utf8_lossy(&error).contains("not-json"));
    }
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&url)
                .header("host", "127.0.0.1:47831")
                .header("origin", "http://127.0.0.1:47831")
                .header("cookie", &cookie)
                .header("x-csrf-token", csrf)
                .header("content-type", "application/json")
                .body(Body::from(valid))
                .expect("valid request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let path = format!("/api/records/{}?scope=meenseek", company.entity_id);
    let before = store.calls();
    let response = app
        .oneshot(
            Request::builder()
                .uri(path)
                .header("host", "127.0.0.1:47831")
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("detail request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(store.calls() - before, 1);
    assert_eq!(response.headers()["content-type"], "application/json");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert!(
        response.headers()["content-security-policy"]
            .to_str()
            .expect("CSP")
            .contains("script-src 'self'")
    );
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    assert!(bytes.len() < MAX_RESPONSE_BYTES);
    let detail: Value = serde_json::from_slice(&bytes).expect("detail JSON");
    assert_eq!(
        detail["areas"],
        json!(["growth-customer", "market-customer"])
    );
    assert_eq!(detail["topics"], json!(["가격", "실험"]));
    assert!(
        detail["projection"]["content"]
            .as_str()
            .expect("content")
            .contains("<script>"),
        "untrusted source is returned as JSON text, never rendered HTML"
    );
    for area in ["market-customer", "growth-customer"] {
        let page = store
            .list(Scope::Meenseek, "document-200", false, Some(area))
            .await
            .expect("area facet");
        assert_eq!(page["total"], 1);
        assert_eq!(page["items"][0]["id"], company.entity_id);
    }
    assert_eq!(
        store
            .list(Scope::Personal, "", false, Some("market-customer"))
            .await,
        Err(Error::Invalid)
    );
    assert_eq!(
        store
            .list(Scope::Meenseek, "", false, Some("unknown"))
            .await,
        Err(Error::Invalid)
    );
    let personal_topic:i64=sqlx::query_scalar("INSERT INTO topics(scope,name) VALUES('personal','범위 밖 주제') ON CONFLICT(scope,name) DO UPDATE SET name=EXCLUDED.name RETURNING id").fetch_one(store.pool()).await.expect("personal topic");
    assert!(
        sqlx::query("INSERT INTO entity_topics(scope,entity_id,topic_id) VALUES('meenseek',$1,$2)")
            .bind(&company.entity_id)
            .bind(personal_topic)
            .execute(store.pool())
            .await
            .is_err(),
        "cross-scope topic mapping must fail"
    );
}

async fn reset_schema(store: &Store) {
    // store() has checked the explicitly supplied loopback ontology_test_ database.
    sqlx::raw_sql("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
        .execute(store.pool())
        .await
        .expect("reset only the isolated test database schema");
}
async fn data_snapshot(store: &Store, upgraded: bool) -> Value {
    // Compare all original columns (including timestamps, IDs, revisions and history).
    let query = r#"SELECT jsonb_build_object(
        'scopes',(SELECT jsonb_agg(to_jsonb(s) ORDER BY id) FROM scopes s),
        'areas',(SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM areas a),
        'sources',(SELECT jsonb_agg(to_jsonb(s) ORDER BY id) FROM sources s),
        'records',(SELECT jsonb_agg(to_jsonb(p) ORDER BY entity_id) FROM source_records p),
        'entities',(SELECT jsonb_agg(to_jsonb(e) ORDER BY id) FROM entities e),
        'topics',(SELECT jsonb_agg(to_jsonb(t) ORDER BY id) FROM topics t),
        'entity_areas',(SELECT jsonb_agg(to_jsonb(a) ORDER BY entity_id,area) FROM entity_areas a),
        'entity_topics',(SELECT jsonb_agg(to_jsonb(t) ORDER BY entity_id,topic_id) FROM entity_topics t),
        'links',(SELECT jsonb_agg(to_jsonb(r) ORDER BY left_id,right_id) FROM related_materials r),
        'history',(SELECT jsonb_agg(to_jsonb(h) ORDER BY id) FROM confirmation_history h))"#;
    let mut result: Value = sqlx::query_scalar(query)
        .fetch_one(store.pool())
        .await
        .expect("snapshot every persisted user table");
    if upgraded {
        for (table, old, new) in [
            ("sources", "verified_commit", "verified_revision"),
            ("records", "absence_commit", "absence_revision"),
        ] {
            if let Some(rows) = result[table].as_array_mut() {
                for row in rows {
                    let object = row.as_object_mut().expect("table rows are JSON objects");
                    let revision = object.remove(new).expect("upgraded revision column exists");
                    object.insert(old.into(), revision);
                    if table == "sources" {
                        object.remove("kind");
                        object.remove("generation");
                    }
                }
            }
        }
    }
    result
}
async fn schema_snapshot(store: &Store) -> Value {
    sqlx::query_scalar(r#"SELECT jsonb_build_object(
        'columns',(SELECT jsonb_agg(jsonb_build_array(table_name,column_name,ordinal_position,data_type,is_nullable,column_default,is_identity) ORDER BY table_name,ordinal_position) FROM information_schema.columns WHERE table_schema='public'),
        'constraints',(SELECT jsonb_agg(jsonb_build_array(c.relname,k.conname,pg_get_constraintdef(k.oid)) ORDER BY c.relname,k.conname) FROM pg_constraint k JOIN pg_class c ON c.oid=k.conrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public'),
        'indexes',(SELECT jsonb_agg(jsonb_build_array(tablename,indexname,indexdef) ORDER BY tablename,indexname) FROM pg_indexes WHERE schemaname='public'))"#)
        .fetch_one(store.pool()).await.expect("capture columns, constraints and indexes")
}

#[tokio::test]
async fn migration_preserves_baseline_contract() {
    use meenseek_ontology::store::{BASELINE, SOURCE_PROVIDERS_MIGRATION};
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    reset_schema(&store).await;
    sqlx::raw_sql(BASELINE)
        .execute(store.pool())
        .await
        .expect("create historical baseline");
    sqlx::raw_sql("CREATE TABLE ontology_baseline (singleton boolean PRIMARY KEY CHECK(singleton), digest text NOT NULL)")
        .execute(store.pool()).await.expect("create historical baseline marker");
    sqlx::query("INSERT INTO ontology_baseline VALUES(true,$1)")
        .bind(digest(BASELINE.as_bytes()))
        .execute(store.pool())
        .await
        .expect("pin original baseline digest");
    sqlx::raw_sql(r#"
        INSERT INTO areas VALUES('strategy-portfolio','전략·포트폴리오'),('market-customer','시장·고객 이해');
        INSERT INTO sources(id,scope,repository,path,status,last_success_at,verified_commit,failure_code) VALUES
          ('old-source-a','meenseek','/fixture/git','a.md','ok',now(),'aaaaaaaa',NULL),
          ('old-source-b','meenseek','/fixture/git','b.md','missing',now(),'bbbbbbbb',NULL),
          ('old-source-personal','personal','/fixture/git','a.md','failed',now(),'cccccccc','git-read-failed');
        INSERT INTO entities(id,scope,source_id,revision) VALUES
          ('old-entity-a','meenseek','old-source-a',4),('old-entity-b','meenseek','old-source-b',2),('old-entity-personal','personal','old-source-personal',1);
        INSERT INTO source_records(entity_id,scope,content,content_digest,source_revision,present,absence_commit) VALUES
          ('old-entity-a','meenseek','회사 합성 자료','digest-a','aaaaaaaa',true,NULL),
          ('old-entity-b','meenseek','삭제 전 합성 내용','digest-b','earlier',false,'bbbbbbbb'),
          ('old-entity-personal','personal','개인 합성 자료','digest-p','cccccccc',true,NULL);
        INSERT INTO entity_areas VALUES('meenseek','old-entity-a','strategy-portfolio'),('meenseek','old-entity-a','market-customer');
        INSERT INTO topics(scope,name) VALUES('meenseek','시장'),('meenseek','정정'),('personal','개인');
        INSERT INTO entity_topics SELECT scope,CASE WHEN scope='personal' THEN 'old-entity-personal' ELSE 'old-entity-a' END,id FROM topics;
        INSERT INTO related_materials(scope,left_id,right_id) VALUES('meenseek','old-entity-a','old-entity-b');
        INSERT INTO confirmation_history(scope,entity_id,revision,kind,previous,confirmed) VALUES
          ('meenseek','old-entity-a',3,'classification','{"topics":["이전"]}','{"topics":["정정"]}'),
          ('meenseek','old-entity-a',4,'link-add','{"linked":false}','{"linked":true}'),
          ('personal','old-entity-personal',1,'classification','{}','{"topics":["개인"]}');
    "#).execute(store.pool()).await.expect("populate historical company and personal confirmations");
    let before = data_snapshot(&store, false).await;
    let old_schema = schema_snapshot(&store).await;
    // Failure after earlier ALTERs must roll back both those ALTERs and the migration marker.
    sqlx::query("ALTER TABLE sources DROP CONSTRAINT sources_failure_code_check")
        .execute(store.pool())
        .await
        .expect("inject mid-migration failure");
    let failure_schema = schema_snapshot(&store).await;
    assert_eq!(store.initialize().await, Err(Error::Baseline));
    assert_eq!(schema_snapshot(&store).await, failure_schema);
    assert_eq!(data_snapshot(&store, false).await, before);
    sqlx::query("ALTER TABLE sources ADD CONSTRAINT sources_failure_code_check CHECK(failure_code IS NULL OR failure_code='git-read-failed')")
        .execute(store.pool()).await.expect("restore fixture baseline constraint");
    assert_eq!(schema_snapshot(&store).await, old_schema);
    store
        .initialize()
        .await
        .expect("upgrade the historical baseline");
    assert_eq!(data_snapshot(&store, true).await, before);
    let kinds: Vec<String> = sqlx::query_scalar("SELECT kind FROM sources")
        .fetch_all(store.pool())
        .await
        .expect("migrated providers");
    assert!(kinds.iter().all(|kind| kind == "git"));
    let upgraded_schema = schema_snapshot(&store).await;
    store.initialize().await.expect("repeat upgrade safely");
    assert_eq!(data_snapshot(&store, true).await, before);
    assert_eq!(schema_snapshot(&store).await, upgraded_schema);
    sqlx::query(
        "UPDATE ontology_migrations SET digest='drift' WHERE name='001-source-providers.sql'",
    )
    .execute(store.pool())
    .await
    .expect("inject digest drift");
    assert_eq!(store.initialize().await, Err(Error::Baseline));
    sqlx::query("UPDATE ontology_migrations SET digest=$1 WHERE name='001-source-providers.sql'")
        .bind(digest(SOURCE_PROVIDERS_MIGRATION.as_bytes()))
        .execute(store.pool())
        .await
        .expect("restore migration digest");
    sqlx::query("INSERT INTO ontology_migrations VALUES('unexpected.sql','unknown')")
        .execute(store.pool())
        .await
        .expect("inject unknown migration");
    assert_eq!(store.initialize().await, Err(Error::Baseline));
    sqlx::query("DELETE FROM ontology_migrations WHERE name='unexpected.sql'")
        .execute(store.pool())
        .await
        .expect("remove injected marker");
    sqlx::query(
        "UPDATE ontology_migrations SET name='renamed.sql' WHERE name='001-source-providers.sql'",
    )
    .execute(store.pool())
    .await
    .expect("inject migration name drift");
    assert_eq!(store.initialize().await, Err(Error::Baseline));
    assert_eq!(data_snapshot(&store, true).await, before);
    reset_schema(&store).await;
    store
        .initialize()
        .await
        .expect("fresh baseline then the same migration");
    assert_eq!(schema_snapshot(&store).await, upgraded_schema);
}

#[tokio::test]
async fn vault_import_contract() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    sqlx::query("TRUNCATE context_source_bindings,context_projection_versions,context_material_versions,context_materials,context_apply_batches").execute(store.pool()).await.expect("context reset");
    let temp = tempfile::tempdir().expect("fixture");
    let root = temp.path().canonicalize().expect("root");
    std::fs::create_dir_all(root.join("personal")).expect("scope");
    std::fs::write(
        root.join("personal/one.md"),
        "---\ntitle: One\nexport: false\n---\n\nBody\n",
    )
    .expect("body");
    let scopes = vec!["personal".parse().expect("scope")];
    let inventory = meenseek_ontology::context::inventory(&root, &scopes).expect("inventory");
    store
        .import_context(&root, &scopes, &inventory.inventory_digest)
        .await
        .expect("original bytes");
    let store_id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
        .fetch_one(store.pool())
        .await
        .expect("store id");
    std::fs::remove_dir_all(&root).expect("no old transport");
    let mut reader = meenseek_ontology::context_importer::ContextReader::new();
    assert_eq!(
        reader
            .import(
                &store,
                &store_id,
                &scopes[0],
                &["one.md".into()],
                Scope::Personal
            )
            .await
            .expect("canonical consumer"),
        1
    );
    assert_eq!(reader.body_calls, 1);
}

#[tokio::test]
async fn context_http_read_download_scope_and_protection_contract() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use meenseek_ontology::{
        api::{AppState, router},
        config::Config,
        context::{ContextScope, MAX_FILE_BYTES, MAX_READ_BYTES, inventory},
    };
    use tower::ServiceExt;
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    sqlx::query("TRUNCATE context_source_bindings,context_projection_versions, context_material_versions, context_materials, context_apply_batches")
        .execute(store.pool())
        .await
        .expect("isolated original fixtures");
    let source = tempfile::tempdir().expect("synthetic originals");
    let root = source
        .path()
        .canonicalize()
        .expect("canonical synthetic root");
    let scopes: Vec<ContextScope> = ["personal", "work/alpha", "work/beta", "work/restricted"]
        .into_iter()
        .map(|value| value.parse().expect("synthetic scope"))
        .collect();
    let text = "\u{feff}---\r\ntitle: 합성\r\n---\r\n<script>alert('fixture')</script>\n"
        .as_bytes()
        .to_vec();
    let binary = vec![0xff; MAX_FILE_BYTES];
    let files = [
        ("personal/한글 \";문서.html", text.clone()),
        ("personal/empty.txt", vec![]),
        ("personal/escaped.txt", vec![0; MAX_READ_BYTES]),
        ("personal/large.txt", vec![b'x'; MAX_READ_BYTES + 1]),
        ("personal/full.bin", binary.clone()),
        ("personal/small.bin", vec![255, 128, 0]),
        ("personal/journal/private.md", b"private-marker".to_vec()),
        ("personal/raw/data", b"private-marker".to_vec()),
        ("personal/.hidden", b"private-marker".to_vec()),
        ("work/alpha/same.md", b"alpha original".to_vec()),
        ("work/beta/same.md", b"beta original".to_vec()),
        (
            "work/restricted/journal/only.md",
            b"private-marker".to_vec(),
        ),
    ];
    for (path, bytes) in &files {
        let target = root.join(path);
        std::fs::create_dir_all(target.parent().expect("synthetic parent")).expect("fixture dirs");
        std::fs::write(target, bytes).expect("synthetic bytes");
    }
    let manifest = inventory(&root, &scopes).expect("scoped fixture inventory");
    store
        .import_context(&root, &scopes, &manifest.inventory_digest)
        .await
        .expect("fixture originals");
    let app = router(AppState::new(
        store.clone(),
        Config {
            address: "127.0.0.1:47831".parse().expect("loopback"),
            web_dist: "web/dist".into(),
        },
    ));
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header("host", "127.0.0.1:47831")
                .body(Body::empty())
                .expect("session request"),
        )
        .await
        .expect("session");
    let cookie = response.headers()["set-cookie"]
        .to_str()
        .expect("cookie")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_owned();
    let session: Value = serde_json::from_slice(
        &response
            .into_body()
            .collect()
            .await
            .expect("session body")
            .to_bytes(),
    )
    .expect("session JSON");
    let csrf = session["csrf"].as_str().expect("CSRF token");
    let get = |path: &str| {
        Request::builder()
            .uri(path)
            .header("host", "127.0.0.1:47831")
            .header("cookie", &cookie)
            .body(Body::empty())
            .expect("read request")
    };
    let before = store.calls();
    let response = app
        .clone()
        .oneshot(get("/api/context/scopes"))
        .await
        .expect("discovery");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("metadata")
        .to_bytes();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).expect("scope JSON"),
        serde_json::json!({"scopes":["personal","work/alpha","work/beta"]})
    );
    assert_eq!(store.calls() - before, 5);
    assert!(bytes.len() < 1024);
    // Cardinality affects only bounded metadata transfer; a page never prefetches its bodies.
    for size in [0usize, 1, 25] {
        sqlx::query("TRUNCATE context_source_bindings,context_projection_versions, context_material_versions, context_materials, context_apply_batches")
            .execute(store.pool())
            .await
            .expect("reset complete isolated context fixture");
        store
            .import_context(&root, &scopes, &manifest.inventory_digest)
            .await
            .expect("restore same visible imported dataset");
        for number in 0..size {
            let path = format!("{number:02}.md");
            sqlx::query("INSERT INTO context_materials(scope,path,source_root,source_path,source_digest,content_digest,content,byte_len,restricted,search_text) VALUES('profile',$1,'/synthetic',$2,$3,$3,$4,7,false,'fixture')")
                .bind(&path).bind(format!("profile/{path}")).bind(digest(b"fixture")).bind(b"fixture".as_slice()).execute(store.pool()).await.expect("synthetic metadata cardinality");
        }
        let before = store.calls();
        let response = app
            .clone()
            .oneshot(get("/api/context?scope=profile&q=fixture&limit=20"))
            .await
            .expect("metadata page");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(store.calls() - before, 5);
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("page")
            .to_bytes();
        assert!(bytes.len() <= 512 + size.min(20) * 512);
        let page: Value = serde_json::from_slice(&bytes).expect("page JSON");
        assert_eq!(page["items"].as_array().expect("items").len(), size.min(20));
        for item in page["items"].as_array().expect("metadata only") {
            for key in ["content", "search_text", "source_root", "restricted"] {
                assert!(item.get(key).is_none(), "omit {key}");
            }
        }
        if size > 20 {
            assert_eq!(page["next_after"], "19.md");
            let before = store.calls();
            let response = app
                .clone()
                .oneshot(get(
                    "/api/context?scope=profile&q=fixture&limit=20&after=19.md",
                ))
                .await
                .expect("next page");
            let page: Value = serde_json::from_slice(
                &response
                    .into_body()
                    .collect()
                    .await
                    .expect("next body")
                    .to_bytes(),
            )
            .expect("next JSON");
            assert_eq!(store.calls() - before, 5);
            assert_eq!(page["items"].as_array().expect("remaining").len(), 5);
            assert!(page["next_after"].is_null());
        }
    }
    let encoded = "%ED%95%9C%EA%B8%80%20%22%3B%EB%AC%B8%EC%84%9C.html";
    for (path, expected) in [
        (
            format!("/api/context/read?scope=personal&path={encoded}"),
            text.clone(),
        ),
        (
            "/api/context/read?scope=personal&path=empty.txt".into(),
            vec![],
        ),
        (
            "/api/context/read?scope=personal&path=escaped.txt".into(),
            vec![0; MAX_READ_BYTES],
        ),
        (
            "/api/context/read?scope=work/alpha&path=same.md".into(),
            b"alpha original".to_vec(),
        ),
        (
            "/api/context/read?scope=work/beta&path=same.md".into(),
            b"beta original".to_vec(),
        ),
    ] {
        let before = store.calls();
        let response = app
            .clone()
            .oneshot(get(&path))
            .await
            .expect("selected text");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(store.calls() - before, 5);
        assert_eq!(response.headers()["content-type"], "application/json");
        assert_eq!(response.headers()["cache-control"], "no-store");
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("text body")
            .to_bytes();
        assert!(bytes.len() <= 6 * MAX_READ_BYTES + 16 * 1024);
        let result: Value = serde_json::from_slice(&bytes).expect("selected JSON");
        assert_eq!(
            result["content"]
                .as_str()
                .expect("UTF-8 original")
                .as_bytes(),
            expected
        );
        assert_eq!(result["metadata"]["content_digest"], digest(&expected));
        assert_eq!(result["metadata"]["source_digest"], digest(&expected));
        assert!(result["metadata"].get("source_root").is_none());
    }
    for (path, expected) in [(encoded, &text), ("full.bin", &binary)] {
        let before = store.calls();
        let response = app
            .clone()
            .oneshot(get(&format!(
                "/api/context/download?scope=personal&path={path}"
            )))
            .await
            .expect("attachment");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(store.calls() - before, 5);
        assert_eq!(
            response.headers()["content-type"],
            "application/octet-stream"
        );
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        assert_eq!(
            response.headers()["content-length"]
                .to_str()
                .expect("length"),
            expected.len().to_string()
        );
        let disposition = response.headers()["content-disposition"]
            .to_str()
            .expect("safe attachment header");
        assert!(disposition.starts_with("attachment; filename=\""));
        assert!(disposition.contains("filename*=UTF-8''"));
        assert!(!disposition.contains(root.to_str().expect("temporary root")));
        if path == encoded {
            assert!(
                disposition
                    .ends_with("%ED%95%9C%EA%B8%80%20%22%3B%EB%AC%B8%EC%84%9C%2E%68%74%6D%6C")
            );
        }
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("original bytes")
            .to_bytes();
        assert_eq!(digest(&bytes), digest(expected));
        assert!(bytes.len() <= MAX_FILE_BYTES);
    }
    for (path, status, calls) in [
        (
            "/api/context/read?scope=personal&path=large.txt",
            StatusCode::PAYLOAD_TOO_LARGE,
            5,
        ),
        (
            "/api/context/read?scope=personal&path=full.bin",
            StatusCode::PAYLOAD_TOO_LARGE,
            5,
        ),
        (
            "/api/context/read?scope=personal&path=absent",
            StatusCode::NOT_FOUND,
            5,
        ),
        (
            "/api/context/download?scope=work/alpha&path=full.bin",
            StatusCode::NOT_FOUND,
            5,
        ),
        (
            "/api/context/read?scope=personal&path=journal/private.md",
            StatusCode::NOT_FOUND,
            4,
        ),
        (
            "/api/context/download?scope=personal&path=raw/data",
            StatusCode::NOT_FOUND,
            4,
        ),
        (
            "/api/context/download?scope=personal&path=.hidden",
            StatusCode::NOT_FOUND,
            4,
        ),
        (
            "/api/context/download?scope=personal&path=%2E%2E/x",
            StatusCode::BAD_REQUEST,
            0,
        ),
        (
            "/api/context/read?scope=work&path=same.md",
            StatusCode::BAD_REQUEST,
            0,
        ),
        (
            "/api/context/read?scope=personal&path=empty.txt&archive=true",
            StatusCode::BAD_REQUEST,
            0,
        ),
        (
            "/api/context/read?scope=personal&scope=work/alpha&path=empty.txt",
            StatusCode::BAD_REQUEST,
            0,
        ),
        (
            "/api/context/download?scope=personal&path=empty.txt&destination=/tmp/out",
            StatusCode::BAD_REQUEST,
            0,
        ),
        (
            "/api/context?scope=personal&limit=21",
            StatusCode::BAD_REQUEST,
            0,
        ),
        (
            "/api/context?scope=personal&limit=0",
            StatusCode::BAD_REQUEST,
            0,
        ),
        (
            "/api/context?scope=personal&after=../x",
            StatusCode::BAD_REQUEST,
            0,
        ),
        (
            "/api/context/scopes?archive=true",
            StatusCode::BAD_REQUEST,
            0,
        ),
        ("/api/context/import", StatusCode::NOT_FOUND, 0),
        ("/api/context/export", StatusCode::NOT_FOUND, 0),
    ] {
        let before = store.calls();
        let response = app
            .clone()
            .oneshot(get(path))
            .await
            .expect("bounded failure");
        assert_eq!(response.status(), status, "{path}");
        assert_eq!(store.calls() - before, calls, "no failure retry: {path}");
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("small error")
            .to_bytes();
        assert!(bytes.len() < 256);
        assert!(!String::from_utf8_lossy(&bytes).contains("private-marker"));
    }
    for path in [
        "/api/context/read?scope=personal&path=small.bin".to_owned(),
        "/api/context?scope=personal&q=%0A".to_owned(),
        format!("/api/context?scope=personal&q={}", "x".repeat(241)),
        format!(
            "/api/context/download?scope=personal&path={}",
            "x".repeat(1025)
        ),
    ] {
        let before = store.calls();
        let response = app
            .clone()
            .oneshot(get(&path))
            .await
            .expect("bounded invalid input");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            store.calls() - before,
            5 * u64::from(path.ends_with("small.bin"))
        );
        assert!(
            response
                .into_body()
                .collect()
                .await
                .expect("error body")
                .to_bytes()
                .len()
                < 256
        );
    }
    let response = app
        .clone()
        .oneshot(get("/api/context?scope=personal&q=private-marker"))
        .await
        .expect("restricted search");
    let page: Value = serde_json::from_slice(
        &response
            .into_body()
            .collect()
            .await
            .expect("metadata body")
            .to_bytes(),
    )
    .expect("metadata JSON");
    assert!(
        page["items"]
            .as_array()
            .expect("restricted entries excluded")
            .is_empty()
    );
    // Synthetic persistence records exercise the real read-only HTTP boundary.
    let apply: String = sqlx::query_scalar("WITH i AS (SELECT gen_random_uuid() AS id) INSERT INTO context_apply_batches(apply_id,store_id,core_run_id,prepared_run_digest,candidate_digest,expected_source_versions,context_targets,core_apply_attempt_id,expected_batch_id,expected_journal_locator,state) SELECT i.id,s.store_id,'synthetic-http',repeat('a',64),repeat('b',64),'{}','[{}]',i.id::text,i.id::text,i.id::text,'pending' FROM i CROSS JOIN context_store s RETURNING apply_id::text")
        .fetch_one(store.pool()).await.expect("synthetic pending HTTP fixture");
    sqlx::query("INSERT INTO context_materials(scope,path,source_root,source_path,source_digest,imported_at,origin_kind,content,content_digest,byte_len,restricted,search_text,last_apply_id) VALUES('personal','native.md',NULL,'personal/native.md',NULL,NULL,'native',$1,$2,6,false,'native',$3::uuid)")
        .bind(b"native".as_slice()).bind(digest(b"native")).bind(&apply).execute(store.pool()).await.expect("native fixture before synthetic commit");
    for state in ["pending", "committed"] {
        if state == "committed" {
            sqlx::query("UPDATE context_apply_batches SET state='committed',actual_batch_id=expected_batch_id,actual_journal_locator=expected_journal_locator,commit_receipt='{\"synthetic\":true}' WHERE apply_id=$1::uuid")
                .bind(&apply).execute(store.pool()).await.expect("synthetic committed fixture");
        }
        for endpoint in [
            "/api/context/scopes",
            "/api/context?scope=personal",
            "/api/context/read?scope=personal&path=native.md",
            "/api/context/download?scope=personal&path=native.md",
            "/api/context/read?scope=personal&path=absent",
            "/api/context/download?scope=personal&path=absent",
            "/api/context/read?scope=personal&path=journal/private.md",
            "/api/context/download?scope=personal&path=raw/data",
        ] {
            let before = store.calls();
            let response = app
                .clone()
                .oneshot(get(endpoint))
                .await
                .expect("pending read boundary");
            assert_eq!(
                response.status(),
                StatusCode::CONFLICT,
                "{state}: {endpoint}"
            );
            assert_eq!(store.calls() - before, 4, "no data query while unresolved");
            let bytes = response
                .into_body()
                .collect()
                .await
                .expect("bounded pending error")
                .to_bytes();
            assert_eq!(
                serde_json::from_slice::<Value>(&bytes).expect("error JSON"),
                serde_json::json!({"error":Error::ContextPending.to_string()})
            );
        }
    }
    sqlx::query("UPDATE context_apply_batches SET state='finalized',final_core_receipt_digest=repeat('c',64) WHERE apply_id=$1::uuid")
        .bind(&apply).execute(store.pool()).await.expect("synthetic final fixture");
    let response = app
        .clone()
        .oneshot(get("/api/context/read?scope=personal&path=native.md"))
        .await
        .expect("native read");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("native response")
        .to_bytes();
    let native: Value = serde_json::from_slice(&bytes).expect("native JSON");
    assert_eq!(native["content"], "native");
    assert_eq!(native["metadata"]["origin_kind"], "native");
    assert_eq!(native["metadata"]["revision"], 1);
    assert!(
        native["metadata"]["material_id"]
            .as_str()
            .is_some_and(|id| id.len() == 36)
    );
    assert!(native["metadata"]["source_digest"].is_null());
    assert!(
        native["metadata"].get("source_root").is_none(),
        "HTTP still omits server paths"
    );
    for endpoint in [
        "/api/context/scopes",
        "/api/context?scope=personal",
        "/api/context/read?scope=personal&path=empty.txt",
        "/api/context/download?scope=personal&path=empty.txt",
    ] {
        for scenario in 0..7 {
            let mut builder = Request::builder().uri(endpoint).header(
                "host",
                if scenario == 0 {
                    "evil.invalid"
                } else {
                    "127.0.0.1:47831"
                },
            );
            if scenario != 1 {
                builder = builder.header("cookie", &cookie);
            }
            match scenario {
                2 => builder = builder.header("origin", "http://evil.invalid"),
                3 => builder = builder.header("sec-fetch-site", "cross-site"),
                4 => builder = builder.header("host", "127.0.0.1:47831"),
                5 => {
                    builder = builder
                        .header("origin", "http://127.0.0.1:47831")
                        .header("origin", "http://127.0.0.1:47831")
                }
                6 => builder = builder.header("cookie", &cookie),
                _ => {}
            }
            let before = store.calls();
            let response = app
                .clone()
                .oneshot(builder.body(Body::empty()).expect("hostile read"))
                .await
                .expect("protection");
            assert_eq!(
                response.status(),
                StatusCode::FORBIDDEN,
                "{endpoint}: {scenario}"
            );
            assert_eq!(store.calls() - before, 0);
        }
        // No write command is routed, even with a valid write session.
        for authorized in [false, true] {
            let mut builder = Request::builder()
                .method("POST")
                .uri(endpoint)
                .header("host", "127.0.0.1:47831")
                .header("cookie", &cookie)
                .header("content-type", "application/json");
            if authorized {
                builder = builder
                    .header("origin", "http://127.0.0.1:47831")
                    .header("x-csrf-token", csrf);
            }
            let before = store.calls();
            let response = app
                .clone()
                .oneshot(
                    builder
                        .body(Body::from("{\"op\":\"export\"}"))
                        .expect("write probe"),
                )
                .await
                .expect("write rejection");
            assert_eq!(
                response.status(),
                if authorized {
                    StatusCode::METHOD_NOT_ALLOWED
                } else {
                    StatusCode::FORBIDDEN
                }
            );
            assert_eq!(store.calls() - before, 0);
        }
    }
}

#[tokio::test]
async fn context_005_upgrade_preserves_exact_imports_and_backfills_one_version() {
    use meenseek_ontology::{
        context::{ContextScope, inventory},
        store::BASELINE,
    };
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    reset_schema(&store).await;
    sqlx::raw_sql(BASELINE)
        .execute(store.pool())
        .await
        .expect("historical baseline");
    sqlx::raw_sql("CREATE TABLE ontology_baseline(singleton boolean PRIMARY KEY CHECK(singleton),digest text NOT NULL); CREATE TABLE ontology_migrations(name text PRIMARY KEY,digest text NOT NULL)")
        .execute(store.pool()).await.expect("historical migration ledger");
    sqlx::query("INSERT INTO ontology_baseline VALUES(true,$1)")
        .bind(digest(BASELINE.as_bytes()))
        .execute(store.pool())
        .await
        .expect("historical baseline digest");
    let areas = serde_json::json!(
        meenseek_ontology::domain::AREAS
            .iter()
            .map(|(id, label)| serde_json::json!({"id":id,"label":label}))
            .collect::<Vec<_>>()
    );
    sqlx::query(
        "INSERT INTO areas SELECT id,label FROM jsonb_to_recordset($1) AS x(id text,label text)",
    )
    .bind(areas)
    .execute(store.pool())
    .await
    .expect("seed canonical baseline areas");
    for (name, sql) in [
        (
            "001-source-providers.sql",
            include_str!("../schema/migrations/001-source-providers.sql"),
        ),
        (
            "002-second-brain.sql",
            include_str!("../schema/migrations/002-second-brain.sql"),
        ),
        (
            "003-evidence-snapshots.sql",
            include_str!("../schema/migrations/003-evidence-snapshots.sql"),
        ),
        (
            "004-curation-reviews.sql",
            include_str!("../schema/migrations/004-curation-reviews.sql"),
        ),
        (
            "005-context-materials.sql",
            include_str!("../schema/migrations/005-context-materials.sql"),
        ),
    ] {
        sqlx::raw_sql(sql)
            .execute(store.pool())
            .await
            .expect("migrate through 005 only");
        sqlx::query("INSERT INTO ontology_migrations VALUES($1,$2)")
            .bind(name)
            .bind(digest(sql.as_bytes()))
            .execute(store.pool())
            .await
            .expect("pin historical digest");
    }
    let temp = tempfile::tempdir().expect("synthetic 005 source");
    let root = temp
        .path()
        .canonicalize()
        .expect("canonical synthetic root");
    for (path, content, restricted) in [
        ("note.md", b"\xef\xbb\xbf---\r\nexact\r\n".as_slice(), false),
        ("raw/binary", &[0, 255, 10], true),
        ("empty.md", b"", false),
    ] {
        let target = root.join("personal").join(path);
        std::fs::create_dir_all(target.parent().expect("synthetic parent")).expect("directories");
        std::fs::write(target, content).expect("exact synthetic bytes");
        sqlx::query("INSERT INTO context_materials(scope,path,source_root,source_path,source_digest,content_digest,content,byte_len,restricted,search_text,imported_at) VALUES('personal',$1,$2,$3,$4,$4,$5,$6,$7,$8,'2025-01-02T03:04:05Z')")
            .bind(path).bind(root.to_str().expect("UTF8 root")).bind(format!("personal/{path}")).bind(digest(content)).bind(content).bind(content.len() as i64).bind(restricted)
            .bind(if restricted {None} else {std::str::from_utf8(content).ok()}).execute(store.pool()).await.expect("populated 005 originals");
    }
    let before: Value =
        sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(m) ORDER BY path) FROM context_materials m")
            .fetch_one(store.pool())
            .await
            .expect("complete old bytes and provenance");
    store.initialize().await.expect("006 additive migration");
    let after: Value = sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(m) - ARRAY['material_id','revision','deleted','origin_kind','created_at','last_apply_id'] ORDER BY path) FROM context_materials m").fetch_one(store.pool()).await.expect("original columns");
    assert_eq!(before, after);
    let exact: bool = sqlx::query_scalar("SELECT bool_and(m.revision=1 AND NOT m.deleted AND m.origin_kind='imported-file' AND m.created_at=m.imported_at AND m.last_apply_id IS NULL AND ROW(m.content,m.content_digest,m.byte_len,m.restricted,m.search_text,m.deleted,m.last_apply_id,m.created_at) IS NOT DISTINCT FROM ROW(v.content,v.content_digest,v.byte_len,v.restricted,v.search_text,v.deleted,v.apply_id,v.recorded_at)) FROM context_materials m JOIN context_material_versions v USING(material_id,revision)").fetch_one(store.pool()).await.expect("backfilled bytes and original timestamps");
    assert!(exact);
    let stable: Value = sqlx::query_scalar("SELECT jsonb_build_object('store',(SELECT to_jsonb(s) FROM context_store s),'materials',(SELECT jsonb_agg(to_jsonb(m) ORDER BY path) FROM context_materials m),'history',(SELECT jsonb_agg(to_jsonb(v) ORDER BY material_id,revision) FROM context_material_versions v))")
        .fetch_one(store.pool()).await.expect("new canonical records");
    let scopes: Vec<ContextScope> = vec!["personal".parse().expect("synthetic scope")];
    let manifest = inventory(&root, &scopes).expect("same originals");
    for _ in 0..2 {
        store.initialize().await.expect("repeat additive init");
        assert_eq!(
            store
                .import_context(&root, &scopes, &manifest.inventory_digest)
                .await
                .expect("idempotent imported originals")["inserted"],
            0
        );
        let unchanged: Value = sqlx::query_scalar("SELECT jsonb_build_object('store',(SELECT to_jsonb(s) FROM context_store s),'materials',(SELECT jsonb_agg(to_jsonb(m) ORDER BY path) FROM context_materials m),'history',(SELECT jsonb_agg(to_jsonb(v) ORDER BY material_id,revision) FROM context_material_versions v))")
            .fetch_one(store.pool()).await.expect("unchanged canonical records");
        assert_eq!(unchanged, stable);
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM context_material_versions")
            .fetch_one(store.pool())
            .await
            .expect("one history per original");
        assert_eq!(count, 3);
    }
}
