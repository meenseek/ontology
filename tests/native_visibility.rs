use ontology::{
    context::{ContextScope, inventory},
    context_importer::ContextReader,
    domain::{Error, ImportedRecord, Scope, SourceKind},
    graph::GraphQuery,
    memory::BrainCommand,
    store::{Store, digest},
};
use serde_json::{Value, json};

async fn graph(store: &Store, scope: Scope, q: &str, focus: Option<&str>) -> Value {
    store
        .graph(GraphQuery {
            scope,
            q: q.into(),
            focus: focus.map(str::to_owned),
            limit: 800,
        })
        .await
        .unwrap()
}
async fn material(store: &Store, path: &str) -> String {
    sqlx::query_scalar(
        "SELECT material_id::text FROM context_materials WHERE scope='personal' AND path=$1",
    )
    .bind(path)
    .fetch_one(store.pool())
    .await
    .unwrap()
}
async fn purpose(store: &Store, scope: &str, identity: &str, write: bool) -> Result<Value, Error> {
    let request = if write {
        json!({"op":"document-subject-set","scope":scope,"document":{
            "identity":{"material_id":identity},"revision":0,"source_revision":"1",
            "content_digest":digest(b"synthetic"),"subject_id":null,"subject_revision":null,"reason":"synthetic eligibility check"}})
    } else {
        json!({"op":"document-subject","scope":scope,"document":{"material_id":identity}})
    };
    store
        .brain(serde_json::from_value::<BrainCommand>(request).unwrap())
        .await
}

async fn purpose_history(store: &Store, scope: &str, identity: &str) -> Result<Value, Error> {
    store.brain(serde_json::from_value::<BrainCommand>(json!({
        "op":"document-subject-history","scope":scope,"document":{"material_id":identity},"limit":10
    })).unwrap()).await
}

async fn purpose_rows(store: &Store) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object('memberships',(SELECT COALESCE(jsonb_agg(to_jsonb(d) ORDER BY scope,source_id),'[]') FROM document_subjects d),'history',(SELECT COALESCE(jsonb_agg(to_jsonb(h) ORDER BY scope,source_id,revision),'[]') FROM document_subject_history h))")
        .fetch_one(store.pool()).await.unwrap()
}

#[tokio::test]
async fn native_projection_owns_graph_visibility_and_binding_never_falls_back() {
    let url = std::env::var("TEST_DATABASE_URL").expect("explicit disposable PostgreSQL");
    assert!(
        ontology::config::database_options(&url)
            .unwrap()
            .get_database()
            .unwrap()
            .starts_with("ontology_test_")
    );
    let store = Store::connect(&url).await.unwrap();
    store.initialize().await.unwrap();
    sqlx::raw_sql("TRUNCATE document_subjects,document_subject_history,subject_history,context_source_bindings,context_projection_versions,context_material_versions,context_manual_edits,context_materials,context_apply_batches,memories,memory_creations,subjects,confirmation_history,related_materials,entity_areas,entity_topics,source_records,entities,sources,topics RESTART IDENTITY CASCADE")
        .execute(store.pool()).await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    for (path, content) in [
        (
            "notes/a.md",
            "# Visible source\n[target](b.md) [excluded](../journal/day.md)\n",
        ),
        ("notes/b.md", "# Visible target\nvisible-target-token\n"),
        (
            "journal/day.md",
            "# Synthetic excluded journal\nexcluded-token\n",
        ),
        (
            "notes/JOURNAL/day.md",
            "# Synthetic nested journal\nexcluded-token\n",
        ),
        ("notes/raw/day.md", "# Synthetic raw\nexcluded-token\n"),
        ("notes/secret.md", "# Synthetic secret\nexcluded-token\n"),
        (
            "notes/unavailable.md",
            "---\n[\n---\n# Invalid metadata remains discoverable\n",
        ),
    ] {
        let destination = root.join("personal").join(path);
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::write(destination, content).unwrap();
    }
    let root = root.canonicalize().unwrap();
    let scopes: Vec<ContextScope> = vec!["personal".parse().unwrap()];
    let manifest = inventory(&root, &scopes).unwrap();
    store
        .import_context(&root, &scopes, &manifest.inventory_digest)
        .await
        .unwrap();
    // Legacy/state-transition fixtures are confined to this disposable DB.
    sqlx::raw_sql("ALTER TABLE context_materials DISABLE TRIGGER USER")
        .execute(store.pool())
        .await
        .unwrap();
    // Reproduce native Core's false flag without reading any real restricted source.
    sqlx::query("UPDATE context_materials SET restricted=false WHERE path IN ('journal/day.md','notes/JOURNAL/day.md','notes/raw/day.md')")
        .execute(store.pool()).await.unwrap();
    let git = ImportedRecord {
        source_id: format!("s_{}", digest(b"synthetic stale Git")),
        entity_id: format!("e_{}", digest(b"synthetic stale Git")),
        scope: Scope::Personal,
        repository: "/synthetic/unbound".into(),
        path: "stale.md".into(),
        kind: SourceKind::Git,
        source_revision: "a".repeat(40),
        digest: None,
        content: None,
    };
    store
        .apply_import(std::slice::from_ref(&git))
        .await
        .unwrap();
    let store_id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
        .fetch_one(store.pool())
        .await
        .unwrap();
    for scope in [Scope::Personal, Scope::Meenseek] {
        ContextReader::new()
            .import(
                &store,
                &store_id,
                &scopes[0],
                &["notes/a.md".into(), "notes/b.md".into()],
                scope,
            )
            .await
            .unwrap();
    }
    let target = material(&store, "notes/b.md").await;
    let source = material(&store, "notes/a.md").await;
    let unavailable = material(&store, "notes/unavailable.md").await;
    for path in [
        "journal/day.md",
        "notes/JOURNAL/day.md",
        "notes/raw/day.md",
        "notes/secret.md",
    ] {
        let id = material(&store, path).await;
        for scope in ["personal", "meenseek"] {
            assert_eq!(
                purpose(&store, scope, &id, false).await,
                Err(Error::NotFound)
            );
            assert_eq!(
                purpose(&store, scope, &id, true).await,
                Err(Error::NotFound)
            );
        }
    }
    let unavailable_source = purpose(&store, "personal", &unavailable, false)
        .await
        .unwrap();
    let unavailable_written = store.brain(serde_json::from_value::<BrainCommand>(json!({
        "op":"document-subject-set","scope":"personal","document":{
            "identity":{"material_id":unavailable},"revision":0,
            "source_revision":unavailable_source["current_source"]["source_revision"],
            "content_digest":unavailable_source["current_source"]["content_digest"],
            "subject_id":null,"subject_revision":null,"reason":"합성 unavailable 원문의 수동 분류"
        }
    })).unwrap()).await.unwrap();
    assert_eq!(unavailable_written["revision"], 1);
    assert_eq!(
        purpose_history(&store, "personal", &unavailable)
            .await
            .unwrap()["items"][0]["revision"],
        1,
        "current eligible unavailable sources allow read, write and history"
    );
    let preserved = purpose_rows(&store).await;
    let mut baseline = Vec::new();
    for scope in [Scope::Personal, Scope::Meenseek] {
        let before = graph(&store, scope, "", None).await;
        assert_eq!(
            before["totals"]["documents"],
            if scope == Scope::Personal { 4 } else { 2 }
        );
        assert!(!before["nodes"].as_array().unwrap().iter().any(|n| {
            n["context_path"].as_str().is_some_and(|p| {
                p.contains("journal")
                    || p.contains("JOURNAL")
                    || p.contains("raw")
                    || p.contains("secret")
            })
        }));
        assert_eq!(
            graph(&store, scope, "excluded-token", None).await["matched"],
            0
        );
        let alias = before["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["material_id"] == target)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let source_alias = before["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["material_id"] == source)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(before["totals"]["links"], 1);
        let source_digest = before["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == source_alias)
            .unwrap()["relation_digest"]
            .clone();
        baseline.push((scope, alias, source_alias, source_digest));
    }
    // Broken derived rows are synthetic fixtures; disable only this disposable table's guards.
    sqlx::raw_sql("ALTER TABLE context_projection_versions DISABLE TRIGGER USER")
        .execute(store.pool())
        .await
        .unwrap();
    let projection: Value = sqlx::query_scalar(
        "DELETE FROM context_projection_versions WHERE material_id=$1::uuid RETURNING payload",
    )
    .bind(&target)
    .fetch_one(store.pool())
    .await
    .unwrap();
    for state in ["missing", "old-digest", "excluded", "restricted", "deleted"] {
        if state != "missing" {
            let mut payload = projection.clone();
            if state == "old-digest" {
                payload["source_digest"] = json!("0".repeat(64));
            }
            if state == "excluded" {
                payload = json!({"status":"excluded","source_digest":projection["source_digest"],"terms":[]});
            }
            sqlx::query("INSERT INTO context_projection_versions(material_id,revision,payload,payload_digest) VALUES($1::uuid,1,$2,encode(sha256(convert_to($2::jsonb::text,'UTF8')),'hex')) ON CONFLICT(material_id,revision) DO UPDATE SET payload=EXCLUDED.payload,payload_digest=EXCLUDED.payload_digest")
                .bind(&target).bind(payload).execute(store.pool()).await.unwrap();
            sqlx::query(
                "UPDATE context_materials SET restricted=$2,deleted=$3,search_text=CASE WHEN $2 OR $3 THEN NULL ELSE search_text END,content=CASE WHEN $3 THEN ''::bytea ELSE content END,byte_len=CASE WHEN $3 THEN 0 ELSE byte_len END,content_digest=CASE WHEN $3 THEN encode(sha256(''::bytea),'hex') ELSE content_digest END WHERE material_id=$1::uuid",
            )
            .bind(&target)
            .bind(state == "restricted")
            .bind(state == "deleted")
            .execute(store.pool())
            .await
            .unwrap();
        }
        for (scope, alias, source_alias, old_digest) in &baseline {
            let scope_name = scope.as_str();
            assert_eq!(
                purpose(&store, scope_name, &target, false).await,
                Err(Error::NotFound),
                "{state} purpose read"
            );
            assert_eq!(
                purpose(&store, scope_name, &target, true).await,
                Err(Error::NotFound),
                "{state} purpose write"
            );
            assert_eq!(
                purpose_history(&store, scope_name, &target).await,
                Err(Error::NotFound),
                "{state} purpose history"
            );
            assert_eq!(
                purpose_rows(&store).await,
                preserved,
                "ineligible APIs preserve every classification and history row"
            );
            let after = graph(&store, *scope, "", None).await;
            assert_eq!(
                after["totals"]["documents"],
                if *scope == Scope::Personal { 3 } else { 1 },
                "{state} native cannot fall back to imported bytes"
            );
            assert_eq!(
                after["totals"]["links"], 0,
                "excluded endpoint is removed before counts"
            );
            assert_eq!(
                graph(&store, *scope, "visible-target-token", None).await["matched"],
                0
            );
            assert_eq!(
                graph(&store, *scope, "", Some(alias)).await["focus"]["found"],
                false
            );
            let digest = &after["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|n| n["id"] == *source_alias)
                .unwrap()["relation_digest"];
            assert_ne!(
                digest, old_digest,
                "excluded edge leaves the relation digest"
            );
        }
    }
    sqlx::raw_sql("ALTER TABLE context_projection_versions ENABLE TRIGGER USER; ALTER TABLE context_materials ENABLE TRIGGER USER")
        .execute(store.pool())
        .await
        .unwrap();
    let personal = graph(&store, Scope::Personal, "", None).await;
    let unavailable_node = personal["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["material_id"] == unavailable)
        .unwrap();
    assert_eq!(unavailable_node["current"], true);
    let stale = personal["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == git.entity_id)
        .unwrap();
    assert_eq!(
        stale["present"], false,
        "unbound missing Git discovery is preserved"
    );
}
