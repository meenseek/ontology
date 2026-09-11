use meenseek_ontology::{
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
    let value = store.graph(q).await.expect("single graph snapshot");
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
        meenseek_ontology::config::database_options(&url)
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
    sqlx::raw_sql("TRUNCATE memories,memory_creations,subjects,confirmation_history,related_materials,entity_areas,entity_topics,source_records,entities,sources,topics RESTART IDENTITY CASCADE")
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
            meenseek_ontology::domain::Classification {
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
    use meenseek_ontology::{
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
