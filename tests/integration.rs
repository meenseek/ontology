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
    test_vault_binary();
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
        assert_eq!(store.calls() - before_db, 3);
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

fn test_vault_binary() -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = std::path::PathBuf::from(
        std::env::var_os("TEST_VAULT_BINARY")
            .expect("TEST_VAULT_BINARY must name the built Vault executable by absolute path"),
    );
    assert!(
        path.is_absolute()
            && std::fs::metadata(&path)
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0),
        "TEST_VAULT_BINARY must be an executable absolute file path"
    );
    path
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
    sqlx::query("UPDATE ontology_migrations SET digest='drift'")
        .execute(store.pool())
        .await
        .expect("inject digest drift");
    assert_eq!(store.initialize().await, Err(Error::Baseline));
    sqlx::query("UPDATE ontology_migrations SET digest=$1")
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
    sqlx::query("UPDATE ontology_migrations SET name='renamed.sql'")
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

fn vault_document(scope: &str, number: usize) -> String {
    format!(
        "---\ntitle: 합성 문서 {number}\nscope: {scope}\nexport: false\n---\n\n자료 연결 합성 {number}\napi_key: synthetic-secret-{number}\n"
    )
}

#[tokio::test]
async fn vault_import_contract() {
    use meenseek_ontology::vault_importer::{VaultReader, VaultScope, identity};
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let temp = tempfile::tempdir().expect("synthetic Vault only");
    let root = temp
        .path()
        .canonicalize()
        .expect("canonical synthetic root");
    for scope in ["profile", "personal", "work"] {
        std::fs::create_dir_all(root.join(scope).join("projects")).expect("create synthetic scope");
    }
    for i in 0..100 {
        std::fs::write(
            root.join(format!("personal/projects/source-{i}.md")),
            vault_document("personal", i),
        )
        .expect("write synthetic notes");
    }
    std::fs::write(
        root.join("work/projects/source-0.md"),
        vault_document("work", 0),
    )
    .expect("write colliding relative work path");
    std::fs::write(
        root.join("profile/projects/source-0.md"),
        vault_document("profile", 0),
    )
    .expect("write synthetic profile");
    let reader = VaultReader::new(test_vault_binary(), vec![root.clone()])
        .expect("explicit binary and root");
    for size in [0, 1, 100] {
        let paths = (0..size)
            .map(|i| format!("projects/source-{i}.md"))
            .collect::<Vec<_>>();
        let before_calls = reader.calls();
        let before_bytes = reader.response_bytes();
        let before_db = store.calls();
        assert_eq!(
            reader
                .import(&store, &root, VaultScope::Personal, &paths, Scope::Meenseek)
                .await
                .expect("one verified Vault batch"),
            size
        );
        assert_eq!(reader.calls() - before_calls, u64::from(size != 0));
        assert_eq!(store.calls() - before_db, if size == 0 { 0 } else { 3 });
        let bytes = reader.response_bytes() - before_bytes;
        assert!(bytes <= MAX_RESPONSE_BYTES as u64);
        assert!(
            bytes <= (size as u64 * 1024) + 32,
            "fixture transfer grows only with requested documents"
        );
        assert_eq!(bytes == 0, size == 0);
    }
    let path = "projects/source-0.md".to_owned();
    let (_, id) =
        identity(&root, "personal/projects/source-0.md", Scope::Meenseek).expect("Vault identity");
    let first = store
        .detail(Scope::Meenseek, &id)
        .await
        .expect("Vault detail");
    assert_eq!(first["source"]["kind"], "vault");
    assert_eq!(
        first["source"]["repository"],
        root.to_str().expect("UTF-8 fixture root")
    );
    assert_eq!(first["source"]["path"], "personal/projects/source-0.md");
    assert_eq!(
        first["source"]["verified_revision"],
        digest(vault_document("personal", 0).as_bytes())
    );
    assert_eq!(
        first["projection"]["source_revision"],
        first["source"]["verified_revision"]
    );
    let content = first["projection"]["content"]
        .as_str()
        .expect("redacted content");
    assert!(content.contains("[REDACTED]") && !content.contains("synthetic-secret"));
    assert_eq!(
        first["projection"]["content_digest"],
        digest(content.as_bytes())
    );
    assert_ne!(
        first["projection"]["content_digest"],
        first["projection"]["source_revision"]
    );
    assert_eq!(
        std::fs::read_to_string(root.join("personal").join(&path)).expect("unchanged fixture"),
        vault_document("personal", 0)
    );
    let git_record = record(501, Scope::Meenseek);
    store
        .apply_import(std::slice::from_ref(&git_record))
        .await
        .expect("same-scope Git source");
    store
        .classify(
            Scope::Meenseek,
            &id,
            Classification {
                revision: 0,
                areas: vec!["strategy-portfolio".into(), "market-customer".into()],
                topics: vec!["정정 주제".into()],
            },
        )
        .await
        .expect("classify Vault projection");
    store
        .link(
            Scope::Meenseek,
            &id,
            LinkChange {
                revision: 1,
                target_id: git_record.entity_id.clone(),
                remove: false,
            },
        )
        .await
        .expect("connect Git and Vault within app scope");
    let confirmed = store
        .detail(Scope::Meenseek, &id)
        .await
        .expect("confirmed detail");
    reader
        .import(
            &store,
            &root,
            VaultScope::Personal,
            std::slice::from_ref(&path),
            Scope::Meenseek,
        )
        .await
        .expect("idempotent import");
    let reimported = store
        .detail(Scope::Meenseek, &id)
        .await
        .expect("reimported detail");
    for key in ["revision", "areas", "topics", "related", "history"] {
        assert_eq!(reimported[key], confirmed[key], "preserve {key}");
    }
    let changed = vault_document("personal", 0).replace("자료 연결 합성 0", "변경된 합성 원문");
    std::fs::write(root.join("personal").join(&path), &changed).expect("change synthetic note");
    reader
        .import(
            &store,
            &root,
            VaultScope::Personal,
            std::slice::from_ref(&path),
            Scope::Meenseek,
        )
        .await
        .expect("new original digest");
    let updated = store
        .detail(Scope::Meenseek, &id)
        .await
        .expect("updated projection");
    assert_eq!(
        updated["projection"]["source_revision"],
        digest(changed.as_bytes())
    );
    assert_ne!(
        updated["projection"]["source_revision"],
        first["projection"]["source_revision"]
    );
    assert_eq!(updated["history"], confirmed["history"]);
    // Same relative path in another Vault scope is a distinct source in the same app scope.
    reader
        .import(
            &store,
            &root,
            VaultScope::Work,
            std::slice::from_ref(&path),
            Scope::Meenseek,
        )
        .await
        .expect("explicit synthetic work scope");
    let (_, work_id) =
        identity(&root, "work/projects/source-0.md", Scope::Meenseek).expect("work identity");
    assert_ne!(id, work_id);
    assert_eq!(
        store
            .detail(Scope::Meenseek, &work_id)
            .await
            .expect("work source")["source"]["path"],
        "work/projects/source-0.md"
    );
    let (_, git_id) = meenseek_ontology::importer::identity(
        &root,
        "personal/projects/source-0.md",
        Scope::Meenseek,
    )
    .expect("existing Git formula");
    assert_ne!(id, git_id);
    reader
        .import(
            &store,
            &root,
            VaultScope::Profile,
            std::slice::from_ref(&path),
            Scope::Personal,
        )
        .await
        .expect("profile scope is separate from app scope");
    let (_, personal_id) = identity(&root, "profile/projects/source-0.md", Scope::Personal)
        .expect("personal app identity");
    assert_eq!(
        store.detail(Scope::Meenseek, &personal_id).await,
        Err(Error::NotFound)
    );
    let listing = store
        .list(Scope::Meenseek, "source-0.md", false, None)
        .await
        .expect("search both Vault scopes");
    assert!(
        listing["items"]
            .as_array()
            .expect("items")
            .iter()
            .all(|item| item["kind"] == "vault")
    );
    assert!(serde_json::to_vec(&listing).expect("bounded JSON").len() < MAX_RESPONSE_BYTES);
    // Middle-document failure preserves every old projection and confirmation, without retry.
    let second_path = "projects/source-1.md".to_owned();
    let (_, second_id) =
        identity(&root, "personal/projects/source-1.md", Scope::Meenseek).expect("second identity");
    let second_before = store
        .detail(Scope::Meenseek, &second_id)
        .await
        .expect("second projection");
    std::fs::write(
        root.join("personal").join(&path),
        changed.replace("변경된", "미반영"),
    )
    .expect("first staged change");
    std::fs::write(root.join("personal").join(&second_path), b"\xff\xfe")
        .expect("malformed second fixture");
    let before_calls = reader.calls();
    let before_db = store.calls();
    assert!(
        reader
            .import(
                &store,
                &root,
                VaultScope::Personal,
                &[path.clone(), second_path.clone()],
                Scope::Meenseek
            )
            .await
            .is_err()
    );
    assert_eq!(reader.calls() - before_calls, 1);
    assert_eq!(store.calls() - before_db, 1);
    for (entity, previous) in [(&id, &updated), (&second_id, &second_before)] {
        let failed = store
            .detail(Scope::Meenseek, entity)
            .await
            .expect("failure preserves previous success");
        assert_eq!(failed["projection"], previous["projection"]);
        assert_eq!(failed["source"]["status"], "failed");
        assert_eq!(failed["source"]["failure_code"], "vault-read-failed");
        assert_eq!(
            failed["source"]["last_success_at"],
            previous["source"]["last_success_at"]
        );
        assert_eq!(failed["history"], previous["history"]);
    }
    std::fs::remove_file(root.join("personal").join(&path)).expect("simulate missing live source");
    assert!(
        reader
            .import(
                &store,
                &root,
                VaultScope::Personal,
                std::slice::from_ref(&path),
                Scope::Meenseek
            )
            .await
            .is_err()
    );
    let missing = store
        .detail(Scope::Meenseek, &id)
        .await
        .expect("missing live source is failure");
    assert_eq!(missing["projection"], updated["projection"]);
    assert_eq!(missing["projection"]["present"], true);
    assert_eq!(missing["projection"]["absence_revision"], Value::Null);
    std::fs::write(root.join("personal").join(&path), &changed).expect("restore synthetic source");
    std::fs::write(
        root.join("personal").join(&second_path),
        vault_document("personal", 1),
    )
    .expect("restore second fixture");
    // The real provider owns filesystem and document policy; the adapter propagates each failure.
    std::os::unix::fs::symlink("source-0.md", root.join("personal/projects/symlink.md"))
        .expect("synthetic symlink");
    std::fs::hard_link(
        root.join("personal").join(&path),
        root.join("personal/projects/hardlink.md"),
    )
    .expect("synthetic hardlink");
    for file in ["projects/symlink.md", "projects/hardlink.md"] {
        assert!(
            reader
                .read(&root, VaultScope::Personal, &[file.into()], Scope::Meenseek)
                .await
                .is_err()
        );
    }
    std::fs::remove_file(root.join("personal/projects/hardlink.md"))
        .expect("remove fixture hardlink");
    std::fs::write(
        root.join("personal/projects/wrong-scope.md"),
        vault_document("work", 999),
    )
    .expect("scope mismatch fixture");
    std::fs::write(
        root.join("personal/projects/invalid.md"),
        "---\ntitle: [\n---\nbody",
    )
    .expect("invalid YAML fixture");
    std::fs::write(root.join("personal/projects/large.md"), "x".repeat(65_537))
        .expect("oversized fixture");
    for file in [
        "projects/wrong-scope.md",
        "projects/invalid.md",
        "projects/large.md",
        "projects/missing.md",
    ] {
        assert!(
            reader
                .read(&root, VaultScope::Personal, &[file.into()], Scope::Meenseek)
                .await
                .is_err()
        );
    }
    for directory in ["journal", "conversations/raw"] {
        std::fs::create_dir_all(root.join("personal").join(directory))
            .expect("excluded fixture directory");
        std::fs::write(
            root.join("personal").join(directory).join("x.md"),
            vault_document("personal", 999),
        )
        .expect("excluded fixture note");
        assert!(
            reader
                .read(
                    &root,
                    VaultScope::Personal,
                    &[format!("{directory}/x.md")],
                    Scope::Meenseek
                )
                .await
                .is_err()
        );
    }
    let absent_root = root.join("absent-root");
    let missing_root_reader = VaultReader::new(test_vault_binary(), vec![absent_root.clone()])
        .expect("register explicit missing root");
    assert!(
        missing_root_reader
            .read(
                &absent_root,
                VaultScope::Personal,
                std::slice::from_ref(&path),
                Scope::Meenseek
            )
            .await
            .is_err()
    );
    assert!(
        !absent_root.exists(),
        "failed read must not create its root"
    );
    // Exercise the installed CLI surface with the explicit canonical test dependency.
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_meenseek-ontology"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env(
            "DATABASE_URL",
            std::env::var("TEST_DATABASE_URL").expect("validated synthetic DB URL"),
        )
        .env("ONTOLOGY_ALLOWED_VAULT_ROOTS", &root)
        .arg("import-vault")
        .arg("--vault-binary")
        .arg(test_vault_binary())
        .arg("--vault-root")
        .arg(&root)
        .args([
            "--vault-scope",
            "personal",
            "--scope",
            "meenseek",
            "--file",
            &path,
        ])
        .output()
        .await
        .expect("run candidate import-vault CLI");
    assert!(
        output.status.success(),
        "candidate CLI must import the real Vault fixture"
    );
    assert!(
        String::from_utf8(output.stdout)
            .expect("CLI status")
            .contains("1 registered Vault documents")
    );
    let restored = store
        .detail(Scope::Meenseek, &id)
        .await
        .expect("CLI persisted success");
    assert_eq!(restored["source"]["status"], "ok");
    assert_eq!(restored["source"]["failure_code"], Value::Null);
    for key in ["revision", "areas", "topics", "related", "history"] {
        assert_eq!(restored[key], confirmed[key]);
    }
    let reconnected = Store::connect(&std::env::var("TEST_DATABASE_URL").expect("synthetic URL"))
        .await
        .expect("restart app storage");
    reconnected
        .initialize()
        .await
        .expect("migration replay on restart");
    assert_eq!(
        reconnected
            .detail(Scope::Meenseek, &id)
            .await
            .expect("persisted confirmation"),
        restored
    );
}
