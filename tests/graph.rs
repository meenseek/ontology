use ontology::{
    context::{ContextScope, inventory},
    context_importer::ContextReader,
    domain::{Error, ImportedRecord, MAX_RESPONSE_BYTES, Scope, SourceKind},
    graph::{GraphQuery, MAX_GRAPH_LINKS, MAX_GRAPH_NODES},
    memory::BrainCommand,
    store::{Store, digest},
};
use serde_json::{Value, json};

fn query(scope: Scope, q: &str, focus: Option<&str>, limit: usize) -> GraphQuery {
    GraphQuery {
        scope,
        q: q.into(),
        focus: focus.map(str::to_owned),
        limit,
    }
}
fn record(index: usize, scope: Scope) -> ImportedRecord {
    let hash = digest(format!("graph:{}:{index}", scope.as_str()).as_bytes());
    ImportedRecord {
        source_id: format!("s_{hash}"),
        entity_id: format!("e_{hash}"),
        scope,
        repository: match index {
            0 => "/synthetic/one/shared",
            1 => "/synthetic/two/shared",
            _ => "/synthetic/graph",
        }
        .into(),
        path: match index {
            0 | 1 => "notes/README.md".into(),
            2 => format!("{}actual-filename.md", "archive/".repeat(40)),
            _ => format!("document-{index}.md"),
        },
        kind: SourceKind::Git,
        source_revision: "a".repeat(40),
        digest: Some(digest(b"body only needle")),
        content: Some("body only needle".into()),
    }
}
async fn brain(store: &Store, command: Value) -> Value {
    store
        .brain(serde_json::from_value::<BrainCommand>(command).expect("typed fixture command"))
        .await
        .expect("valid fixture operation")
}
async fn fetch(store: &Store, q: GraphQuery) -> Value {
    let before = store.calls();
    let context = format!(
        "scope={:?}, query={:?}, focus={:?}, limit={}",
        q.scope, q.q, q.focus, q.limit
    );
    let value = store
        .graph(q)
        .await
        .unwrap_or_else(|error| panic!("single graph snapshot ({context}): {error:?}"));
    assert_eq!(
        store.calls() - before,
        1,
        "one DB statement and no retries/fan-out"
    );
    assert!(serde_json::to_vec(&value).expect("serialize graph").len() <= MAX_RESPONSE_BYTES);
    let nodes = value["nodes"].as_array().expect("nodes");
    let links = value["links"].as_array().expect("links");
    assert!(nodes.len() <= MAX_GRAPH_NODES && links.len() <= MAX_GRAPH_LINKS);
    assert_eq!(
        value["returned"]["knowledge"].as_u64().expect("knowledge")
            + value["returned"]["markers"].as_u64().expect("markers"),
        nodes.len() as u64
    );
    assert_eq!(value["returned"]["links"], links.len());
    let totals = &value["totals"];
    assert_eq!(
        totals["documents"].as_u64().expect("documents")
            + totals["memories"].as_u64().expect("memories")
            + totals["markers"].as_u64().expect("markers"),
        nodes.len() as u64 + value["omitted"]["nodes"].as_u64().expect("omitted")
    );
    assert_eq!(
        totals["links"].as_u64().expect("total links"),
        links.len() as u64 + value["omitted"]["links"].as_u64().expect("omitted links")
    );
    assert!(
        nodes
            .iter()
            .all(|n| n.get("content").is_none() && n.get("body").is_none())
    );
    assert!(
        links
            .iter()
            .all(|l| nodes.iter().any(|n| n["id"] == l["source"])
                && nodes.iter().any(|n| n["id"] == l["target"]))
    );
    value
}
fn node<'a>(value: &'a Value, id: &str) -> &'a Value {
    value["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|n| n["id"] == id)
        .expect("requested fixture node")
}

#[tokio::test]
async fn graph_snapshot_contract() {
    let url = std::env::var("TEST_DATABASE_URL").expect("explicit isolated PostgreSQL");
    assert!(
        ontology::config::database_options(&url)
            .expect("loopback test URL")
            .get_database()
            .is_some_and(|n| n.starts_with("ontology_test_"))
    );
    let store = Store::connect(&url)
        .await
        .expect("connect isolated database");
    store
        .initialize()
        .await
        .expect("initialize isolated schema");
    sqlx::raw_sql("TRUNCATE context_source_bindings,context_projection_versions,context_material_versions,context_manual_edits,context_materials,context_apply_batches,memories,memory_creations,subjects,confirmation_history,related_materials,entity_areas,entity_topics,source_records,entities,sources,topics RESTART IDENTITY CASCADE")
        .execute(store.pool()).await.expect("reset only test rows");
    for size in [0, 1, 120, MAX_GRAPH_NODES + 21] {
        for batch in (0..size)
            .map(|i| record(i, Scope::Meenseek))
            .collect::<Vec<_>>()
            .chunks(100)
        {
            store
                .apply_import(batch)
                .await
                .expect("bounded synthetic import");
        }
        let result = fetch(&store, query(Scope::Meenseek, "", None, MAX_GRAPH_NODES)).await;
        assert_eq!(result["totals"]["documents"], size);
        assert_eq!(
            result["nodes"].as_array().expect("nodes").len(),
            size.min(MAX_GRAPH_NODES)
        );
        assert_eq!(result["truncated"], size > MAX_GRAPH_NODES);
        assert_eq!(result["totals"]["memories"], 0);
    }
    let initial = fetch(&store, query(Scope::Meenseek, "", None, MAX_GRAPH_NODES)).await;
    for index in 0..3 {
        let source = record(index, Scope::Meenseek);
        let identity = fetch(
            &store,
            query(Scope::Meenseek, "", Some(&source.entity_id), 1),
        )
        .await;
        let document = node(&identity, &source.entity_id);
        assert_eq!(
            document["label"], source.path,
            "full path survives the graph boundary"
        );
        assert_eq!(
            document["repository"], source.repository,
            "same relative paths retain their local repository"
        );
        if index == 2 {
            assert!(source.path.len() > 160);
            assert!(
                document["label"]
                    .as_str()
                    .expect("document path")
                    .ends_with("actual-filename.md")
            );
        }
    }
    let outside = (0..MAX_GRAPH_NODES + 21)
        .map(|i| record(i, Scope::Meenseek))
        .find(|r| {
            !initial["nodes"]
                .as_array()
                .expect("nodes")
                .iter()
                .any(|n| n["id"] == r.entity_id)
        })
        .expect("fixture exceeds cap");
    let result = fetch(&store, query(Scope::Meenseek, &outside.path, None, 10)).await;
    assert_eq!(node(&result, &outside.entity_id)["kind"], "document");
    let result = fetch(
        &store,
        query(Scope::Meenseek, "", Some(&outside.entity_id), 1),
    )
    .await;
    assert_eq!(result["nodes"][0]["id"], outside.entity_id);
    assert_eq!(result["focus"]["found"], true);
    let result = fetch(&store, query(Scope::Meenseek, "body only needle", None, 5)).await;
    assert_eq!(result["matched"], MAX_GRAPH_NODES + 21);
    for words in [
        "document-3.md needle",
        "needle   document-3.md",
        "DOCUMENT-3.MD\tNEEDLE",
    ] {
        let result = fetch(&store, query(Scope::Meenseek, words, None, 10)).await;
        assert_eq!(result["matched"], 1, "words match across path and body");
        assert_eq!(
            node(&result, &record(3, Scope::Meenseek).entity_id)["search_match"],
            true
        );
    }
    assert_eq!(
        fetch(
            &store,
            query(Scope::Meenseek, "document-3.md absent", None, 10)
        )
        .await["matched"],
        0,
        "every search word must match"
    );
    let personal = record(0, Scope::Personal);
    store
        .apply_import(std::slice::from_ref(&personal))
        .await
        .expect("other scope fixture");
    let result = fetch(
        &store,
        query(Scope::Meenseek, "", Some(&personal.entity_id), 2),
    )
    .await;
    assert_eq!(result["focus"]["found"], false);
    assert!(
        result["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .all(|n| n["scope"] == "meenseek")
    );
    let result = fetch(&store, query(Scope::Personal, "", None, 10)).await;
    assert_eq!(result["totals"]["documents"], 1);
    assert_eq!(result["totals"]["markers"], 0);
    let fixture = tempfile::tempdir().expect("isolated originals");
    let root = fixture
        .path()
        .canonicalize()
        .expect("canonical fixture root");
    let original_support = "---\ncustom: frontmatteronlytoken\n---\n# Support\n지원 현황 marker\n";
    for (path, content) in [
        (
            "personal/decisions/support.md",
            original_support.as_bytes(),
        ),
        (
            "personal/ontology/index.md",
            b"---\ntitle: Index\nontology: true\nrelated: [personal/ontology/schema.md]\n---\n# Index\n",
        ),
        ("personal/ontology/schema.md", b"# Schema\n"),
        ("personal/decisions/second.markdown", b"# Second Markdown\n"),
        ("personal/decisions/THIRD.MD", b"# Third Markdown\n"),
        ("personal/attachments/image.bin", b"\0\xff"),
        ("personal/raw/private.md", b"restricted marker"),
        ("profile/rules/example.md", b"# Profile\n"),
    ] {
        let destination = root.join(path);
        std::fs::create_dir_all(destination.parent().expect("parent")).expect("fixture directory");
        std::fs::write(destination, content).expect("fixture original");
    }
    let scopes: Vec<ContextScope> = ["personal", "profile"]
        .into_iter()
        .map(|value| value.parse().expect("context scope"))
        .collect();
    let expected = inventory(&root, &scopes).expect("fixture inventory");
    store
        .import_context(&root, &scopes, &expected.inventory_digest)
        .await
        .expect("store exact originals");
    let originals = fetch(&store, query(Scope::Personal, "", None, 20)).await;
    assert_eq!(originals["totals"]["documents"], 5);
    assert_eq!(originals["totals"]["links"], 1);
    assert_eq!(
        originals["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .filter(|n| n["source_kind"] == "original")
            .count(),
        4
    );
    assert!(
        !originals["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .any(|n| n["context_path"] == "decisions/second.markdown")
    );
    assert!(
        !originals["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .any(|n| n["context_path"] == "decisions/THIRD.MD")
    );
    assert!(
        !originals["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .any(|n| n["context_path"] == "attachments/image.bin")
    );
    assert_eq!(
        fetch(&store, query(Scope::Personal, "image.bin", None, 10)).await["matched"],
        0
    );
    assert_eq!(
        store
            .context_history(&scopes[0], "attachments/image.bin", None)
            .await
            .expect("attachment remains stored")["items"]
            .as_array()
            .expect("attachment history")
            .len(),
        1
    );
    assert!(
        !originals["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .any(|n| n["context_path"] == "raw/private.md")
    );
    let found = fetch(&store, query(Scope::Personal, "지원 현황", None, 10)).await;
    assert_eq!(found["matched"], 1);
    let support = &found["nodes"][0];
    assert_eq!(support["context_path"], "decisions/support.md");
    assert_eq!(support["source_kind"], "original");
    assert_eq!(support["title"], "Support");
    assert!(support.get("content").is_none());
    let exact = store
        .read_context_material(&scopes[0], "decisions/support.md")
        .await
        .expect("current original");
    assert_eq!(exact.title.as_deref(), Some("Support"));
    assert_eq!(
        originals["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .find(|node| node["context_path"] == "ontology/index.md")
            .expect("ontology original")["title"],
        "Index"
    );
    let separated = fetch(&store, query(Scope::Personal, "support 지원", None, 10)).await;
    assert_eq!(separated["matched"], 1, "path and body words combine");
    assert!(
        separated["nodes"][0]["excerpt"]
            .as_str()
            .unwrap()
            .contains("지원")
    );
    let focused = fetch(
        &store,
        query(Scope::Personal, "", support["id"].as_str(), 1),
    )
    .await;
    assert_eq!(focused["focus"]["found"], true);
    let history = store
        .context_history(&scopes[0], "decisions/support.md", None)
        .await
        .expect("original history");
    assert_eq!(history["items"][0]["revision"], 1);
    let version = store
        .read_context_revision(&scopes[0], "decisions/support.md", 1)
        .await
        .expect("exact historical text");
    assert_eq!(version["content"], original_support);
    assert_eq!(version["title"], "Support");
    assert_eq!(
        store
            .read_context_revision(&scopes[0], "raw/private.md", 1)
            .await,
        Err(Error::Invalid)
    );
    let store_id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
        .fetch_one(store.pool())
        .await
        .expect("canonical store identity");
    ContextReader::new()
        .import(
            &store,
            &store_id,
            &scopes[0],
            &["decisions/support.md".into()],
            Scope::Personal,
        )
        .await
        .expect("bind original to a document");
    let metadata_match = fetch(
        &store,
        query(Scope::Personal, "frontmatteronlytoken", None, 10),
    )
    .await;
    assert_eq!(metadata_match["matched"], 1);
    assert_eq!(
        metadata_match["nodes"][0]["context_path"],
        "decisions/support.md"
    );
    let edited = store
        .edit_context(
            &scopes[0],
            "decisions/support.md",
            1,
            &digest(original_support.as_bytes()),
            "---\ncustom: newfrontmattertoken\n---\n# Support\nfreshlyboundmarker\n",
        )
        .await
        .expect("human edit updates canonical original");
    let fresh = fetch(
        &store,
        query(Scope::Personal, "freshlyboundmarker", None, 10),
    )
    .await;
    assert_eq!(fresh["matched"], 1);
    let bound = &fresh["nodes"][0];
    assert!(bound["created_at"].is_string());
    assert!(bound["content_updated_at"].is_string());
    assert_ne!(
        bound["content_updated_at"],
        node(&focused, focused["focus"]["id"].as_str().unwrap())["content_updated_at"]
    );
    let unchanged = store
        .edit_context(
            &scopes[0],
            "decisions/support.md",
            edited.revision,
            &edited.content_digest,
            "---\ncustom: newfrontmattertoken\n---\n# Support\nfreshlyboundmarker\n",
        )
        .await
        .expect("no-op edit");
    assert!(!unchanged.changed);
    let refreshed = fetch(
        &store,
        query(Scope::Personal, "freshlyboundmarker", None, 10),
    )
    .await;
    assert_eq!(
        refreshed["nodes"][0]["content_updated_at"], bound["content_updated_at"],
        "no-op saves never promote originals"
    );
    assert_eq!(bound["context_path"], "decisions/support.md");
    assert_eq!(bound["revision"], "2");
    assert_eq!(bound["content_digest"], edited.content_digest);
    assert_eq!(bound["status"], "ok");
    assert_eq!(
        bound["current"], true,
        "the available original stays active while its consumer evidence refreshes"
    );
    assert!(
        bound["excerpt"]
            .as_str()
            .expect("current excerpt")
            .contains("freshlyboundmarker")
    );
    assert_eq!(
        fetch(&store, query(Scope::Personal, "지원 현황", None, 10)).await["matched"],
        0,
        "retired original content must not remain searchable through its bound document"
    );
    assert_eq!(
        fetch(
            &store,
            query(Scope::Personal, "frontmatteronlytoken", None, 10)
        )
        .await["matched"],
        0,
        "removed frontmatter must not remain searchable after a manual edit"
    );
    assert_eq!(
        fetch(
            &store,
            query(Scope::Personal, "newfrontmattertoken", None, 10)
        )
        .await["matched"],
        1,
        "new frontmatter remains discoverable after a manual edit"
    );
    let missing = format!("e_{}", "0".repeat(64));
    assert_eq!(
        fetch(&store, query(Scope::Meenseek, "", Some(&missing), 2)).await["focus"]["found"],
        false
    );
    for invalid in [
        query(Scope::Personal, "", Some("a_market-customer"), 2),
        query(Scope::Meenseek, "", Some("m_invalid"), 2),
        query(Scope::Meenseek, "", Some("t_-1"), 2),
        query(Scope::Meenseek, &"x".repeat(121), None, 2),
        query(Scope::Meenseek, "", None, 0),
        query(Scope::Meenseek, "", None, MAX_GRAPH_NODES + 1),
    ] {
        let before = store.calls();
        assert_eq!(store.graph(invalid).await, Err(Error::Invalid));
        assert_eq!(
            store.calls(),
            before,
            "rejected input never reaches PostgreSQL"
        );
    }
    let subject = brain(&store, json!({"op":"subject-create","scope":"meenseek","idempotency_key":"graph-subject","name":"same label"})).await;
    store
        .classify(
            Scope::Meenseek,
            &outside.entity_id,
            ontology::domain::Classification {
                revision: 0,
                topics: vec!["same label".into()],
                areas: vec!["market-customer".into()],
            },
        )
        .await
        .expect("classification fixture");
    let input = json!({"kind":"fact","title":"same label","body":"unique memory body","subject_id":subject["id"],"evidence":[{"entity_id":outside.entity_id,"source_revision":outside.source_revision,"content_digest":outside.digest,"generation":0}]});
    let memory = brain(
        &store,
        json!({"op":"remember","scope":"meenseek","idempotency_key":"graph-memory","memory":input}),
    )
    .await;
    let id = memory["id"].as_str().expect("memory ID");
    let result = fetch(&store, query(Scope::Meenseek, "same label", Some(id), 50)).await;
    for kind in ["memory", "document", "topic", "subject"] {
        assert!(
            result["nodes"]
                .as_array()
                .expect("nodes")
                .iter()
                .any(|n| n["kind"] == kind)
        );
    }
    let topic_and_body = fetch(&store, query(Scope::Meenseek, "same needle", None, 10)).await;
    assert_eq!(
        node(&topic_and_body, &outside.entity_id)["search_match"],
        true
    );
    for kind in ["evidence", "topic", "subject"] {
        assert!(
            result["links"]
                .as_array()
                .expect("links")
                .iter()
                .any(|l| l["kind"] == kind)
        );
    }
    assert!(node(&result, id)["supported"].as_bool().expect("supported"));
    let single = fetch(
        &store,
        query(Scope::Meenseek, "unique memory body", Some(id), 1),
    )
    .await;
    assert_eq!(
        node(&single, id)["relation_digest"],
        node(&result, id)["relation_digest"],
        "response-window links do not change authoritative relation digest"
    );
    let evidence = result["links"]
        .as_array()
        .expect("links")
        .iter()
        .find(|l| l["kind"] == "evidence")
        .expect("evidence link");
    assert_eq!(evidence["current"], memory["evidence"][0]["current"]);
    // Timestamp-only observations never masquerade as content/relation revisions.
    store
        .apply_import(std::slice::from_ref(&outside))
        .await
        .expect("same source rechecked");
    let rechecked = fetch(&store, query(Scope::Meenseek, "same label", Some(id), 50)).await;
    for field in [
        "revision",
        "generation",
        "content_digest",
        "source_revision",
    ] {
        assert_eq!(
            node(&result, &outside.entity_id)[field],
            node(&rechecked, &outside.entity_id)[field]
        );
    }
    store
        .mark_failed(std::slice::from_ref(&outside.source_id), SourceKind::Git)
        .await
        .expect("source failure");
    store
        .apply_import(std::slice::from_ref(&outside))
        .await
        .expect("same source recovery");
    let stale = fetch(&store, query(Scope::Meenseek, "same label", Some(id), 50)).await;
    let current_memory = store
        .memory_detail(Scope::Meenseek, id)
        .await
        .expect("memory freshness reference");
    assert_eq!(node(&stale, id)["supported"], false);
    assert_eq!(
        stale["links"]
            .as_array()
            .expect("links")
            .iter()
            .find(|l| l["kind"] == "evidence")
            .expect("historical link")["current"],
        current_memory["evidence"][0]["current"]
    );
    brain(
        &store,
        json!({"op":"withdraw","scope":"meenseek","id":id,"revision":1}),
    )
    .await;
    assert_eq!(
        node(
            &fetch(&store, query(Scope::Meenseek, "", Some(id), 10)).await,
            id
        )["status"],
        "withdrawn"
    );
    brain(
        &store,
        json!({"op":"forget","scope":"meenseek","id":id,"revision":2}),
    )
    .await;
    let forgotten = fetch(
        &store,
        query(Scope::Meenseek, "unique memory body", Some(id), 10),
    )
    .await;
    assert_eq!(forgotten["focus"]["found"], false);
    assert_eq!(forgotten["matched"], 0);
    assert!(forgotten["nodes"].as_array().expect("nodes").is_empty());
    assert!(forgotten["links"].as_array().expect("links").is_empty());
    for (key, op, from, until, state, status) in [
        (
            "future-graph",
            "remember",
            Some(253402300000_i64),
            None,
            "future",
            "accepted",
        ),
        (
            "expired-graph",
            "remember",
            None,
            Some(1_i64),
            "expired",
            "accepted",
        ),
        (
            "proposed-graph",
            "propose",
            None,
            None,
            "current",
            "proposed",
        ),
    ] {
        let value = brain(&store, json!({"op":op,"scope":"personal","idempotency_key":key,"memory":{"kind":"idea","title":key,"body":"synthetic","effective_from":from,"effective_until":until}})).await;
        let graph = fetch(
            &store,
            query(
                Scope::Personal,
                "",
                Some(value["id"].as_str().expect("ID")),
                10,
            ),
        )
        .await;
        assert_eq!(
            node(&graph, value["id"].as_str().expect("ID"))["temporal"],
            state
        );
        assert_eq!(
            node(&graph, value["id"].as_str().expect("ID"))["status"],
            status
        );
        if op == "propose" {
            let accepted = brain(
                &store,
                json!({"op":"accept","scope":"personal","id":value["id"],"revision":1}),
            )
            .await;
            assert_eq!(accepted["origin"], "assistant");
            let accepted_graph = fetch(
                &store,
                query(
                    Scope::Personal,
                    "",
                    Some(value["id"].as_str().expect("ID")),
                    1,
                ),
            )
            .await;
            assert_eq!(accepted_graph["nodes"][0]["status"], "accepted");
            assert_eq!(
                accepted_graph["nodes"][0]["support"], "user-recorded",
                "wire token describes absent evidence, not authorship"
            );
        }
    }
    // Dense metadata and four-byte labels exercise both count and serialized-byte bounds.
    sqlx::query("UPDATE sources SET path=repeat('🌌',160)||id WHERE scope='meenseek'")
        .execute(store.pool())
        .await
        .expect("large synthetic labels");
    sqlx::raw_sql("INSERT INTO topics(scope,name) VALUES('meenseek','dense one'),('meenseek','dense two'),('meenseek','dense three'); INSERT INTO entity_topics SELECT e.scope,e.id,t.id FROM entities e JOIN topics t ON t.scope=e.scope WHERE e.scope='meenseek' ON CONFLICT DO NOTHING")
        .execute(store.pool()).await.expect("dense synthetic classification");
    sqlx::raw_sql("WITH numbered AS (SELECT id, row_number() OVER (ORDER BY id) AS n FROM entities WHERE scope='meenseek') INSERT INTO related_materials(scope,left_id,right_id) SELECT 'meenseek',a.id,b.id FROM numbered a JOIN numbered b ON b.n>a.n AND b.n<=a.n+3 ON CONFLICT DO NOTHING")
        .execute(store.pool()).await.expect("dense same-scope relations exercise byte cap");
    // Include markers by giving each one focus, beyond the truncated default node window.
    let dense = fetch(
        &store,
        query(Scope::Meenseek, "", Some("t_2"), MAX_GRAPH_NODES),
    )
    .await;
    assert_eq!(dense["truncated"], true);
    assert_eq!(
        dense["limits"]["byte_limited"], true,
        "worst-case bounded labels and links exceed the raw byte budget"
    );
    assert_eq!(
        dense["nodes"][0]["id"], "t_2",
        "byte pruning preserves requested focus"
    );
    assert!(dense["omitted"]["links"].as_u64().expect("omissions") > 0);
    api_contract(&store).await;
    sorting_contract(&store).await;
    store.pool().close().await;
    let before = store.calls();
    assert_eq!(
        store.graph(query(Scope::Meenseek, "", None, 10)).await,
        Err(Error::Storage)
    );
    assert_eq!(store.calls() - before, 1, "storage failure does not retry");
}

async fn api_contract(store: &Store) {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use ontology::{
        api::{AppState, router},
        config::Config,
    };
    use tower::ServiceExt;
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
        .expect("session");
    let cookie = session.headers()["set-cookie"]
        .to_str()
        .expect("cookie")
        .split(';')
        .next()
        .expect("pair")
        .to_owned();
    for (path, auth, status, calls) in [
        ("/api/graph?scope=personal", false, 403, 0),
        ("/api/graph", true, 400, 0),
        ("/api/graph?scope=other", true, 400, 0),
        ("/api/graph?scope=personal&extra=1", true, 400, 0),
        ("/api/graph?scope=personal&focus=invalid", true, 400, 0),
        ("/api/graph?scope=personal&limit=1", true, 200, 1),
        ("/api/graph?scope=meenseek", true, 200, 1),
    ] {
        let mut request = Request::builder()
            .uri(path)
            .header("host", "127.0.0.1:47831");
        if auth {
            request = request.header("cookie", &cookie);
        }
        let before = store.calls();
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).expect("request"))
            .await
            .expect("response");
        assert_eq!(response.status().as_u16(), status);
        assert_eq!(store.calls() - before, calls);
        if status == 200 {
            assert_eq!(response.headers()["cache-control"], "no-store");
            assert!(
                response.headers()["content-security-policy"]
                    .to_str()
                    .expect("CSP")
                    .contains("script-src 'self'")
            );
        }
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("response body")
            .to_bytes();
        assert!(bytes.len() <= MAX_RESPONSE_BYTES);
    }
}

async fn sorting_contract(store: &Store) {
    let mut records = Vec::new();
    for i in 0..3 {
        let record = brain(store, json!({"op":"remember","scope":"personal","idempotency_key":format!("sorting-fixture-{i}"),"memory":{"title":format!("sortingfixture {i}"),"body":"sortingfixture","grouping_preference":"off"}})).await;
        records.push(record);
    }
    records.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    for (i, record) in records.iter().enumerate() {
        let year = if i == 0 {
            "2020-01-01T00:00:00Z"
        } else {
            "2021-01-01T00:00:00Z"
        };
        sqlx::query(
            "UPDATE memory_history SET changed_at=$2::text::timestamptz WHERE memory_id=$1",
        )
        .bind(record["id"].as_str().unwrap())
        .bind(year)
        .execute(store.pool())
        .await
        .unwrap();
        sqlx::query("UPDATE memories SET created_at=$2::text::timestamptz WHERE id=$1")
            .bind(record["id"].as_str().unwrap())
            .bind(year)
            .execute(store.pool())
            .await
            .unwrap();
    }
    let subject = brain(store, json!({"op":"subject-create","scope":"personal","idempotency_key":"sorting-subject-z","name":"sortingfixture 하하"})).await;
    let grouped = brain(store, json!({"op":"grouping-set","scope":"personal","id":records[0]["id"],"revision":1,"subject_id":subject["id"],"mode":"manual"})).await;
    assert_eq!(
        grouped["content_updated_at"], "2020-01-01T00:00:00+00:00",
        "grouping history cannot change the content timestamp"
    );
    let graph = fetch(store, query(Scope::Personal, "sortingfixture", None, 1)).await;
    assert_eq!(
        graph["nodes"][0]["id"], records[0]["id"],
        "search keeps the existing candidate order before the graph limit"
    );
    let newest = brain(store, json!({"op":"remember","scope":"meenseek","idempotency_key":"sorting-graph-latest","memory":{"title":"newest sorting fixture","body":"newest sorting fixture","grouping_preference":"off"}})).await;
    let latest_graph = fetch(store, query(Scope::Meenseek, "", None, 1)).await;
    assert_eq!(
        latest_graph["nodes"][0]["id"], newest["id"],
        "an unfiltered graph limit chooses recent content first"
    );
    let mut expected = vec![
        records[1]["id"].as_str().unwrap(),
        records[2]["id"].as_str().unwrap(),
    ];
    expected.sort_unstable();
    expected.push(records[0]["id"].as_str().unwrap());
    let mut actual = Vec::new();
    let mut after = Value::Null;
    let mut first_page = Value::Null;
    loop {
        let before = store.calls();
        let page = brain(store, json!({"op":"list","scope":"personal","query":"sortingfixture","limit":1,"after":after})).await;
        assert_eq!(store.calls() - before, 1);
        let item = &page["items"][0];
        assert!(item.get("_cursor").is_none());
        actual.push(item["id"].as_str().unwrap().to_owned());
        if first_page.is_null() {
            first_page = page.clone();
        }
        after = page["next_after"].clone();
        if after.is_null() {
            break;
        }
        assert!(actual.len() <= 3, "pagination must advance");
    }
    assert_eq!(actual, expected);
    brain(store, json!({"op":"forget","scope":"personal","id":first_page["items"][0]["id"],"revision":first_page["items"][0]["revision"]})).await;
    let next = brain(store, json!({"op":"list","scope":"personal","query":"sortingfixture","limit":1,"after":first_page["next_after"]})).await;
    assert_eq!(
        next["items"][0]["id"], expected[1],
        "cursor survives deletion of the previous row"
    );
    let withdrawn = brain(store, json!({"op":"withdraw","scope":"personal","id":records[0]["id"],"revision":grouped["revision"]})).await;
    assert_eq!(
        withdrawn["content_updated_at"], grouped["content_updated_at"],
        "status history cannot change content time"
    );
    for key in ["a", "b"] {
        brain(store, json!({"op":"subject-create","scope":"personal","idempotency_key":format!("sorting-subject-{key}"),"name":"sortingfixture 가나다"})).await;
    }
    let first = brain(
        store,
        json!({"op":"subjects","scope":"personal","query":"sortingfixture","limit":1}),
    )
    .await;
    assert_eq!(first["items"][0]["name"], "sortingfixture 가나다");
    brain(
        store,
        json!({"op":"subject-delete","scope":"personal","id":first["items"][0]["id"]}),
    )
    .await;
    let second = brain(store, json!({"op":"subjects","scope":"personal","query":"sortingfixture","limit":1,"after":first["next_after"]})).await;
    assert_eq!(
        second["items"][0]["name"], "sortingfixture 가나다",
        "name tie cursor survives deletion"
    );
    let third = brain(store, json!({"op":"subjects","scope":"personal","query":"sortingfixture","limit":1,"after":second["next_after"]})).await;
    assert_eq!(third["items"][0]["name"], "sortingfixture 하하");
    assert!(third["next_after"].is_null());
    let before = store.calls();
    let invalid = serde_json::from_value::<BrainCommand>(json!({"op":"list","scope":"personal","after":json!({"id":records[0]["id"],"at":i64::MIN}).to_string()})).unwrap();
    assert_eq!(store.brain(invalid).await, Err(Error::Invalid));
    assert_eq!(store.calls(), before, "malformed cursors never reach SQL");
}
