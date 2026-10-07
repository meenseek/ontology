use ontology::{
    context::{ContextScope, inventory},
    context_importer::ContextReader,
    domain::{Error, ImportedRecord, Scope, SourceKind},
    graph::GraphQuery,
    source_references::ImportedReferenceSource,
    store::{Store, digest},
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn record(scope: Scope, repository: &str, path: &str, content: &str) -> ImportedRecord {
    let identity = digest(format!("{scope:?}:{repository}:{path}").as_bytes());
    ImportedRecord {
        source_id: format!("s_{identity}"),
        entity_id: format!("e_{identity}"),
        scope,
        repository: repository.into(),
        path: path.into(),
        kind: SourceKind::Git,
        source_revision: "a".repeat(40),
        digest: Some(digest(content.as_bytes())),
        content: Some(content.into()),
    }
}
fn proof(record: &ImportedRecord) -> ImportedReferenceSource {
    ImportedReferenceSource {
        entity_id: record.entity_id.clone(),
        source_revision: record.source_revision.clone(),
        content_digest: record.digest.clone().unwrap(),
    }
}
async fn references(store: &Store, scope: Scope) -> BTreeSet<(String, String)> {
    let before = store.calls();
    let graph = store
        .graph(GraphQuery {
            scope,
            q: String::new(),
            focus: None,
            limit: 800,
        })
        .await
        .unwrap();
    assert_eq!(
        store.calls() - before,
        1,
        "reference projection shares one PostgreSQL snapshot"
    );
    graph["links"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|l| l["kind"] == "reference")
        .map(|l| {
            (
                l["source"].as_str().unwrap().into(),
                l["target"].as_str().unwrap().into(),
            )
        })
        .collect()
}
async fn preserved(store: &Store, ids: &[String]) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object('records',(SELECT jsonb_agg(to_jsonb(r)-'reference_paths' ORDER BY entity_id) FROM source_records r WHERE entity_id=ANY($1)),'sources',(SELECT jsonb_agg(to_jsonb(s) ORDER BY id) FROM sources s WHERE id IN (SELECT source_id FROM entities WHERE id=ANY($1))),'entities',(SELECT jsonb_agg(to_jsonb(e) ORDER BY id) FROM entities e WHERE id=ANY($1)))")
        .bind(ids).fetch_one(store.pool()).await.unwrap()
}

#[tokio::test]
async fn references_preserve_originals_direction_scope_and_current_source_proofs() {
    let url = std::env::var("TEST_DATABASE_URL").expect("explicit disposable DB");
    assert!(
        ontology::config::database_options(&url)
            .unwrap()
            .get_database()
            .unwrap()
            .starts_with("ontology_test_")
    );
    let store = Store::connect(&url).await.unwrap();
    store.initialize().await.unwrap();
    let a = record(
        Scope::Personal,
        "/synthetic/reference-repo",
        "refs/a.md",
        "---\nontology: deliberately-not-context-metadata\nlink: '[meta](meta.md)'\n---\n[one](b.md#part) [again](b.md)\n![image](c.md) `[code](c.md)`\n",
    );
    let b = record(
        Scope::Personal,
        "/synthetic/reference-repo",
        "refs/b.md",
        "# Target\n",
    );
    let other = record(
        Scope::Personal,
        "/synthetic/other-repo",
        "refs/c.md",
        "# Separate repo\n",
    );
    let company = record(
        Scope::Meenseek,
        "/synthetic/reference-repo",
        "refs/c.md",
        "# Separate scope\n",
    );
    store
        .apply_import(&[a.clone(), b.clone(), other, company])
        .await
        .unwrap();
    assert!(
        references(&store, Scope::Personal)
            .await
            .contains(&(a.entity_id.clone(), b.entity_id.clone()))
    );
    assert!(
        !references(&store, Scope::Personal)
            .await
            .contains(&(b.entity_id.clone(), a.entity_id.clone())),
        "authored direction is not reversed"
    );
    assert!(
        references(&store, Scope::Meenseek).await.is_empty(),
        "repository path alone cannot cross app scope"
    );
    let ids = vec![a.entity_id.clone(), b.entity_id.clone()];
    sqlx::query("UPDATE source_records SET reference_paths='{}' WHERE entity_id=ANY($1)")
        .bind(&ids)
        .execute(store.pool())
        .await
        .unwrap();
    let original = preserved(&store, &ids).await;
    let mut stale = proof(&b);
    stale.source_revision = "b".repeat(40);
    assert_eq!(
        store
            .refresh_imported_references(Scope::Personal, &[proof(&a), stale])
            .await,
        Err(Error::Conflict)
    );
    assert!(
        references(&store, Scope::Personal).await.is_empty(),
        "stale batch changes no earlier record"
    );
    let rebuilt = store
        .refresh_imported_references(Scope::Personal, &[proof(&a), proof(&b)])
        .await
        .unwrap();
    assert_eq!(rebuilt, json!({"reviewed":2,"changed":1}));
    assert_eq!(
        preserved(&store, &ids).await,
        original,
        "metadata refresh preserves all original/source/entity fields"
    );
    assert_eq!(
        store
            .refresh_imported_references(Scope::Personal, &[proof(&a), proof(&b)])
            .await
            .unwrap()["changed"],
        0
    );
    assert_eq!(
        store
            .refresh_imported_references(Scope::Meenseek, &[proof(&a)])
            .await,
        Err(Error::Conflict)
    );
    let mut missing = b.clone();
    missing.content = None;
    missing.digest = None;
    missing.source_revision = "c".repeat(40);
    store.apply_import(&[missing]).await.unwrap();
    assert!(
        references(&store, Scope::Personal).await.is_empty(),
        "a missing target cannot emit a current reference"
    );
    store.apply_import(std::slice::from_ref(&b)).await.unwrap();
    store
        .mark_failed(std::slice::from_ref(&a.source_id), SourceKind::Git)
        .await
        .unwrap();
    assert!(
        references(&store, Scope::Personal).await.is_empty(),
        "failed source references are not current"
    );
    store.apply_import(std::slice::from_ref(&a)).await.unwrap();

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    for dir in [
        "personal/refs",
        "personal/raw",
        "personal/journal",
        "profile/rules",
        "work/common",
    ] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    let body = "# Native source\n[local](b.md#part) [again](b.md) [profile](../../profile/rules/ref.md) [work](../../work/common/ref.md) [private](../raw/private.md) [journal](../journal/day.md)\n";
    for (path, content) in [
        ("personal/refs/a.md", body),
        ("personal/refs/b.md", "# Native target\n"),
        ("profile/rules/ref.md", "# Profile target\n"),
        ("work/common/ref.md", "# Work target\n"),
        ("personal/raw/private.md", "# Private\n"),
        ("personal/journal/day.md", "# Journal\n"),
    ] {
        std::fs::write(root.join(path), content).unwrap();
    }
    let root = root.canonicalize().unwrap();
    let scopes: Vec<ContextScope> = vec![
        "personal".parse().unwrap(),
        "profile".parse().unwrap(),
        "work/common".parse().unwrap(),
    ];
    let inventory = inventory(&root, &scopes).unwrap();
    store
        .import_context(&root, &scopes, &inventory.inventory_digest)
        .await
        .unwrap();
    let native_id = |scope: &str, path: &str| {
        let pool = store.pool().clone();
        let scope = scope.to_owned();
        let path = path.to_owned();
        async move {
            sqlx::query_scalar::<_, String>(
                "SELECT 'c_'||material_id::text FROM context_materials WHERE scope=$1 AND path=$2",
            )
            .bind(scope)
            .bind(path)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let source = native_id("personal", "refs/a.md").await;
    let target = native_id("personal", "refs/b.md").await;
    let profile = native_id("profile", "rules/ref.md").await;
    let work = native_id("work/common", "ref.md").await;
    let refs = references(&store, Scope::Personal).await;
    for expected in [&target, &profile, &work] {
        assert!(refs.contains(&(source.clone(), expected.clone())));
    }
    assert_eq!(
        refs.iter().filter(|(s, _)| s == &source).count(),
        3,
        "repeats deduplicate and restricted originals stay excluded"
    );
    let store_id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
        .fetch_one(store.pool())
        .await
        .unwrap();
    ContextReader::new()
        .import(
            &store,
            &store_id,
            &scopes[0],
            &["refs/a.md".into()],
            Scope::Personal,
        )
        .await
        .unwrap();
    let bound:String=sqlx::query_scalar("SELECT e.id FROM entities e JOIN context_source_bindings b ON b.source_id=e.source_id WHERE b.material_id=$1::uuid AND e.scope='personal'").bind(source.strip_prefix("c_")).fetch_one(store.pool()).await.unwrap();
    let refs = references(&store, Scope::Personal).await;
    assert!(
        refs.contains(&(bound.clone(), target.clone())) && !refs.iter().any(|(s, _)| s == &source),
        "binding aliases retain canonical authored references once"
    );
    let before = store
        .graph(GraphQuery {
            scope: Scope::Meenseek,
            q: String::new(),
            focus: None,
            limit: 800,
        })
        .await
        .unwrap();
    assert!(
        !before["links"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["kind"] == "reference")
    );
    ContextReader::new()
        .import(
            &store,
            &store_id,
            &scopes[0],
            &["refs/a.md".into()],
            Scope::Meenseek,
        )
        .await
        .unwrap();
    assert!(
        references(&store, Scope::Meenseek).await.is_empty(),
        "hidden targets cannot escape through bound native source"
    );
    ContextReader::new()
        .import(
            &store,
            &store_id,
            &scopes[2],
            &["ref.md".into()],
            Scope::Meenseek,
        )
        .await
        .unwrap();
    assert_eq!(
        references(&store, Scope::Meenseek).await.len(),
        1,
        "only the two explicitly visible bound native ends connect"
    );
    store
        .edit_context(
            &scopes[0],
            "refs/a.md",
            1,
            &digest(body.as_bytes()),
            "# Native source\nNo authored links now.\n",
        )
        .await
        .unwrap();
    assert!(
        !references(&store, Scope::Personal)
            .await
            .iter()
            .any(|(s, _)| s == &bound),
        "current revision replaces obsolete source references"
    );
    assert!(references(&store, Scope::Meenseek).await.is_empty());
    assert_eq!(
        store
            .read_context(&scopes[1], "rules/ref.md", false)
            .await
            .unwrap(),
        "# Profile target\n"
    );
}
