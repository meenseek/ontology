//! Consumer proofs use real PostgreSQL and the retained actual Core fixture boundary.
use context_core::harness::ContextSource;
use ontology::{
    context::{ContextScope, inventory},
    context_importer::{
        ContextReader, identity, revision_token, validate_paths, validate_store_id,
    },
    domain::{Error, Scope, SourceKind},
    memory::BrainCommand,
    store::{Store, digest},
    sync::{SyncConfig, refresh},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};
#[allow(dead_code)]
#[path = "native_harness.rs"]
mod harness_fixture;
static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn cs(s: &str) -> ContextScope {
    s.parse().expect("synthetic exact scope")
}
async fn store() -> Store {
    let url = std::env::var("TEST_DATABASE_URL").expect("isolated test DB");
    assert!(
        ontology::config::database_options(&url)
            .expect("loopback")
            .get_database()
            .is_some_and(|n| n.starts_with("ontology_test_"))
    );
    let s = Store::connect(&url).await.expect("owned PostgreSQL");
    s.initialize().await.expect("append migrations");
    sqlx::query("TRUNCATE memory_grouping,context_source_bindings,context_projection_versions,context_material_versions,context_manual_edits,context_materials,context_apply_batches,evidence_snapshots,evidence_contents,memory_history,memories,memory_creations,subjects,curation_reviews,confirmation_history,related_materials,entity_areas,entity_topics,source_records,entities,sources,topics RESTART IDENTITY").execute(s.pool()).await.expect("owned fixture reset");
    s
}
async fn store_id(s: &Store) -> String {
    sqlx::query_scalar("SELECT store_id::text FROM context_store")
        .fetch_one(s.pool())
        .await
        .expect("identity")
}
fn write(root: &Path, path: &str, body: &str) {
    let p = root.join(path);
    fs::create_dir_all(p.parent().expect("parent")).expect("directory");
    fs::write(p, body).expect("fixture bytes");
}
async fn seed(
    s: &Store,
    files: &[(String, String)],
    scopes: &[ContextScope],
) -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("owned originals");
    let root = temp.path().canonicalize().expect("root");
    for (path, body) in files {
        write(&root, path, body);
    }
    let inv = inventory(&root, scopes).expect("inventory");
    s.import_context(&root, scopes, &inv.inventory_digest)
        .await
        .expect("original import");
    (temp, root)
}
fn cmd(v: Value) -> BrainCommand {
    serde_json::from_value(v).expect("strict command")
}
async fn brain(s: &Store, v: Value) -> Value {
    s.brain(cmd(v)).await.expect("brain operation")
}
async fn source_evidence(s: &Store, id: &str) -> Value {
    let options = brain(s, json!({"op":"evidence","scope":"personal"})).await;
    let e = options["items"]
        .as_array()
        .expect("options")
        .iter()
        .find(|e| e["entity_id"] == id)
        .expect("current source");
    json!({"entity_id":e["entity_id"],"source_revision":e["source_revision"],"content_digest":e["content_digest"],"generation":e["generation"]})
}
async fn source_generation(s: &Store, scope: Scope, entity: &str) -> i64 {
    sqlx::query_scalar("SELECT s.generation FROM sources s JOIN entities e ON e.scope=s.scope AND e.source_id=s.id WHERE e.scope=$1 AND e.id=$2")
        .bind(scope.as_str()).bind(entity).fetch_one(s.pool()).await.expect("numeric persisted source generation")
}
async fn native_prepare_many(
    s: &Store,
    root: &Path,
    run: &str,
    mut paths: Vec<String>,
    body: &str,
    mut deletes: Vec<String>,
) -> context_core::harness::PreparedHarnessRun {
    let policy:String=sqlx::query_scalar("SELECT content_digest FROM context_materials WHERE scope='profile' AND path='rules/agent-harness.md'").fetch_one(s.pool()).await.expect("bound policy");
    paths.sort();
    deletes.sort();
    assert!(
        deletes.iter().all(|p| paths.contains(p)),
        "every deletion is an issued target"
    );
    eprintln!("consumer phase: prepare {run} targets={paths:?} deletes={deletes:?}");
    let prepared = harness_fixture::prepare(
        s,
        root.to_owned(),
        root.to_owned(),
        harness_fixture::envelope(&policy, paths, deletes, false),
        run.into(),
        body.into(),
    )
    .await;
    eprintln!("consumer phase: prepared {run}");
    prepared
}
async fn native_prepare(
    s: &Store,
    root: &Path,
    run: &str,
    path: &str,
    body: &str,
    deletes: Vec<String>,
) -> context_core::harness::PreparedHarnessRun {
    native_prepare_many(s, root, run, vec![path.into()], body, deletes).await
}
async fn native_write(s: &Store, root: &Path, run: &str, path: &str, body: &str) {
    let prepared = native_prepare(s, root, run, path, body, vec![]).await;
    eprintln!("consumer phase: apply {run}");
    consumer_apply(s, root.to_owned(), root.to_owned(), run.into(), prepared).await;
    eprintln!("consumer phase: applied {run}");
}
fn strict_boundaries() {
    // Applicable assertions transferred from vault_importer::tests::strict_response_and_request_boundaries.
    // The old transport JSON/process parser is retired; A owns exact SQL/metadata/body validation.
    for bad in ["", "all", "work", "Work/acme", "work/../acme"] {
        assert!(bad.parse::<ContextScope>().is_err());
    }
    for bad in ["", "A0000000-0000-0000-0000-000000000001", "not-a-uuid"] {
        assert!(validate_store_id(bad).is_err());
    }
    for bad in [
        "../note.md",
        "/note.md",
        "a//note.md",
        "a/./note.md",
        "note.txt",
        "journal/note.md",
        "raw/note.md",
        ".hidden/note.md",
        "secrets.md",
        "credentials.md",
        "a\\note.md",
        "a\0.md",
    ] {
        assert!(
            validate_paths(&cs("personal"), &[bad.into()], true).is_err(),
            "{bad}"
        );
    }
    assert!(validate_paths(&cs("personal"), &[], true).is_err());
    assert!(validate_paths(&cs("personal"), &["note.md".into(), "note.md".into()], true).is_err());
    assert!(
        validate_paths(
            &cs("personal"),
            &(0..101).map(|i| format!("{i}.md")).collect::<Vec<_>>(),
            true
        )
        .is_err()
    );
    let id = "00000000-0000-0000-0000-000000000001";
    for sha in ["a".repeat(63), "A".repeat(64), "g".repeat(64)] {
        assert!(revision_token(id, id, 2, false, &sha, "native", None).is_err());
    }
    assert!(revision_token(id, id, 0, false, &"a".repeat(64), "native", None).is_err());
    assert!(
        revision_token(
            id,
            id,
            1,
            false,
            &"a".repeat(64),
            "imported-file",
            Some(&"b".repeat(64))
        )
        .is_err()
    );
}
fn measured(
    s: &Store,
    dependency: &str,
    success: bool,
    max_calls: u64,
    max_input: usize,
    max_returned: usize,
) -> ontology::store::DependencyObservation {
    let value = s
        .dependency_observations()
        .into_iter()
        .rev()
        .find(|o| o.dependency == dependency && o.succeeded == success && o.store_call_delta > 0)
        .expect("executed dependency observation");
    assert!(value.store_call_delta <= max_calls, "{value:?}");
    assert!(value.input_bytes <= max_input, "{value:?}");
    assert!(value.returned_bytes <= max_returned, "{value:?}");
    assert!(value.started < value.finished);
    assert_eq!(s.dependency_observations_dropped(), 0);
    let mut children = s
        .dependency_observations()
        .into_iter()
        .filter(|o| value.started < o.started && o.finished < value.finished)
        .collect::<Vec<_>>();
    children.sort_by_key(|o| o.started);
    let unique = children
        .iter()
        .map(|o| (o.dependency, &o.request_digest))
        .collect::<std::collections::BTreeSet<_>>();
    let duplicate_requests = children.len() - unique.len();
    assert_eq!(
        duplicate_requests, 0,
        "changed helper requests do not retry"
    );
    // Nested spans are distinct helper boundaries; siblings complete serially.
    for pair in children.windows(2) {
        assert!(pair[0].finished < pair[1].started || pair[1].finished < pair[0].finished);
    }
    eprintln!(
        "consumer measured dependency={dependency} success={success} calls={} sequential_depth={} duplicate_helper_requests={duplicate_requests} retries={duplicate_requests} input_bytes={} returned_bytes={} returned_format={}",
        value.store_call_delta,
        value.store_call_delta,
        value.input_bytes,
        value.returned_bytes,
        value.returned_format
    );
    value
}
fn report_path(s: &Store, from: usize, n: usize, success: bool) {
    assert_eq!(
        s.dependency_observations_dropped(),
        0,
        "complete observation window"
    );
    let all = s.dependency_observations();
    let entries = &all[from..];
    let mut seen = std::collections::BTreeSet::new();
    let mut duplicate = 0;
    for e in entries {
        if !seen.insert((e.dependency, e.request_digest.as_str())) {
            duplicate += 1;
        }
    }
    let root = entries
        .iter()
        .find(|e| e.dependency == "context-import")
        .expect("completed root path");
    assert_eq!(root.succeeded, success);
    // All explicit calls are awaited sequentially. A exact-documents contributes three
    // SQL calls (metadata, store identity, one body); its metadata/identity repeat values
    // already checked by the consumer. These two source-derived duplicates are explicit.
    let duplicate_sql = if entries
        .iter()
        .any(|e| e.dependency == "A-exact-documents" && e.store_call_delta == 3)
    {
        2
    } else {
        0
    };
    assert!(duplicate <= 1);
    assert!(duplicate_sql <= 2);
    assert!(root.store_call_delta <= if success { 15 } else { 17 });
    let retries = [
        "consumer-store-identity",
        "A-exact-metadata",
        "consumer-imported-roots",
        "A-exact-documents",
        "apply_import_in",
        "consumer-bindings",
    ]
    .iter()
    .map(|name| {
        entries
            .iter()
            .filter(|e| e.dependency == *name)
            .count()
            .saturating_sub(1)
    })
    .sum::<usize>();
    assert_eq!(retries, 0, "observed attempts have no automatic retry");
    let mut leaves = entries
        .iter()
        .filter(|e| !matches!(e.dependency, "context-import" | "context-read"))
        .collect::<Vec<_>>();
    leaves.sort_by_key(|e| e.started);
    assert!(
        leaves.windows(2).all(|p| p[0].finished < p[1].started),
        "sequential dependency completion"
    );
    println!(
        "consumer dependency path size={n} success={success} calls={} sequential_depth={} duplicate_helper_requests={duplicate} source_derived_duplicate_sql={duplicate_sql} application_retries={retries} helper_input_bytes={} helper_returned_bytes={}",
        root.store_call_delta,
        root.store_call_delta,
        entries.iter().map(|e| e.input_bytes).sum::<usize>(),
        entries.iter().map(|e| e.returned_bytes).sum::<usize>()
    );
}
#[tokio::test]
async fn context_consumer_identity_and_projection() {
    let _guard = TEST_LOCK.lock().await;
    strict_boundaries();
    let s = store().await;
    let id = store_id(&s).await;
    for n in [1usize, 100] {
        let files: Vec<_> = (0..n)
            .map(|i| {
                (
                    format!("personal/n{n}-{i}.md"),
                    format!(
                        "---\ntitle: Title {i}\nscope: personal\nexport: false\n---\n\nBody {i}\n"
                    ),
                )
            })
            .collect();
        let (_temp, root) = seed(&s, &files, &[cs("personal")]).await;
        let paths: Vec<_> = (0..n).map(|i| format!("n{n}-{i}.md")).collect();
        let mut reader = ContextReader::new();
        let observed_from = s.dependency_observations().len();
        let before = s.calls();
        assert_eq!(
            reader
                .import(&s, &id, &cs("personal"), &paths, Scope::Personal)
                .await
                .expect("bounded consumer"),
            n
        );
        let calls = s.calls() - before;
        report_path(&s, observed_from, n, true);
        measured(&s, "A-exact-metadata", true, 1, 65536, n * 2048);
        measured(&s, "A-exact-documents", true, 3, 65536, 1048576);
        measured(&s, "consumer-imported-roots", true, 1, 65536, n * 8192);
        measured(&s, "consumer-store-identity", true, 1, 8, 64);
        measured(&s, "consumer-bindings", true, 1, n * 256, n * 128);
        measured(&s, "apply_import_in", true, 1, 1048576, 4);
        let listing = s
            .list(Scope::Personal, &format!("n{n}-"), false, None)
            .await
            .expect("bounded list");
        assert_eq!(listing["items"].as_array().expect("list").len(), n);
        measured(&s, "source-list", true, 1, 1024, 1048576);
        let graph = s
            .graph(
                serde_json::from_value(json!({"scope":"personal","q":format!("n{n}-"),"limit":n}))
                    .expect("bounded graph query"),
            )
            .await
            .expect("graph");
        assert_eq!(graph["nodes"].as_array().expect("graph").len(), n);
        measured(&s, "consumer-graph", true, 1, 1024, 1048576);

        assert_eq!(
            calls, 15,
            "begin/import gate/context gate/pending/savepoint/store/meta/root/A meta/store/body/publish/bind/release/commit count"
        );
        assert_eq!(reader.body_calls, 1);
        assert!(reader.response_bytes <= (n * 2048 + 4096) as u64);
        for (path, body) in &files {
            let (source, entity) = identity(
                Scope::Personal,
                SourceKind::Vault,
                root.to_str().expect("root"),
                path,
            );
            let detail = s
                .detail(Scope::Personal, &entity)
                .await
                .expect("preserved legacy identity");
            assert_eq!(detail["source"]["kind"], "vault");
            assert_eq!(detail["source"]["repository"], root.to_str().expect("root"));
            assert_eq!(
                detail["source"]["verified_revision"],
                digest(body.as_bytes())
            );
            assert_eq!(detail["current"], true);
            assert_eq!(sqlx::query_scalar::<_,String>("SELECT material_id::text FROM context_source_bindings WHERE source_id=$1").bind(&source).fetch_one(s.pool()).await.expect("bound material"),sqlx::query_scalar::<_,String>("SELECT material_id::text FROM context_materials WHERE source_root=$1 AND source_path=$2").bind(root.to_str().expect("root")).bind(path).fetch_one(s.pool()).await.expect("canonical identity"));
        }
        // Both one reference and the ten-reference memory boundary acquire ordered
        // source rows once; acceptance with no replacement input rechecks stored evidence.
        let options = brain(
            &s,
            json!({"op":"evidence","scope":"personal","query":format!("n{n}-")}),
        )
        .await;
        let evidence:Vec<_>=options["items"].as_array().expect("options").iter().take(n.min(10)).map(|e|json!({"entity_id":e["entity_id"],"source_revision":e["source_revision"],"content_digest":e["content_digest"],"generation":e["generation"]})).collect();
        let proposal=brain(&s,json!({"op":"propose","scope":"personal","idempotency_key":format!("measured-{n}"),"memory":{"body":"Measured evidence","evidence":evidence}})).await;
        measured(&s, "memory-source-rows", true, 1, 8192, 10 * 131072);
        measured(&s, "prepare-memory", true, 3, 32768, 32768);
        brain(
            &s,
            json!({"op":"accept","scope":"personal","id":proposal["id"],"revision":1}),
        )
        .await;
        measured(&s, "memory-evidence-revalidate", true, 2, 32768, 4);
        measured(&s, "accept-source-locks", true, 1, 8192, 10 * 128);
        measured(&s, "change-memory", true, 10, 32768, 32768);
        let (_, entity) = identity(
            Scope::Personal,
            SourceKind::Vault,
            root.to_str().expect("root"),
            &files[0].0,
        );
        s.classify(
            Scope::Personal,
            &entity,
            ontology::domain::Classification {
                revision: 0,
                areas: vec![],
                topics: vec![format!("Preserved {n}")],
            },
        )
        .await
        .expect("legacy confirmation");
        if n == 100 {
            let (_, other) = identity(
                Scope::Personal,
                SourceKind::Vault,
                root.to_str().expect("root"),
                &files[1].0,
            );
            s.link(
                Scope::Personal,
                &entity,
                ontology::domain::LinkChange {
                    revision: 1,
                    target_id: other,
                    remove: false,
                },
            )
            .await
            .expect("legacy link");
        }
        let failure_proposal=brain(&s,json!({"op":"propose","scope":"personal","idempotency_key":format!("failure-{n}"),"memory":{"body":"Retain rejected proposal","evidence":evidence}})).await;
        let source_ids = options["items"]
            .as_array()
            .expect("options")
            .iter()
            .take(n.min(10))
            .map(|e| e["source_id"].as_str().expect("source").to_owned())
            .collect::<Vec<_>>();
        s.mark_failed(&source_ids, SourceKind::Vault)
            .await
            .expect("explicit failure fixture");
        assert_eq!(s.brain(cmd(json!({"op":"propose","scope":"personal","idempotency_key":format!("failed-prepare-{n}"),"memory":{"body":"Cannot use failed sources","evidence":evidence}}))).await, Err(Error::Conflict));
        measured(&s, "prepare-memory", false, 1, 32768, 0);
        let history = brain(
            &s,
            json!({"op":"history","scope":"personal","id":failure_proposal["id"]}),
        )
        .await;
        assert_eq!(
            s.brain(cmd(
                json!({"op":"accept","scope":"personal","id":failure_proposal["id"],"revision":1})
            ))
            .await,
            Err(Error::Conflict)
        );
        measured(&s, "memory-evidence-revalidate", false, 2, 32768, 0);
        measured(&s, "change-memory", false, 5, 32768, 0);
        assert_eq!(
            brain(
                &s,
                json!({"op":"history","scope":"personal","id":failure_proposal["id"]})
            )
            .await,
            history
        );
        reader
            .import(&s, &id, &cs("personal"), &paths, Scope::Personal)
            .await
            .expect("recover exact source");
        let before = s.detail(Scope::Personal, &entity).await.expect("before");
        let generation_before = source_generation(&s, Scope::Personal, &entity).await;
        fs::remove_dir_all(&root).expect("retire original path");
        reader
            .import(&s, &id, &cs("personal"), &paths, Scope::Personal)
            .await
            .expect("no filesystem dependency");
        let after = s.detail(Scope::Personal, &entity).await.expect("after");
        assert_eq!(
            before["projection"]["content"],
            after["projection"]["content"]
        );
        for key in [
            "revision", "areas", "topics", "related", "history", "current",
        ] {
            assert_eq!(before[key], after[key], "preserve {key}");
        }
        assert_eq!(
            source_generation(&s, Scope::Personal, &entity).await,
            generation_before
        );
        let before = s.calls();
        assert_eq!(
            reader
                .import(&s, &id, &cs("personal"), &[], Scope::Personal)
                .await
                .expect("internal empty"),
            0
        );
        assert_eq!(s.calls(), before);
    }
    let files = vec![
        (
            "work/acme/company.md".into(),
            "---\ntitle: Company\nscope: work\n---\nCompany bytes".into(),
        ),
        (
            "personal/bad.md".into(),
            "---\ntitle: Bad\nscope: profile\n---\nWrong scope".into(),
        ),
        (
            "personal/redaction.md".into(),
            "---\ntitle: Redacted\n---\npassword: fake-credential\nPublic".into(),
        ),
    ];
    let (_temp, _root) = seed(&s, &files, &[cs("personal"), cs("work/acme")]).await;
    let mut reader = ContextReader::new();
    assert!(
        reader
            .import(
                &s,
                &id,
                &cs("work/acme"),
                &["company.md".into()],
                Scope::Meenseek
            )
            .await
            .is_ok()
    );
    assert!(
        reader
            .import(
                &s,
                &id,
                &cs("personal"),
                &["bad.md".into()],
                Scope::Personal
            )
            .await
            .is_err()
    );
    reader
        .import(
            &s,
            &id,
            &cs("personal"),
            &["redaction.md".into()],
            Scope::Personal,
        )
        .await
        .expect("Core redaction");
    let redacted = s
        .list(Scope::Personal, "redaction.md", false, None)
        .await
        .expect("redacted listing");
    let redacted = s
        .detail(
            Scope::Personal,
            redacted["items"][0]["id"].as_str().expect("id"),
        )
        .await
        .expect("redacted projection");
    let projected = redacted["projection"]["content"].as_str().expect("body");
    assert!(projected.contains("[REDACTED]"));
    assert!(!projected.contains("fake-credential"));
    assert_eq!(
        redacted["projection"]["content_digest"],
        digest(projected.as_bytes())
    );
    boundary_reads(&s, &id).await;
    let before = s.calls();
    assert!(
        reader
            .import(
                &s,
                &id,
                &cs("personal"),
                &["journal/note.md".into()],
                Scope::Personal
            )
            .await
            .is_err()
    );
    assert_eq!(s.calls(), before);
    let files = vec![(
        format!(
            "personal/{}/{}/{}.md",
            "a".repeat(200),
            "b".repeat(200),
            "c".repeat(98)
        ),
        "---\ntitle: Boundary\n---\nBody".into(),
    )];
    let (_temp, _root) = seed(&s, &files, &[cs("personal")]).await;
    assert!(
        reader
            .import(
                &s,
                &id,
                &cs("personal"),
                &[files[0].0[9..].into()],
                Scope::Personal
            )
            .await
            .is_ok()
    );
    let snapshot = "SELECT jsonb_build_object('memories',(SELECT jsonb_agg(to_jsonb(m) ORDER BY id) FROM memories m),'memory_history',(SELECT jsonb_agg(to_jsonb(h) ORDER BY memory_id,revision) FROM memory_history h),'snapshots',(SELECT jsonb_agg(to_jsonb(e) ORDER BY memory_id,revision,entity_id) FROM evidence_snapshots e),'contents',(SELECT jsonb_agg(to_jsonb(c) ORDER BY scope,digest) FROM evidence_contents c),'classifications',(SELECT jsonb_agg(to_jsonb(t) ORDER BY entity_id,topic_id) FROM entity_topics t),'links',(SELECT jsonb_agg(to_jsonb(r) ORDER BY left_id,right_id) FROM related_materials r),'sources',(SELECT jsonb_agg(to_jsonb(s) ORDER BY id) FROM sources s),'entities',(SELECT jsonb_agg(to_jsonb(e) ORDER BY id) FROM entities e),'records',(SELECT jsonb_agg(to_jsonb(p) ORDER BY entity_id) FROM source_records p),'history',(SELECT jsonb_agg(to_jsonb(h) ORDER BY id) FROM confirmation_history h))";
    let previous: Value = sqlx::query_scalar(snapshot)
        .fetch_one(s.pool())
        .await
        .expect("all existing app values");
    sqlx::raw_sql("DROP TABLE memory_grouping; DELETE FROM ontology_migrations WHERE name='011-personal-memory-grouping.sql'; DELETE FROM ontology_migrations WHERE name IN ('009-context-manual-edits.sql','010-profile-manual-edits.sql'); ALTER TABLE context_materials DROP COLUMN last_manual_edit_id; ALTER TABLE context_material_versions DROP COLUMN manual_edit_id; DROP TABLE context_manual_edits; DROP FUNCTION context_manual_edit_complete(); DROP TRIGGER context_invalidate_consumers ON context_materials; DROP TABLE context_source_bindings; DROP FUNCTION context_invalidate_consumers(); DROP FUNCTION context_source_revision(uuid,uuid,bigint,boolean,text,text,text); ALTER TABLE sources DROP CONSTRAINT sources_kind_check; ALTER TABLE sources ADD CONSTRAINT sources_kind_check CHECK(kind IN ('git','vault')); ALTER TABLE sources DROP CONSTRAINT sources_failure_code_check; ALTER TABLE sources ADD CONSTRAINT sources_failure_code_check CHECK(failure_code IS NULL OR (kind='git' AND failure_code='git-read-failed') OR (kind='vault' AND failure_code IN ('vault-read-failed','context-read-failed'))); ALTER TABLE sources ADD CONSTRAINT sources_vault_status_check CHECK(kind<>'vault' OR status<>'missing'); DELETE FROM ontology_migrations WHERE name='008-context-consumers.sql'; ").execute(s.pool()).await.expect("owned exact pre-008 shape");
    let unmatched: (String, String) =
        sqlx::query_as("SELECT id,path FROM sources WHERE kind='vault' ORDER BY id LIMIT 1")
            .fetch_one(s.pool())
            .await
            .expect("legacy source");
    sqlx::query("UPDATE sources SET path='personal/unmatched-migration.md' WHERE id=$1")
        .bind(&unmatched.0)
        .execute(s.pool())
        .await
        .expect("owned unmatched legacy fixture");
    assert_eq!(s.initialize().await, Err(Error::Baseline));
    assert_eq!(
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT to_regclass('context_source_bindings')::text"
        )
        .fetch_one(s.pool())
        .await
        .expect("rollback schema"),
        None
    );
    sqlx::query("UPDATE sources SET path=$2 WHERE id=$1")
        .bind(&unmatched.0)
        .bind(&unmatched.1)
        .execute(s.pool())
        .await
        .expect("restore exact legacy path");
    sqlx::query("CREATE TABLE context_source_bindings(failure_fixture boolean)")
        .execute(s.pool())
        .await
        .expect("injected migration collision");
    assert_eq!(s.initialize().await, Err(Error::Baseline));
    assert_eq!(
        sqlx::query_scalar::<_, Value>(snapshot)
            .fetch_one(s.pool())
            .await
            .expect("rollback preserves rows"),
        previous
    );
    sqlx::query("DROP TABLE context_source_bindings")
        .execute(s.pool())
        .await
        .expect("remove injected collision");
    s.initialize().await.expect("actual 007 to008 upgrade");
    s.initialize().await.expect("repeat initialize");
    assert_eq!(
        sqlx::query_scalar::<_, Value>(snapshot)
            .fetch_one(s.pool())
            .await
            .expect("preserved rows"),
        previous
    );
    let migration_sha =
        digest(include_str!("../schema/migrations/008-context-consumers.sql").as_bytes());
    sqlx::query("UPDATE ontology_migrations SET digest=$1 WHERE name='008-context-consumers.sql'")
        .bind("a".repeat(64))
        .execute(s.pool())
        .await
        .expect("tampered marker fixture");
    assert_eq!(s.initialize().await, Err(Error::Baseline));
    sqlx::query("UPDATE ontology_migrations SET digest=$1 WHERE name='008-context-consumers.sql'")
        .bind(&migration_sha)
        .execute(s.pool())
        .await
        .expect("restore exact marker");
    sqlx::query("DELETE FROM ontology_migrations WHERE name='008-context-consumers.sql'")
        .execute(s.pool())
        .await
        .expect("missing marker fixture");
    assert_eq!(s.initialize().await, Err(Error::Baseline));
    sqlx::query(
        "INSERT INTO ontology_migrations(name,digest) VALUES('008-context-consumers.sql',$1)",
    )
    .bind(migration_sha)
    .execute(s.pool())
    .await
    .expect("restore actual test marker");
}
#[tokio::test]
async fn context_consumer_revision_and_evidence() {
    let _guard = TEST_LOCK.lock().await;
    let s = store().await;
    harness_fixture::seed(&s).await;
    let view = harness_fixture::view();
    let root = view.path().canonicalize().expect("view");
    let id = store_id(&s).await;
    let path = "vault/personal/consumer/revision.md";
    let body = "---\ntitle: Revision\nscope: personal\n---\nOriginal body";
    let imported_path = "vault/personal/consumer/imported-revision.md";
    let (_imported, imported_root) = seed(
        &s,
        &[("personal/consumer/imported-revision.md".into(), body.into())],
        &[cs("personal")],
    )
    .await;
    let (_, imported_entity) = identity(
        Scope::Personal,
        SourceKind::Vault,
        imported_root.to_str().expect("imported root"),
        "personal/consumer/imported-revision.md",
    );
    let mut imported_reader = ContextReader::new();
    imported_reader
        .import(
            &s,
            &id,
            &cs("personal"),
            &["consumer/imported-revision.md".into()],
            Scope::Personal,
        )
        .await
        .expect("legacy imported revision one");
    let imported_before = s
        .detail(Scope::Personal, &imported_entity)
        .await
        .expect("legacy row");
    assert_eq!(
        imported_before["source"]["verified_revision"],
        digest(body.as_bytes())
    );
    fs::remove_dir_all(&imported_root).expect("old root not needed by later native changes");
    native_write(&s, &root, "consumer-revision-1", path, body).await;
    let mut reader = ContextReader::new();
    reader
        .import(
            &s,
            &id,
            &cs("personal"),
            &["consumer/revision.md".into()],
            Scope::Personal,
        )
        .await
        .expect("native consumer");
    let material:String=sqlx::query_scalar("SELECT material_id::text FROM context_materials WHERE scope='personal' AND path='consumer/revision.md'").fetch_one(s.pool()).await.expect("material");
    let (source, entity) = identity(Scope::Personal, SourceKind::Context, &id, &material);
    reader
        .import(
            &s,
            &id,
            &cs("personal"),
            &["consumer/revision.md".into()],
            Scope::Meenseek,
        )
        .await
        .expect("separate app scope");
    let (_, company_entity) = identity(Scope::Meenseek, SourceKind::Context, &id, &material);
    assert_ne!(entity, company_entity);
    let company_generation_before = source_generation(&s, Scope::Meenseek, &company_entity).await;
    let company_before = s
        .detail(Scope::Meenseek, &company_entity)
        .await
        .expect("company native projection");
    assert_eq!(
        company_before["source"]["repository"],
        format!("ontology-context:{id}")
    );
    assert_eq!(
        s.detail(Scope::Personal, &company_entity).await,
        Err(Error::NotFound)
    );
    let e = source_evidence(&s, &entity).await;
    let proposal=brain(&s,json!({"op":"propose","scope":"personal","idempotency_key":"consumer-proposal","memory":{"body":"Keep exact source","evidence":[e]}})).await;
    let native=brain(&s,json!({"op":"remember","scope":"personal","idempotency_key":"consumer-native-basis","memory":{"body":"Accepted basis","evidence":[source_evidence(&s,&entity).await]}})).await;
    let option = source_evidence(&s, native["id"].as_str().expect("native id")).await;
    let flat=brain(&s,json!({"op":"propose","scope":"personal","idempotency_key":"consumer-flat-proposal","memory":{"body":"Flat source dependencies","evidence":[option]}})).await;
    let mut curations = Vec::new();
    for (key, basis, quote, knowledge) in [
        ("consumer-curation", &e, "Original body", true),
        ("consumer-no-change", &e, "Original body", false),
        ("consumer-flat-no-change", &option, "Accepted basis", false),
    ] {
        curations.push(curation_prepare_fixture(&s, key, basis, quote, knowledge).await);
    }
    let before = s.detail(Scope::Personal, &entity).await.expect("before");
    concurrent_native_invalidation(
        &s,
        &root,
        vec![path.into(), imported_path.into()],
        body,
        &proposal,
    )
    .await;
    let imported_after = s
        .detail(Scope::Personal, &imported_entity)
        .await
        .expect("native revision of imported identity");
    assert_eq!(imported_after["source"]["kind"], "vault");
    assert_ne!(
        imported_after["source"]["verified_revision"],
        imported_before["source"]["verified_revision"]
    );
    assert_eq!(imported_after["projection"], imported_before["projection"]);
    assert_eq!(imported_after["current"], false);
    let after = s
        .detail(Scope::Personal, &entity)
        .await
        .expect("stale inspectable");
    assert_ne!(
        before["source"]["verified_revision"],
        after["source"]["verified_revision"]
    );
    let company_after = s
        .detail(Scope::Meenseek, &company_entity)
        .await
        .expect("both bindings invalidated");
    assert_eq!(company_after["current"], false);
    assert_eq!(company_after["projection"], company_before["projection"]);
    assert_eq!(
        source_generation(&s, Scope::Meenseek, &company_entity).await,
        company_generation_before + 1
    );
    assert_eq!(after["current"], false);
    assert_eq!(after["projection"], before["projection"]);
    assert_eq!(
        brain(
            &s,
            json!({"op":"evidence","scope":"personal","query":"revision.md"})
        )
        .await["items"],
        json!([])
    );
    for item in [&proposal, &flat] {
        let previous = brain(
            &s,
            json!({"op":"history","scope":"personal","id":item["id"]}),
        )
        .await;
        assert_eq!(
            s.brain(cmd(
                json!({"op":"accept","scope":"personal","id":item["id"],"revision":1})
            ))
            .await,
            Err(Error::Conflict)
        );
        assert_eq!(
            brain(
                &s,
                json!({"op":"history","scope":"personal","id":item["id"]})
            )
            .await,
            previous
        );
    }
    for (prepared, candidate) in curations {
        let current=brain(&s,json!({"op":"curation","scope":"personal","command":{"action":"read","id":prepared["id"]}})).await;
        assert_eq!(current["current"], false);
        let apply = json!({"op":"curation","scope":"personal","command":{"action":"apply","id":prepared["id"],"review":{"candidate_digest":prepared["candidate_digest"],"reviewer":"synthetic-independent-reviewer","decision":"approve","reason":"Reviewed exact synthetic candidate"}}});
        assert_eq!(s.brain(cmd(apply)).await, Err(Error::Conflict));
        assert_eq!(s.brain(cmd(json!({"op":"curation","scope":"personal","command":{"action":"prepare","idempotency_key":format!("stale-{}",prepared["id"].as_str().expect("review id")),"author":"synthetic-writer","candidate":candidate}}))).await,Err(Error::Conflict));
    }
    let mut forged = e.clone();
    forged["generation"] =
        sqlx::query_scalar::<_, i64>("SELECT generation FROM sources WHERE id=$1")
            .bind(&source)
            .fetch_optional(s.pool())
            .await
            .expect("generation")
            .map_or(json!(999), |v| json!(v));
    assert!(s.brain(cmd(json!({"op":"remember","scope":"personal","idempotency_key":"forged-old-projection","memory":{"body":"reject old snapshot with new generation","evidence":[forged]}}))).await.is_err());
    reader
        .import(
            &s,
            &id,
            &cs("personal"),
            &["consumer/revision.md".into()],
            Scope::Personal,
        )
        .await
        .expect("refresh");
    assert_eq!(
        s.detail(Scope::Personal, &entity).await.expect("current")["current"],
        true
    );
    let snap=brain(&s,json!({"op":"evidence-read","scope":"personal","id":proposal["id"],"revision":1,"entity_id":entity})).await;
    assert_eq!(snap["content"], before["projection"]["content"]);
    let deletion = native_prepare_many(
        &s,
        &root,
        "consumer-revision-delete",
        vec![path.into(), imported_path.into()],
        "",
        vec![path.into(), imported_path.into()],
    )
    .await;
    consumer_apply(
        &s,
        root.clone(),
        root.clone(),
        "consumer-revision-delete".into(),
        deletion,
    )
    .await;
    reader
        .import(
            &s,
            &id,
            &cs("personal"),
            &["consumer/revision.md".into()],
            Scope::Personal,
        )
        .await
        .expect("tombstone consumer");
    let missing = s
        .detail(Scope::Personal, &entity)
        .await
        .expect("preserved missing identity");
    assert_eq!(missing["source"]["status"], "missing");
    assert_eq!(missing["current"], false);
    assert_eq!(
        missing["projection"]["content"],
        before["projection"]["content"]
    );
    assert!(missing["projection"]["absence_revision"].is_string());
    assert_eq!(reader.body_calls, 0);
    imported_reader
        .import(
            &s,
            &id,
            &cs("personal"),
            &["consumer/imported-revision.md".into()],
            Scope::Personal,
        )
        .await
        .expect("Vault identity can be tombstoned");
    let imported_missing = s
        .detail(Scope::Personal, &imported_entity)
        .await
        .expect("legacy missing projection");
    assert_eq!(imported_missing["source"]["status"], "missing");
    assert_eq!(
        imported_missing["projection"]["content"],
        imported_before["projection"]["content"]
    );
    assert_eq!(imported_reader.body_calls, 0);
    let recreated = native_prepare_many(
        &s,
        &root,
        "consumer-revision-recreate",
        vec![path.into(), imported_path.into()],
        body,
        vec![],
    )
    .await;
    consumer_apply(
        &s,
        root.clone(),
        root.clone(),
        "consumer-revision-recreate".into(),
        recreated,
    )
    .await;
    imported_reader
        .import(
            &s,
            &id,
            &cs("personal"),
            &["consumer/imported-revision.md".into()],
            Scope::Personal,
        )
        .await
        .expect("recreated imported identity");
    assert_eq!(
        s.detail(Scope::Personal, &imported_entity)
            .await
            .expect("same legacy entity")["current"],
        true
    );
    reader
        .import(
            &s,
            &id,
            &cs("personal"),
            &["consumer/revision.md".into()],
            Scope::Personal,
        )
        .await
        .expect("recreated same identity");
    assert_eq!(
        s.detail(Scope::Personal, &entity).await.expect("recreated")["current"],
        true
    );
    let plain=brain(&s,json!({"op":"propose","scope":"personal","idempotency_key":"plain-proposal","memory":{"body":"Evidence-free preserved"}})).await;
    brain(
        &s,
        json!({"op":"accept","scope":"personal","id":plain["id"],"revision":1}),
    )
    .await;
    for rev in [1, 2, 9] {
        for deleted in [false, true] {
            let sha = digest(b"same");
            let rust =
                revision_token(&id, &material, rev, deleted, &sha, "native", None).expect("token");
            let sql: String = sqlx::query_scalar(
                "SELECT context_source_revision($1::uuid,$2::uuid,$3,$4,$5,'native',NULL)",
            )
            .bind(&id)
            .bind(&material)
            .bind(rev)
            .bind(deleted)
            .bind(&sha)
            .fetch_one(s.pool())
            .await
            .expect("SQL token");
            assert_eq!(rust, sql);
        }
    }
}
#[tokio::test]
async fn context_consumer_atomic_read_publish() {
    let _guard = TEST_LOCK.lock().await;
    let s = store().await;
    let id = store_id(&s).await;
    let (_alternate, _alternate_root) = seed(
        &s,
        &[(
            "personal/alternate-binding.md".into(),
            "---\ntitle: Alternate\n---\nBody".into(),
        )],
        &[cs("personal")],
    )
    .await;
    let alternate: String = sqlx::query_scalar(
        "SELECT material_id::text FROM context_materials WHERE path='alternate-binding.md'",
    )
    .fetch_one(s.pool())
    .await
    .expect("alternate material");
    for n in [1usize, 100] {
        let files: Vec<_> = (0..n)
            .map(|i| {
                (
                    format!("personal/atomic-{n}-{i}.md"),
                    format!("---\ntitle: Atomic\n---\nBody {i}"),
                )
            })
            .collect();
        let (_temp, root) = seed(&s, &files, &[cs("personal")]).await;
        let paths: Vec<_> = (0..n).map(|i| format!("atomic-{n}-{i}.md")).collect();
        let mut reader = ContextReader::new();
        reader
            .import(&s, &id, &cs("personal"), &paths, Scope::Personal)
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "initial: {e:?}; observations {:?}",
                    s.dependency_observations()
                )
            });
        let (source, entity) = identity(
            Scope::Personal,
            SourceKind::Vault,
            root.to_str().expect("root"),
            &files[0].0,
        );
        let before = s.detail(Scope::Personal, &entity).await.expect("before");
        let actual: String = sqlx::query_scalar(
            "SELECT material_id::text FROM context_source_bindings WHERE source_id=$1",
        )
        .bind(&source)
        .fetch_one(s.pool())
        .await
        .expect("actual binding");
        sqlx::query("UPDATE context_source_bindings SET material_id=$2::uuid WHERE source_id=$1")
            .bind(&source)
            .bind(&alternate)
            .execute(s.pool())
            .await
            .expect("owned rebind conflict fixture");
        let from = s.dependency_observations().len();
        let calls = s.calls();
        assert_eq!(
            reader
                .import(&s, &id, &cs("personal"), &paths, Scope::Personal)
                .await,
            Err(Error::Conflict)
        );
        assert!(s.calls() - calls <= 16);
        report_path(&s, from, n, false);
        measured(&s, "consumer-bindings", true, 1, n * 256, n * 128);
        assert_eq!(
            s.detail(Scope::Personal, &entity)
                .await
                .expect("conflict rollback"),
            before
        );
        sqlx::query("UPDATE context_source_bindings SET material_id=$2::uuid WHERE source_id=$1")
            .bind(&source)
            .bind(actual)
            .execute(s.pool())
            .await
            .expect("restore exact owned binding");
        let mut pending = s.pool().begin().await.expect("exclusive connection");
        sqlx::query("SELECT pg_advisory_xact_lock(478310003)")
            .execute(&mut *pending)
            .await
            .expect("held gate");
        let calls = s.calls();
        assert_eq!(
            reader
                .import(&s, &id, &cs("personal"), &paths, Scope::Personal)
                .await,
            Err(Error::ContextPending)
        );
        assert!(s.calls() - calls <= 5);
        pending.rollback().await.expect("release");
        assert_eq!(
            s.detail(Scope::Personal, &entity)
                .await
                .expect("pending unchanged"),
            before
        );
        // A SQL fault after the read must roll publication back before marking only resolved IDs.
        sqlx::raw_sql("CREATE FUNCTION consumer_fail_publish() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'owned injected publication failure'; END $$; CREATE TRIGGER consumer_fail_publish BEFORE INSERT OR UPDATE ON context_source_bindings FOR EACH STATEMENT EXECUTE FUNCTION consumer_fail_publish();").execute(s.pool()).await.expect("fault injection");
        let observed_from = s.dependency_observations().len();
        let calls = s.calls();
        assert_eq!(
            reader
                .import(&s, &id, &cs("personal"), &paths, Scope::Personal)
                .await,
            Err(Error::Storage)
        );
        report_path(&s, observed_from, n, false);
        measured(&s, "consumer-bindings", false, 1, n * 256, 0);
        measured(&s, "mark_failed_in", true, 1, n * 128 + 128, 4);

        assert!(s.calls() - calls <= 17);
        let after = s.detail(Scope::Personal, &entity).await.expect("preserved");
        assert_eq!(after["projection"], before["projection"]);
        assert_eq!(after["source"]["failure_code"], "context-read-failed");
        sqlx::raw_sql("DROP TRIGGER consumer_fail_publish ON context_source_bindings; DROP FUNCTION consumer_fail_publish();").execute(s.pool()).await.expect("remove owned fault");
        reader
            .import(&s, &id, &cs("personal"), &paths, Scope::Personal)
            .await
            .expect("retry current");
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT status FROM sources WHERE id=$1")
                .bind(source)
                .fetch_one(s.pool())
                .await
                .expect("status"),
            "ok"
        );
    }
    // Hold the real publication query after the body has been read. A competing real
    // native commit must fail at the shared gate until this same connection commits.
    harness_fixture::seed(&s).await;
    let view = harness_fixture::view();
    let root = view.path().canonicalize().expect("view");
    let path = "vault/personal/consumer/atomic-native.md";
    native_write(
        &s,
        &root,
        "consumer-atomic-native-1",
        path,
        "---\ntitle: Before\n---\nBefore",
    )
    .await;
    let prepared = native_prepare(
        &s,
        &root,
        "consumer-atomic-native-2",
        path,
        "---\ntitle: After\n---\nAfter",
        vec![],
    )
    .await;
    sqlx::raw_sql("CREATE FUNCTION consumer_hold_publish() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock(478310099); RETURN NEW; END $$; CREATE TRIGGER consumer_hold_publish BEFORE INSERT ON sources FOR EACH ROW EXECUTE FUNCTION consumer_hold_publish();").execute(s.pool()).await.expect("owned publication barrier");
    let mut barrier = s.pool().begin().await.expect("barrier connection");
    sqlx::query("SELECT pg_advisory_xact_lock(478310099)")
        .execute(&mut *barrier)
        .await
        .expect("hold publication");
    let task_store = s.clone();
    let task_id = id.clone();
    let pending = tokio::spawn(async move {
        ContextReader::new()
            .import(
                &task_store,
                &task_id,
                &cs("personal"),
                &["consumer/atomic-native.md".into()],
                Scope::Personal,
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2),async {loop{let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_locks WHERE locktype='advisory' AND objid=478310099 AND NOT granted)").fetch_one(s.pool()).await.expect("barrier metadata");if waiting{break;}tokio::time::sleep(std::time::Duration::from_millis(10)).await;}}).await.expect("publication reached held barrier");
    let workspace = root.clone();
    let run = "consumer-atomic-native-2".to_owned();
    let commit_store = s.clone();
    let commit_root = root.clone();
    let commit_id = id.clone();
    let competing = tokio::spawn(async move {
        commit_store
            .with_native_commit(commit_root, commit_id, move |session| {
                let source = session.fresh_source().map_err(|_| Error::Storage)?;
                let engine = context_core::harness::HarnessEngine::with_source(
                    source.view_root(),
                    &workspace,
                    source.clone(),
                )
                .map_err(|_| Error::Storage)?;
                context_core::harness::HarnessExecutionRecord::apply_durable(
                    &engine,
                    &run,
                    Some(session),
                )
                .map(|_| ())
                .map_err(|_| Error::Storage)
            })
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        !competing.is_finished(),
        "real native commit waits for the consumer's shared gate"
    );
    barrier.rollback().await.expect("release publication");
    assert_eq!(
        pending
            .await
            .expect("consumer task")
            .expect("same connection publication"),
        1
    );
    sqlx::raw_sql(
        "DROP TRIGGER consumer_hold_publish ON sources; DROP FUNCTION consumer_hold_publish();",
    )
    .execute(s.pool())
    .await
    .expect("remove owned barrier");
    competing
        .await
        .expect("competing commit task")
        .expect("commit after consumer publication");
    consumer_apply(
        &s,
        root.clone(),
        root,
        "consumer-atomic-native-2".into(),
        prepared,
    )
    .await;
    let material:String=sqlx::query_scalar("SELECT material_id::text FROM context_materials WHERE scope='personal' AND path='consumer/atomic-native.md'").fetch_one(s.pool()).await.expect("material");
    let (_, entity) = identity(Scope::Personal, SourceKind::Context, &id, &material);
    assert_eq!(
        s.detail(Scope::Personal, &entity)
            .await
            .expect("old projection immediately invalidated")["current"],
        false
    );
    // Cancellation of a caller-owned import transaction releases the global lock.
    let tx = s.lock_import().await.expect("owned import lock");
    drop(tx);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            match s.lock_import().await {
                Ok(tx) => {
                    s.finish_import(tx)
                        .await
                        .expect("released after cancellation");
                    break;
                }
                Err(Error::Conflict) => tokio::task::yield_now().await,
                Err(e) => panic!("unexpected lock error: {e:?}"),
            }
        }
    })
    .await
    .expect("drop closes transaction");
}
#[tokio::test]
async fn context_consumer_commit_invalidation() {
    let _guard = TEST_LOCK.lock().await;
    let s = store().await;
    harness_fixture::seed(&s).await;
    let view = harness_fixture::view();
    let root = view.path().canonicalize().expect("root");
    let id = store_id(&s).await;
    let path = "vault/personal/consumer/invalidation.md";
    native_write(
        &s,
        &root,
        "consumer-invalidation-1",
        path,
        "---\ntitle: First\n---\nFirst body",
    )
    .await;
    let mut reader = ContextReader::new();
    reader
        .import(
            &s,
            &id,
            &cs("personal"),
            &["consumer/invalidation.md".into()],
            Scope::Personal,
        )
        .await
        .expect("bind");
    let material:String=sqlx::query_scalar("SELECT material_id::text FROM context_materials WHERE scope='personal' AND path='consumer/invalidation.md'").fetch_one(s.pool()).await.expect("material");
    let (source, entity) = identity(Scope::Personal, SourceKind::Context, &id, &material);
    let before = s.detail(Scope::Personal, &entity).await.expect("before");
    let generation: i64 = sqlx::query_scalar("SELECT generation FROM sources WHERE id=$1")
        .bind(&source)
        .fetch_one(s.pool())
        .await
        .expect("generation");
    let prepared = native_prepare(
        &s,
        &root,
        "consumer-invalidation-2",
        path,
        "---\ntitle: Second\n---\nSecond body",
        vec![],
    )
    .await;
    consumer_apply(
        &s,
        root.clone(),
        root.clone(),
        "consumer-invalidation-2".into(),
        prepared.clone(),
    )
    .await;
    let after = s
        .detail(Scope::Personal, &entity)
        .await
        .expect("immediate invalidation");
    assert_eq!(after["current"], false);
    assert_eq!(after["projection"], before["projection"]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT generation FROM sources WHERE id=$1")
            .bind(&source)
            .fetch_one(s.pool())
            .await
            .expect("increment"),
        generation + 1
    );
    let graph = brain(
        &s,
        json!({"op":"search","scope":"personal","query":"invalidation.md"}),
    )
    .await;
    // The graph shows the available original while consumer evidence stays stale.
    assert!(
        graph["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .any(|n| n["id"] == entity && n["current"] == true)
    );
    // Replaying the exact accepted apply returns the original Core attempt and does not invalidate twice.
    consumer_apply(
        &s,
        root.clone(),
        root,
        "consumer-invalidation-2".into(),
        prepared,
    )
    .await;
    assert_eq!(
        s.detail(Scope::Personal, &entity)
            .await
            .expect("replay unchanged"),
        after
    );
    reader
        .import(
            &s,
            &id,
            &cs("personal"),
            &["consumer/invalidation.md".into()],
            Scope::Personal,
        )
        .await
        .expect("fresh projection");
    assert_eq!(
        s.detail(Scope::Personal, &entity).await.expect("refreshed")["current"],
        true
    );
    let snapshot = "SELECT jsonb_build_object('materials',(SELECT jsonb_agg(to_jsonb(m) ORDER BY material_id) FROM context_materials m),'versions',(SELECT jsonb_agg(to_jsonb(v) ORDER BY material_id,revision) FROM context_material_versions v),'sources',(SELECT jsonb_agg(to_jsonb(s) ORDER BY id) FROM sources s),'records',(SELECT jsonb_agg(to_jsonb(p) ORDER BY entity_id) FROM source_records p))";
    let rollback_before: Value = sqlx::query_scalar(snapshot)
        .fetch_one(s.pool())
        .await
        .expect("complete canonical and consumer snapshot");
    let root = view.path().canonicalize().expect("view remains owned");
    let rollback_run = "consumer-invalidation-rollback";
    let rollback_prepared = native_prepare(
        &s,
        &root,
        rollback_run,
        path,
        "---\ntitle: Must rollback\n---\nRollback",
        vec![],
    )
    .await;
    sqlx::raw_sql("CREATE FUNCTION zz_consumer_commit_fail() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'owned failure after consumer invalidation'; END $$; CREATE TRIGGER zz_consumer_commit_fail AFTER UPDATE ON context_materials FOR EACH STATEMENT EXECUTE FUNCTION zz_consumer_commit_fail();").execute(s.pool()).await.expect("late injected failure");
    let workspace = root.clone();
    let failed = s
        .with_native_commit(root, id.clone(), move |session| {
            let source = session.fresh_source().map_err(|_| Error::Storage)?;
            let engine = context_core::harness::HarnessEngine::with_source(
                source.view_root(),
                &workspace,
                source.clone(),
            )
            .map_err(|_| Error::Storage)?;
            let result = context_core::harness::HarnessExecutionRecord::apply_durable(
                &engine,
                rollback_run,
                Some(session),
            );
            report_failed_native_sql(session, &rollback_prepared);
            result.map(|_| ()).map_err(|_| Error::Storage)
        })
        .await;
    assert!(failed.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, Value>(snapshot)
            .fetch_one(s.pool())
            .await
            .expect("read rolled-back rows"),
        rollback_before
    );
    sqlx::raw_sql("DROP TRIGGER zz_consumer_commit_fail ON context_materials; DROP FUNCTION zz_consumer_commit_fail();").execute(s.pool()).await.expect("remove only owned failure hook");
    let root = view.path().canonicalize().expect("owned view");
    let recovered = harness_fixture::command(
        &s,
        &root,
        &root,
        "recover",
        &["--run-id".into(), rollback_run.into()],
    )
    .await;
    assert!(recovered.status.success(), "actual late-failure recovery");
    let generation: i64 = sqlx::query_scalar("SELECT generation FROM sources WHERE id=$1")
        .bind(&source)
        .fetch_one(s.pool())
        .await
        .expect("current generation");
    sqlx::query("UPDATE sources SET generation=9223372036854775807 WHERE id=$1")
        .bind(&source)
        .execute(s.pool())
        .await
        .expect("owned overflow boundary fixture");
    let overflow_before: Value = sqlx::query_scalar(snapshot)
        .fetch_one(s.pool())
        .await
        .expect("overflow baseline");
    let overflow_run = "consumer-invalidation-overflow";
    let overflow_prepared = native_prepare(
        &s,
        &root,
        overflow_run,
        path,
        "---\ntitle: Overflow\n---\nMust roll back",
        vec![],
    )
    .await;
    let workspace = root.clone();
    let overflow = s
        .with_native_commit(root.clone(), id.clone(), move |session| {
            let source = session.fresh_source().map_err(|_| Error::Storage)?;
            let engine = context_core::harness::HarnessEngine::with_source(
                source.view_root(),
                &workspace,
                source.clone(),
            )
            .map_err(|_| Error::Storage)?;
            let result = context_core::harness::HarnessExecutionRecord::apply_durable(
                &engine,
                overflow_run,
                Some(session),
            );
            report_failed_native_sql(session, &overflow_prepared);
            result.map(|_| ()).map_err(|_| Error::Storage)
        })
        .await;
    assert_eq!(overflow, Err(Error::Storage));
    let overflow_after: Value = sqlx::query_scalar(snapshot)
        .fetch_one(s.pool())
        .await
        .expect("rolled back overflow");
    assert!(
        overflow_after == overflow_before,
        "overflow cannot change canonical bytes, revisions or consumers"
    );
    sqlx::query("UPDATE sources SET generation=$2 WHERE id=$1")
        .bind(&source)
        .bind(generation)
        .execute(s.pool())
        .await
        .expect("restore only boundary fixture");
    let recovered = harness_fixture::command(
        &s,
        &root,
        &root,
        "recover",
        &["--run-id".into(), overflow_run.into()],
    )
    .await;
    assert!(
        recovered.status.success(),
        "actual overflow recovery after restoring capacity"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT generation FROM sources WHERE id=$1")
            .bind(&source)
            .fetch_one(s.pool())
            .await
            .expect("one recovered invalidation"),
        generation + 1
    );
}
#[tokio::test]
async fn context_consumer_config_and_cli() {
    let _guard = TEST_LOCK.lock().await;
    strict_boundaries();
    let s = store().await;
    let id = store_id(&s).await;
    let temp = tempfile::tempdir().expect("config");
    let config = temp.path().join("sync.json");
    let valid = json!({"interval_seconds":15,"sources":[{"kind":"context","store_id":id,"context_scope":"personal","scope":"personal","paths":["one.md"]}]});
    let mut too_many_paths = valid.clone();
    too_many_paths["sources"][0]["paths"] =
        json!((0..101).map(|i| format!("{i}.md")).collect::<Vec<_>>());
    let mut unknown = valid.clone();
    unknown["sources"][0]["binary"] = json!("/never-probe");
    let mut upper = valid.clone();
    upper["sources"][0]["store_id"] = json!(id.to_uppercase());
    let mut aggregate = valid.clone();
    aggregate["sources"] = json!([valid["sources"][0],{"kind":"context","store_id":id,"context_scope":"personal","scope":"meenseek","paths":(0..100).map(|i|format!("{i}.md")).collect::<Vec<_>>()}]);
    for bad in [too_many_paths, unknown, upper, aggregate] {
        fs::write(&config, serde_json::to_vec(&bad).expect("config")).expect("config");
        let before = s.calls();
        assert!(refresh(&s, &config).await.is_err());
        assert_eq!(s.calls(), before);
    }
    for bad in [
        json!({"interval_seconds":14,"sources":valid["sources"]}),
        json!({"interval_seconds":3601,"sources":valid["sources"]}),
        json!({"interval_seconds":15,"sources":[]}),
        json!({"interval_seconds":15,"sources":[valid["sources"][0],valid["sources"][0]]}),
        json!({"interval_seconds":15,"sources":[{"kind":"vault","root":"/no-probe","scope":"personal","vault_scope":"personal","binary":"/no-execute","paths":["one.md"]}]}),
        json!({"interval_seconds":15,"sources":[{"kind":"context","store_id":id,"context_scope":"work","scope":"personal","paths":["one.md"]}]}),
        json!({"interval_seconds":15,"sources":[{"kind":"context","store_id":id,"context_scope":"personal","scope":"personal","paths":[]}]}),
        json!({"interval_seconds":15,"sources":vec![valid["sources"][0].clone();9]}),
    ] {
        fs::write(&config, serde_json::to_vec(&bad).expect("JSON")).expect("config");
        let before = s.calls();
        assert!(refresh(&s, &config).await.is_err());
        assert_eq!(s.calls(), before);
    }
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_ontology"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("DATABASE_URL", "invalid-before-connect")
        .env("ONTOLOGY_SYNC_CONFIG", &config)
        .arg("sync-once")
        .kill_on_drop(true)
        .output()
        .await
        .expect("actual invalid sync CLI");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Git/Context"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Storage"));
    fs::write(&config, " ".repeat(32769)).expect("over limit");
    assert!(SyncConfig::load(&config).is_err());
    for n in [1usize, 100] {
        let mut v = valid.clone();
        v["sources"][0]["paths"] = json!((0..n).map(|i| format!("{i}.md")).collect::<Vec<_>>());
        let mut bytes = serde_json::to_vec(&v).expect("JSON");
        bytes.resize(32768, b' ');
        fs::write(&config, bytes).expect("exact byte boundary");
        assert!(SyncConfig::load(&config).is_ok());
    }
    let (_temp, root) = seed(
        &s,
        &[(
            "personal/one.md".into(),
            "---\ntitle: One\n---\nCLI body".into(),
        )],
        &[cs("personal")],
    )
    .await;
    fs::remove_dir_all(root).expect("no old transport");
    for args in [
        vec!["import-vault"],
        vec!["import-context", "--vault-root", "/not-probed"],
        vec!["import-context", "--store-id", "INVALID"],
    ] {
        let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_ontology"))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("DATABASE_URL", "invalid-before-connect")
            .args(args)
            .output()
            .await
            .expect("actual CLI");
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("Storage"));
    }
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_ontology"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env(
            "DATABASE_URL",
            std::env::var("TEST_DATABASE_URL").expect("owned DB"),
        )
        .args([
            "import-context",
            "--store-id",
            &id,
            "--context-scope",
            "personal",
            "--scope",
            "personal",
            "--file",
            "one.md",
        ])
        .output()
        .await
        .expect("actual replacement CLI");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    mixed_sync_fixture(&s, &id, &config, temp.path()).await;
}

// Exact read policy and failure transfer from the retired transport contract.
async fn boundary_reads(s: &Store, id: &str) {
    let mut reader = ContextReader::new();
    for scope in [cs("personal"), cs("work/acme")] {
        for bad in [
            "../escape.md",
            "/absolute.md",
            "./note.md",
            "a/../note.md",
            "a//note.md",
            "note.txt",
            "a/*.md",
            "a\n.md",
            "a\\b.md",
            ":(glob)*.md",
            ".hidden/n.md",
            "journal/n.md",
            "conversations/raw/n.md",
            "raw/n.md",
            "secret.md",
            "secrets.md",
            "credentials.md",
            "target/n.md",
        ] {
            let before = s.calls();
            assert!(
                reader
                    .import(s, id, &scope, &[bad.into()], Scope::Personal)
                    .await
                    .is_err(),
                "{bad}"
            );
            assert_eq!(s.calls(), before, "pre-SQL policy");
            assert_eq!(reader.body_calls, 0);
        }
        let long = format!("{}.md", "a".repeat(512));
        let before = s.calls();
        assert!(
            reader
                .import(s, id, &scope, &[long], Scope::Personal)
                .await
                .is_err()
        );
        assert_eq!(s.calls(), before);
    }
    for n in [1usize, 100] {
        let missing = (0..n)
            .map(|i| format!("unregistered-{i}.md"))
            .collect::<Vec<_>>();
        let start = s.dependency_observations().len();
        let before = s.calls();
        assert_eq!(
            reader
                .import(s, id, &cs("personal"), &missing, Scope::Personal)
                .await,
            Err(Error::NotFound)
        );
        assert_eq!(reader.body_calls, 0);
        assert!(s.calls() - before <= 12);
        report_path(s, start, n, false);
        measured(s, "A-exact-metadata", true, 1, 65536, 2);
        let before = s.calls();
        assert_eq!(
            reader
                .import(
                    s,
                    &uuid::Uuid::new_v4().to_string(),
                    &cs("personal"),
                    &missing,
                    Scope::Personal
                )
                .await,
            Err(Error::Conflict)
        );
        assert!(s.calls() - before <= 10);
        assert_eq!(reader.body_calls, 0);
    }
    let header = format!("---\ntitle: Boundary\n# {}\n---\n", "padding".repeat(300));
    let files = vec![
        (
            "personal/limit-ok.md".into(),
            format!(
                "{header}{}",
                "x ".repeat(65536)
                    .chars()
                    .take(65536 - header.len())
                    .collect::<String>()
            ),
        ),
        (
            "personal/limit-over.md".into(),
            format!(
                "{header}{}",
                "x ".repeat(65537)
                    .chars()
                    .take(65537 - header.len())
                    .collect::<String>()
            ),
        ),
    ];
    let (_temp, _root) = seed(s, &files, &[cs("personal")]).await;
    reader
        .import(
            s,
            id,
            &cs("personal"),
            &["limit-ok.md".into()],
            Scope::Personal,
        )
        .await
        .expect("inclusive original limit");
    assert_eq!(reader.body_calls, 1);
    let before = s.calls();
    assert_eq!(
        reader
            .import(
                s,
                id,
                &cs("personal"),
                &["limit-over.md".into()],
                Scope::Personal
            )
            .await,
        Err(Error::Limit)
    );
    assert_eq!(reader.body_calls, 0);
    assert!(s.calls() - before <= 13);
    for (n, escaped) in [(17usize, false), (24, true)] {
        let text = if escaped {
            "\" ".repeat(15000)
        } else {
            "x".repeat(65500)
        };
        let files = (0..n)
            .map(|i| {
                (
                    format!("personal/aggregate-{n}-{i}.md"),
                    format!("---\ntitle: Limit\n---\n{text}"),
                )
            })
            .collect::<Vec<_>>();
        let (_temp, _root) = seed(s, &files, &[cs("personal")]).await;
        let paths = (0..n)
            .map(|i| format!("aggregate-{n}-{i}.md"))
            .collect::<Vec<_>>();
        let before = s.calls();
        let from = s.dependency_observations().len();
        assert_eq!(
            reader
                .import(s, id, &cs("personal"), &paths, Scope::Personal)
                .await,
            Err(Error::Limit)
        );
        assert_eq!(reader.body_calls, u64::from(escaped));
        assert!(s.calls() - before <= 17);
        report_path(s, from, n, false);
    }
    for n in [1usize, 100] {
        let files = (0..n)
            .map(|i| {
                (
                    format!("personal/parse-{n}-{i}.md"),
                    if i + 1 == n {
                        "---\ntitle: [\n---\nmalformed".into()
                    } else {
                        "---\ntitle: Valid\n---\nBody".into()
                    },
                )
            })
            .collect::<Vec<_>>();
        let (_temp, _root) = seed(s, &files, &[cs("personal")]).await;
        let paths = (0..n)
            .map(|i| format!("parse-{n}-{i}.md"))
            .collect::<Vec<_>>();
        let before = s.calls();
        let from = s.dependency_observations().len();
        assert_eq!(
            reader
                .import(s, id, &cs("personal"), &paths, Scope::Personal)
                .await,
            Err(Error::Invalid)
        );
        assert_eq!(reader.body_calls, 1);
        assert!(s.calls() - before <= 17);
        report_path(s, from, n, false);
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM sources WHERE path LIKE $1")
            .bind(format!("personal/parse-{n}-%"))
            .fetch_one(s.pool())
            .await
            .expect("no partial publication");
        assert_eq!(count, 0);
    }
    let originals = tempfile::tempdir().expect("owned malformed byte fixtures");
    let root = originals.path().canonicalize().expect("root");
    fs::create_dir(root.join("personal")).expect("scope");
    fs::write(root.join("personal/utf8.md"), [255, 254]).expect("invalid UTF8 bytes");
    let scopes = [cs("personal")];
    let manifest = inventory(&root, &scopes).expect("exact invalid byte inventory");
    s.import_context(&root, &scopes, &manifest.inventory_digest)
        .await
        .expect("canonical non-UTF8 bytes preserved");
    // NUL cannot enter PostgreSQL text/search columns. Keep this negative exact-byte
    // fixture in bytea with no search text; it creates no Core acceptance receipt.
    let nul = b"---\ntitle: NUL\n---\nbody\0end";
    sqlx::query("INSERT INTO context_materials(scope,path,source_root,source_path,source_digest,content_digest,content,byte_len,restricted,search_text) VALUES('personal','nul.md',$1,'personal/nul.md',$2,$2,$3,$4,false,NULL)").bind(root.to_str().expect("root")).bind(digest(nul)).bind(nul.as_slice()).bind(nul.len() as i64).execute(s.pool()).await.expect("owned NUL bytea boundary fixture");
    for path in ["utf8.md", "nul.md"] {
        let before = s.calls();
        assert_eq!(
            reader
                .import(s, id, &cs("personal"), &[path.into()], Scope::Personal)
                .await,
            Err(Error::Invalid)
        );
        assert_eq!(reader.body_calls, 1);
        assert!(s.calls() - before <= 17);
    }
}

async fn curation_prepare_fixture(
    s: &Store,
    key: &str,
    evidence: &Value,
    quote: &str,
    knowledge: bool,
) -> (Value, Value) {
    let candidate = json!({"source_id":evidence["entity_id"],"basis":[evidence],"quotations":[{"entity_id":evidence["entity_id"],"quote":quote}],"reason":"Preserve synthetic scope and conditions","finding":if knowledge {json!({"kind":"knowledge","body":"Bounded synthetic conclusion","applicability":"Synthetic fixture only"})}else{json!({"kind":"no-change"})}});
    let prepared=brain(s,json!({"op":"curation","scope":"personal","command":{"action":"prepare","idempotency_key":key,"author":"synthetic-writer","candidate":candidate}})).await;
    (prepared, candidate)
}
async fn concurrent_native_invalidation(
    s: &Store,
    root: &Path,
    paths: Vec<String>,
    body: &str,
    proposal: &Value,
) {
    let prepared = native_prepare_many(s, root, "consumer-revision-2", paths, body, vec![]).await;
    sqlx::raw_sql("CREATE FUNCTION consumer_accept_barrier() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock(478310096); RETURN NEW; END $$; CREATE TRIGGER consumer_accept_barrier AFTER UPDATE ON sources FOR EACH ROW EXECUTE FUNCTION consumer_accept_barrier();").execute(s.pool()).await.expect("owned invalidation barrier");
    let mut barrier = s.pool().begin().await.expect("barrier");
    sqlx::query("SELECT pg_advisory_xact_lock(478310096)")
        .execute(&mut *barrier)
        .await
        .expect("hold source update");
    let applying = s.clone();
    let view = root.to_owned();
    let workspace = root.to_owned();
    let apply = tokio::spawn(async move {
        consumer_apply(
            &applying,
            view,
            workspace,
            "consumer-revision-2".into(),
            prepared,
        )
        .await
    });
    // Actual Core may perform source verification before reaching the short SQL barrier.
    tokio::time::timeout(std::time::Duration::from_secs(60),async {loop {
        let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_locks WHERE locktype='advisory' AND objid=478310096 AND NOT granted)").fetch_one(s.pool()).await.expect("barrier metadata");
        if waiting {break;} tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }}).await.expect("actual native consumer invalidation reached barrier");
    let accepting = s.clone();
    let request = cmd(json!({"op":"accept","scope":"personal","id":proposal["id"],"revision":1}));
    let accept = tokio::spawn(async move { accepting.brain(request).await });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        !accept.is_finished(),
        "accept must wait for native invalidation's source lock"
    );
    barrier.rollback().await.expect("release actual commit");
    tokio::time::timeout(std::time::Duration::from_secs(60), apply)
        .await
        .expect("bounded apply")
        .expect("actual apply task");
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(5), accept)
            .await
            .expect("bounded acceptance")
            .expect("accept task"),
        Err(Error::Conflict)
    );
    sqlx::raw_sql(
        "DROP TRIGGER consumer_accept_barrier ON sources; DROP FUNCTION consumer_accept_barrier();",
    )
    .execute(s.pool())
    .await
    .expect("remove owned barrier");
}

fn fixture_git(root: &Path, args: &[&str]) {
    let status = std::process::Command::new("/usr/bin/git")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("owned Git fixture");
    assert!(status.success());
}
async fn mixed_sync_fixture(s: &Store, id: &str, config: &Path, base: &Path) {
    let repo = base.join("git");
    fs::create_dir(&repo).expect("owned Git repository");
    let repo = repo.canonicalize().expect("canonical Git identity");
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.name", "Synthetic"],
        vec!["config", "user.email", "synthetic@example.invalid"],
    ] {
        fixture_git(&repo, &args);
    }
    fs::write(repo.join("note.md"), "Initial Git bytes").expect("Git source");
    fixture_git(&repo, &["add", "."]);
    fixture_git(&repo, &["commit", "-qm", "Initial fixture"]);
    for n in [1usize, 99] {
        let files = (0..n)
            .map(|i| {
                (
                    format!("personal/mixed-{n}-{i}.md"),
                    "---\ntitle: Context\n---\nOriginal Context bytes".into(),
                )
            })
            .collect::<Vec<_>>();
        let (_source, root) = seed(s, &files, &[cs("personal")]).await;
        let paths = (0..n)
            .map(|i| format!("mixed-{n}-{i}.md"))
            .collect::<Vec<_>>();
        let git =
            json!({"kind":"git","root":repo,"scope":"personal","ref":"HEAD","paths":["note.md"]});
        let context = json!({"kind":"context","store_id":id,"context_scope":"personal","scope":"personal","paths":paths});
        let config_value = json!({"interval_seconds":15,"sources":[git,context]});
        fs::write(config, serde_json::to_vec(&config_value).expect("config"))
            .expect("owned config");
        let before = s.calls();
        let first = refresh(s, config).await.expect("mixed source refresh");
        assert!(first.ok, "mixed outcomes: {first:?}");
        assert!(s.calls() - before <= 20);
        assert_eq!(first.sources.len(), 2);
        assert_eq!(first.sources[1].provider_calls, 1);
        assert!(first.sources[1].response_bytes <= ((n * 2048) + 4096) as u64);
        measured(s, "sync-refresh", true, 20, 8192, 4096);
        let (_, entity) = identity(
            Scope::Personal,
            SourceKind::Vault,
            root.to_str().expect("root"),
            &files[n - 1].0,
        );
        let old = s
            .detail(Scope::Personal, &entity)
            .await
            .expect("old Context projection");
        let mut failing = config_value.clone();
        failing["sources"][1]["paths"][0] = json!("unregistered-mixed.md");
        fs::write(repo.join("note.md"), format!("Committed Git change {n}")).expect("changed Git");
        fixture_git(&repo, &["add", "."]);
        fixture_git(&repo, &["commit", "-qm", "Change fixture"]);
        fs::write(config, serde_json::to_vec(&failing).expect("config")).expect("config");
        let before = s.calls();
        let failed = refresh(s, config)
            .await
            .expect("earlier Git success remains");
        assert!(!failed.ok && failed.sources[0].ok && !failed.sources[1].ok);
        assert_eq!(failed.sources[1].provider_calls, 0);
        assert!(s.calls() - before <= 20);
        measured(s, "sync-refresh", true, 20, 8192, 4096);
        let (_, git_entity) = ontology::importer::identity(&repo, "note.md", Scope::Personal)
            .expect("unchanged Git identity");
        assert_eq!(
            s.detail(Scope::Personal, &git_entity)
                .await
                .expect("Git committed")["projection"]["content"],
            format!("Committed Git change {n}")
        );
        assert_eq!(
            s.detail(Scope::Personal, &entity)
                .await
                .expect("Context cache retained")["projection"],
            old["projection"]
        );
        let mut reverse = json!({"interval_seconds":15,"sources":[context,git]});
        reverse["sources"][1]["ref"] = json!("refs/heads/does-not-exist");
        fs::write(config, serde_json::to_vec(&reverse).expect("config")).expect("config");
        let before = s.calls();
        let failed = refresh(s, config)
            .await
            .expect("earlier Context success remains");
        assert!(!failed.ok && failed.sources[0].ok && !failed.sources[1].ok);
        assert!(s.calls() - before <= 21);
        measured(s, "sync-refresh", true, 21, 8192, 4096);
        assert_eq!(
            s.detail(Scope::Personal, &entity)
                .await
                .expect("Context success")["current"],
            true
        );
        fs::write(config, serde_json::to_vec(&config_value).expect("config")).expect("config");
        let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_ontology"))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env(
                "DATABASE_URL",
                std::env::var("TEST_DATABASE_URL").expect("owned URL"),
            )
            .env("ONTOLOGY_SYNC_CONFIG", config)
            .arg("sync-once")
            .kill_on_drop(true)
            .output()
            .await
            .expect("actual mixed sync CLI");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_slice(&output.stdout).expect("bounded JSON report");
        assert_eq!(report["ok"], true);
        assert!(output.stdout.len() < 4096);
    }
}

async fn consumer_apply(
    store: &Store,
    view: PathBuf,
    workspace: PathBuf,
    run: String,
    prepared: context_core::harness::PreparedHarnessRun,
) -> (
    context_core::harness::HarnessExecutionRecord,
    context_core::harness::HarnessApplyAttemptReceipt,
) {
    use context_core::harness::*;
    let store_id = prepared
        .plan
        .bound_source_versions()
        .store_identity
        .as_ref()
        .expect("actual native identity")
        .store_id
        .clone();
    let targets = prepared.plan.frozen_targets.targets.len();
    let version_bytes = serde_json::to_vec(prepared.plan.bound_source_versions())
        .expect("actual source versions")
        .len();
    store.with_native_commit(view,store_id,move |session| {
        let source=session.fresh_source().expect("fresh gated source");
        let engine=HarnessEngine::with_source(source.view_root(),&workspace,source.clone()).expect("actual native engine");
        let result=HarnessExecutionRecord::apply_durable(&engine,&run,Some(session)).expect("actual Core apply");
        result.1.validate(&prepared).expect("actual receipt validates");
        let calls=session.native_sql_observations().expect("complete existing SQL observations");
        // Core replay rechecks stored receipts and versions; it performs no SQL mutation.
        if !calls.is_empty() {
            let replay = calls.iter().all(|c| c.statement.trim_start().starts_with("SELECT"));
            assert_eq!(calls.len(), if replay { 8 } else { 17 });
            assert!(calls.iter().all(|c|c.succeeded&&c.started<c.finished));
            assert!(calls.windows(2).all(|p|p[0].finished<p[1].started));
            let unique= calls.iter().map(|c|&c.request_digest).collect::<std::collections::BTreeSet<_>>();
            if replay { assert!(calls.len()-unique.len() <= 7); }
            else { assert_eq!(calls.len()-unique.len(),7,"only the existing integrity rechecks repeat"); }
            let parameters=calls.iter().map(|c|c.parameter_bytes).sum::<usize>();let rows=calls.iter().map(|c|c.row_bytes).sum::<usize>();
            assert!(parameters<=20*targets*65536+12*version_bytes+65536);assert!(rows<=12*version_bytes+65536);
            eprintln!("consumer native SQL: targets={targets} replay={replay} calls={} duplicate_integrity_requests={} sequential_depth={} parameter_bytes={parameters} row_bytes={rows}",calls.len(),calls.len()-unique.len(),calls.len());
        }
        Ok(result)
    }).await.expect("owned native commit session")
}

fn report_failed_native_sql(
    session: &ontology::native_context::NativeContextSession,
    prepared: &context_core::harness::PreparedHarnessRun,
) {
    let calls = session
        .native_sql_observations()
        .expect("complete failed SQL window");
    assert!(!calls.is_empty() && calls.len() <= 17);
    assert_eq!(calls.iter().filter(|call| !call.succeeded).count(), 1);
    assert!(calls.iter().all(|c| c.started < c.finished));
    assert!(calls.windows(2).all(|p| p[0].finished < p[1].started));
    let unique = calls
        .iter()
        .map(|c| &c.request_digest)
        .collect::<std::collections::BTreeSet<_>>();
    assert!(calls.len() - unique.len() <= 7);
    let failed_request = &calls
        .iter()
        .find(|c| !c.succeeded)
        .expect("one failed query")
        .request_digest;
    let retries = calls
        .iter()
        .filter(|c| &c.request_digest == failed_request)
        .count()
        - 1;
    assert_eq!(
        retries, 0,
        "failed publication is not automatically retried"
    );
    let version_bytes = serde_json::to_vec(prepared.plan.bound_source_versions())
        .expect("bound versions")
        .len();
    let parameters = calls.iter().map(|c| c.parameter_bytes).sum::<usize>();
    let rows = calls.iter().map(|c| c.row_bytes).sum::<usize>();
    assert!(
        parameters
            <= 20 * prepared.plan.frozen_targets.targets.len() * 65536 + 12 * version_bytes + 65536
    );
    assert!(rows <= 12 * version_bytes + 65536);
    eprintln!(
        "consumer failed native SQL: calls={} duplicate_integrity_requests={} sequential_depth={} retries={retries} parameter_bytes={parameters} row_bytes={rows}",
        calls.len(),
        calls.len() - unique.len(),
        calls.len()
    );
}

// Native evidence takes a distinct row-lock branch from imported document evidence.
// Its one batched lock is bounded at both one reference and the ten-reference limit.
fn measured_native_accept(s: &Store, from: usize, n: usize, outcome: &str) {
    let success = outcome == "accepted";
    let lock_success = outcome != "lock-failed";
    let calls = if success {
        10
    } else if lock_success {
        5
    } else {
        4
    };
    let root = measured(s, "change-memory", success, calls, 256, 32768);
    assert_eq!(root.store_call_delta, calls);
    let checked = measured(
        s,
        "memory-evidence-revalidate",
        success,
        if lock_success { 2 } else { 1 },
        32768,
        if success { 4 } else { 0 },
    );
    let locks = measured(
        s,
        "accept-native-locks",
        lock_success,
        1,
        16 + 41 * n,
        38 * n,
    );
    assert_eq!(locks.store_call_delta, 1);
    assert!(locks.input_bytes > 0);
    assert_eq!(locks.returned_bytes, if lock_success { 38 * n } else { 0 });
    let all = s.dependency_observations();
    let window = &all[from..];
    assert_eq!(window.len(), if lock_success { 4 } else { 3 });
    assert!(
        window
            .iter()
            .all(|o| root.started <= o.started && o.finished <= root.finished)
    );
    let unique: std::collections::BTreeSet<_> = window
        .iter()
        .map(|o| (o.dependency, &o.request_digest))
        .collect();
    assert_eq!(
        unique.len(),
        window.len(),
        "no repeated request or automatic retry"
    );
    assert!(root.started < checked.started && checked.started < locks.started);
    assert!(locks.finished < checked.finished && checked.finished < root.finished);
    assert!(!window.iter().any(|o| o.dependency == "accept-source-locks"));
    if lock_success {
        let current = window
            .iter()
            .find(|o| o.dependency == "accept-current")
            .expect("freshness query follows native row lock");
        assert!(locks.finished < current.started && current.finished < checked.finished);
        assert_eq!(current.store_call_delta, 1);
        assert!(current.succeeded);
        assert!(current.input_bytes <= 64);
        assert_eq!(current.returned_bytes, if success { 4 } else { 5 });
    }
    eprintln!(
        "native accept measured: {}",
        json!({"references":n,"outcome":outcome,
        "calls":calls,"sequential_depth":root.store_call_delta,"duplicate_requests":window.len()-unique.len(),
        "retries":window.iter().filter(|o|o.dependency=="accept-native-locks").count()-1,
        "native_lock_input_bytes":locks.input_bytes,"native_lock_returned_bytes":locks.returned_bytes,
        "native_lock_returned_format":locks.returned_format})
    );
}
#[tokio::test]
async fn context_consumer_native_accept_measurements() {
    let _guard = TEST_LOCK.lock().await;
    for n in [1usize, 10] {
        let s = store().await;
        let mut ids = Vec::new();
        let mut evidence = Vec::new();
        for i in 0..n {
            let base = brain(
                &s,
                json!({"op":"remember","scope":"personal",
                "idempotency_key":format!("native-basis-{n}-{i}"),
                "memory":{"body":format!("Accepted native basis {n}-{i}")}}),
            )
            .await;
            let id = base["id"]
                .as_str()
                .expect("created native memory")
                .to_owned();
            evidence.push(source_evidence(&s, &id).await);
            ids.push(id);
        }
        let mut proposals = Vec::new();
        for label in ["success", "failure"] {
            let proposal = brain(
                &s,
                json!({"op":"propose","scope":"personal",
                "idempotency_key":format!("native-proposal-{n}-{label}"),
                "memory":{"body":"Retain exact native evidence", "evidence":evidence}}),
            )
            .await;
            let refs = proposal["evidence"]
                .as_array()
                .expect("persisted flat evidence");
            assert_eq!(refs.len(), n);
            assert!(refs.iter().all(|e| {
                e["entity_id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("m_"))
                    && e["source_id"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("m_"))
            }));
            proposals.push(proposal);
        }
        let from = s.dependency_observations().len();
        let accepted = brain(
            &s,
            json!({"op":"accept","scope":"personal","id":proposals[0]["id"],"revision":1}),
        )
        .await;
        measured_native_accept(&s, from, n, "accepted");
        assert_eq!(accepted["status"], "accepted");
        assert_eq!(accepted["revision"], 2);
        let pending = &proposals[1]["id"];
        let history = brain(&s, json!({"op":"history","scope":"personal","id":pending})).await;
        // Hold an actual selected native row; the configured PostgreSQL lock timeout
        // makes the direct lock query fail before any freshness or mutation request.
        let mut blocker = s.pool().begin().await.expect("owned row lock transaction");
        sqlx::query("SELECT id FROM memories WHERE scope='personal' AND id=$1 FOR UPDATE")
            .bind(&ids[n - 1])
            .fetch_one(&mut *blocker)
            .await
            .expect("hold selected native evidence");
        let from = s.dependency_observations().len();
        let failure = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            s.brain(cmd(
                json!({"op":"accept","scope":"personal","id":pending,"revision":1}),
            )),
        )
        .await;
        blocker.rollback().await.expect("release owned row lock");
        assert_eq!(
            failure.expect("bounded PostgreSQL lock failure"),
            Err(Error::Storage)
        );
        measured_native_accept(&s, from, n, "lock-failed");
        assert_eq!(
            brain(&s, json!({"op":"history","scope":"personal","id":pending})).await,
            history
        );
        brain(
            &s,
            json!({"op":"correct","scope":"personal","id":ids[n-1],"revision":1,
            "memory":{"body":"Changed native basis"}}),
        )
        .await;
        let from = s.dependency_observations().len();
        assert_eq!(
            s.brain(cmd(
                json!({"op":"accept","scope":"personal","id":pending,"revision":1})
            ))
            .await,
            Err(Error::Conflict)
        );
        measured_native_accept(&s, from, n, "stale");
        assert_eq!(
            brain(&s, json!({"op":"history","scope":"personal","id":pending})).await,
            history
        );
        let snapshot = brain(
            &s,
            json!({"op":"evidence-read","scope":"personal","id":pending,
            "revision":1,"entity_id":ids[n-1]}),
        )
        .await;
        assert_eq!(snapshot["available"], true);
        assert_eq!(
            snapshot["content"],
            format!("Accepted native basis {n}-{}", n - 1)
        );
        s.pool().close().await;
    }
}
