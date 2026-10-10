#[cfg(test)]
use harness_fixture::context_fixture;

use context_core::harness::{ContextSource, SourcePathKind};
use ontology::{
    context::{
        ContextScope, MAX_OUTPUT_BYTES, MAX_READ_DOCUMENT_BYTES, MAX_READ_DOCUMENT_METADATA_BYTES,
        MAX_READ_DOCUMENT_TOTAL_BYTES, inventory,
    },
    domain::Error,
    store::Store,
};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Stdio,
};
static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn scope() -> ContextScope {
    "personal".parse().expect("synthetic scope")
}
async fn store() -> Store {
    let url = std::env::var("TEST_DATABASE_URL").expect("owned test DB");
    assert!(
        ontology::config::database_options(&url)
            .expect("local DB")
            .get_database()
            .is_some_and(|s| s.starts_with("ontology_test_"))
    );
    let store = Store::connect(&url).await.expect("test PG");
    store.initialize().await.expect("007 migration");
    sqlx::query("TRUNCATE document_grouping,document_subjects,memory_grouping,context_source_bindings,context_projection_versions,context_material_versions,context_manual_edits,context_materials,context_apply_batches,evidence_snapshots,evidence_contents,memory_history,memories,memory_creations,subjects,curation_reviews,confirmation_history,related_materials,entity_areas,entity_topics,source_records,entities,sources,topics").execute(store.pool()).await.expect("reset fixture");
    store
}
fn fixture(count: usize, body: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("fixture");
    let root = dir.path().canonicalize().expect("canonical fixture");
    fs::create_dir(root.join("personal")).expect("scope");
    for n in 0..count {
        fs::write(root.join(format!("personal/{n:04}.md")), body).expect("synthetic document");
    }
    (dir, root)
}
async fn import(store: &Store, root: &Path) {
    let i = inventory(root, &[scope()]).expect("inventory");
    store
        .import_context(root, &[scope()], &i.inventory_digest)
        .await
        .expect("import with projection");
}
async fn cli(value: Value) -> std::process::Output {
    use tokio::io::AsyncWriteExt;
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_ontology"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env(
            "DATABASE_URL",
            std::env::var("TEST_DATABASE_URL").expect("DB"),
        )
        .arg("context")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("CLI");
    let mut stdin = child.stdin.take().expect("stdin");
    stdin
        .write_all(value.to_string().as_bytes())
        .await
        .expect("command");
    drop(stdin);
    child.wait_with_output().await.expect("output")
}
#[tokio::test]
async fn native_read_documents_one_response_for_one_and_one_hundred_paths() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(100, b"---\nexport: false\n---\n# Native\nBody");
    import(&store, &root).await;
    for count in [1, 100] {
        let paths: Vec<_> = (0..count).map(|n| format!("{n:04}.md")).collect();
        let before = store.calls();
        let output = store
            .read_context_documents(&scope(), &paths)
            .await
            .expect("exact batch");
        assert_eq!(output["documents"].as_array().expect("array").len(), count);
        assert_eq!(
            store.calls() - before,
            7,
            "one gate transaction plus three queries, independent of batch size"
        );
        let output = cli(json!({"op":"read-documents","scope":"personal","paths":paths})).await;
        assert!(output.status.success(), "CLI batch succeeds");
        let value: Value = serde_json::from_slice(&output.stdout).expect("single complete JSON");
        assert_eq!(
            value["documents"].as_array().expect("documents").len(),
            count
        );
    }
}
#[tokio::test]
async fn native_read_documents_no_partial_output_on_invalid_or_later_failure() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(1, b"# Safe");
    fs::write(
        root.join("personal/bad.md"),
        b"---\nscope: work\n---\nwrong",
    )
    .expect("mismatch");
    import(&store, &root).await;
    for paths in [
        vec!["0000.md", "bad.md"],
        vec!["0000.md", "missing.md"],
        vec!["0000.md", "../escape.md"],
        vec!["secret.md"],
        vec![],
    ] {
        let output = cli(json!({"op":"read-documents","scope":"personal","paths":paths})).await;
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    let before = store.calls();
    assert_eq!(
        store
            .read_context_documents(&scope(), &["credentials.md".into()])
            .await,
        Err(Error::Invalid)
    );
    assert_eq!(store.calls(), before);
}
#[tokio::test]
async fn native_unclosed_metadata_stays_unavailable_and_exact_cli_writes_nothing() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(1, b"# Safe");
    let malformed = [
        "---\ntitle: private-unclosed-marker\nBody without a closing delimiter",
        "---\r\ntitle: private-unclosed-marker\r\nBody without a closing delimiter",
        "\u{feff}---\ntitle: private-unclosed-marker\nBody without a closing delimiter",
    ];
    for (index, original) in malformed.iter().enumerate() {
        fs::write(root.join(format!("personal/unclosed-{index}.md")), original)
            .expect("synthetic unclosed metadata");
    }
    import(&store, &root).await;
    let status = store
        .projection_status(&[scope()])
        .await
        .expect("readiness");
    assert_eq!(status["ready"], false);
    assert_eq!(status["counts"]["searchable"], 1);
    assert_eq!(status["counts"]["unavailable"], malformed.len());
    for (index, original) in malformed.iter().enumerate() {
        let path = format!("unclosed-{index}.md");
        let payload: Value = sqlx::query_scalar(
            "SELECT p.payload FROM context_materials m JOIN context_projection_versions p USING(material_id,revision) WHERE m.scope='personal' AND m.path=$1",
        )
        .bind(&path)
        .fetch_one(store.pool())
        .await
        .expect("current unavailable projection");
        assert_eq!(
            payload,
            json!({"status":"unavailable","source_digest":ontology::store::digest(original.as_bytes()),"terms":[]})
        );
        assert_eq!(
            store
                .read_context(&scope(), &path, false)
                .await
                .expect("exact stored original"),
            *original,
            "import preserves the malformed original bytes"
        );
        for paths in [vec![path.clone()], vec!["0000.md".into(), path.clone()]] {
            let before = store.calls();
            assert_eq!(
                store.read_context_documents(&scope(), &paths).await,
                Err(Error::Invalid)
            );
            assert_eq!(store.calls() - before, 7, "one bounded failing batch");
            let output = cli(json!({"op":"read-documents","scope":"personal","paths":paths})).await;
            assert!(!output.status.success());
            assert!(
                output.stdout.is_empty(),
                "no partial valid document is written"
            );
            assert!(output.stderr.len() < 1024);
            assert!(!String::from_utf8_lossy(&output.stderr).contains("private-unclosed-marker"));
        }
    }
    for edges in [false, true] {
        assert_eq!(
            store
                .semantic_context(&[scope()], "private-unclosed-marker", 10, edges)
                .await,
            Err(Error::ContextProjectionUnavailable)
        );
    }
    let rebuilt = store
        .project_context(
            &[scope()],
            status["manifest_digest"].as_str().expect("manifest digest"),
        )
        .await
        .expect("verify unchanged unavailable projections");
    assert_eq!(rebuilt["inserted"], 0);
    assert_eq!(
        store
            .projection_status(&[scope()])
            .await
            .expect("same status"),
        status
    );
}

#[tokio::test]
async fn native_read_documents_rejects_work_absolute_paths_before_database_access() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(0, b"");
    fs::create_dir_all(root.join("work/acme")).expect("synthetic company scope");
    fs::write(
        root.join("work/acme/a.md"),
        b"---\nscope: work\n---\n# Work",
    )
    .expect("synthetic company document");
    let selected: ContextScope = "work/acme".parse().expect("exact company scope");
    let current = inventory(&root, std::slice::from_ref(&selected)).expect("company inventory");
    store
        .import_context(
            &root,
            std::slice::from_ref(&selected),
            &current.inventory_digest,
        )
        .await
        .expect("company import");
    let before = store.calls();
    let valid = store
        .read_context_documents(&selected, &["a.md".into()])
        .await
        .expect("relative company document remains readable");
    assert_eq!(valid["documents"][0]["path"], "work/acme/a.md");
    assert_eq!(store.calls() - before, 7);
    for count in [1, 100] {
        for absolute in ["/a.md", "//a.md", "/nested/a.md", "/work/acme/a.md"] {
            let mut paths: Vec<String> = (1..count).map(|n| format!("{n}.md")).collect();
            paths.push(absolute.into());
            let before = store.calls();
            assert_eq!(
                store.read_context_documents(&selected, &paths).await,
                Err(Error::Invalid)
            );
            assert_eq!(store.calls(), before, "reject the entire batch before SQL");
            let output =
                cli(json!({"op":"read-documents","scope":"work/acme","paths":paths})).await;
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
        }
    }
}
#[tokio::test]
async fn native_read_documents_response_limit_writes_nothing() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    // Original total is below 10 MiB, but repeating a long heading as title/body
    // makes the actual returned JSON exceed its independent 16 MiB budget.
    let (_fixture, root) = fixture(100, format!("# {}", "x".repeat(90000)).as_bytes());
    import(&store, &root).await;
    let paths: Vec<_> = (0..100).map(|n| format!("{n:04}.md")).collect();
    let before = store.calls();
    assert_eq!(
        store.read_context_documents(&scope(), &paths).await,
        Err(Error::Limit)
    );
    assert_eq!(
        store.calls() - before,
        7,
        "final response limit after body query"
    );
    let output = cli(json!({"op":"read-documents","scope":"personal","paths":paths})).await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[tokio::test]
async fn native_read_documents_checks_later_scope_after_response_budget_exceeded() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(100, format!("# {}", "x".repeat(90000)).as_bytes());
    fs::write(
        root.join("personal/0099.md"),
        b"---\nscope: work\n---\n# Wrong scope",
    )
    .unwrap();
    import(&store, &root).await;
    let paths: Vec<_> = (0..100).map(|n| format!("{n:04}.md")).collect();
    // Earlier documents already exceed the serialized response budget. A later
    // invalid source still takes precedence; no partially valid result escapes.
    let before = store.calls();
    assert_eq!(
        store.read_context_documents(&scope(), &paths).await,
        Err(Error::Invalid)
    );
    assert_eq!(store.calls() - before, 7);
    let output = cli(json!({"op":"read-documents","scope":"personal","paths":paths})).await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[tokio::test]
async fn native_read_documents_ignores_discarded_search_payload_size() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let content = format!(
        "# Unique\n{}\npassword: small-fictional-secret",
        (0..5000)
            .map(|n| format!("word{n:04} "))
            .collect::<String>()
    );
    assert!(content.len() < 65536);
    let (_fixture, root) = fixture(1, content.as_bytes());
    import(&store, &root).await;
    let payload: Value = sqlx::query_scalar(
        "SELECT payload FROM context_projection_versions p JOIN context_materials m USING(material_id,revision) WHERE m.scope='personal' AND m.path='0000.md'",
    )
    .fetch_one(store.pool())
    .await
    .expect("fictional search projection");
    assert!(serde_json::to_vec(&payload).unwrap().len() > 65536);
    let output = cli(json!({"op":"read-documents","scope":"personal","paths":["0000.md"]})).await;
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value["documents"][0]["source_digest"],
        ontology::store::digest(content.as_bytes())
    );
    assert_eq!(value["documents"][0]["title"], "Unique");
    assert!(
        value["documents"][0]["body"]
            .as_str()
            .unwrap()
            .ends_with("password: [REDACTED]")
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("small-fictional-secret"));
    assert!(value["documents"][0].get("terms").is_none());
}

#[tokio::test]
async fn native_read_documents_original_boundaries_and_unavailable_projection() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(0, b"");
    let header = "---\nscope: personal\n---\n# Boundary\n";
    for (name, length) in [
        ("below.md", MAX_READ_DOCUMENT_BYTES - 1),
        ("exact.md", MAX_READ_DOCUMENT_BYTES),
        ("over.md", MAX_READ_DOCUMENT_BYTES + 1),
    ] {
        fs::write(
            root.join("personal").join(name),
            format!("{header}{}", "x".repeat(length - header.len())),
        )
        .unwrap();
    }
    import(&store, &root).await;
    let status = store.projection_status(&[scope()]).await.unwrap();
    assert_eq!(status["counts"]["unavailable"], 3);
    for name in ["below.md", "exact.md"] {
        let original = fs::read(root.join("personal").join(name)).unwrap();
        let before = store.calls();
        let value = store
            .read_context_documents(&scope(), &[name.into()])
            .await
            .unwrap();
        assert_eq!(store.calls() - before, 7);
        assert_eq!(
            value["documents"][0]["source_digest"],
            ontology::store::digest(&original)
        );
        assert_eq!(value["documents"][0]["revision"], 1);
        assert_eq!(
            value["documents"][0]["source_path"],
            format!("personal/{name}")
        );
        assert_eq!(
            value["documents"][0]["body"].as_str().unwrap().len(),
            original.len() - "---\nscope: personal\n---\n".len()
        );
        drop(value);
        let output = cli(json!({"op":"read-documents","scope":"personal","paths":[name]})).await;
        assert!(output.status.success());
        assert!(output.stdout.len() <= MAX_OUTPUT_BYTES);
        serde_json::from_slice::<Value>(&output.stdout).unwrap();
        assert_eq!(
            store.read_context(&scope(), name, false).await,
            Err(Error::Limit),
            "raw limit remains 1MiB"
        );
    }
    let before = store.calls();
    assert_eq!(
        store
            .read_context_documents(&scope(), &["over.md".into()])
            .await,
        Err(Error::Limit)
    );
    assert_eq!(
        store.calls() - before,
        5,
        "oversized original rejected before body query"
    );
    let output =
        cli(json!({"op":"read-documents","scope":"personal","paths":["below.md","over.md"]})).await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        store.projection_status(&[scope()]).await.unwrap(),
        status,
        "exact read does not rebuild projections"
    );
}

#[tokio::test]
async fn native_read_documents_ten_mib_batch_boundary_preflights_before_bodies() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(0, b"");
    let paths: Vec<_> = (0..100).map(|n| format!("{n:04}.md")).collect();
    let over_paths: Vec<_> = (0..100).map(|n| format!("over/{n:04}.md")).collect();
    fs::create_dir(root.join("personal/over")).unwrap();
    for (n, path) in paths.iter().enumerate() {
        let length = MAX_READ_DOCUMENT_TOTAL_BYTES / 100
            + usize::from(n < MAX_READ_DOCUMENT_TOTAL_BYTES % 100);
        let text = format!("# Batch\n{}", "x".repeat(length - 8));
        fs::write(root.join("personal").join(path), &text).unwrap();
        let over = if n == 99 { format!("{text}x") } else { text };
        fs::write(root.join("personal").join(&over_paths[n]), over).unwrap();
    }
    import(&store, &root).await;
    let before = store.calls();
    let value = store
        .read_context_documents(&scope(), &paths)
        .await
        .unwrap();
    assert_eq!(store.calls() - before, 7);
    assert_eq!(value["documents"].as_array().unwrap().len(), 100);
    drop(value);
    let output = cli(json!({"op":"read-documents","scope":"personal","paths":paths})).await;
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["documents"]
            .as_array()
            .unwrap()
            .len(),
        100
    );
    let before = store.calls();
    assert_eq!(
        store.read_context_documents(&scope(), &over_paths).await,
        Err(Error::Limit)
    );
    assert_eq!(
        store.calls() - before,
        5,
        "oversized aggregate rejected before body query"
    );
    let output = cli(json!({"op":"read-documents","scope":"personal","paths":over_paths})).await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[tokio::test]
async fn native_read_documents_json_boundary_includes_cli_newline() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(0, b"");
    // UUIDs and digests have fixed-width encodings. Seed each boundary only once;
    // never modify an imported original just to construct a response-size case.
    let uuid = "00000000-0000-0000-0000-000000000000";
    let sha = "0".repeat(64);
    let skeleton = json!({"documents":[{"scope":"personal","path":"personal/0000.md","title":"Boundary","body":"# Boundary\nx","source_digest":sha,"material_id":uuid,"store_id":uuid,"revision":1,"origin_kind":"imported-file","origin_source_digest":sha,"source_path":"personal/0000.md"}]});
    let extra = MAX_OUTPUT_BYTES - 1 - serde_json::to_vec(&skeleton).unwrap().len();
    // ASCII control characters escape to six JSON bytes each, without changing
    // title or provenance field lengths. The original stays well below 10MiB.
    let mut content = format!(
        "# Boundary\nx{}{}",
        "\u{1}".repeat(extra / 6),
        "x".repeat(extra % 6)
    );
    assert!(content.len() < MAX_READ_DOCUMENT_BYTES);
    fs::write(root.join("personal/0000.md"), &content).unwrap();
    content.push('x');
    fs::write(root.join("personal/0001.md"), content).unwrap();
    import(&store, &root).await;
    let value = store
        .read_context_documents(&scope(), &["0000.md".into()])
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_vec(&value).unwrap().len(),
        MAX_OUTPUT_BYTES - 1
    );
    drop(value);
    let output = cli(json!({"op":"read-documents","scope":"personal","paths":["0000.md"]})).await;
    assert!(output.status.success());
    assert_eq!(output.stdout.len(), MAX_OUTPUT_BYTES);
    assert!(output.stdout.ends_with(b"\n"));
    serde_json::from_slice::<Value>(&output.stdout).unwrap();
    drop(output);
    let before = store.calls();
    assert_eq!(
        store
            .read_context_documents(&scope(), &["0001.md".into()])
            .await,
        Err(Error::Limit)
    );
    assert_eq!(store.calls() - before, 7, "late escaped response limit");
    let output = cli(json!({"op":"read-documents","scope":"personal","paths":["0001.md"]})).await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[tokio::test]
async fn native_read_documents_redaction_expansion_is_also_bounded() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let content = format!("# Expanded\n{}", "token:x\n".repeat(500_000));
    assert!(content.len() < MAX_READ_DOCUMENT_BYTES);
    let (_fixture, root) = fixture(2, content.as_bytes());
    import(&store, &root).await;
    let before = store.calls();
    assert_eq!(
        store
            .read_context_documents(&scope(), &["0000.md".into(), "0001.md".into()])
            .await,
        Err(Error::Limit)
    );
    assert_eq!(store.calls() - before, 7);
    let output =
        cli(json!({"op":"read-documents","scope":"personal","paths":["0000.md","0001.md"]})).await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[tokio::test]
async fn native_read_documents_limits_yaml_before_parsing_expanded_aliases() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(0, b"");
    for (name, length) in [
        ("metadata-below.md", MAX_READ_DOCUMENT_METADATA_BYTES - 1),
        ("metadata-exact.md", MAX_READ_DOCUMENT_METADATA_BYTES),
        ("metadata-over.md", MAX_READ_DOCUMENT_METADATA_BYTES + 1),
    ] {
        let prefix = "title: Meta\n#";
        let raw = format!("{prefix}{}", "p".repeat(length - prefix.len()));
        fs::write(
            root.join("personal").join(name),
            format!("---\n{raw}\n---\n# Meta\nbody"),
        )
        .unwrap();
    }
    let prefix = "---\naliases: [";
    let suffix = "a]\n---\n# Meta\nbody";
    let count = (MAX_READ_DOCUMENT_BYTES - prefix.len() - suffix.len()) / 2;
    let heavy = format!("{prefix}{}{suffix}", "a,".repeat(count));
    assert!(heavy.len() <= MAX_READ_DOCUMENT_BYTES);
    fs::write(root.join("personal/metadata-heavy.md"), heavy).unwrap();
    import(&store, &root).await;
    for name in ["metadata-below.md", "metadata-exact.md"] {
        let output = cli(json!({"op":"read-documents","scope":"personal","paths":[name]})).await;
        assert!(output.status.success());
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap()["documents"][0]["body"],
            "# Meta\nbody"
        );
    }
    for name in ["metadata-over.md", "metadata-heavy.md"] {
        let before = store.calls();
        assert_eq!(
            store.read_context_documents(&scope(), &[name.into()]).await,
            Err(Error::Limit)
        );
        assert_eq!(store.calls() - before, 7);
        let output = cli(json!({"op":"read-documents","scope":"personal","paths":[name]})).await;
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Operation limit exceeded"));
    }
}

#[tokio::test]
async fn native_read_documents_preflights_body_without_changing_formatted_titles() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let header = "# **Visible** `title`\n";
    let escaped = format!("{header}{}", "\u{1}".repeat(3 * 1024 * 1024));
    let (_fixture, root) = fixture(1, escaped.as_bytes());
    let formatted = format!("\n{header}{}\n", "*text* ".repeat(150_000));
    fs::write(root.join("personal/formatted.md"), &formatted).unwrap();
    import(&store, &root).await;
    let output = cli(json!({"op":"read-documents","scope":"personal","paths":["0000.md"]})).await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let output =
        cli(json!({"op":"read-documents","scope":"personal","paths":["formatted.md"]})).await;
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["documents"][0]["title"], "Visible title");
    assert_eq!(value["documents"][0]["body"], formatted.trim());
    assert_eq!(
        value["documents"][0]["source_digest"],
        ontology::store::digest(formatted.as_bytes())
    );
}
#[tokio::test]
async fn native_provider_is_lazy_batched_and_preserves_inode() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(2000, b"# Selected\nbody");
    import(&store, &root).await;
    let view = tempfile::tempdir().expect("view");
    fs::set_permissions(view.path(), fs::Permissions::from_mode(0o700)).expect("owned view");
    let view_path = view.path().canonicalize().expect("view root");
    let observed = store.clone();
    store
        .with_native_context(view_path, move |source| {
            for count in [1, 2000] {
                let paths: Vec<_> = (0..count)
                    .map(|n| PathBuf::from(format!("vault/personal/{n:04}.md")))
                    .collect();
                let before = source.metadata_queries();
                assert_eq!(
                    source
                        .source_versions(&paths)
                        .expect("batch metadata")
                        .len(),
                    count
                );
                assert_eq!(source.metadata_queries() - before, 1);
                assert_eq!(source.body_queries(), 0);
            }
            let p = Path::new("vault/personal/0000.md");
            assert_eq!(
                source.metadata(p).expect("metadata").kind,
                SourcePathKind::RegularFile
            );
            assert_eq!(source.body_queries(), 0);
            let paths = [
                "vault/personal/1999.md",
                "vault/personal/missing.md",
                "vault/personal/0000.md",
                "vault/personal/1999.md",
            ]
            .map(PathBuf::from);
            let versions = source
                .stored_versions(&paths)
                .expect("ordered sparse duplicates");
            assert_eq!(
                versions
                    .iter()
                    .map(|v| v.logical_path.as_str())
                    .collect::<Vec<_>>(),
                paths
                    .iter()
                    .map(|p| p.to_str().unwrap())
                    .collect::<Vec<_>>()
            );
            assert_eq!(versions[0], versions[3]);
            assert!(matches!(
                versions[1].state,
                context_core::harness::StoredSourceState::Missing
            ));
            let calls = observed.calls();
            let first = source.open_file(p, 65536).expect("selected bytes");
            let second = source.open_file(p, 65536).expect("repeat selected");
            assert_eq!(
                observed.calls() - calls,
                3,
                "fresh metadata per open; verified body query once per shared gate"
            );
            assert_eq!(
                first.metadata().expect("inode").ino(),
                second.metadata().expect("inode").ino()
            );
            assert_eq!(source.body_queries(), 1);
            assert!(!source.view_root().join("vault/personal/0001.md").exists());
            assert!(source.children(Path::new("vault/personal"), 1999).is_err());
            assert!(source.open_file(p, 1).is_err());
            assert_eq!(
                source.body_queries(),
                1,
                "a smaller limit cannot bypass the cache bound"
            );
            let materialized = source.view_root().join(p);
            fs::remove_file(&materialized).unwrap();
            std::os::unix::fs::symlink("/never-opened", &materialized).unwrap();
            assert!(
                source.open_file(p, 65536).is_err(),
                "a cache hit still rejects view links"
            );
            fs::remove_file(&materialized).unwrap();
            source.open_file(p, 65536).unwrap();
            for n in 1..=256 {
                source
                    .open_file(Path::new(&format!("vault/personal/{n:04}.md")), 65536)
                    .unwrap();
            }
            let before = source.body_queries();
            source
                .open_file(Path::new("vault/personal/0256.md"), 65536)
                .unwrap();
            assert_eq!(
                source.body_queries() - before,
                1,
                "entry cap skips caching without rejecting a valid read"
            );
            Ok(())
        })
        .await
        .expect("native session");
}

#[tokio::test]
async fn native_career_discovery_batches_inventory_and_rejects_forged_or_stale_comparisons() {
    use context_core::{
        career::*,
        harness::{CareerOutputSurface, DataOwner},
    };
    let _guard = TEST_LOCK.lock().await;
    let mut store = store().await;
    let fixture = context_fixture::build();
    let root = fixture.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("personal/facts/cases")).unwrap();
    let body = "# Case\nTogether we discussed design and reduced contribution burden.\n";
    for n in 0..100 {
        fs::write(root.join(format!("personal/facts/cases/{n:04}.md")), body).unwrap();
    }
    let request = CareerDiscoveryRequest {
        original_request: "Describe helping others".into(),
        surface: CareerOutputSurface::General,
        approved_scopes: vec!["personal".into()],
        axis_queries: vec![CareerAxisQuery {
            axis: CareerAxis::Competency,
            query: "does-not-match".into(),
        }],
    };
    let mut one_wave_queries = None;
    let mut full_inventory = None;
    for count in [1, 100] {
        if count == 100 {
            store = self::store().await;
        }
        let routes = (0..count)
            .map(|n| CareerRoute {
                owner: DataOwner::Personal,
                kind: CareerRouteKind::Fact,
                path: format!("vault/personal/facts/cases/{n:04}.md"),
                label: "case".into(),
                supporting_sources: vec![],
            })
            .collect::<Vec<_>>();
        fs::write(
            root.join("personal/writing/evidence.md"),
            format!("```career-routes\n{}\n```\n", json!({"routes":routes})),
        )
        .unwrap();
        import(&store, &root).await;
        let before = store.calls();
        let result = store
            .career_candidates(
                &context_fixture::configuration_path(),
                request.clone(),
                None,
                false,
            )
            .await
            .unwrap();
        let queries = store.calls() - before;
        assert_eq!(
            queries,
            *one_wave_queries.get_or_insert(queries),
            "fixed wave count, not one query per candidate"
        );
        result.inventory.require_complete().unwrap();
        assert_eq!(result.inventory.candidates.len(), count + 1);
        assert!(
            result
                .inventory
                .candidates
                .iter()
                .all(|c| c.matches.is_empty()),
            "no-match is not exclusion"
        );
        assert!(!result.finalization_structure_checked);
        assert!(result.semantic_review_required);
        full_inventory = Some(result.inventory);
    }
    let inventory = full_inventory.unwrap();
    let selected = "vault/personal/facts/cases/0000.md";
    let comparison = CareerComparison {
        requirements: vec![CareerRequirement {
            id: "help".into(),
            original_span: CareerSpan {
                start: 0,
                end: request.original_request.len(),
            },
            kind: CareerRequirementKind::Requirement,
            coverage: CareerCoverage::Direct,
            material: true,
        }],
        decisions: inventory
            .candidates
            .iter()
            .map(|c| CareerDecision {
                requirement_id: "help".into(),
                candidate_path: c.path.clone(),
                disposition: if c.path == selected {
                    CareerDisposition::Selected
                } else {
                    CareerDisposition::Excluded
                },
                reason: "The exact original facet is available for independent interpretation"
                    .into(),
                facets: vec![CareerFacet {
                    axis: CareerAxis::Competency,
                    original: CareerLocator {
                        path: c.path.clone(),
                        original_span: CareerSpan { start: 0, end: 5 },
                    },
                }],
            })
            .collect(),
        placements: vec![CareerPlacement {
            claim_id: "help".into(),
            candidate_path: selected.into(),
            requirement_ids: vec!["help".into()],
            source_locators: vec![CareerLocator {
                path: selected.into(),
                original_span: CareerSpan {
                    start: 7,
                    end: body.len() - 1,
                },
            }],
            target: "answer.md".into(),
            slot: "help".into(),
        }],
        inventory,
    };
    let command = |comparison: &CareerComparison| json!({"op":"career-candidates","original_request":request.original_request,"surface":"general","approved_scopes":["personal"],"axis_queries":request.axis_queries,"policy_config":context_fixture::configuration_path(),"comparison":comparison,"finalization":true});
    let output = cli(command(&comparison)).await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["finalization_structure_checked"],
        true
    );
    let mut forged = comparison.clone();
    let removed = forged.inventory.candidates.pop().unwrap();
    forged
        .decisions
        .retain(|d| d.candidate_path != removed.path);
    forged.inventory.inventory_digest = forged.inventory.content_digest().unwrap();
    let output = cli(command(&forged)).await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    // Negative storage fixture: same original bytes with a newer native source identity.
    sqlx::raw_sql("ALTER TABLE context_materials DISABLE TRIGGER context_revision; ALTER TABLE context_materials DISABLE TRIGGER context_version; UPDATE context_materials SET revision=revision+1 WHERE source_path='personal/facts/cases/0000.md'; ALTER TABLE context_materials ENABLE TRIGGER context_revision; ALTER TABLE context_materials ENABLE TRIGGER context_version;").execute(store.pool()).await.unwrap();
    let output = cli(command(&comparison)).await;
    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "stale comparison cannot produce a partial acceptance"
    );
}
#[tokio::test]
async fn native_projection_is_explicit_current_and_redacted() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(
        1,
        b"---\naliases: [nativealias]\nexport: false\n---\n# Context\nhello nativealias",
    );
    import(&store, &root).await;
    let status = store.projection_status(&[scope()]).await.expect("status");
    assert_eq!(status["ready"], true);
    let results = store
        .semantic_context(&[scope()], "nativealias", 10, false)
        .await
        .expect("semantic");
    assert_eq!(results["items"].as_array().expect("items").len(), 1);
    assert_eq!(results["items"][0]["exportable"], false);
    let result = store
        .project_context(
            &[scope()],
            status["manifest_digest"].as_str().expect("digest"),
        )
        .await
        .expect("verify immutable projection");
    assert_eq!(result["inserted"], 0);
    assert_eq!(
        store.project_context(&[scope()], &"a".repeat(64)).await,
        Err(Error::Conflict)
    );
}

#[tokio::test]
async fn native_core_resolve_prepare_uses_only_canonical_store_source_versions() {
    use context_core::harness::{
        CurationKind, DataOwner, DecisionRecord, DecisionTrace, DraftTaskRequest, DraftValue,
        DraftVaultReadContract, HARNESS_SCHEMA_VERSION, HarnessEngine, HarnessExecutionProfile,
        HarnessPlan, HarnessRole, HarnessRuntimeCapabilities, PolicyDefaultRule,
        PreparedHarnessRun, RequestEnvelope, RequestSource, RoleLifecycleLimits,
        UserConfirmationStatus, UserStatement,
    };
    use serde::Serialize;
    fn decision<T: Serialize>(
        value: T,
        n: u64,
        records: &mut Vec<DecisionRecord>,
    ) -> DraftValue<T> {
        let identifier = format!("decision-{n:016x}");
        records.push(DecisionRecord::UserStatement {
            identifier: identifier.clone(),
            value_digest: ontology::store::digest(
                &serde_json::to_vec(&value).expect("serializable decision"),
            ),
            statement_identifiers: vec!["statement-0000000000000001".into()],
        });
        DraftValue::Resolved {
            value,
            decision_identifier: identifier,
        }
    }
    fn default<T: Serialize>(
        value: T,
        n: u64,
        rule: PolicyDefaultRule,
        policy_sha: &str,
        records: &mut Vec<DecisionRecord>,
    ) -> DraftValue<T> {
        let identifier = format!("decision-{n:016x}");
        records.push(DecisionRecord::PolicyDefault {
            identifier: identifier.clone(),
            value_digest: ontology::store::digest(
                &serde_json::to_vec(&value).expect("serializable default"),
            ),
            policy_identifier: "control".into(),
            policy_content_digest: policy_sha.into(),
            rule,
        });
        DraftValue::Resolved {
            value,
            decision_identifier: identifier,
        }
    }
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let synthetic = context_fixture::build();
    let root = synthetic
        .path()
        .canonicalize()
        .expect("synthetic fixture root");
    let fixture = root.clone();
    let selected: Vec<ContextScope> = context_fixture::SCOPES
        .iter()
        .map(|s| s.parse().expect("declared synthetic scope"))
        .collect();
    let current = inventory(&fixture, &selected).expect("synthetic test fixture");
    assert_eq!(current.entries.len(), context_fixture::DOCUMENT_COUNT);
    assert_eq!(current.total_bytes, context_fixture::TOTAL_BYTES);
    let mut bindings = String::new();
    for (path, expected) in context_fixture::documents() {
        let bytes = fs::read(root.join(path)).expect("declared synthetic fixture file");
        assert_eq!(bytes, expected.as_bytes(), "declared bytes for {path}");
        bindings.push_str(&format!(
            "{path}\t{}\t{}\n",
            bytes.len(),
            ontology::store::digest(&bytes)
        ));
    }
    assert_eq!(
        ontology::store::digest(bindings.as_bytes()),
        context_fixture::CONTRACT_DIGEST,
        "declared fixture paths, lengths, and content digests"
    );
    store
        .import_context(&fixture, &selected, &current.inventory_digest)
        .await
        .expect("canonical fixture import");
    let policy_sha = ontology::store::digest(
        &fs::read(fixture.join("profile/rules/control.md")).expect("selected policy original"),
    );
    let view = tempfile::tempdir().expect("view");
    fs::set_permissions(view.path(), fs::Permissions::from_mode(0o700)).expect("owned view");
    let root = view.path().canonicalize().expect("view root");
    let prepared = store
        .with_native_context(root.clone(), move |source| {
            let mut records = Vec::new();
            let envelope = RequestEnvelope {
                version: HARNESS_SCHEMA_VERSION,
                source: RequestSource::UserLanguage {
                    statements: vec![UserStatement {
                        identifier: "statement-0000000000000001".into(),
                        text: "Read my sample personal project context.".into(),
                    }],
                },
                draft: DraftTaskRequest::VaultRead(DraftVaultReadContract {
                    owner: decision(
                        DataOwner::PersonalProject {
                            project: "sample".into(),
                        },
                        1,
                        &mut records,
                    ),
                    task_statement: decision(
                        "Read my sample personal project context.".to_owned(),
                        2,
                        &mut records,
                    ),
                    execution_profile: default(
                        HarnessExecutionProfile::Standard,
                        3,
                        PolicyDefaultRule::StandardExecutionProfile,
                        &policy_sha,
                        &mut records,
                    ),
                    curation_kind: decision(CurationKind::Knowledge, 4, &mut records),
                    query: decision("sample".to_owned(), 5, &mut records),
                    maximum_context_bytes: decision(65536, 6, &mut records),
                    confirmation: default(
                        UserConfirmationStatus::NotRequired,
                        7,
                        PolicyDefaultRule::NonJournalReadConfirmationNotRequired,
                        &policy_sha,
                        &mut records,
                    ),
                }),
                decision_trace: DecisionTrace { records },
            };
            let engine = HarnessEngine::with_source(
                source.view_root(),
                source.view_root(),
                source.clone(),
                context_fixture::configuration(),
            )
            .expect("real Core source engine");
            let resolved = engine
                .resolve_envelope(&envelope, None)
                .expect("real Core resolution");
            let lifecycle = RoleLifecycleLimits {
                max_role_execution_millis: 60000,
                max_role_grace_millis: 1000,
                max_role_close_millis: 1000,
                max_total_role_millis: 62000,
            };
            let plan = HarnessPlan::from_resolved(resolved, source.view_root(), lifecycle.clone())
                .expect("real Core plan");
            assert!(plan.bound_source_versions().store_identity.is_some());
            assert!(!plan.bound_source_versions().versions.is_empty());
            let bytes = serde_json::to_vec(&plan).expect("exact plan serialization");
            let capabilities = HarnessRuntimeCapabilities {
                version: HARNESS_SCHEMA_VERSION,
                available_roles: vec![
                    HarnessRole::Writer,
                    HarnessRole::Verifier,
                    HarnessRole::Reviewer,
                    HarnessRole::Specialist,
                ],
                max_concurrent_roles: 1,
                separate_contexts: true,
                file_reading: true,
                tool_execution: true,
                deterministic_validation: true,
                max_role_bundle_bytes: 8 * 1024 * 1024,
                max_role_invocation_bytes: 16 * 1024 * 1024,
                lifecycle,
            };
            let prepared =
                PreparedHarnessRun::prepare(&engine, &bytes, plan, capabilities, None, None)
                    .expect("real Core preparation");
            let prepared_bytes =
                serde_json::to_vec(&prepared).expect("exact prepared serialization");
            prepared
                .validate_with_engine(&prepared_bytes, &engine)
                .expect("actual version revalidation");
            assert!(source.body_queries() > 0);
            Ok(std::sync::Arc::new(prepared))
        })
        .await
        .expect("real native Core session");
    for _ in 0..2 {
        // A deliberately corrupted metadata fixture tests rejection, never a claimed native commit.
        // The original bytes and digest stay identical while the selected policy revision changes.
        sqlx::raw_sql("ALTER TABLE context_materials DISABLE TRIGGER context_revision; ALTER TABLE context_materials DISABLE TRIGGER context_version; UPDATE context_materials SET revision=revision+1 WHERE scope='profile' AND path='rules/control.md'; ALTER TABLE context_materials ENABLE TRIGGER context_revision; ALTER TABLE context_materials ENABLE TRIGGER context_version;").execute(store.pool()).await.expect("same-byte revision drift fixture");
        let prepared = prepared.clone();
        store
            .with_native_context(root.clone(), move |source| {
                let engine = HarnessEngine::with_source(
                    source.view_root(),
                    source.view_root(),
                    source.clone(),
                    context_fixture::configuration(),
                )
                .expect("fresh source engine");
                let raw = serde_json::to_vec(&*prepared).expect("unchanged prepared receipt");
                assert!(
                    prepared.validate_with_engine(&raw, &engine).is_err(),
                    "same original SHA never conceals source revision drift"
                );
                Ok(())
            })
            .await
            .expect("version drift rejected");
    }
    assert_eq!(
        inventory(&fixture, &selected)
            .expect("synthetic fixture unchanged")
            .entries,
        current.entries
    );
}

#[test]
fn native_parser_retains_declared_scope_and_exact_normalization_boundaries() {
    use context_core::{
        Scope, context::normalize_exact_read_paths, document::parse_markdown_bytes,
    };
    for (text, expected) in [
        ("# Title", None),
        ("---\nscope: personal\n---\n# Title", Some("personal")),
        ("---\nscope: work\n---\n# Title", Some("work")),
    ] {
        let parsed =
            parse_markdown_bytes(Path::new("personal/a.md"), text.as_bytes()).expect("pure parse");
        assert_eq!(parsed.declared_scope(), expected);
        assert_eq!(
            parsed.original_content_digest(),
            ontology::store::digest(text.as_bytes())
        );
    }
    assert!(
        parse_markdown_bytes(
            Path::new("personal/a.md"),
            b"---\nscope: [personal]\n---\nwrong"
        )
        .is_err()
    );
    let path = format!("{}/{}.md", "a".repeat(250), "b".repeat(249));
    assert_eq!(
        normalize_exact_read_paths(Scope::Personal, &[PathBuf::from(&path)])
            .expect("512 inclusive")[0]
            .as_os_str()
            .len(),
        512
    );
    assert!(
        normalize_exact_read_paths(Scope::Personal, &[PathBuf::from(format!("x{path}"))]).is_err()
    );
    for bad in [
        "../outside.md",
        ".hidden/a.md",
        "journal/a.md",
        "secret.md",
        "credentials.md",
        "target/a.md",
        "a/*.md",
        "a/../b.md",
    ] {
        assert!(
            normalize_exact_read_paths(Scope::Personal, &[PathBuf::from(bad)]).is_err(),
            "excluded path {bad}"
        );
    }
    assert!(
        normalize_exact_read_paths(
            Scope::Personal,
            &[PathBuf::from("a.md"), PathBuf::from("personal/a.md")]
        )
        .is_err()
    );
}

#[tokio::test]
async fn native_provider_rejects_foreign_views_links_and_file_descendant_collisions() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(1, b"# Original");
    import(&store, &root).await;
    let view = tempfile::tempdir().expect("view");
    let path = view.path().canonicalize().expect("root");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("private root");
    fs::write(path.join("unexpected.txt"), b"not ours").expect("unmarked content");
    assert!(
        store
            .with_native_context(path.clone(), |_| Ok(()))
            .await
            .is_err()
    );
    fs::remove_file(path.join("unexpected.txt")).expect("remove synthetic object");
    store
        .with_native_context(path.clone(), |source| {
            source
                .open_file(Path::new("vault/personal/0000.md"), 65536)
                .expect("materialized");
            Ok(())
        })
        .await
        .expect("session");
    let original = path.join("vault/personal/0000.md");
    let hardlink = path.join("linked.md");
    fs::hard_link(&original, &hardlink).expect("synthetic hardlink");
    assert!(
        store
            .with_native_context(path.clone(), |source| {
                source
                    .open_file(Path::new("vault/personal/0000.md"), 65536)
                    .map(|_| ())
                    .map_err(|_| Error::Invalid)
            })
            .await
            .is_err()
    );
    fs::remove_file(hardlink).expect("remove link");
    fs::remove_file(&original).expect("remove view file");
    std::os::unix::fs::symlink(root.join("personal/0000.md"), &original)
        .expect("synthetic symlink");
    assert!(
        store
            .with_native_context(path.clone(), |source| {
                source
                    .open_file(Path::new("vault/personal/0000.md"), 65536)
                    .map(|_| ())
                    .map_err(|_| Error::Invalid)
            })
            .await
            .is_err()
    );
    fs::remove_file(original).expect("remove link");
    fs::write(path.join(".context-store"), "foreign-store").expect("foreign marker");
    assert!(
        store
            .with_native_context(path.clone(), |_| Ok(()))
            .await
            .is_err()
    );
    let bytes = b"nested";
    sqlx::query("INSERT INTO context_materials(scope,path,source_root,source_path,source_digest,content_digest,content,byte_len,restricted,search_text) VALUES('personal','0000.md/child.md','/synthetic','personal/0000.md/child.md',$1,$1,$2,6,false,'nested')").bind(ontology::store::digest(bytes)).bind(bytes.as_slice()).execute(store.pool()).await.expect("stored namespace collision");
    let view = tempfile::tempdir().expect("collision view");
    fs::set_permissions(view.path(), fs::Permissions::from_mode(0o700)).expect("private root");
    store
        .with_native_context(view.path().canonicalize().expect("view"), |source| {
            assert!(
                source
                    .metadata(Path::new("vault/personal/0000.md"))
                    .is_err()
            );
            assert_eq!(source.body_queries(), 0);
            assert!(
                source
                    .open_file(Path::new("vault/personal/0000.md"), 65536)
                    .is_err()
            );
            assert_eq!(
                source.body_queries(),
                0,
                "live file with children never returns a body"
            );
            Ok(())
        })
        .await
        .expect("collision rejected without body");
}

#[tokio::test]
async fn native_provider_rejects_live_ancestors_before_direct_descendant_access() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    for count in [1, 2000] {
        sqlx::query("TRUNCATE document_grouping,document_subjects,context_source_bindings,context_projection_versions,context_material_versions,context_manual_edits,context_materials,context_apply_batches")
            .execute(store.pool()).await.expect("owned collision fixture reset");
        let (_fixture, root) = fixture(count, b"# Original");
        import(&store, &root).await;
        let bytes = b"nested";
        sqlx::query("INSERT INTO context_materials(scope,path,source_root,source_path,source_digest,content_digest,content,byte_len,restricted,search_text) VALUES('personal','0000.md/child.md','/synthetic','personal/0000.md/child.md',$1,$1,$2,6,false,'nested')")
            .bind(ontology::store::digest(bytes)).bind(bytes.as_slice())
            .execute(store.pool()).await.expect("stored file and descendant collision");
        let view = tempfile::tempdir().expect("fresh collision view");
        fs::set_permissions(view.path(), fs::Permissions::from_mode(0o700)).expect("private root");
        let observed = store.clone();
        store
            .with_native_context(view.path().canonicalize().expect("view"), move |source| {
                for path in [
                    "vault/personal/0000.md/child.md",
                    "vault/personal/0000.md/missing.md",
                ] {
                    let path = Path::new(path);
                    for operation in 0..3 {
                        let before = observed.calls();
                        let metadata_before = source.metadata_queries();
                        let result = match operation {
                            0 => source.metadata(path).map(|_| ()),
                            1 => source.validate_regular_file(path).map(|_| ()),
                            _ => source.open_file(path, 65536).map(|_| ()),
                        };
                        assert!(
                            result.is_err(),
                            "direct descendant must reject a live ancestor"
                        );
                        assert_eq!(observed.calls() - before, 1, "one bounded metadata SQL");
                        assert_eq!(source.metadata_queries() - metadata_before, 1);
                        assert_eq!(source.body_queries(), 0);
                        assert!(
                            !source.view_root().join("vault").exists(),
                            "no projection parents"
                        );
                    }
                }
                Ok(())
            })
            .await
            .expect("direct descendant rejected before materialization");
    }
}

#[tokio::test]
async fn native_projection_status_and_rebuild_are_atomic_and_batched() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    for count in [1, 101] {
        sqlx::query("TRUNCATE document_grouping,document_subjects,context_source_bindings,context_projection_versions,context_material_versions,context_manual_edits,context_materials,context_apply_batches").execute(store.pool()).await.expect("reset");
        let (_fixture, root) = fixture(
            count,
            b"---\nexport: false\naliases: [growth]\n---\n# Native\nGrowth",
        );
        let i = inventory(&root, &[scope()]).expect("inventory");
        let before = store.calls();
        store
            .import_context(&root, &[scope()], &i.inventory_digest)
            .await
            .expect("batch import");
        let pages = count.div_ceil(100) as u64;
        assert_eq!(
            store.calls() - before,
            5 + 3 * pages,
            "one insert and two projection queries per page"
        );
        let before = store.calls();
        assert_eq!(
            store
                .import_context(&root, &[scope()], &i.inventory_digest)
                .await
                .expect("no-op")["inserted"],
            0
        );
        assert_eq!(
            store.calls() - before,
            5 + pages,
            "no-op adds no projection calls"
        );
        sqlx::query("TRUNCATE context_projection_versions")
            .execute(store.pool())
            .await
            .expect("owned missing projection fixture");
        assert_eq!(
            store
                .semantic_context(&[scope()], "growth", 10, false)
                .await,
            Err(Error::ContextProjectionUnavailable)
        );
        let status = store
            .projection_status(&[scope()])
            .await
            .expect("missing status");
        assert_eq!(status["ready"], false);
        let before = store.calls();
        store
            .project_context(
                &[scope()],
                status["manifest_digest"].as_str().expect("manifest"),
            )
            .await
            .expect("explicit rebuild");
        assert_eq!(
            store.calls() - before,
            7 + 3 * pages,
            "bounded page queries plus refresh permission and final manifest"
        );
        let original:Value=sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(p) ORDER BY material_id,revision) FROM context_projection_versions p").fetch_one(store.pool()).await.expect("projection snapshot");
        assert!(
            sqlx::query(
                "UPDATE context_projection_versions SET payload=payload||'{\"title\":\"tamper\"}'"
            )
            .execute(store.pool())
            .await
            .is_err()
        );
        assert!(
            sqlx::query("DELETE FROM context_projection_versions")
                .execute(store.pool())
                .await
                .is_err()
        );
        let status = store
            .projection_status(&[scope()])
            .await
            .expect("current status");
        assert_eq!(
            store.project_context(&[scope()], &"0".repeat(64)).await,
            Err(Error::Conflict)
        );
        store
            .project_context(
                &[scope()],
                status["manifest_digest"].as_str().expect("manifest"),
            )
            .await
            .expect("verify immutable payloads");
        let unchanged:Value=sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(p) ORDER BY material_id,revision) FROM context_projection_versions p").fetch_one(store.pool()).await.expect("unchanged projection");
        assert_eq!(original, unchanged);
    }
}

#[tokio::test]
async fn native_migration_007_preserves_006_rows_and_rolls_back_failed_upgrade() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(2, b"# Historical original\nexact bytes");
    import(&store, &root).await;
    // Historical fixture metadata is deliberately not a Core commit/acceptance proof.
    sqlx::query("WITH i AS (SELECT gen_random_uuid() AS id) INSERT INTO context_apply_batches(apply_id,store_id,core_run_id,prepared_run_digest,candidate_digest,expected_source_versions,context_targets,core_apply_attempt_id,expected_batch_id,expected_journal_locator,state) SELECT i.id,s.store_id,'historical-fixture',repeat('a',64),repeat('b',64),'{}','[{}]',i.id::text,i.id::text,i.id::text,'pending' FROM i CROSS JOIN context_store s").execute(store.pool()).await.expect("legacy batch fixture");
    sqlx::query("UPDATE context_apply_batches SET state='finalized',actual_batch_id=expected_batch_id,actual_journal_locator=expected_journal_locator,commit_receipt='{\"historical_fixture\":true}',final_core_receipt_digest=repeat('c',64)").execute(store.pool()).await.expect("historical terminal metadata fixture");
    sqlx::raw_sql("ALTER TABLE memory_history DROP COLUMN grouping_only; DELETE FROM ontology_migrations WHERE name='016-memory-grouping-history.sql'; DROP TABLE document_grouping; DELETE FROM ontology_migrations WHERE name='015-document-grouping.sql'; DROP TABLE document_subjects; DROP TABLE document_subject_history; DROP TABLE subject_history; ALTER TABLE subjects DROP COLUMN definition, DROP COLUMN revision; ALTER TABLE source_records DROP COLUMN reference_paths; DELETE FROM ontology_migrations WHERE name='013-knowledge-taxonomy.sql'; DROP FUNCTION context_projection_refresh_guard() CASCADE; DELETE FROM ontology_migrations WHERE name='012-context-projection-refresh.sql'; DELETE FROM ontology_migrations WHERE name='014-grouping-claim-ownership.sql'; DROP TABLE memory_grouping; DELETE FROM ontology_migrations WHERE name='011-personal-memory-grouping.sql'; DELETE FROM ontology_migrations WHERE name IN ('009-context-manual-edits.sql','010-profile-manual-edits.sql'); ALTER TABLE context_materials DROP COLUMN last_manual_edit_id; ALTER TABLE context_material_versions DROP COLUMN manual_edit_id; DROP TABLE context_manual_edits; DROP FUNCTION context_manual_edit_complete(); DROP TRIGGER context_invalidate_consumers ON context_materials; DROP TABLE context_source_bindings; DROP FUNCTION context_invalidate_consumers(); DROP FUNCTION context_source_revision(uuid,uuid,bigint,boolean,text,text,text); ALTER TABLE sources DROP CONSTRAINT sources_kind_check; ALTER TABLE sources ADD CONSTRAINT sources_kind_check CHECK(kind IN ('git','vault')); ALTER TABLE sources DROP CONSTRAINT sources_failure_code_check; ALTER TABLE sources ADD CONSTRAINT sources_failure_code_check CHECK(failure_code IS NULL OR (kind='git' AND failure_code='git-read-failed') OR (kind='vault' AND failure_code='vault-read-failed')); ALTER TABLE sources ADD CONSTRAINT sources_vault_status_check CHECK(kind<>'vault' OR status<>'missing'); DELETE FROM ontology_migrations WHERE name='008-context-consumers.sql'; DROP TABLE context_projection_versions; DROP FUNCTION context_projection_validate(); DROP TRIGGER context_core_contract_immutable ON context_apply_batches; DROP FUNCTION context_core_contract_immutable(); ALTER TABLE context_apply_batches DROP COLUMN core_contract, DROP COLUMN core_contract_digest; DELETE FROM ontology_migrations WHERE name='007-context-native.sql';").execute(store.pool()).await.expect("owned pre-007 fixture");
    let snapshot_sql = "SELECT jsonb_build_object('store',(SELECT to_jsonb(s) FROM context_store s),'materials',(SELECT jsonb_agg(to_jsonb(m)-'last_manual_edit_id' ORDER BY scope,path) FROM context_materials m),'history',(SELECT jsonb_agg(to_jsonb(v)-'manual_edit_id' ORDER BY material_id,revision) FROM context_material_versions v),'batches',(SELECT jsonb_agg(to_jsonb(b)-ARRAY['core_contract','core_contract_digest'] ORDER BY apply_id) FROM context_apply_batches b))";
    let before: Value = sqlx::query_scalar(snapshot_sql)
        .fetch_one(store.pool())
        .await
        .expect("all original native storage columns");
    sqlx::query("ALTER TABLE context_apply_batches ADD COLUMN core_contract bytea")
        .execute(store.pool())
        .await
        .expect("deliberate later DDL conflict");
    assert_eq!(store.initialize().await, Err(Error::Baseline));
    let absent:bool=sqlx::query_scalar("SELECT to_regclass('context_projection_versions') IS NULL AND to_regprocedure('context_projection_validate()') IS NULL AND NOT EXISTS(SELECT 1 FROM ontology_migrations WHERE name='007-context-native.sql')").fetch_one(store.pool()).await.expect("transactional DDL rollback");
    assert!(absent);
    sqlx::query("ALTER TABLE context_apply_batches DROP COLUMN core_contract")
        .execute(store.pool())
        .await
        .expect("remove intentional conflict");
    assert_eq!(
        sqlx::query_scalar::<_, Value>(snapshot_sql)
            .fetch_one(store.pool())
            .await
            .expect("failure preserved values"),
        before
    );
    store.initialize().await.expect("actual 006 to007 upgrade");
    assert_eq!(
        sqlx::query_scalar::<_, Value>(snapshot_sql)
            .fetch_one(store.pool())
            .await
            .expect("every old column preserved"),
        before
    );
    let unchanged:bool=sqlx::query_scalar("SELECT bool_and(core_contract IS NULL AND core_contract_digest IS NULL) FROM context_apply_batches").fetch_one(store.pool()).await.expect("no invented legacy contract");
    assert!(unchanged);
    assert!(sqlx::query("UPDATE context_apply_batches SET core_contract='invented'::bytea,core_contract_digest=encode(sha256('invented'::bytea),'hex')").execute(store.pool()).await.is_err(),"cannot retrofit acceptance bytes");
}

#[tokio::test]
async fn native_source_gate_blocks_mutation_and_projection_work_until_session_closes() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(1, b"# Gate");
    import(&store, &root).await;
    let status = store.projection_status(&[scope()]).await.expect("manifest");
    let view = tempfile::tempdir().expect("view");
    fs::set_permissions(view.path(), fs::Permissions::from_mode(0o700)).expect("owned view");
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let reader = store.clone();
    let view_path = view.path().canonicalize().expect("view");
    let session = tokio::spawn(async move {
        reader
            .with_native_context(view_path, move |source| {
                source
                    .metadata(Path::new("vault/personal/0000.md"))
                    .expect("metadata");
                entered_tx.send(()).expect("signal entered");
                release_rx.recv().expect("release gate");
                Ok(())
            })
            .await
    });
    entered_rx.await.expect("active native source");
    let sha = status["manifest_digest"]
        .as_str()
        .expect("digest")
        .to_owned();
    let blocked = store.project_context(&[scope()], &sha).await;
    release_tx.send(()).expect("release source");
    session.await.expect("join source").expect("source session");
    assert_eq!(
        blocked,
        Err(Error::ContextPending),
        "exclusive projection fails closed while source read holds the gate"
    );
    store
        .project_context(&[scope()], &sha)
        .await
        .expect("projection after source closes");
    store
        .with_native_context(view.path().canonicalize().expect("view"), |_| Ok(()))
        .await
        .expect("dedicated connection drop released gate and flock");
}

#[tokio::test]
async fn native_projection_preserves_multilingual_ontology_and_excludes_unsafe_documents() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(0, b"");
    let markdown = "---\ntitle: 성장 실험\nscope: personal\nlanguage: ko\naliases: [마케팅, 成長]\nexport: false\nontology: true\nentities: [project:tarot, topic:growth]\nrelations:\n  - from: project:tarot\n    type: supports\n    to: topic:growth\n---\n성장 실험 마케팅\napi_key: sk-test-abcdefghijklmnopqrstuvwxyz123456789\n";
    context_core::document::parse_markdown_bytes(
        Path::new("personal/ontology.md"),
        markdown.as_bytes(),
    )
    .expect("valid synthetic ontology schema");
    fs::write(root.join("personal/ontology.md"), markdown).expect("multilingual ontology");
    fs::write(root.join("personal/invalid.md"), [255u8]).expect("invalid UTF8 preserved");
    fs::write(
        root.join("personal/wrong-scope.md"),
        b"---\nscope: work\n---\nwrong",
    )
    .expect("scope mismatch preserved");
    fs::write(root.join("personal/attachment.bin"), [0, 255]).expect("binary preserved");
    fs::create_dir(root.join("personal/raw")).expect("raw");
    fs::write(root.join("personal/raw/note.md"), b"hidden raw text").expect("restricted original");
    import(&store, &root).await;
    let rows:Vec<(String,Value)>=sqlx::query_as("SELECT m.path,p.payload FROM context_materials m JOIN context_projection_versions p USING(material_id,revision) ORDER BY m.path").fetch_all(store.pool()).await.expect("projection fixture");
    for (path, payload) in &rows {
        let expected = match path.as_str() {
            "ontology.md" => "searchable",
            "invalid.md" | "wrong-scope.md" => "unavailable",
            _ => "excluded",
        };
        assert_eq!(
            payload["status"], expected,
            "projection status for synthetic {path}"
        );
        if expected != "searchable" {
            assert!(payload.get("body").is_none());
        }
    }
    let document = store
        .read_context_documents(&scope(), &["ontology.md".into()])
        .await
        .expect("explicit export:false read");
    let output = document.to_string();
    assert!(!output.contains("sk-test-abcdefghijklmnopqrstuvwxyz123456789"));
    let before = store.calls();
    assert_eq!(
        store
            .read_context_documents(&scope(), &["raw/note.md".into()])
            .await,
        Err(Error::Invalid)
    );
    assert_eq!(store.calls(), before, "raw paths fail before DB access");
    // Restrict semantic scope to a clean scoped fixture while retaining unavailable originals above.
    let company: ContextScope = "work/acme".parse().expect("company scope");
    fs::create_dir_all(root.join("work/acme")).expect("company");
    fs::write(
        root.join("work/acme/ontology.md"),
        markdown.replace("scope: personal", "scope: work"),
    )
    .expect("matching top-level declared scope");
    let i = inventory(&root, std::slice::from_ref(&company)).expect("company inventory");
    store
        .import_context(&root, std::slice::from_ref(&company), &i.inventory_digest)
        .await
        .expect("company projection");
    for query in ["마케팅", "成長", "project:tarot"] {
        let found = store
            .semantic_context(std::slice::from_ref(&company), query, 10, false)
            .await
            .expect("multilingual semantic query");
        assert_eq!(
            found["items"].as_array().expect("results").len(),
            1,
            "semantic terms {query}"
        );
    }
    let edges = store
        .semantic_context(std::slice::from_ref(&company), "project:tarot", 10, true)
        .await
        .expect("ontology edges");
    assert_eq!(edges["items"].as_array().expect("edges").len(), 1);
    assert_eq!(edges["items"][0]["relation"]["to"], "topic:growth");
    assert_eq!(
        store
            .read_context_documents(&scope(), &["invalid.md".into()])
            .await,
        Err(Error::Invalid)
    );
}

#[tokio::test]
async fn native_current_revision_and_tombstone_never_reuse_old_projections_or_view_bytes() {
    use context_core::harness::StoredSourceState;
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(1, b"# Current\noriginal");
    import(&store, &root).await;
    let view = tempfile::tempdir().expect("view");
    fs::set_permissions(view.path(), fs::Permissions::from_mode(0o700)).expect("owned");
    let view_path = view.path().canonicalize().expect("root");
    let first = store
        .with_native_context(view_path.clone(), |source| {
            source
                .open_file(Path::new("vault/personal/0000.md"), 65536)
                .expect("view original");
            Ok(source
                .stored_versions(&[PathBuf::from("vault/personal/0000.md")])
                .expect("initial version")
                .remove(0))
        })
        .await
        .expect("initial source");
    // Negative fixture: simulate corrupted storage metadata without manufacturing any Core receipt.
    sqlx::raw_sql("ALTER TABLE context_materials DISABLE TRIGGER context_revision; ALTER TABLE context_materials DISABLE TRIGGER context_version; UPDATE context_materials SET revision=revision+1; ALTER TABLE context_materials ENABLE TRIGGER context_revision; ALTER TABLE context_materials ENABLE TRIGGER context_version;").execute(store.pool()).await.expect("same-byte revision drift fixture");
    assert_eq!(
        store
            .semantic_context(&[scope()], "original", 10, false)
            .await,
        Err(Error::ContextProjectionUnavailable),
        "old projection cannot serve a new current revision"
    );
    store
        .with_native_context(view_path.clone(), move |source| {
            let current = source
                .stored_versions(&[PathBuf::from("vault/personal/0000.md")])
                .expect("current metadata")
                .remove(0);
            assert_ne!(current, first);
            assert!(matches!(
                current.state,
                StoredSourceState::Live { revision: 2, .. }
            ));
            assert_eq!(source.body_queries(), 0);
            Ok(())
        })
        .await
        .expect("version source");
    sqlx::raw_sql("ALTER TABLE context_materials DISABLE TRIGGER context_revision; ALTER TABLE context_materials DISABLE TRIGGER context_version; UPDATE context_materials SET revision=revision+1,deleted=true,content=''::bytea,content_digest=encode(sha256(''::bytea),'hex'),byte_len=0,search_text=NULL; ALTER TABLE context_materials ENABLE TRIGGER context_revision; ALTER TABLE context_materials ENABLE TRIGGER context_version;").execute(store.pool()).await.expect("tombstone fixture");
    assert!(
        store
            .semantic_context(&[scope()], "original", 10, false)
            .await
            .expect("deleted scope readable")["items"]
            .as_array()
            .expect("items")
            .is_empty()
    );
    store
        .with_native_context(view_path.clone(), |source| {
            let m = source
                .metadata(Path::new("vault/personal/0000.md"))
                .expect("tombstone metadata");
            assert_eq!(m.kind, SourcePathKind::Missing);
            assert!(matches!(
                m.stored_version.expect("stable tombstone identity").state,
                StoredSourceState::Deleted { revision: 3, .. }
            ));
            assert_eq!(source.body_queries(), 0);
            Ok(())
        })
        .await
        .expect("stale named view removed");
    assert!(!view_path.join("vault/personal/0000.md").exists());
}

#[tokio::test]
async fn native_projection_failure_rolls_back_earlier_pages() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(101, b"# Original projection");
    import(&store, &root).await;
    let last:(String,i64,Value)=sqlx::query_as("SELECT material_id::text,revision,payload FROM context_projection_versions ORDER BY material_id DESC LIMIT 1").fetch_one(store.pool()).await.expect("final page fixture");
    sqlx::query("TRUNCATE context_projection_versions")
        .execute(store.pool())
        .await
        .expect("missing projections");
    sqlx::query("INSERT INTO context_projection_versions(material_id,revision,payload,payload_digest) VALUES($1::uuid,$2,$3,encode(sha256(convert_to($3::jsonb::text,'UTF8')),'hex'))").bind(&last.0).bind(last.1).bind(&last.2).execute(store.pool()).await.expect("final page fixture");
    sqlx::raw_sql("CREATE FUNCTION native_test_late_projection_failure() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF EXISTS(SELECT 1 FROM context_projection_versions WHERE material_id=NEW.material_id) THEN RAISE EXCEPTION 'Synthetic late projection failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER native_test_late_projection_failure BEFORE INSERT ON context_projection_versions FOR EACH ROW EXECUTE FUNCTION native_test_late_projection_failure();").execute(store.pool()).await.expect("owned late-page failure trigger");
    let status = store
        .projection_status(&[scope()])
        .await
        .expect("metadata manifest");
    assert_eq!(
        store
            .project_context(
                &[scope()],
                status["manifest_digest"].as_str().expect("digest")
            )
            .await,
        Err(Error::Storage)
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM context_projection_versions")
        .fetch_one(store.pool())
        .await
        .expect("rolled back first page");
    assert_eq!(
        count, 1,
        "a later refresh failure cannot publish earlier pages"
    );
    sqlx::raw_sql("DROP TRIGGER native_test_late_projection_failure ON context_projection_versions; DROP FUNCTION native_test_late_projection_failure();").execute(store.pool()).await.expect("remove owned trigger");
}

#[path = "native_harness.rs"]
mod harness_fixture;

// These checks run on actual Core-issued contracts and the real PostgreSQL
// session. Observe each effect phase separately so a failed or repeated attempt
// cannot consume the normal path's budget or conceal automatic retries.
struct SqlMeasuredSession<'a> {
    inner: &'a mut ontology::native_context::NativeContextSession,
    store: Store,
    body_bytes: usize,
    projection_failure: bool,
    phases: Vec<String>,
}
impl SqlMeasuredSession<'_> {
    fn phase<T>(
        &mut self,
        phase: &str,
        contract: &context_core::harness::ContextApplyContract,
        direct: usize,
        helper: u64,
        operation: impl FnOnce(
            &mut ontology::native_context::NativeContextSession,
        ) -> context_core::harness::HarnessResult<T>,
    ) -> context_core::harness::HarnessResult<T> {
        self.phases.push(phase.to_owned());
        let start = self
            .inner
            .native_sql_observations()
            .expect("complete SQL observation")
            .len();
        let projections = self
            .inner
            .native_projection_observations()
            .expect("complete helper observation")
            .len();
        let calls = self.store.calls();
        let result = operation(self.inner);
        assert_sql_phase(self.inner, contract, self.body_bytes, start, direct, phase);
        assert_eq!(
            self.store.calls() - calls,
            direct as u64 + helper,
            "{phase}: all direct calls and actual shared-helper SQL calls"
        );
        let observed = self
            .inner
            .native_projection_observations()
            .expect("complete helper observation");
        let observed = &observed[projections..];
        assert_eq!(
            observed
                .iter()
                .map(|call| call.store_call_delta)
                .sum::<u64>(),
            helper
        );
        for call in observed {
            assert_eq!(call.records, contract.targets().len());
            assert_eq!(call.succeeded, result.is_ok());
            // Compact JSON strings can escape each byte to six bytes; the
            // metadata envelope is bounded by the actual issued contract.
            let contract_bytes = serde_json::to_vec(contract).expect("issued contract").len();
            assert!(call.input_bytes > 0);
            assert!(call.input_bytes <= 6 * self.body_bytes + 4 * contract_bytes + 65536);
        }
        result
    }
}
fn assert_sql_phase(
    session: &ontology::native_context::NativeContextSession,
    contract: &context_core::harness::ContextApplyContract,
    body_bytes: usize,
    start: usize,
    expected: usize,
    phase: &str,
) {
    let calls = session
        .native_sql_observations()
        .expect("bounded complete SQL observation");
    let calls = &calls[start..];
    assert_eq!(
        calls.len(),
        expected,
        "{phase}: source-derived SQL phase bound"
    );
    assert!(calls.iter().all(|call| call.started < call.finished));
    assert!(
        calls
            .windows(2)
            .all(|pair| pair[0].finished < pair[1].started),
        "one completed dependency call before the next starts"
    );
    let unique: std::collections::BTreeSet<_> =
        calls.iter().map(|call| &call.request_digest).collect();
    assert_eq!(
        unique.len(),
        calls.len(),
        "{phase}: no duplicate SQL/parameter request or automatic retry within a phase"
    );
    let parameters: usize = calls.iter().map(|call| call.parameter_bytes).sum();
    let rows: usize = calls.iter().map(|call| call.row_bytes).sum();
    let contract_bytes = serde_json::to_vec(contract)
        .expect("actual issued contract bytes")
        .len();
    // Both material INSERT and UPDATE encode the same page. A JSON byte array
    // needs at most four bytes per original byte and a JSON string at most six;
    // twice both is 20 B. Twelve contract copies plus the fixed SQL metadata
    // envelope cover identifiers, source versions and receipt metadata. Returned
    // rows contain those envelopes and digests, never original material bodies.
    assert!(
        parameters > 0 && parameters <= 20 * body_bytes + 12 * contract_bytes + 65536,
        "{phase}: encoded PostgreSQL parameter bytes {parameters}"
    );
    assert!(
        rows <= 12 * contract_bytes + 65536,
        "{phase}: returned PostgreSQL payload bytes {rows}"
    );
    assert!(
        calls.iter().all(|call| call.succeeded),
        "direct SQL succeeds even when a later semantic or shared-helper check rejects the phase"
    );
    eprintln!(
        "native SQL observation: {}",
        serde_json::json!({
            "phase": phase, "targets": contract.targets().len(), "body_bytes": body_bytes,
            "contract_bytes": contract_bytes, "direct_calls": calls.len(),
            "duplicate_requests": calls.len() - unique.len(), "sequential_depth": calls.len(),
            "encoded_parameter_bytes": parameters, "returned_row_bytes": rows,
        })
    );
}
impl context_core::harness::ContextCommitSession for SqlMeasuredSession<'_> {
    fn verify_pre_apply_close(
        &mut self,
        c: &context_core::harness::ContextPreApplyCloseContract,
    ) -> context_core::harness::HarnessResult<()> {
        self.inner.verify_pre_apply_close(c)
    }
    fn store_identity(&self) -> &context_core::harness::SourceStoreIdentity {
        self.inner.store_identity()
    }
    fn begin(
        &mut self,
        c: &context_core::harness::ContextApplyContract,
    ) -> context_core::harness::HarnessResult<()> {
        self.phase("begin", c, 4, 0, |s| s.begin(c))
    }
    fn commit(
        &mut self,
        v: &context_core::harness::VerifiedContextCommit<'_>,
    ) -> context_core::harness::HarnessResult<context_core::harness::ContextCommitReceipt> {
        // Core admits at most 64 targets, below the native 100-record page.
        assert!((1..=64).contains(&v.contract().targets().len()));
        let pages = v.contract().targets().len().div_ceil(100);
        let (direct, helper) = if self.projection_failure {
            (3 + 3 * pages, 2)
        } else {
            (5 + 3 * pages, 2 * pages as u64)
        };
        self.phase("commit", v.contract(), direct, helper, |s| s.commit(v))
    }
    fn finalize(
        &mut self,
        p: &context_core::harness::ContextTerminalProof,
    ) -> context_core::harness::HarnessResult<()> {
        self.phase("finalize", p.contract(), 5, 0, |s| s.finalize(p))
    }
    fn recover(
        &mut self,
        p: &context_core::harness::ContextRecoveryContract,
    ) -> context_core::harness::HarnessResult<context_core::harness::ContextRecoveryStatus> {
        self.inner.recover(p)
    }
    fn abort(
        &mut self,
        p: &context_core::harness::ContextAbortProof,
    ) -> context_core::harness::HarnessResult<()> {
        self.phase("abort", p.contract(), 4, 0, |s| s.abort(p))
    }
}
async fn measured_native_apply(
    store: &Store,
    root: &Path,
    run: String,
    prepared: context_core::harness::PreparedHarnessRun,
    body_bytes: usize,
) -> (
    context_core::harness::HarnessExecutionRecord,
    context_core::harness::HarnessApplyAttemptReceipt,
) {
    use context_core::harness::*;
    let id = prepared
        .plan
        .bound_source_versions()
        .store_identity
        .as_ref()
        .expect("actual native identity")
        .store_id
        .clone();
    let workspace = root.to_owned();
    let observed = store.clone();
    store
        .with_native_commit(root.to_owned(), id, move |session| {
            let source = session.fresh_source().expect("fresh native source");
            let engine = HarnessEngine::with_source(
                source.view_root(),
                &workspace,
                source.clone(),
                context_fixture::configuration(),
            )
            .expect("actual native engine");
            let mut measured = SqlMeasuredSession {
                inner: session,
                store: observed,
                body_bytes,
                projection_failure: false,
                phases: Vec::new(),
            };
            let result = HarnessExecutionRecord::apply_durable(&engine, &run, Some(&mut measured))
                .expect("measured actual native apply");
            result
                .1
                .validate(&prepared)
                .expect("actual terminal receipt");
            assert_eq!(measured.phases, ["begin", "commit", "finalize"]);
            let calls = measured
                .inner
                .native_sql_observations()
                .expect("complete SQL calls");
            let unique: std::collections::BTreeSet<_> =
                calls.iter().map(|call| &call.request_digest).collect();
            // Across the three phases: effect, non-target versions and target
            // versions each occur three times; committed-row verification twice.
            // Those seven integrity rechecks are intentional, with no extra retry.
            assert_eq!(calls.len(), 17);
            assert_eq!(calls.len() - unique.len(), 7);
            Ok(result)
        })
        .await
        .expect("measured native session")
}

#[tokio::test]
async fn native_commit_revision_and_scope_integrity() {
    use context_core::harness::HarnessApplyAttemptState;
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let policy = harness_fixture::seed(&store).await;
    let view = harness_fixture::view();
    let root = view.path().canonicalize().expect("owned view");
    let logical = "vault/personal/knowledge/native-runtime-proof.md";
    let mut identity = None;
    for (index, content, deleted) in [
        (1, "# Native\n첫 원문 growth", false),
        (2, "# Native\n첫 원문 growth", false),
        (3, "# Native\nChanged growth", false),
        (4, "", true),
        (5, "# Native\nRecreated growth", false),
    ] {
        let run = format!("native-revision-{index}");
        let request = harness_fixture::envelope(
            &policy,
            vec![logical.into()],
            if deleted {
                vec![logical.into()]
            } else {
                vec![]
            },
            false,
        );
        let prepared = harness_fixture::prepare(
            &store,
            root.clone(),
            root.clone(),
            request,
            run.clone(),
            content.into(),
        )
        .await;
        let (_, attempt) = measured_native_apply(&store, &root, run, prepared, content.len()).await;
        let HarnessApplyAttemptState::AppliedFinalized { apply_receipt, .. } =
            attempt.attempt_state
        else {
            panic!("Core must finalize actual apply")
        };
        let receipt = apply_receipt
            .context_commit_receipt
            .expect("real commit receipt");
        receipt.validate().expect("Core receipt validates");
        let row:(String,i64,String,bool,Option<String>,Option<String>,String)=sqlx::query_as("SELECT material_id::text,revision,origin_kind,deleted,source_root,source_digest,content_digest FROM context_materials WHERE scope='personal' AND path='knowledge/native-runtime-proof.md'").fetch_one(store.pool()).await.expect("actual native row");
        if let Some(expected) = &identity {
            assert_eq!(&row.0, expected);
        } else {
            identity = Some(row.0.clone());
        }
        assert_eq!(row.1, index);
        assert_eq!(row.2, "native");
        assert_eq!(row.3, deleted);
        assert_eq!(row.4, None);
        assert_eq!(row.5, None);
        assert_eq!(row.6, ontology::store::digest(content.as_bytes()));
        let states:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM context_material_versions WHERE material_id=$1::uuid),(SELECT count(*) FROM context_projection_versions WHERE material_id=$1::uuid)").bind(&row.0).fetch_one(store.pool()).await.expect("append history and projections");
        assert_eq!(states, (index, index));
        let state: String = sqlx::query_scalar(
            "SELECT state FROM context_apply_batches WHERE core_apply_attempt_id=$1",
        )
        .bind(receipt.attempt_identifier())
        .fetch_one(store.pool())
        .await
        .expect("finalized cleanup");
        assert_eq!(state, "finalized");
    }
    // Reuse this already-seeded native fixture for the actual Core target limit.
    // The five one-target operations above retain every original revision proof.
    let targets = (0..64)
        .map(|n| format!("vault/personal/knowledge/native-size-{n:02}.md"))
        .collect();
    let body = "# Native size boundary\nExact accepted bytes";
    let prepared = harness_fixture::prepare(
        &store,
        root.clone(),
        root.clone(),
        harness_fixture::envelope(&policy, targets, vec![], false),
        "native-size-boundary".into(),
        body.into(),
    )
    .await;
    let (_, attempt) = measured_native_apply(
        &store,
        &root,
        "native-size-boundary".into(),
        prepared,
        64 * body.len(),
    )
    .await;
    let HarnessApplyAttemptState::AppliedFinalized { apply_receipt, .. } = attempt.attempt_state
    else {
        panic!("actual boundary finalized")
    };
    assert_eq!(
        apply_receipt
            .context_commit_receipt
            .expect("actual boundary receipt")
            .resulting()
            .len(),
        64
    );
}

async fn native_candidate(
    store: &Store,
    root: &Path,
    run: &str,
    path: &str,
    body: &str,
    curation: bool,
) -> context_core::harness::PreparedHarnessRun {
    let policy:String=sqlx::query_scalar("SELECT content_digest FROM context_materials WHERE scope='profile' AND path='rules/control.md'").fetch_one(store.pool()).await.expect("actual policy SHA");
    harness_fixture::prepare(
        store,
        root.to_owned(),
        root.to_owned(),
        harness_fixture::envelope(&policy, vec![path.into()], vec![], curation),
        run.into(),
        body.into(),
    )
    .await
}
async fn native_write(
    store: &Store,
    root: &Path,
    run: &str,
    path: &str,
    body: &str,
    curation: bool,
) {
    let prepared = native_candidate(store, root, run, path, body, curation).await;
    harness_fixture::apply(
        store,
        root.to_owned(),
        root.to_owned(),
        run.into(),
        prepared,
    )
    .await;
}
#[tokio::test]
async fn native_projection_search_semantics() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    harness_fixture::seed(&store).await;
    let view = harness_fixture::view();
    let root = view.path().canonicalize().expect("view");
    native_write(&store,&root,"native-search","vault/personal/knowledge/native-search.md","---\ntitle: 검색\nscope: personal\naliases: [nativegrowth, 成長]\nexport: false\n---\n# 검색\n마케팅 nativegrowth",false).await;
    for query in ["nativegrowth", "成長", "마케팅"] {
        let results = store
            .semantic_context(&[scope()], query, 100, false)
            .await
            .expect("current semantic projection");
        let item = results["items"]
            .as_array()
            .expect("items")
            .iter()
            .find(|v| v.to_string().contains("native-search.md"))
            .expect("accepted native document remains searchable");
        assert_eq!(item["exportable"], false);
    }
    let exact = store
        .read_context_documents(&scope(), &["knowledge/native-search.md".into()])
        .await
        .expect("accepted exact native read");
    assert_eq!(
        exact["documents"]
            .as_array()
            .expect("exact documents")
            .len(),
        1
    );
}
#[tokio::test]
async fn native_projection_ontology_edges() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    harness_fixture::seed(&store).await;
    let view = harness_fixture::view();
    let root = view.path().canonicalize().expect("view");
    for (i, target) in ["topic:first", "topic:second"].iter().enumerate() {
        let body = format!(
            "---\ntitle: Native ontology\nscope: personal\nontology: true\nentities: [project:nativeproof, {target}]\nrelations:\n  - from: project:nativeproof\n    type: supports\n    to: {target}\n---\n# Native ontology\nExact current relation\n"
        );
        native_write(
            &store,
            &root,
            &format!("native-edges-{i}"),
            "vault/personal/knowledge/native-edges.md",
            &body,
            false,
        )
        .await;
        let edges = store
            .semantic_context(&[scope()], "project:nativeproof", 10, true)
            .await
            .expect("current edge projection");
        assert_eq!(edges["items"].as_array().expect("edges").len(), 1);
        assert_eq!(edges["items"][0]["relation"]["to"], *target);
    }
}
#[tokio::test]
async fn native_curation_commit_and_next_read() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    harness_fixture::seed(&store).await;
    let view = harness_fixture::view();
    let root = view.path().canonicalize().expect("view");
    native_write(
        &store,
        &root,
        "native-curation",
        "vault/personal/knowledge/native-curation.md",
        "---\ntitle: Curation\nscope: personal\n---\n# Curation\nActual curated knowledge",
        true,
    )
    .await;
    assert!(
        store
            .read_context(&scope(), "knowledge/native-curation.md", false)
            .await
            .expect("next curated read")
            .contains("Actual curated knowledge")
    );
}
#[tokio::test]
async fn native_context_commit_excludes_external_targets() {
    use context_core::harness::HarnessApplyAttemptState;
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let policy = harness_fixture::seed(&store).await;
    let view = harness_fixture::view();
    let root = view.path().canonicalize().expect("view");
    let workspace = tempfile::tempdir().expect("external workspace");
    let workspace = workspace.path().canonicalize().expect("workspace");
    fs::write(workspace.join("document.md"), "# Existing external file").expect("external target");
    let request = harness_fixture::envelope(&policy, vec!["document.md".into()], vec![], false);
    let prepared = harness_fixture::prepare(
        &store,
        root.clone(),
        workspace.clone(),
        request,
        "native-external".into(),
        "# Accepted external file".into(),
    )
    .await;
    let (_, attempt) = harness_fixture::apply(
        &store,
        root,
        workspace.clone(),
        "native-external".into(),
        prepared,
    )
    .await;
    let HarnessApplyAttemptState::AppliedFinalized { apply_receipt, .. } = attempt.attempt_state
    else {
        panic!("actual external finalization")
    };
    assert!(apply_receipt.context_commit_receipt.is_none());
    assert_eq!(
        fs::read_to_string(workspace.join("document.md")).expect("external bytes"),
        "# Accepted external file"
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM context_apply_batches")
        .fetch_one(store.pool())
        .await
        .expect("effects");
    assert_eq!(count, 0);
}
#[tokio::test]
async fn native_cli_exact_read_has_no_old_runtime() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    harness_fixture::seed(&store).await;
    let view = harness_fixture::view();
    let root = view.path().canonicalize().expect("view");
    native_write(
        &store,
        &root,
        "native-cli-read",
        "vault/personal/knowledge/native-cli-read.md",
        "# Native CLI\n독립 원문",
        false,
    )
    .await;
    let output = cli(
        json!({"op":"read-documents","scope":"personal","paths":["knowledge/native-cli-read.md"]}),
    )
    .await;
    assert!(output.status.success());
    let response: Value =
        serde_json::from_slice(&output.stdout).expect("one exact native response");
    assert!(response.to_string().contains("독립 원문"));
    assert!(output.stderr.is_empty());
}
struct InterruptedSession<'a> {
    inner: &'a mut ontology::native_context::NativeContextSession,
    point: &'static str,
    directory: PathBuf,
    earlier_head: Vec<u8>,
}
impl context_core::harness::ContextCommitSession for InterruptedSession<'_> {
    fn verify_pre_apply_close(
        &mut self,
        c: &context_core::harness::ContextPreApplyCloseContract,
    ) -> context_core::harness::HarnessResult<()> {
        self.inner.verify_pre_apply_close(c)
    }
    fn store_identity(&self) -> &context_core::harness::SourceStoreIdentity {
        self.inner.store_identity()
    }
    fn begin(
        &mut self,
        c: &context_core::harness::ContextApplyContract,
    ) -> context_core::harness::HarnessResult<()> {
        self.inner.begin(c)?;
        #[cfg(target_os = "macos")]
        if self.point == "partial-rollback" {
            let view = self
                .directory
                .ancestors()
                .nth(3)
                .expect("owned run directory ancestry");
            assert!(
                std::process::Command::new("/bin/chmod")
                    .args(["+a", "everyone deny add_file"])
                    .arg(view.join("vault/personal/knowledge/rollback-blocked"))
                    .status()
                    .expect("inject owned second-target ACL fault")
                    .success()
            );
        }
        if self.point == "after-begin" {
            panic!("intentional synthetic process boundary")
        };
        Ok(())
    }
    fn recover(
        &mut self,
        c: &context_core::harness::ContextRecoveryContract,
    ) -> context_core::harness::HarnessResult<context_core::harness::ContextRecoveryStatus> {
        self.inner.recover(c)
    }
    fn commit(
        &mut self,
        c: &context_core::harness::VerifiedContextCommit<'_>,
    ) -> context_core::harness::HarnessResult<context_core::harness::ContextCommitReceipt> {
        if self.point == "before-commit" {
            panic!("intentional synthetic file-complete boundary")
        };
        let receipt = self.inner.commit(c)?;
        if self.point == "after-commit" {
            panic!("intentional synthetic committed-response loss")
        };
        if self.point == "terminal-replacement-failure" {
            let path = self.directory.join("apply-attempt.json");
            fs::rename(&path, self.directory.join("owned-attempt-backup.json"))
                .expect("save actual Core attempt");
            use std::os::unix::fs::DirBuilderExt as _;
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .expect("inject terminal file replacement fault");
        }
        if self.point == "head-cas" {
            fs::write(self.directory.join("head.json"), &self.earlier_head)
                .expect("inject actual earlier valid Core head");
        }
        Ok(receipt)
    }
    fn finalize(
        &mut self,
        p: &context_core::harness::ContextTerminalProof,
    ) -> context_core::harness::HarnessResult<()> {
        if self.point == "before-finalize" {
            panic!("intentional synthetic terminal-before-cleanup boundary")
        };
        self.inner.finalize(p)
    }
    fn abort(
        &mut self,
        p: &context_core::harness::ContextAbortProof,
    ) -> context_core::harness::HarnessResult<()> {
        self.inner.abort(p)
    }
}
#[tokio::test]
async fn native_commit_recovery_crash_boundaries() {
    let _guard = TEST_LOCK.lock().await;
    for point in [
        "after-begin",
        "before-commit",
        "after-commit",
        "before-finalize",
        "terminal-replacement-failure",
        "head-cas",
    ] {
        native_crash_case(point).await;
    }
    #[cfg(target_os = "macos")]
    native_crash_case("partial-rollback").await;
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn native_partial_publication_rollback_aborts() {
    let _guard = TEST_LOCK.lock().await;
    native_crash_case("partial-rollback").await;
}

async fn native_crash_case(point: &'static str) {
    use context_core::harness::{HarnessEngine, HarnessExecutionRecord};
    let store = store().await;
    let policy = harness_fixture::seed(&store).await;
    let view = harness_fixture::view();
    let root = view.path().canonicalize().expect("view");
    let run = format!("native-crash-{point}");
    let path = if point == "before-commit" {
        "vault/personal/knowledge/recovery-empty/nested/native-crash.md"
    } else {
        "vault/personal/knowledge/native-crash.md"
    };
    let body = "# Native crash proof\nExact accepted content";
    let mut targets = vec![path.to_owned()];
    if point == "partial-rollback" {
        let (_fixture, fixture_root) = fixture(0, b"");
        fs::create_dir_all(fixture_root.join("personal/knowledge/rollback-blocked"))
            .expect("synthetic second parent");
        fs::write(
            fixture_root.join("personal/knowledge/rollback-blocked/keep.md"),
            "# Synthetic directory source",
        )
        .expect("synthetic parent binding");
        import(&store, &fixture_root).await;
        store
            .with_native_context(root.clone(), |source| {
                source
                    .open_file(
                        Path::new("vault/personal/knowledge/rollback-blocked/keep.md"),
                        65536,
                    )
                    .expect("materialize existing private second parent");
                Ok(())
            })
            .await
            .expect("owned source directory");
        targets.push("vault/personal/knowledge/rollback-blocked/second.md".into());
    }
    let (_, heads) = harness_fixture::prepare_with_heads(
        &store,
        root.clone(),
        root.clone(),
        harness_fixture::envelope(&policy, targets, vec![], false),
        run.clone(),
        body.into(),
    )
    .await;
    let earlier_head = serde_json::to_vec(&heads[0]).expect("actual earlier Core head");
    let directory = root.join(format!(".llm-context-vault-harness/runs/{run}"));
    let saved_head = fs::read(directory.join("head.json")).expect("actual validated Core head");
    let fault_directory = directory.clone();
    let id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
        .fetch_one(store.pool())
        .await
        .expect("identity");
    let workspace = root.clone();
    let run_copy = run.clone();
    store
        .with_native_commit(root.clone(), id, move |session| {
            let source = session.fresh_source().expect("fresh source");
            let engine = HarnessEngine::with_source(
                source.view_root(),
                &workspace,
                source.clone(),
                context_fixture::configuration(),
            )
            .expect("native engine");
            let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut boundary = InterruptedSession {
                    inner: session,
                    point,
                    directory: fault_directory,
                    earlier_head,
                };
                HarnessExecutionRecord::apply_durable(&engine, &run_copy, Some(&mut boundary))
            }));
            if matches!(
                point,
                "terminal-replacement-failure" | "head-cas" | "partial-rollback"
            ) {
                assert!(
                    matches!(interrupted, Ok(Err(_))),
                    "real Core persistence/CAS must reject the injected fault"
                );
            } else {
                assert!(
                    interrupted.is_err(),
                    "actual requested boundary must execute"
                );
            }
            Ok(())
        })
        .await
        .expect("connection released after synthetic interruption");
    #[cfg(target_os = "macos")]
    if point == "partial-rollback" {
        assert!(
            std::process::Command::new("/bin/chmod")
                .args(["-a", "everyone deny add_file"])
                .arg(root.join("vault/personal/knowledge/rollback-blocked"))
                .status()
                .expect("remove only injected ACL")
                .success()
        );
        let attempt: context_core::harness::HarnessApplyAttemptReceipt =
            context_core::harness::decode_current_json(
                &fs::read(directory.join("apply-attempt.json")).expect("actual rollback attempt"),
                "actual rollback attempt",
            )
            .expect("Core rollback proof");
        let context_core::harness::HarnessApplyAttemptState::Failure {
            evidence:
                context_core::harness::HarnessApplyFailureEvidence::BatchApply {
                    receipt: rollback_receipt,
                    ..
                },
        } = &attempt.attempt_state
        else {
            panic!("actual pending failure must preserve the completed rollback receipt")
        };
        assert!(
            rollback_receipt.targets[0]
                .repository_apply
                .as_ref()
                .expect("actual first target publication report")
                .target_mutation_committed
        );
        assert!(
            !rollback_receipt.targets[1]
                .repository_apply
                .as_ref()
                .expect("actual second target failure report")
                .target_mutation_committed
        );
        let completion = root.join(&attempt.expected_completion_relative_path);
        let journal: Value = serde_json::from_slice(
            &fs::read(completion).expect("actual rollback completion journal"),
        )
        .expect("completion JSON");
        assert!(
            journal.to_string().contains("rolled-back"),
            "Core completed actual rollback"
        );
    }
    if point == "terminal-replacement-failure" {
        fs::remove_dir(directory.join("apply-attempt.json")).expect("remove only injected fault");
        fs::rename(
            directory.join("owned-attempt-backup.json"),
            directory.join("apply-attempt.json"),
        )
        .expect("restore exact saved Core attempt");
    }
    if point == "head-cas" {
        fs::write(directory.join("head.json"), saved_head)
            .expect("restore exact actual validated Core head");
    }
    let state: String =
        sqlx::query_scalar("SELECT state FROM context_apply_batches WHERE core_run_id=$1")
            .bind(&run)
            .fetch_one(store.pool())
            .await
            .expect("durable effect");
    assert_eq!(
        state,
        if matches!(
            point,
            "after-commit" | "before-finalize" | "terminal-replacement-failure" | "head-cas"
        ) {
            "committed"
        } else {
            "pending"
        }
    );
    {
        assert_eq!(
            store
                .read_context(&scope(), "knowledge/native-crash.md", false)
                .await,
            Err(Error::ContextPending)
        );
        assert_eq!(
            store
                .read_context_documents(&scope(), &["knowledge/native-crash.md".into()])
                .await,
            Err(Error::ContextPending)
        );
        assert_eq!(
            store
                .semantic_context(&[scope()], "native", 10, false)
                .await,
            Err(Error::ContextPending)
        );
        assert_eq!(
            store.projection_status(&[scope()]).await,
            Err(Error::ContextPending)
        );
        let blocked: Result<(), Error> = store
            .with_native_context(root.clone(), |_| {
                panic!("pending effect must deny ordinary source construction")
            })
            .await;
        assert!(matches!(blocked, Err(Error::ContextPending)));
    }
    if point == "after-commit" {
        fs::write(root.join(path), "CORRUPTED VIEW ONLY").expect("owned corruption fixture");
        let rejected = harness_fixture::command(
            &store,
            &root,
            &root,
            "recover",
            &["--run-id".into(), run.clone()],
        )
        .await;
        assert!(!rejected.status.success());
        assert_eq!(
            fs::read_to_string(root.join(path)).expect("corruption survives rejection"),
            "CORRUPTED VIEW ONLY"
        );
        fs::write(root.join(path), body).expect("restore exact committed fixture");
    }
    let verb = if point == "before-finalize" {
        "apply"
    } else {
        "recover"
    };
    let output = harness_fixture::command(
        &store,
        &root,
        &root,
        verb,
        &["--run-id".into(), run.clone()],
    )
    .await;
    assert!(
        output.status.success(),
        "source-free {point} recovery: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    if point == "before-commit" {
        assert_eq!(
            fs::read(root.join(path)).expect("retained nested recovery target"),
            body.as_bytes(),
            "DB-Missing ancestors must not erase a preserved recovery target"
        );
    }
    let final_state: String =
        sqlx::query_scalar("SELECT state FROM context_apply_batches WHERE core_run_id=$1")
            .bind(&run)
            .fetch_one(store.pool())
            .await
            .expect("recovery cleanup");
    assert_eq!(
        final_state,
        if matches!(point, "after-begin" | "partial-rollback") {
            "aborted"
        } else {
            "finalized"
        }
    );
    if point == "partial-rollback" {
        let actual: context_core::harness::HarnessApplyAttemptReceipt =
            context_core::harness::decode_current_json(
                &fs::read(directory.join("apply-attempt.json")).expect("recovered actual attempt"),
                "persisted abort proof",
            )
            .expect("strict recovered attempt");
        assert!(matches!(
            actual.attempt_state,
            context_core::harness::HarnessApplyAttemptState::AbortedAfterRollback { .. }
        ));
        assert!(!root.join(path).exists());
        assert!(
            !root
                .join("vault/personal/knowledge/rollback-blocked/second.md")
                .exists()
        );
    }
    let repeat =
        harness_fixture::command(&store, &root, &root, "recover", &["--run-id".into(), run]).await;
    assert!(
        repeat.status.success(),
        "idempotent recovery: {}",
        String::from_utf8_lossy(&repeat.stderr)
    );
}

#[tokio::test]
async fn native_projection_atomicity_and_freshness() {
    use context_core::harness::{HarnessEngine, HarnessExecutionRecord};
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    harness_fixture::seed(&store).await;
    let view = harness_fixture::view();
    let root = view.path().canonicalize().expect("view");
    let _prepared = native_candidate(
        &store,
        &root,
        "native-projection-rollback",
        "vault/personal/knowledge/native-atomic.md",
        "# Atomic projection",
        false,
    )
    .await;
    sqlx::raw_sql("CREATE FUNCTION native_test_projection_failure() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic projection failure'; END $$; CREATE TRIGGER native_test_projection_failure BEFORE INSERT ON context_projection_versions FOR EACH ROW EXECUTE FUNCTION native_test_projection_failure();").execute(store.pool()).await.expect("owned transactional failure fixture");
    let id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
        .fetch_one(store.pool())
        .await
        .expect("store identity");
    let workspace = root.clone();
    let observed = store.clone();
    let result = store
        .with_native_commit(root.clone(), id, move |session| {
            let source = session.fresh_source().expect("source gate");
            let engine = HarnessEngine::with_source(
                source.view_root(),
                &workspace,
                source.clone(),
                context_fixture::configuration(),
            )
            .expect("engine");
            let mut measured = SqlMeasuredSession {
                inner: session,
                store: observed,
                body_bytes: "# Atomic projection".len(),
                projection_failure: true,
                phases: Vec::new(),
            };
            Ok(HarnessExecutionRecord::apply_durable(
                &engine,
                "native-projection-rollback",
                Some(&mut measured),
            )
            .is_err())
        })
        .await
        .expect("transaction failure returned");
    assert!(result);
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context_materials WHERE path='knowledge/native-atomic.md'",
    )
    .fetch_one(store.pool())
    .await
    .expect("rollback count");
    assert_eq!(count, 0);
    sqlx::raw_sql("DROP TRIGGER native_test_projection_failure ON context_projection_versions; DROP FUNCTION native_test_projection_failure();").execute(store.pool()).await.expect("remove owned failure fixture");
    let output = harness_fixture::command(
        &store,
        &root,
        &root,
        "recover",
        &["--run-id".into(), "native-projection-rollback".into()],
    )
    .await;
    assert!(
        output.status.success(),
        "recover from transaction rollback: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        store
            .read_context(&scope(), "knowledge/native-atomic.md", false)
            .await
            .expect("committed after recovery"),
        "# Atomic projection"
    );
}

async fn composed_cli_fixture(
    store: &Store,
    root: &Path,
    statement: &str,
    live: bool,
) -> (tempfile::TempDir, context_core::harness::PreparedHarnessRun) {
    composed_cli_fixture_for_target(
        store,
        root,
        statement,
        live,
        "vault/personal/knowledge/composed-cli.md",
    )
    .await
}
async fn composed_cli_fixture_for_target(
    store: &Store,
    root: &Path,
    statement: &str,
    live: bool,
    target: &str,
) -> (tempfile::TempDir, context_core::harness::PreparedHarnessRun) {
    use context_core::harness::*;
    let sha = harness_fixture::seed(store).await;
    let mut request = harness_fixture::envelope(&sha, vec![target.into()], vec![], false);
    let DraftTaskRequest::Write(task) = &mut request.draft else {
        unreachable!()
    };
    let DraftValue::Resolved { value, .. } = &mut task.common.task_statement else {
        unreachable!()
    };
    *value = statement.to_owned();
    let RequestSource::UserLanguage { statements } = &mut request.source else {
        unreachable!()
    };
    statements[0].identifier = "request".to_owned();
    statements[0].text = statement.to_owned();
    let mut input = serde_json::to_value(&request).unwrap();
    for record in input["decision_trace"]["records"].as_array_mut().unwrap() {
        record["value_digest"] = json!("");
        if record["kind"] == "user-statement" {
            record["statement_identifiers"] = json!(["request"]);
        }
    }
    let files = tempfile::tempdir().unwrap();
    let request_path = files.path().join("request.json");
    let capabilities_path = files.path().join("capabilities.json");
    let plan_path = files.path().join("plan.json");
    let prepared_path = files.path().join("prepared.json");
    fs::write(&request_path, serde_json::to_vec(&input).unwrap()).unwrap();
    let mut capabilities = harness_fixture::capabilities();
    if live {
        capabilities.lifecycle.max_role_execution_millis = 180_000;
        // The observed CLI did not exit within 500ms of turn.completed.
        // Declare a finite real-process closing budget; fixture/fault tests retain
        // their short deadlines and the adapter's termination checks are unchanged.
        capabilities.lifecycle.max_role_close_millis = 10_000;
        capabilities.lifecycle.max_total_role_millis = 191_000;
    }
    fs::write(
        &capabilities_path,
        serde_json::to_vec(&capabilities).unwrap(),
    )
    .unwrap();
    let mut args = vec![
        "--request-envelope".into(),
        request_path.display().to_string(),
        "--runtime-capabilities".into(),
        capabilities_path.display().to_string(),
    ];
    let uncomposed = harness_fixture::command(store, root, root, "resolve", &args).await;
    assert!(!uncomposed.status.success());
    assert!(
        uncomposed.stdout.is_empty(),
        "incomplete envelopes remain non-executable"
    );
    args.push("--compose-decisions".into());
    let resolved = harness_fixture::command(store, root, root, "resolve", &args).await;
    assert!(
        resolved.status.success(),
        "{}",
        String::from_utf8_lossy(&resolved.stderr)
    );
    let plan: HarnessPlan = serde_json::from_slice(&resolved.stdout).unwrap();
    assert_eq!(plan.resolved_request.request.objective, statement);
    assert_eq!(plan.resolved_request.request.owner, DataOwner::Personal);
    assert_eq!(plan.resolved_request.request.targets, [target]);
    fs::write(&plan_path, &resolved.stdout).unwrap();
    let prepared = harness_fixture::command(
        store,
        root,
        root,
        "prepare",
        &[
            "--plan".into(),
            plan_path.display().to_string(),
            "--runtime-capabilities".into(),
            capabilities_path.display().to_string(),
            "--prepared-output".into(),
            prepared_path.display().to_string(),
        ],
    )
    .await;
    assert!(
        prepared.status.success(),
        "{}",
        String::from_utf8_lossy(&prepared.stderr)
    );
    let prepared: PreparedHarnessRun = serde_json::from_slice(&prepared.stdout).unwrap();
    assert!(prepared.accepted_tool_plan.is_none());
    (files, prepared)
}

#[tokio::test]
async fn native_cli_releases_gate_during_model_wait_and_rejects_changed_original() {
    native_gate_wait_case(0).await;
}
#[tokio::test]
async fn native_cli_rejects_same_byte_revision_drift_after_model_wait() {
    native_gate_wait_case(1).await;
}
#[tokio::test]
async fn native_cli_pending_apply_after_model_wait_retains_rejected_result() {
    native_gate_wait_case(2).await;
}
// 0: changed bytes; 1: original bytes restored at a newer revision; 2: pending apply.
async fn native_gate_wait_case(mutation: u8) {
    use context_core::harness::*;
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let view = harness_fixture::view();
    let root = view.path().canonicalize().unwrap();
    let (files, prepared) = composed_cli_fixture_for_target(
        &store,
        &root,
        "Update the selected synthetic document and independently review it.",
        true,
        "vault/personal/knowledge/skills.md",
    )
    .await;
    let binary = files.path().join("synthetic-codex");
    fs::write(
        &binary,
        r#"#!/bin/sh
cat > "$0.input"
printf '%s\n' '{"type":"thread.started","thread_id":"synthetic-wait"}' '{"type":"turn.started"}'
touch "$0.ready"
while [ ! -f "$0.events" ]; do sleep 0.02; done
cat "$0.events"
"#,
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let args = vec![
        "--prepared-run".into(),
        files.path().join("prepared.json").display().to_string(),
        "--run-id".into(),
        "gate-drift".into(),
        "--codex-binary".into(),
        binary.display().to_string(),
    ];
    let run = harness_fixture::command(&store, &root, &root, "begin", &args);
    let mutate = async {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !binary.with_extension("ready").exists() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("native role must reach its wait point");
        let head_path = root.join(".llm-context-vault-harness/runs/gate-drift/head.json");
        let before = fs::read(&head_path).unwrap();
        let head: HarnessExecutionRecord = serde_json::from_slice(&before).unwrap();
        let invocation = &head.ready_role_invocations[0];
        assert_eq!(
            fs::read(binary.with_extension("input")).unwrap(),
            serde_json::to_vec(&invocation.segments).unwrap()
        );
        let original = store
            .read_context(&scope(), "knowledge/skills.md", false)
            .await
            .unwrap();
        let updated = if mutation == 2 {
            sqlx::query("WITH i AS (SELECT gen_random_uuid() AS id) INSERT INTO context_apply_batches(apply_id,store_id,core_run_id,prepared_run_digest,candidate_digest,expected_source_versions,context_targets,core_apply_attempt_id,expected_batch_id,expected_journal_locator,state) SELECT i.id,s.store_id,'synthetic-pending',repeat('a',64),repeat('b',64),'{}','[{}]',i.id::text,i.id::text,i.id::text,'pending' FROM i CROSS JOIN context_store s")
                .execute(store.pool()).await.map(|_| ()).map_err(|_| Error::Storage)
        } else {
            match store
                .edit_context(
                    &scope(),
                    "knowledge/skills.md",
                    1,
                    &ontology::store::digest(original.as_bytes()),
                    "# Edited while the model waits",
                )
                .await
            {
                Ok(edited) if mutation == 1 => store
                    .edit_context(
                        &scope(),
                        "knowledge/skills.md",
                        edited.revision,
                        &edited.content_digest,
                        &original,
                    )
                    .await
                    .map(|_| ()),
                Ok(_) => Ok(()),
                Err(error) => Err(error),
            }
        };
        // Unblock even on a test assertion failure, so the owned child exits.
        let event =
            harness_fixture::role_event(&prepared, invocation, "# Proposed stale candidate");
        let HarnessExecutionEvent::RoleResult { result } = event else {
            unreachable!()
        };
        let events = [
            json!({"type":"item.completed","item":{"id":"answer","type":"agent_message",
                "text":json!({"outcome":result.outcome}).to_string()}}),
            json!({"type":"turn.completed","usage":{"input_tokens":100,"cached_input_tokens":20,
                "cache_write_input_tokens":0,"output_tokens":10,"reasoning_output_tokens":5}}),
        ]
        .map(|value| value.to_string())
        .join("\n")
            + "\n";
        fs::write(binary.with_extension("events"), events).unwrap();
        assert!(
            updated.is_ok(),
            "mutation must succeed while Codex waits: {updated:?}"
        );
        (before, head_path, original)
    };
    let (output, (before, head_path, original)) = tokio::join!(run, mutate);
    assert!(
        !output.status.success(),
        "stale role result must be rejected"
    );
    assert!(output.stdout.is_empty());
    assert_eq!(
        fs::read(head_path).unwrap(),
        before,
        "rejection preserves the accepted durable head"
    );
    let diagnostics: Vec<Value> = String::from_utf8(output.stderr)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let observation = diagnostics
        .iter()
        .find(|value| value["kind"] == "native-role-observation")
        .unwrap();
    assert_eq!(observation["usage"]["input_tokens"], 100);
    assert_eq!(
        observation["serialized_input_bytes"],
        observation["delivered_input_bytes"]
    );
    let rejected = diagnostics.last().unwrap();
    assert_eq!(rejected["kind"], "role-submission-error");
    assert!(rejected["submission"]["submitted_result"].is_object());
    if mutation == 2 {
        sqlx::query("DELETE FROM context_apply_batches WHERE core_run_id='synthetic-pending'")
            .execute(store.pool())
            .await
            .expect("reacquisition failure released the gate");
    }
    assert_eq!(
        store
            .read_context(&scope(), "knowledge/skills.md", false)
            .await
            .unwrap(),
        if mutation == 0 {
            "# Edited while the model waits"
        } else {
            &original
        }
    );
}

#[tokio::test]
async fn native_shared_sessions_read_concurrently_with_separate_views() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(1, b"# Concurrent original");
    import(&store, &root).await;
    let first_view = harness_fixture::view();
    let second_view = harness_fixture::view();
    let first_root = first_view.path().canonicalize().unwrap();
    let second_root = second_view.path().canonicalize().unwrap();
    let (ready, started) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let first_store = store.clone();
    let first = tokio::spawn(async move {
        first_store
            .with_native_context(first_root, move |source| {
                source
                    .open_file(Path::new("vault/personal/0000.md"), 65536)
                    .unwrap();
                ready.send(()).unwrap();
                wait.recv_timeout(std::time::Duration::from_secs(10))
                    .unwrap();
                Ok(())
            })
            .await
    });
    started.await.unwrap();
    let concurrent = store
        .with_native_context(second_root, |source| {
            source
                .open_file(Path::new("vault/personal/0000.md"), 65536)
                .unwrap();
            Ok(())
        })
        .await;
    let blocked_write = store
        .edit_context(
            &scope(),
            "0000.md",
            1,
            &ontology::store::digest(b"# Concurrent original"),
            "# Concurrent edit",
        )
        .await;
    release.send(()).unwrap();
    first.await.unwrap().unwrap();
    assert!(
        concurrent.is_ok(),
        "shared readers must coexist: {concurrent:?}"
    );
    assert!(matches!(blocked_write, Err(Error::ContextPending)));
    store
        .edit_context(
            &scope(),
            "0000.md",
            1,
            &ontology::store::digest(b"# Concurrent original"),
            "# Concurrent edit",
        )
        .await
        .expect("write succeeds once both reading sessions close");
}

#[tokio::test]
async fn native_cli_blocked_stdout_releases_database_and_view_locks() {
    native_blocked_output_case(false).await;
}

#[tokio::test]
async fn native_cli_blocked_observations_release_database_and_view_locks() {
    native_blocked_output_case(true).await;
}

// Fill only the dedicated child's pipe before exec. No production descriptor
// flags, process-lifecycle rules or application database are changed by this test.
async fn native_blocked_output_case(block_stderr: bool) {
    use context_core::harness::HarnessExecutionRecord;
    use std::os::unix::{io::AsRawFd, process::CommandExt};
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let view = harness_fixture::view();
    let root = view.path().canonicalize().unwrap();
    let (files, _) = composed_cli_fixture_for_target(
        &store,
        &root,
        "Update the exact synthetic source.",
        // A synthetic role must use the 1s fixture close budget, not the live
        // model's 10s budget: the lock proof below has a separate 5s bound.
        false,
        "vault/personal/knowledge/skills.md",
    )
    .await;
    let id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
        .fetch_one(store.pool())
        .await
        .unwrap();
    let original = store
        .read_context(&scope(), "knowledge/skills.md", false)
        .await
        .unwrap();
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_ontology"));
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("TMPDIR", std::env::temp_dir())
        .env("DATABASE_URL", std::env::var("TEST_DATABASE_URL").unwrap())
        .args(["harness", "begin", "--context-view"])
        .arg(&root)
        .arg("--workspace-root")
        .arg(&root)
        .arg("--policy-config")
        .arg(context_fixture::configuration_path())
        .args([
            "--store-id",
            &id,
            "--run-id",
            "blocked-output",
            "--prepared-run",
        ])
        .arg(files.path().join("prepared.json"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if block_stderr {
        let binary = files.path().join("cancelled-codex");
        fs::write(&binary, r#"#!/bin/sh
cat >/dev/null
printf '%s\n' '{"type":"thread.started","thread_id":"blocked-output"}' '{"type":"turn.started"}' '{"type":"item.completed","item":{"id":"answer","type":"agent_message","text":"{\"outcome\":{\"status\":\"cancelled\"}}"}}' '{"type":"turn.completed","usage":{"input_tokens":100,"cached_input_tokens":20,"cache_write_input_tokens":0,"output_tokens":10,"reasoning_output_tokens":5}}'
# Keep both role pipes open after the terminal event to exercise forced closing.
exec sleep 30
"#).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        command.arg("--codex-binary").arg(binary);
    }
    // SAFETY: pre_exec uses only fcntl/write and errno inspection on this child's
    // owned pipe, with no allocation, locks or non-async-signal-safe operations.
    unsafe {
        command.as_std_mut().pre_exec(move || {
            let fd = if block_stderr {
                libc::STDERR_FILENO
            } else {
                libc::STDOUT_FILENO
            };
            let flags = libc::fcntl(fd, libc::F_GETFL);
            if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let fill = [b'x'; 4096];
            for size in [4096, 1] {
                while libc::write(fd, fill.as_ptr().cast(), size) >= 0 {}
                if std::io::Error::last_os_error().kind() != std::io::ErrorKind::WouldBlock {
                    return Err(std::io::Error::last_os_error());
                }
            }
            if libc::fcntl(fd, libc::F_SETFL, flags) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().unwrap();
    let head_path = root.join(".llm-context-vault-harness/runs/blocked-output/head.json");
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join(".context-lock"))
        .unwrap();
    let proof = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let ready = fs::read(&head_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<HarnessExecutionRecord>(&bytes).ok())
                .is_some_and(|head| !block_stderr || !head.role_execution.role_results.is_empty());
            if ready {
                // SAFETY: this is a valid descriptor to the synthetic view lock.
                if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                    unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_UN) };
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        loop {
            match store
                .edit_context(
                    &scope(),
                    "knowledge/skills.md",
                    1,
                    &ontology::store::digest(original.as_bytes()),
                    "# Edited during blocked output",
                )
                .await
            {
                Err(Error::ContextPending) => {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await
                }
                result => break result,
            }
        }
    })
    .await;
    let still_blocked = child.try_wait().unwrap().is_none();
    child.start_kill().unwrap();
    child.wait().await.unwrap();
    assert!(still_blocked, "the pipe must keep the actual CLI running");
    proof
        .expect("output must not retain the DB gate or owned view lock")
        .expect("manual edit must succeed while CLI output stays blocked");
}

#[tokio::test]
async fn native_body_cache_byte_capacity_keeps_reads_available() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(3, &vec![b'x'; 6 * 1024 * 1024]);
    import(&store, &root).await;
    let view = harness_fixture::view();
    store
        .with_native_context(view.path().canonicalize().unwrap(), |source| {
            for n in 0..3 {
                source
                    .open_file(
                        Path::new(&format!("vault/personal/{n:04}.md")),
                        16 * 1024 * 1024,
                    )
                    .unwrap();
            }
            let before = source.body_queries();
            source
                .open_file(Path::new("vault/personal/0000.md"), 16 * 1024 * 1024)
                .unwrap();
            assert_eq!(source.body_queries(), before);
            source
                .open_file(Path::new("vault/personal/0002.md"), 16 * 1024 * 1024)
                .unwrap();
            assert_eq!(
                source.body_queries(),
                before + 1,
                "byte cap skips caching without rejecting reads"
            );
            Ok(())
        })
        .await
        .unwrap();
    let id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
        .fetch_one(store.pool())
        .await
        .unwrap();
    store
        .with_native_commit(view.path().canonicalize().unwrap(), id, |session| {
            let source = session.fresh_source().unwrap();
            let path = Path::new("vault/personal/0000.md");
            source.open_file(path, 16 * 1024 * 1024).unwrap();
            source.open_file(path, 16 * 1024 * 1024).unwrap();
            assert_eq!(
                source.body_queries(),
                2,
                "exclusive recovery source cannot retain stale bodies"
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn native_cli_composition_and_rejected_manual_role_preserve_source_and_head() {
    use context_core::harness::*;
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let view = harness_fixture::view();
    let root = view.path().canonicalize().unwrap();
    let (files, prepared) = composed_cli_fixture(
        &store,
        &root,
        "Create the exact synthetic document; independently review its candidate.",
        false,
    )
    .await;
    let begun = harness_fixture::command(
        &store,
        &root,
        &root,
        "begin",
        &[
            "--prepared-run".into(),
            files.path().join("prepared.json").display().to_string(),
            "--run-id".into(),
            "composed-cli".into(),
        ],
    )
    .await;
    assert!(
        begun.status.success(),
        "{}",
        String::from_utf8_lossy(&begun.stderr)
    );
    let head: HarnessExecutionRecord = serde_json::from_slice(&begun.stdout).unwrap();
    let mut event = harness_fixture::role_event(
        &prepared,
        &head.ready_role_invocations[0],
        "# Unaccepted candidate",
    );
    let HarnessExecutionEvent::RoleResult { result } = &mut event else {
        unreachable!()
    };
    result.invocation_digest = "f".repeat(64);
    let rejected_result = result.clone();
    let event_path = files.path().join("rejected-event.json");
    fs::write(&event_path, serde_json::to_vec(&event).unwrap()).unwrap();
    let rejected = harness_fixture::command(
        &store,
        &root,
        &root,
        "advance",
        &[
            "--run-id".into(),
            "composed-cli".into(),
            "--event".into(),
            event_path.display().to_string(),
        ],
    )
    .await;
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty());
    let diagnostic: Value = serde_json::from_slice(&rejected.stderr).unwrap();
    assert_eq!(diagnostic["kind"], "role-submission-error");
    assert_eq!(
        diagnostic["submission"]["submitted_result"],
        serde_json::to_value(rejected_result).unwrap()
    );
    let stored: HarnessExecutionRecord = serde_json::from_slice(
        &fs::read(root.join(".llm-context-vault-harness/runs/composed-cli/head.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        stored, head,
        "a rejected result must not update the accepted head"
    );
    assert_eq!(
        store
            .read_context(&scope(), "knowledge/composed-cli.md", false)
            .await,
        Err(Error::NotFound)
    );
}

#[tokio::test]
#[ignore = "explicit authenticated Codex integration; synthetic sources and isolated test database only"]
async fn native_cli_real_codex_producer_and_reviewer() {
    use context_core::harness::*;
    let _guard = TEST_LOCK.lock().await;
    let binary =
        std::env::var("ONTOLOGY_TEST_CODEX_BINARY").expect("explicit installed Codex binary");
    assert!(Path::new(&binary).is_absolute());
    let store = store().await;
    let view = harness_fixture::view();
    let root = view.path().canonicalize().unwrap();
    let (files, prepared) = composed_cli_fixture(&store, &root,
        "Create only vault/personal/knowledge/composed-cli.md with exactly this Markdown: # Synthetic CLI probe\nVerified fixture content.\n. Independently review the proposed document using the supplied policies and assigned requirements.", true,
    ).await;
    let id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
        .fetch_one(store.pool())
        .await
        .unwrap();
    // Preserve the existing authenticated CLI environment, overriding only the
    // database with this owned, freshly reset synthetic test store.
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_ontology"))
        .env("DATABASE_URL", std::env::var("TEST_DATABASE_URL").unwrap())
        .args(["harness", "begin", "--context-view"])
        .arg(&root)
        .arg("--workspace-root")
        .arg(&root)
        .args(["--store-id", &id, "--prepared-run"])
        .arg(files.path().join("prepared.json"))
        .arg("--policy-config")
        .arg(context_fixture::configuration_path())
        .args(["--run-id", "real-composed-cli", "--codex-binary", &binary])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let head: HarnessExecutionRecord = serde_json::from_slice(&output.stdout).unwrap();
    if head.role_execution.role_results.len() != 2 {
        eprintln!(
            "actual Codex stopped before independent review: {}",
            serde_json::to_string(&head.role_execution.role_results).unwrap()
        );
    }
    assert_eq!(head.state, HarnessExecutionState::Evaluated);
    assert_eq!(head.role_execution.role_results.len(), 2);
    assert_eq!(
        head.role_execution.role_results[0].role,
        HarnessRole::Writer
    );
    assert_eq!(
        head.role_execution.role_results[1].role,
        HarnessRole::Reviewer
    );
    assert_ne!(
        head.role_execution.role_results[0].context_id,
        head.role_execution.role_results[1].context_id
    );
    assert_eq!(
        head.evaluation.as_ref().unwrap().subject_status,
        SubjectStatus::Accepted
    );
    let validated = harness_fixture::command(
        &store,
        &root,
        &root,
        "validate",
        &["--run-id".into(), "real-composed-cli".into()],
    )
    .await;
    assert!(
        validated.status.success(),
        "{}",
        String::from_utf8_lossy(&validated.stderr)
    );
    assert_eq!(
        store
            .read_context(&scope(), "knowledge/composed-cli.md", false)
            .await,
        Err(Error::NotFound)
    );
    assert!(prepared.accepted_tool_plan.is_none());
    for result in &head.role_execution.role_results {
        assert_eq!(result.lifecycle.context_id, result.context_id);
        assert_eq!(
            result.lifecycle.terminal_state,
            RoleTerminalState::Completed
        );
        assert!(result.lifecycle.closed_at_millis >= result.lifecycle.terminal_at_millis);
    }
}

#[tokio::test]
async fn native_harness_cli_write_revise_and_terminal_reentry() {
    use context_core::harness::HarnessExecutionState;
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let policy = harness_fixture::seed(&store).await;
    let view = harness_fixture::view();
    let root = view.path().canonicalize().expect("view");
    let request = harness_fixture::envelope(
        &policy,
        vec!["vault/personal/knowledge/native-cli-write.md".into()],
        vec![],
        false,
    );
    let (artifacts, prepared) = harness_fixture::cli_prepared(&store, &root, &root, request).await;
    let evaluated = harness_fixture::cli_roles(
        &store,
        &root,
        &root,
        artifacts.path(),
        &prepared,
        "native-cli-write",
        "# Accepted CLI write",
        false,
    )
    .await;
    assert_eq!(evaluated.state, HarnessExecutionState::Evaluated);
    for verb in ["validate", "apply", "apply", "recover", "recover"] {
        let output = harness_fixture::command(
            &store,
            &root,
            &root,
            verb,
            &["--run-id".into(), "native-cli-write".into()],
        )
        .await;
        assert!(
            output.status.success(),
            "native CLI {verb}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
    let request = harness_fixture::envelope(
        &policy,
        vec!["vault/personal/knowledge/native-cli-write.md".into()],
        vec![],
        false,
    );
    let (artifacts, prepared) = harness_fixture::cli_prepared(&store, &root, &root, request).await;
    harness_fixture::cli_roles(
        &store,
        &root,
        &root,
        artifacts.path(),
        &prepared,
        "native-cli-revise",
        "# Rejected CLI write",
        true,
    )
    .await;
    let revised = harness_fixture::command(
        &store,
        &root,
        &root,
        "revise",
        &["--run-id".into(), "native-cli-revise".into()],
    )
    .await;
    assert!(
        revised.status.success(),
        "native CLI revise: {}",
        String::from_utf8_lossy(&revised.stderr)
    );
    assert_eq!(
        store
            .read_context(&scope(), "knowledge/native-cli-write.md", false)
            .await
            .expect("rejection preserves accepted current material"),
        "# Accepted CLI write"
    );
}

#[tokio::test]
async fn native_promotion_commit_search_and_next_read() {
    use context_core::harness::*;
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let policy = harness_fixture::seed(&store).await;
    let view = harness_fixture::view();
    let root = view.path().canonicalize().expect("owned promotion view");
    let source_path = "vault/personal/knowledge/promotion-source.md";
    native_write(
        &store,
        &root,
        "promotion-source-write",
        source_path,
        "# Synthetic source\nVerified synthetic observation",
        false,
    )
    .await;
    let mut request = harness_fixture::envelope(&policy, vec![source_path.into()], vec![], false);
    let DraftTaskRequest::Write(write) = request.draft else {
        panic!("synthetic initial request")
    };
    request.decision_trace.records.retain(|record| !matches!(record, DecisionRecord::UserStatement { identifier, .. } if identifier == "decision-0000000000000005"));
    request.draft = DraftTaskRequest::Analysis(DraftAnalysisContract {
        common: write.common,
        analysis_kind: harness_fixture::value(
            AnalysisKind::Investigation,
            5,
            &mut request.decision_trace.records,
        ),
        targets: Some(write.targets),
    });
    let prepared = harness_fixture::prepare(
        &store,
        root.clone(),
        root.clone(),
        request,
        "promotion-investigation".into(),
        "Verified synthetic observation".into(),
    )
    .await;
    let handoff = store
        .with_native_context(root.clone(), move |source| {
            let engine = HarnessEngine::with_source(
                source.view_root(),
                source.view_root(),
                source.clone(),
                context_fixture::configuration(),
            )
            .expect("native investigation engine");
            let (_, actual) = HarnessExecutionRecord::load_durable_prepared(
                source.view_root(),
                "promotion-investigation",
            )
            .expect("actual durable source preparation");
            assert_eq!(actual, prepared);
            let head: HarnessExecutionRecord = decode_current_json(
                &fs::read(
                    source
                        .view_root()
                        .join(".llm-context-vault-harness/runs/promotion-investigation/head.json"),
                )
                .expect("actual head"),
                "actual investigation head",
            )
            .expect("strict source evaluation");
            let evaluation = engine
                .evaluate_execution(
                    &prepared.plan.resolved_request,
                    &prepared.role_run,
                    &head.revision_history,
                    &head.role_execution,
                )
                .expect("fully recomputed source evaluation");
            assert_eq!(evaluation.subject_status, SubjectStatus::Accepted);
            let handoff = PromotionHandoff::from_evaluation(
                &prepared.plan.resolved_request.plan,
                &prepared.role_run,
                &head.revision_history,
                &evaluation,
                "proposal-0000000000000003",
            )
            .expect("accepted SpecialistMemory handoff");
            assert_eq!(head.evaluation.as_ref(), Some(&evaluation));
            Ok(handoff)
        })
        .await
        .expect("source evaluation and actual handoff");
    let PromotionProposalOrigin::SpecialistMemory { evidence, .. } = &handoff.proposal.origin
    else {
        panic!("Specialist origin required")
    };
    assert_eq!(evidence[0].source_owner, DataOwner::Personal);
    assert_eq!(evidence[0].relative_path, source_path);
    assert_eq!(
        evidence[0].content_digest,
        ontology::store::digest(b"# Synthetic source\nVerified synthetic observation")
    );
    let target = "vault/personal/knowledge/promoted-native.md";
    let mut request = harness_fixture::envelope(&policy, vec![target.into()], vec![], true);
    let DraftTaskRequest::Curation(curation) = &mut request.draft else {
        panic!("separate curation")
    };
    let handoff_sha =
        ontology::store::digest(&serde_json::to_vec(&handoff).expect("exact handoff value"));
    request
        .decision_trace
        .records
        .push(DecisionRecord::ModelProposal {
            identifier: "decision-0000000000000009".into(),
            value_digest: handoff_sha.clone(),
            based_on_record_identifiers: vec!["decision-0000000000000001".into()],
        });
    request
        .decision_trace
        .records
        .push(DecisionRecord::UserConfirmation {
            identifier: "decision-000000000000000a".into(),
            value_digest: handoff_sha,
            confirms_record_identifier: "decision-0000000000000009".into(),
            statement_identifiers: vec!["statement-0000000000000001".into()],
        });
    curation.promotion_handoff = Some(DraftValue::Resolved {
        value: handoff.clone(),
        decision_identifier: "decision-000000000000000a".into(),
    });
    let prepared = harness_fixture::prepare(
        &store,
        root.clone(),
        root.clone(),
        request,
        "promotion-curation".into(),
        handoff.proposal.content.clone(),
    )
    .await;
    assert_eq!(
        prepared
            .plan
            .resolved_request
            .plan
            .promotion_handoff
            .as_ref(),
        Some(&handoff)
    );
    let (head, _) = harness_fixture::apply(
        &store,
        root.clone(),
        root.clone(),
        "promotion-curation".into(),
        prepared,
    )
    .await;
    let writer = head
        .role_execution
        .role_results
        .iter()
        .find(|r| r.role == HarnessRole::Writer)
        .expect("actual Writer result");
    let RoleExecutionOutcome::Completed {
        result:
            CompletedRoleResult::Writer {
                artifact: WriterArtifact::Curation { entries, .. },
            },
    } = &writer.outcome
    else {
        panic!("actual curation entry")
    };
    assert_eq!(
        entries[0].promotion_handoff_digest.as_ref(),
        Some(&handoff.handoff_digest)
    );
    assert_eq!(
        store
            .read_context(&scope(), "knowledge/promoted-native.md", false)
            .await
            .expect("next native read"),
        handoff.proposal.content
    );
    let search = store
        .semantic_context(&[scope()], "nativepromotionsearchtoken", 10, false)
        .await
        .expect("native promotion search");
    assert_eq!(search["items"].as_array().expect("search results").len(), 1);
    store
        .with_native_context(root, |source| {
            let mut file = source
                .open_file(
                    Path::new("vault/personal/knowledge/promoted-native.md"),
                    65536,
                )
                .expect("fresh provider next read");
            let mut bytes = String::new();
            std::io::Read::read_to_string(&mut file, &mut bytes).expect("exact read");
            assert!(bytes.contains("nativepromotionsearchtoken"));
            Ok(())
        })
        .await
        .expect("reopened private view");
}

// Revision proof: the contracts and verified bytes below are issued by the actual
// durable Core path. Corruption is confined to this owned PostgreSQL fixture.
async fn native_rows_fingerprint(store: &Store) -> String {
    sqlx::query_scalar("SELECT encode(sha256(convert_to(jsonb_build_object('current',(SELECT jsonb_agg(to_jsonb(m) ORDER BY material_id) FROM context_materials m),'history',(SELECT jsonb_agg(to_jsonb(v) ORDER BY material_id,revision) FROM context_material_versions v),'projections',(SELECT jsonb_agg(to_jsonb(p) ORDER BY material_id,revision) FROM context_projection_versions p))::text,'UTF8')),'hex')")
        .fetch_one(store.pool()).await.expect("owned current/history/projection fingerprint")
}
struct NativeContractCapture<'a> {
    inner: &'a mut ontology::native_context::NativeContextSession,
    captured: Option<context_core::harness::ContextApplyContract>,
    identity: context_core::harness::SourceStoreIdentity,
    recover_called: bool,
}
impl context_core::harness::ContextCommitSession for NativeContractCapture<'_> {
    fn verify_pre_apply_close(
        &mut self,
        c: &context_core::harness::ContextPreApplyCloseContract,
    ) -> context_core::harness::HarnessResult<()> {
        self.inner.verify_pre_apply_close(c)
    }
    fn store_identity(&self) -> &context_core::harness::SourceStoreIdentity {
        &self.identity
    }
    fn begin(
        &mut self,
        contract: &context_core::harness::ContextApplyContract,
    ) -> context_core::harness::HarnessResult<()> {
        self.captured = Some(contract.clone());
        panic!("owned capture at actual Core begin before database effect")
    }
    fn recover(
        &mut self,
        contract: &context_core::harness::ContextRecoveryContract,
    ) -> context_core::harness::HarnessResult<context_core::harness::ContextRecoveryStatus> {
        self.recover_called = true;
        self.inner.recover(contract)
    }
    fn commit(
        &mut self,
        verified: &context_core::harness::VerifiedContextCommit<'_>,
    ) -> context_core::harness::HarnessResult<context_core::harness::ContextCommitReceipt> {
        self.inner.commit(verified)
    }
    fn finalize(
        &mut self,
        proof: &context_core::harness::ContextTerminalProof,
    ) -> context_core::harness::HarnessResult<()> {
        self.inner.finalize(proof)
    }
    fn abort(
        &mut self,
        proof: &context_core::harness::ContextAbortProof,
    ) -> context_core::harness::HarnessResult<()> {
        self.inner.abort(proof)
    }
}
async fn capture_native_contract(
    store: &Store,
    root: &Path,
    run: &str,
) -> context_core::harness::ContextApplyContract {
    use context_core::harness::*;
    let id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
        .fetch_one(store.pool())
        .await
        .expect("owned identity");
    let workspace = root.to_owned();
    let run = run.to_owned();
    store
        .with_native_commit(root.to_owned(), id, move |session| {
            let source = session.fresh_source().expect("no effect before capture");
            let engine = HarnessEngine::with_source(
                source.view_root(),
                &workspace,
                source.clone(),
                context_fixture::configuration(),
            )
            .expect("actual native Core engine");
            let mut capture = NativeContractCapture {
                identity: session.store_identity().clone(),
                inner: session,
                captured: None,
                recover_called: false,
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                HarnessExecutionRecord::apply_durable(&engine, &run, Some(&mut capture))
            }));
            assert!(result.is_err(), "actual begin capture reached");
            Ok(capture.captured.expect("opaque Core contract captured"))
        })
        .await
        .expect("capture session released")
}
async fn native_begin_rejects(
    store: &Store,
    root: &Path,
    contract: context_core::harness::ContextApplyContract,
) {
    use context_core::harness::ContextCommitSession;
    let before = native_rows_fingerprint(store).await;
    let id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
        .fetch_one(store.pool())
        .await
        .expect("owned identity");
    store
        .with_native_commit(root.to_owned(), id, move |session| {
            assert!(
                session.begin(&contract).is_err(),
                "actual PostgreSQL begin rejects contract"
            );
            Ok(())
        })
        .await
        .expect("rejected begin session released");
    assert_eq!(native_rows_fingerprint(store).await, before);
}
async fn native_recovery_rejects(
    store: &Store,
    view: &Path,
    workspace: &Path,
    contract: &context_core::harness::ContextApplyContract,
) {
    use context_core::harness::*;
    let before = native_rows_fingerprint(store).await;
    let id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
        .fetch_one(store.pool())
        .await
        .expect("owned identity");
    let workspace = workspace.to_owned();
    let run = contract.attempt().run_identifier.clone();
    let identity = contract.store_identity().expect("native contract").clone();
    let attempt_path = workspace.join(format!(
        ".llm-context-vault-harness/runs/{run}/apply-attempt.json"
    ));
    let attempt = fs::read(&attempt_path).expect("actual retained Core attempt");
    store
        .with_native_commit(view.to_owned(), id, move |session| {
            // Advertise the issued identity solely to route the opaque recovery
            // contract through the real PostgreSQL implementation's own store check.
            let mut boundary = NativeContractCapture {
                inner: session,
                identity,
                captured: None,
                recover_called: false,
            };
            let rejected = HarnessExecutionRecord::open_durable_recovery(
                &workspace,
                &run,
                Some(&mut boundary),
            )
            .is_err();
            assert!(rejected);
            assert!(boundary.recover_called, "actual native recover was reached");
            assert!(
                boundary.inner.source().is_err(),
                "failed recovery cannot release source gate"
            );
            Ok(())
        })
        .await
        .expect("recovery rejection session released");
    assert_eq!(native_rows_fingerprint(store).await, before);
    assert!(fs::read(attempt_path).expect("retained attempt") == attempt);
}

#[tokio::test]
async fn native_revision_contract_boundaries_preserve_pending_and_rows() {
    use context_core::harness::*;
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let policy = harness_fixture::seed(&store).await;
    let view = harness_fixture::view();
    let root = view.path().canonicalize().expect("owned view");
    for (run, path) in [
        (
            "revision-contract-a",
            "vault/personal/knowledge/contract-a.md",
        ),
        (
            "revision-contract-b",
            "vault/personal/knowledge/contract-b.md",
        ),
    ] {
        harness_fixture::prepare(
            &store,
            root.clone(),
            root.clone(),
            harness_fixture::envelope(&policy, vec![path.into()], vec![], false),
            run.into(),
            "# Bound contract body".into(),
        )
        .await;
    }
    let a = capture_native_contract(&store, &root, "revision-contract-a").await;
    let b = capture_native_contract(&store, &root, "revision-contract-b").await;
    assert_ne!(
        a.attempt().attempt_identifier,
        b.attempt().attempt_identifier
    );
    let base_url = std::env::var("TEST_DATABASE_URL").expect("owned DB URL");
    let database = "ontology_test_contract_foreign";
    sqlx::query("CREATE DATABASE ontology_test_contract_foreign")
        .execute(store.pool())
        .await
        .expect("owned foreign database");
    let foreign_url = format!(
        "{}/{database}",
        base_url.rsplit_once('/').expect("owned PostgreSQL URL").0
    );
    let foreign = Store::connect(&foreign_url)
        .await
        .expect("owned foreign store");
    foreign.initialize().await.expect("foreign migrations");
    let foreign_view = harness_fixture::view();
    let foreign_root = foreign_view
        .path()
        .canonicalize()
        .expect("foreign owned view");
    native_begin_rejects(&foreign, &foreign_root, a.clone()).await;
    native_recovery_rejects(&foreign, &foreign_root, &root, &a).await;
    let id = a
        .store_identity()
        .expect("native identity")
        .store_id
        .clone();
    let first = a.clone();
    store
        .with_native_commit(root.clone(), id, move |session| {
            session.begin(&first).expect("actual pending effect");
            session.begin(&first).expect("identical begin idempotent");
            Ok(())
        })
        .await
        .expect("pending session");
    let effect_before: Value =
        sqlx::query_scalar("SELECT to_jsonb(b) FROM context_apply_batches b")
            .fetch_one(store.pool())
            .await
            .expect("actual effect");
    native_begin_rejects(&store, &root, b.clone()).await;
    native_recovery_rejects(&store, &root, &root, &b).await;
    assert!(
        sqlx::query_scalar::<_, Value>("SELECT to_jsonb(b) FROM context_apply_batches b")
            .fetch_one(store.pool())
            .await
            .expect("retained effect")
            == effect_before
    );
    let selected = a
        .non_target_versions()
        .iter()
        .find(|version| version.logical_path == "vault/profile/rules/control.md")
        .expect("actual prepared non-target policy");
    let StoredSourceState::Live { revision, .. } = selected.state else {
        panic!("live prepared policy")
    };
    sqlx::raw_sql("ALTER TABLE context_materials DISABLE TRIGGER USER; UPDATE context_materials SET revision=revision+1 WHERE source_path='profile/rules/control.md'; ALTER TABLE context_materials ENABLE TRIGGER USER;").execute(store.pool()).await.expect("owned same-byte non-target revision drift");
    native_begin_rejects(&store, &root, a.clone()).await;
    native_recovery_rejects(&store, &root, &root, &a).await;
    sqlx::query("ALTER TABLE context_materials DISABLE TRIGGER USER")
        .execute(store.pool())
        .await
        .expect("restore only owned drift");
    sqlx::query(
        "UPDATE context_materials SET revision=$1 WHERE source_path='profile/rules/control.md'",
    )
    .bind(i64::try_from(revision).expect("stored revision"))
    .execute(store.pool())
    .await
    .expect("restore exact prepared version");
    sqlx::query("ALTER TABLE context_materials ENABLE TRIGGER USER")
        .execute(store.pool())
        .await
        .expect("restore material invariants");
    // Each corruption keeps an actual pending effect; rejection must not rewrite,
    // delete, finalize, or abort it. Restore only the injected fixture field.
    for field in [
        "core_contract",
        "core_contract_digest",
        "expected_source_versions",
        "context_targets",
    ] {
        sqlx::query("ALTER TABLE context_apply_batches DISABLE TRIGGER USER")
            .execute(store.pool())
            .await
            .expect("owned corruption setup");
        let mutation = match field {
            "core_contract" => {
                "UPDATE context_apply_batches SET core_contract=convert_to('{}','UTF8'),core_contract_digest=encode(sha256(convert_to('{}','UTF8')),'hex')"
            }
            "core_contract_digest" => {
                "UPDATE context_apply_batches SET core_contract=NULL,core_contract_digest=NULL"
            }
            "expected_source_versions" => {
                "UPDATE context_apply_batches SET expected_source_versions='{}'::jsonb"
            }
            _ => "UPDATE context_apply_batches SET context_targets='[{}]'::jsonb",
        };
        // Disable the owned fixture table's user triggers only while injecting
        // a damaged persisted boundary; restore them before native checks.
        let changed = sqlx::query(mutation).execute(store.pool()).await;
        sqlx::query("ALTER TABLE context_apply_batches ENABLE TRIGGER USER")
            .execute(store.pool())
            .await
            .expect("restore trigger");
        changed.expect("owned contract corruption");
        let damaged: Value = sqlx::query_scalar("SELECT to_jsonb(b) FROM context_apply_batches b")
            .fetch_one(store.pool())
            .await
            .expect("damaged effect");
        native_begin_rejects(&store, &root, a.clone()).await;
        native_recovery_rejects(&store, &root, &root, &a).await;
        assert!(
            sqlx::query_scalar::<_, Value>("SELECT to_jsonb(b) FROM context_apply_batches b")
                .fetch_one(store.pool())
                .await
                .expect("retained damaged effect")
                == damaged
        );
        sqlx::query("ALTER TABLE context_apply_batches DISABLE TRIGGER USER")
            .execute(store.pool())
            .await
            .expect("restore fixture contract");
        sqlx::query("UPDATE context_apply_batches SET core_contract=$1,core_contract_digest=$2,expected_source_versions=$3,context_targets=$4,updated_at=$5::timestamptz")
            .bind(serde_json::to_vec(&a).expect("exact actual contract bytes"))
            .bind(effect_before["core_contract_digest"].as_str().expect("stored digest")).bind(&effect_before["expected_source_versions"]).bind(&effect_before["context_targets"])
            .bind(effect_before["updated_at"].as_str().expect("stored timestamp"))
            .execute(store.pool()).await.expect("restore exact actual effect values");
        sqlx::query("ALTER TABLE context_apply_batches ENABLE TRIGGER USER")
            .execute(store.pool())
            .await
            .expect("restore immutable trigger");
    }
    let recovered = harness_fixture::command(
        &store,
        &root,
        &root,
        "recover",
        &["--run-id".into(), "revision-contract-a".into()],
    )
    .await;
    assert!(recovered.status.success(), "actual no-mutation recovery");
    let state: String = sqlx::query_scalar("SELECT state FROM context_apply_batches")
        .fetch_one(store.pool())
        .await
        .expect("terminal cleanup");
    assert_eq!(state, "aborted");
    assert!(!root.join("vault/personal/knowledge/contract-a.md").exists());
    let absent_recovery = harness_fixture::command(
        &store,
        &root,
        &root,
        "recover",
        &["--run-id".into(), "revision-contract-b".into()],
    )
    .await;
    assert!(
        absent_recovery.status.success(),
        "second actual contract closes without an aliased effect"
    );
    foreign.pool().close().await;
    sqlx::query("DROP DATABASE ontology_test_contract_foreign")
        .execute(store.pool())
        .await
        .expect("remove only owned foreign database");
}

struct NativeCommitRejectionProbe<'a> {
    inner: &'a mut ontology::native_context::NativeContextSession,
    store: Store,
    handle: tokio::runtime::Handle,
    checked: bool,
}
impl context_core::harness::ContextCommitSession for NativeCommitRejectionProbe<'_> {
    fn verify_pre_apply_close(
        &mut self,
        c: &context_core::harness::ContextPreApplyCloseContract,
    ) -> context_core::harness::HarnessResult<()> {
        self.inner.verify_pre_apply_close(c)
    }
    fn store_identity(&self) -> &context_core::harness::SourceStoreIdentity {
        self.inner.store_identity()
    }
    fn begin(
        &mut self,
        c: &context_core::harness::ContextApplyContract,
    ) -> context_core::harness::HarnessResult<()> {
        self.inner.begin(c)
    }
    fn recover(
        &mut self,
        c: &context_core::harness::ContextRecoveryContract,
    ) -> context_core::harness::HarnessResult<context_core::harness::ContextRecoveryStatus> {
        self.inner.recover(c)
    }
    fn finalize(
        &mut self,
        c: &context_core::harness::ContextTerminalProof,
    ) -> context_core::harness::HarnessResult<()> {
        self.inner.finalize(c)
    }
    fn abort(
        &mut self,
        c: &context_core::harness::ContextAbortProof,
    ) -> context_core::harness::HarnessResult<()> {
        self.inner.abort(c)
    }
    fn commit(
        &mut self,
        verified: &context_core::harness::VerifiedContextCommit<'_>,
    ) -> context_core::harness::HarnessResult<context_core::harness::ContextCommitReceipt> {
        use context_core::harness::*;
        let contract = verified.contract();
        assert_eq!(contract.targets().len(), 2);
        for target in contract.targets() {
            let bytes = verified
                .target_content(&target.previous().logical_path)
                .expect("both issued changes have exact bodies");
            assert_eq!(bytes, b"# Imported update\nExact native accepted bytes");
            assert_eq!(
                Some(ontology::store::digest(bytes).as_str()),
                target.resulting_content_digest()
            );
        }
        assert!(
            verified
                .target_content("vault/personal/knowledge/unissued.md")
                .is_none()
        );
        assert!(
            ContextCommitReceipt::from_persisted(verified, vec![], vec![], "a".repeat(64)).is_err(),
            "actual verified contract rejects missing target/body receipt cardinality"
        );
        let before = self.handle.block_on(native_rows_fingerprint(&self.store));
        let revision = contract
            .targets()
            .iter()
            .find_map(|target| match target.previous().state {
                StoredSourceState::Live { revision, .. } => Some(revision),
                _ => None,
            })
            .expect("actual imported target version");
        self.handle.block_on(async {
            sqlx::raw_sql("ALTER TABLE context_materials DISABLE TRIGGER USER; UPDATE context_materials SET revision=revision+1 WHERE path='knowledge/boundary-imported.md'; ALTER TABLE context_materials ENABLE TRIGGER USER;").execute(self.store.pool()).await.expect("owned target version drift at verified commit");
        });
        let drifted = self.handle.block_on(native_rows_fingerprint(&self.store));
        let sql_start = self
            .inner
            .native_sql_observations()
            .expect("complete native trace")
            .len();
        let sql_calls = self.store.calls();
        assert!(
            self.inner.commit(verified).is_err(),
            "actual commit rejects same-byte target revision drift"
        );
        assert_sql_phase(
            self.inner,
            contract,
            2 * b"# Imported update\nExact native accepted bytes".len(),
            sql_start,
            3,
            "target-version-rejection",
        );
        assert_eq!(self.store.calls() - sql_calls, 3);
        assert_eq!(
            self.handle.block_on(native_rows_fingerprint(&self.store)),
            drifted
        );
        self.handle.block_on(async {
            sqlx::query("ALTER TABLE context_materials DISABLE TRIGGER USER").execute(self.store.pool()).await.expect("restore target fixture");
            sqlx::query("UPDATE context_materials SET revision=$1 WHERE path='knowledge/boundary-imported.md'").bind(i64::try_from(revision).expect("stored revision")).execute(self.store.pool()).await.expect("restore exact target version");
            sqlx::query("ALTER TABLE context_materials ENABLE TRIGGER USER").execute(self.store.pool()).await.expect("restore target invariants");
        });
        assert_eq!(
            self.handle.block_on(native_rows_fingerprint(&self.store)),
            before
        );
        let bytes = serde_json::to_vec(contract).expect("actual canonical contract");
        self.handle.block_on(async {
            sqlx::raw_sql("ALTER TABLE context_apply_batches DISABLE TRIGGER USER; UPDATE context_apply_batches SET core_contract=NULL,core_contract_digest=NULL; ALTER TABLE context_apply_batches ENABLE TRIGGER USER;").execute(self.store.pool()).await.expect("owned persisted contract corruption");
        });
        let sql_start = self
            .inner
            .native_sql_observations()
            .expect("complete native trace")
            .len();
        let sql_calls = self.store.calls();
        assert!(
            self.inner.commit(verified).is_err(),
            "actual native commit rejects missing persisted contract"
        );
        assert_sql_phase(
            self.inner,
            contract,
            2 * b"# Imported update\nExact native accepted bytes".len(),
            sql_start,
            1,
            "persisted-contract-rejection",
        );
        assert_eq!(self.store.calls() - sql_calls, 1);
        assert_eq!(
            self.handle.block_on(native_rows_fingerprint(&self.store)),
            before
        );
        self.handle.block_on(async {
            sqlx::query("ALTER TABLE context_apply_batches DISABLE TRIGGER USER").execute(self.store.pool()).await.expect("restore actual contract");
            sqlx::query("UPDATE context_apply_batches SET core_contract=$1,core_contract_digest=$2").bind(&bytes).bind(ontology::store::digest(&bytes)).execute(self.store.pool()).await.expect("restore exact issued bytes");
            sqlx::query("ALTER TABLE context_apply_batches ENABLE TRIGGER USER").execute(self.store.pool()).await.expect("restore invariant triggers");
            sqlx::raw_sql("CREATE FUNCTION native_revision_skip_target() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NULL; END $$; CREATE TRIGGER native_revision_skip_target BEFORE INSERT ON context_materials FOR EACH ROW WHEN (NEW.path='knowledge/boundary-new.md') EXECUTE FUNCTION native_revision_skip_target();").execute(self.store.pool()).await.expect("owned missing-row fault");
        });
        let sql_start = self
            .inner
            .native_sql_observations()
            .expect("complete native trace")
            .len();
        let sql_calls = self.store.calls();
        let failure = self
            .inner
            .commit(verified)
            .expect_err("actual native commit rejects fewer resulting rows than verified targets");
        assert_sql_phase(
            self.inner,
            contract,
            2 * b"# Imported update\nExact native accepted bytes".len(),
            sql_start,
            6,
            "cardinality-rejection",
        );
        assert_eq!(self.store.calls() - sql_calls, 6);

        assert_eq!(
            self.handle.block_on(native_rows_fingerprint(&self.store)),
            before,
            "failed cardinality rolls back imported update, history and projections together"
        );
        let state: String = self
            .handle
            .block_on(
                sqlx::query_scalar("SELECT state FROM context_apply_batches")
                    .fetch_one(self.store.pool()),
            )
            .expect("retained real pending effect");
        assert_eq!(state, "pending");
        self.checked = true;
        Err(failure)
    }
}
#[tokio::test]
async fn native_revision_imported_update_and_commit_cardinality_recovery() {
    use context_core::harness::*;
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let policy = harness_fixture::seed(&store).await;
    let (_fixture, fixture_root) = fixture(0, b"");
    fs::create_dir(fixture_root.join("personal/knowledge")).expect("synthetic imported parent");
    fs::write(
        fixture_root.join("personal/knowledge/boundary-imported.md"),
        b"# Imported original\nImmutable origin bytes",
    )
    .expect("synthetic imported original");
    import(&store, &fixture_root).await;
    let origin_sql = "SELECT jsonb_build_object('material_id',material_id,'origin_kind',origin_kind,'source_root',source_root,'source_digest',source_digest,'imported_at',imported_at) FROM context_materials WHERE path='knowledge/boundary-imported.md'";
    let origin: Value = sqlx::query_scalar(origin_sql)
        .fetch_one(store.pool())
        .await
        .expect("actual imported origin");
    assert_eq!(origin["origin_kind"], "imported-file");
    assert!(!origin["imported_at"].is_null());
    let view = harness_fixture::view();
    let root = view.path().canonicalize().expect("owned view");
    let body = "# Imported update\nExact native accepted bytes";
    let prepared = harness_fixture::prepare(
        &store,
        root.clone(),
        root.clone(),
        harness_fixture::envelope(
            &policy,
            vec![
                "vault/personal/knowledge/boundary-imported.md".into(),
                "vault/personal/knowledge/boundary-new.md".into(),
            ],
            vec![],
            false,
        ),
        "revision-imported".into(),
        body.into(),
    )
    .await;
    let before = native_rows_fingerprint(&store).await;
    let id = prepared
        .plan
        .bound_source_versions()
        .store_identity
        .as_ref()
        .expect("actual store identity")
        .store_id
        .clone();
    let workspace = root.clone();
    let observed = store.clone();
    let handle = tokio::runtime::Handle::current();
    store
        .with_native_commit(root.clone(), id, move |session| {
            let source = session.fresh_source().expect("fresh source gate");
            let engine = HarnessEngine::with_source(
                source.view_root(),
                &workspace,
                source.clone(),
                context_fixture::configuration(),
            )
            .expect("real Core engine");
            let mut probe = NativeCommitRejectionProbe {
                inner: session,
                store: observed,
                handle,
                checked: false,
            };
            assert!(
                HarnessExecutionRecord::apply_durable(
                    &engine,
                    "revision-imported",
                    Some(&mut probe)
                )
                .is_err()
            );
            assert!(probe.checked, "actual verified commit boundary executed");
            Ok(())
        })
        .await
        .expect("failed native commit leaves recovery evidence");
    // Close the failed dedicated transaction before DDL teardown. PostgreSQL
    // releases its row/table locks when the session connection is closed.
    sqlx::raw_sql("DROP TRIGGER native_revision_skip_target ON context_materials; DROP FUNCTION native_revision_skip_target();").execute(store.pool()).await.expect("remove owned fault after connection release");
    assert_eq!(native_rows_fingerprint(&store).await, before);
    for path in ["boundary-imported.md", "boundary-new.md"] {
        assert_eq!(
            fs::read(root.join("vault/personal/knowledge").join(path))
                .expect("retained file-complete target"),
            body.as_bytes()
        );
    }
    let output = harness_fixture::command(
        &store,
        &root,
        &root,
        "recover",
        &["--run-id".into(), "revision-imported".into()],
    )
    .await;
    assert!(
        output.status.success(),
        "actual CLI recovery after rolled-back PG transaction: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).expect("recovered Core artifacts");
    let attempt: HarnessApplyAttemptReceipt = decode_current_json(
        &serde_json::to_vec(&result["apply_attempt"]).expect("actual output attempt"),
        "recovered attempt",
    )
    .expect("strict actual attempt");
    attempt
        .validate(&prepared)
        .expect("actual recovered terminal receipt");
    let HarnessApplyAttemptState::RecoveredFinalized { apply_receipt, .. } = attempt.attempt_state
    else {
        panic!("actual recovery finalized")
    };
    let receipt = apply_receipt
        .context_commit_receipt
        .expect("real PG receipt");
    assert_eq!(receipt.resulting().len(), 2);
    receipt.validate().expect("actual Core receipt");
    assert!(
        sqlx::query_scalar::<_, Value>(origin_sql)
            .fetch_one(store.pool())
            .await
            .expect("immutable imported origin after commit")
            == origin
    );
    let (revision, history, projections, bytes): (i64,i64,i64,Vec<u8>) = sqlx::query_as("SELECT m.revision,(SELECT count(*) FROM context_material_versions WHERE material_id=m.material_id),(SELECT count(*) FROM context_projection_versions WHERE material_id=m.material_id),m.content FROM context_materials m WHERE path='knowledge/boundary-imported.md'").fetch_one(store.pool()).await.expect("actual imported update history");
    assert_eq!((revision, history, projections), (2, 2, 2));
    assert_eq!(bytes, body.as_bytes());
    let original: Vec<u8> = sqlx::query_scalar("SELECT v.content FROM context_material_versions v JOIN context_materials m USING(material_id) WHERE m.path='knowledge/boundary-imported.md' AND v.revision=1").fetch_one(store.pool()).await.expect("immutable imported historical bytes");
    assert_eq!(original, b"# Imported original\nImmutable origin bytes");
    let final_before = native_rows_fingerprint(&store).await;
    let repeat = harness_fixture::command(
        &store,
        &root,
        &root,
        "recover",
        &["--run-id".into(), "revision-imported".into()],
    )
    .await;
    assert!(repeat.status.success());
    assert_eq!(native_rows_fingerprint(&store).await, final_before);
}

#[tokio::test]
async fn native_revision_career_binary_attests_and_composes_without_mutation() {
    use context_core::harness::*;
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let policy = harness_fixture::seed(&store).await;
    let view = harness_fixture::view();
    let root = view.path().canonicalize().expect("owned context view");
    let workspace = tempfile::tempdir().expect("owned career workspace");
    let workspace = workspace.path().canonicalize().expect("career workspace");
    let body = b"# Synthetic career artifact\nA bounded fixture claim.\n";
    fs::write(workspace.join("README.md"), body).expect("synthetic frozen artifact");
    let owner = DataOwner::PersonalProject {
        project: "sample".into(),
    };
    let source = "vault/personal/projects/sample.md";
    let statement =
        "Review the exact synthetic career artifact and its declared evidence without changing it.";
    let comparison = store
        .with_native_context(root.clone(), move |source| {
            Ok(harness_fixture::context_fixture::career_comparison(
                source.as_ref(),
                statement,
                "synthetic-claim",
                "README.md",
            ))
        })
        .await
        .unwrap();
    let manifest = CareerCompositionManifest {
        comparison: Some(comparison),
        version: HARNESS_SCHEMA_VERSION,
        career_output_surface: CareerOutputSurface::Resume,
        artifact_targets: vec!["README.md".into()],
        coverage: CareerCoverageMode::Selected,
        complete_coverage_confirmation_reported: false,
        evidence_owners: vec![owner.clone()],
        canonical_evidence_owners: vec![],
        excluded_evidence_owners: vec![],
        claim_lineage: vec![CareerClaimLineage {
            claim_id: "synthetic-claim".into(),
            evidence_owner: owner.clone(),
            evidence_source_paths: vec![source.into()],
        }],
    };
    let before = native_rows_fingerprint(&store).await;
    let artifacts = tempfile::tempdir().expect("actual career CLI artifacts");
    let mut receipts = Vec::new();
    let mut contexts = std::collections::BTreeSet::new();
    for evidence in [true, false] {
        let mut request =
            harness_fixture::envelope(&policy, vec!["README.md".into()], vec![], false);
        let DraftTaskRequest::Write(mut write) = request.draft else {
            panic!("fixture draft")
        };
        request
            .decision_trace
            .records
            .retain(|record| match record {
                DecisionRecord::PolicyDefault { identifier, .. }
                | DecisionRecord::UserStatement { identifier, .. } => ![
                    "decision-0000000000000002",
                    "decision-0000000000000003",
                    "decision-0000000000000005",
                ]
                .contains(&identifier.as_str()),
                _ => true,
            });
        let statement = if evidence {
            "Review the exact synthetic career artifact and its declared evidence without changing it.".to_owned()
        } else {
            manifest
                .holistic_task_statement(&receipts)
                .expect("current owner receipt handoff")
        };
        request.source = RequestSource::UserLanguage {
            statements: vec![UserStatement {
                identifier: "statement-0000000000000001".into(),
                text: statement.clone(),
            }],
        };
        write.common.intent = harness_fixture::value(
            HarnessIntent::CareerArtifact {
                surface: CareerOutputSurface::Resume,
            },
            2,
            &mut request.decision_trace.records,
        );
        write.common.task_statement =
            harness_fixture::value(statement, 3, &mut request.decision_trace.records);
        if !evidence {
            write.common.evidence_source_paths = Some(harness_fixture::value(
                vec!["vault/personal/profile.md".into()],
                9,
                &mut request.decision_trace.records,
            ));
        }
        if evidence {
            write.common.context_grants = Some(harness_fixture::value(
                vec![ContextGrant {
                    owner: owner.clone(),
                    purpose: ContextGrantPurpose::CareerWritingEvidence,
                    access: ContextAccess::ReadOnly,
                }],
                8,
                &mut request.decision_trace.records,
            ));
            write.common.evidence_source_paths = Some(harness_fixture::value(
                vec![source.to_owned()],
                9,
                &mut request.decision_trace.records,
            ));
        }
        request.draft = DraftTaskRequest::Review(DraftReviewContract {
            common: write.common,
            review_kind: harness_fixture::value(
                ReviewKind::Document,
                5,
                &mut request.decision_trace.records,
            ),
            targets: write.targets,
        });
        let (files, prepared) = harness_fixture::cli_prepared_with_manifest(
            &store,
            &root,
            &workspace,
            request,
            Some(&manifest),
        )
        .await;
        assert!(!prepared.plan.source_write_allowed);
        assert_eq!(
            prepared.plan.frozen_targets.targets[0].operation,
            TargetOperation::Inspect
        );
        let run = if evidence {
            "revision-career-owner"
        } else {
            "revision-career-holistic"
        };
        let head = harness_fixture::cli_roles(
            &store,
            &root,
            &workspace,
            files.path(),
            &prepared,
            run,
            "",
            false,
        )
        .await;
        assert_eq!(head.state, HarnessExecutionState::Evaluated);
        let output = harness_fixture::command(
            &store,
            &root,
            &workspace,
            "validate",
            &["--run-id".into(), run.into()],
        )
        .await;
        assert!(output.status.success(), "career CLI validation");
        let validated: HarnessExecutionRecord =
            decode_current_json(&output.stdout, "validated CLI career record")
                .expect("strict validated output");
        assert_eq!(validated.state, HarnessExecutionState::Validated);
        validated
            .validate(
                &prepared,
                &fs::read(files.path().join("prepared.json")).expect("exact CLI prepared file"),
            )
            .expect("bound validated CLI record");
        assert!(validated.candidate_digest.is_none());
        assert!(validated.finalization_digest.is_none());
        for role in &validated.role_execution.role_results {
            assert!(
                contexts.insert(role.context_id.clone()),
                "review and evidence roles use distinct actual invocation contexts"
            );
        }
        let record_path = files.path().join("validated.json");
        fs::write(&record_path, &output.stdout).expect("exact validated CLI output");
        let attested = harness_fixture::command(
            &store,
            &root,
            &workspace,
            "attest-career",
            &[
                "--prepared-run".into(),
                files.path().join("prepared.json").display().to_string(),
                "--execution-record".into(),
                record_path.display().to_string(),
            ],
        )
        .await;
        assert!(
            attested.status.success(),
            "actual career CLI attestation: {}",
            String::from_utf8_lossy(&attested.stderr)
        );
        assert!(attested.stderr.is_empty());
        let receipt: CareerExecutionReviewReceipt =
            decode_current_json(&attested.stdout, "native CLI career receipt")
                .expect("strict real review receipt");
        assert!(receipt.resolved == prepared.plan.resolved_request);
        assert!(receipt.prepared == prepared.role_run);
        assert_eq!(
            receipt.evaluation.execution_record,
            validated.role_execution
        );
        assert_eq!(receipt.evaluation.subject_status, SubjectStatus::Accepted);
        assert_eq!(
            receipt.artifact_set_digest,
            ontology::store::digest(
                &serde_json::to_vec(&prepared.plan.resolved_request.plan.targets)
                    .expect("frozen artifact set")
            )
        );
        let target = &receipt.resolved.plan.targets[0];
        assert_eq!(target.workspace_relative_path, "README.md");
        assert_eq!(
            target.state,
            TargetState::Existing {
                content_digest: ontology::store::digest(body)
            }
        );
        assert!(receipt.evidence_bundle_digest.is_some());
        let path = artifacts.path().join(if evidence {
            "owner.json"
        } else {
            "holistic.json"
        });
        fs::write(path, &attested.stdout).expect("exact attestation artifact");
        receipts.push(receipt);
        assert_eq!(native_rows_fingerprint(&store).await, before);
        assert_eq!(
            fs::read(workspace.join("README.md")).expect("unchanged artifact"),
            body
        );
    }
    assert_eq!(contexts.len(), 4);
    assert_eq!(
        receipts[0].artifact_set_digest,
        receipts[1].artifact_set_digest
    );
    let manifest_path = artifacts.path().join("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec(&manifest).expect("manifest"),
    )
    .expect("composition input");
    let args = vec![
        "--career-manifest".into(),
        manifest_path.display().to_string(),
        "--holistic-receipt".into(),
        artifacts.path().join("holistic.json").display().to_string(),
        "--evidence-receipt".into(),
        artifacts.path().join("owner.json").display().to_string(),
    ];
    let mut json_args = args.clone();
    json_args.push("--json".into());
    let output =
        harness_fixture::command(&store, &root, &workspace, "compose-career", &json_args).await;
    assert!(
        output.status.success(),
        "actual career CLI composition: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let composed: CareerExecutionCompositionReceipt =
        decode_current_json(&output.stdout, "native career composition")
            .expect("strict actual composition");
    assert_eq!(composed.assurance, ExecutionAssurance::Advisory);
    assert!(composed.coverage_is_caller_attested);
    assert_eq!(composed.coverage, CareerCoverageMode::Selected);
    assert_eq!(composed.artifact_targets, receipts[0].resolved.plan.targets);
    assert_eq!(
        composed.artifact_set_digest,
        receipts[0].artifact_set_digest
    );
    assert_eq!(
        composed.manifest_digest,
        ontology::store::digest(&serde_json::to_vec(&manifest).expect("exact manifest"))
    );
    assert_eq!(composed.declared_evidence_owners, vec![owner]);
    let text = harness_fixture::command(&store, &root, &workspace, "compose-career", &args).await;
    assert!(text.status.success());
    assert!(text.stderr.is_empty());
    assert_eq!(
        String::from_utf8(text.stdout).expect("advisory output"),
        format!(
            "경력 산출물 조립 검증 통과 (advisory)\n조립 해시: {}\n완전성은 선언된 manifest와 claim lineage 범위이며 Harness engine가 산출물 의미를 자동 판독하지 않음\n",
            composed.composition_digest
        )
    );
    assert_eq!(native_rows_fingerprint(&store).await, before);
    assert_eq!(
        fs::read(workspace.join("README.md")).expect("unchanged career artifact"),
        body
    );
    let effects: i64 = sqlx::query_scalar("SELECT count(*) FROM context_apply_batches")
        .fetch_one(store.pool())
        .await
        .expect("nonmutating effect count");
    assert_eq!(effects, 0);
}
fn private_staging_directory(path: &Path) {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .expect("owned staging directories");
}

#[tokio::test]
async fn native_primary_only_career_composes_without_foreign_receipts_and_binds_leaf_facts() {
    use context_core::{career::*, harness::*};
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let fixture = context_fixture::build();
    let leaf = "vault/personal/facts/award.md";
    fs::write(
        fixture.path().join("personal/facts/award.md"),
        "# Award\nA fictional team award.\n",
    )
    .unwrap();
    fs::write(fixture.path().join("personal/writing/evidence.md"), format!("```career-routes\n{}\n```\n", json!({"routes":[{"owner":{"kind":"personal"},"kind":"fact","path":leaf,"label":"award"}]}))).unwrap();
    let scopes = context_fixture::SCOPES
        .iter()
        .map(|s| s.parse().unwrap())
        .collect::<Vec<ContextScope>>();
    let fixture_root = fixture.path().canonicalize().unwrap();
    let initial = inventory(&fixture_root, &scopes).unwrap();
    store
        .import_context(&fixture_root, &scopes, &initial.inventory_digest)
        .await
        .unwrap();
    let sha = ontology::store::digest(
        &fs::read(fixture.path().join("profile/rules/control.md")).unwrap(),
    );
    let view = harness_fixture::view();
    let root = view.path().canonicalize().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let workspace = workspace.path().canonicalize().unwrap();
    fs::write(
        workspace.join("README.md"),
        "# Career\nA fictional team award.\n",
    )
    .unwrap();
    let statement = "Review the exact fictional team award artifact.";
    let inventory = store
        .career_candidates(
            &context_fixture::configuration_path(),
            CareerDiscoveryRequest {
                original_request: statement.into(),
                surface: CareerOutputSurface::Resume,
                approved_scopes: vec!["personal".into()],
                axis_queries: vec![],
            },
            None,
            false,
        )
        .await
        .unwrap()
        .inventory;
    let comparison = CareerComparison {
        requirements: vec![CareerRequirement {
            id: "award".into(),
            original_span: CareerSpan {
                start: 0,
                end: statement.len(),
            },
            kind: CareerRequirementKind::Requirement,
            coverage: CareerCoverage::Direct,
            material: true,
        }],
        decisions: inventory
            .candidates
            .iter()
            .map(|c| CareerDecision {
                requirement_id: "award".into(),
                candidate_path: c.path.clone(),
                disposition: if c.path == leaf {
                    CareerDisposition::Selected
                } else {
                    CareerDisposition::Excluded
                },
                reason: "Bounded original fixture facet".into(),
                facets: vec![CareerFacet {
                    axis: CareerAxis::Result,
                    original: CareerLocator {
                        path: c.path.clone(),
                        original_span: CareerSpan { start: 0, end: 5 },
                    },
                }],
            })
            .collect(),
        placements: vec![CareerPlacement {
            claim_id: "award".into(),
            candidate_path: leaf.into(),
            requirement_ids: vec!["award".into()],
            source_locators: vec![CareerLocator {
                path: leaf.into(),
                original_span: CareerSpan { start: 8, end: 30 },
            }],
            target: "README.md".into(),
            slot: "award".into(),
        }],
        inventory,
    };
    let manifest = CareerCompositionManifest {
        version: HARNESS_SCHEMA_VERSION,
        comparison: Some(comparison),
        career_output_surface: CareerOutputSurface::Resume,
        artifact_targets: vec!["README.md".into()],
        coverage: CareerCoverageMode::Selected,
        complete_coverage_confirmation_reported: false,
        evidence_owners: vec![],
        canonical_evidence_owners: vec![],
        excluded_evidence_owners: vec![],
        claim_lineage: vec![CareerClaimLineage {
            claim_id: "award".into(),
            evidence_owner: DataOwner::Personal,
            evidence_source_paths: vec![leaf.into()],
        }],
    };
    let mut records = vec![];
    let request = RequestEnvelope {
        version: HARNESS_SCHEMA_VERSION,
        source: RequestSource::UserLanguage {
            statements: vec![UserStatement {
                identifier: "statement-0000000000000001".into(),
                text: statement.into(),
            }],
        },
        draft: DraftTaskRequest::Review(DraftReviewContract {
            common: DraftTaskCommon {
                owner: harness_fixture::value(DataOwner::Personal, 1, &mut records),
                intent: harness_fixture::value(
                    HarnessIntent::CareerArtifact {
                        surface: CareerOutputSurface::Resume,
                    },
                    2,
                    &mut records,
                ),
                task_statement: harness_fixture::value(statement.to_owned(), 3, &mut records),
                execution_profile: harness_fixture::default(
                    HarnessExecutionProfile::Standard,
                    4,
                    PolicyDefaultRule::StandardExecutionProfile,
                    &sha,
                    &mut records,
                ),
                context_grants: None,
                evidence_source_paths: Some(harness_fixture::value(
                    vec![leaf.into(), "vault/personal/profile.md".into()],
                    9,
                    &mut records,
                )),
            },
            review_kind: harness_fixture::value(ReviewKind::Document, 5, &mut records),
            targets: harness_fixture::value(vec!["README.md".into()], 6, &mut records),
        }),
        decision_trace: DecisionTrace { records },
    };
    let before = native_rows_fingerprint(&store).await;
    let (files, prepared) = harness_fixture::cli_prepared_with_manifest(
        &store,
        &root,
        &workspace,
        request,
        Some(&manifest),
    )
    .await;
    assert!(prepared.role_run.role_metadata.iter().any(|role| {
        matches!(&role.task, RoleTaskContract::Reviewer { verification_requirements, .. }
            if verification_requirements.iter().any(|r| r.unit == VerificationUnit::CareerHolisticCoherence))
    }));
    let head = harness_fixture::cli_roles(
        &store,
        &root,
        &workspace,
        files.path(),
        &prepared,
        "primary-award-review",
        "",
        false,
    )
    .await;
    assert_eq!(head.state, HarnessExecutionState::Evaluated);
    let validated = harness_fixture::command(
        &store,
        &root,
        &workspace,
        "validate",
        &["--run-id".into(), "primary-award-review".into()],
    )
    .await;
    assert!(
        validated.status.success(),
        "{}",
        String::from_utf8_lossy(&validated.stderr)
    );
    let validated_record: HarnessExecutionRecord =
        decode_current_json(&validated.stdout, "primary career validation").unwrap();
    assert_eq!(validated_record.state, HarnessExecutionState::Validated);
    let record = files.path().join("accepted.json");
    fs::write(&record, validated.stdout).unwrap();
    let attested = harness_fixture::command(
        &store,
        &root,
        &workspace,
        "attest-career",
        &[
            "--execution-record".into(),
            record.display().to_string(),
            "--prepared-run".into(),
            files.path().join("prepared.json").display().to_string(),
        ],
    )
    .await;
    assert!(
        attested.status.success(),
        "{}",
        String::from_utf8_lossy(&attested.stderr)
    );
    let receipt = files.path().join("receipt.json");
    fs::write(&receipt, attested.stdout).unwrap();
    let composed = harness_fixture::command(
        &store,
        &root,
        &workspace,
        "compose-career",
        &[
            "--career-manifest".into(),
            files.path().join("manifest.json").display().to_string(),
            "--holistic-receipt".into(),
            receipt.display().to_string(),
            "--json".into(),
        ],
    )
    .await;
    assert!(
        composed.status.success(),
        "{}",
        String::from_utf8_lossy(&composed.stderr)
    );
    let receipt: CareerExecutionCompositionReceipt =
        decode_current_json(&composed.stdout, "primary only composition").unwrap();
    assert!(receipt.declared_evidence_owners.is_empty());
    assert_eq!(native_rows_fingerprint(&store).await, before);
}

fn private_staging_file(path: &Path, bytes: &[u8]) {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .expect("new private staging file")
        .write_all(bytes)
        .expect("stale synthetic projection bytes");
}

fn staging_identity(path: &Path) -> (u64, u64) {
    let metadata = fs::symlink_metadata(path).expect("retained staging parent");
    assert!(metadata.is_dir());
    assert_eq!(metadata.mode() & 0o777, 0o700);
    (metadata.dev(), metadata.ino())
}

fn business_knowledge_request(sha: &str, target: &str) -> context_core::harness::RequestEnvelope {
    use context_core::harness::*;
    let mut request = harness_fixture::envelope(sha, vec![target.into()], vec![], false);
    let statement =
        "Store the synthetic business Knowledge document at the declared nested target.".to_owned();
    request.source = RequestSource::UserLanguage {
        statements: vec![UserStatement {
            identifier: "statement-0000000000000001".into(),
            text: statement.clone(),
        }],
    };
    request.decision_trace.records.retain(|record| {
        !matches!(record, DecisionRecord::UserStatement { identifier, .. }
            if ["decision-0000000000000001", "decision-0000000000000003"]
                .contains(&identifier.as_str()))
    });
    let DraftTaskRequest::Write(write) = &mut request.draft else {
        panic!("the declared fixture is a document-write request");
    };
    write.common.owner = harness_fixture::value(
        DataOwner::PersonalBusiness,
        1,
        &mut request.decision_trace.records,
    );
    write.common.task_statement =
        harness_fixture::value(statement, 3, &mut request.decision_trace.records);
    request
}

#[tokio::test]
async fn native_business_create_cli_preserves_missing_staging_parents() {
    use context_core::harness::{
        HarnessExecutionRecord, HarnessExecutionState, HarnessRole, StoredSourceState,
        TargetOperation, decode_current_json,
    };
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let policy = harness_fixture::seed(&store).await;
    let before = native_rows_fingerprint(&store).await;
    let sources_before: i64 = sqlx::query_scalar("SELECT count(*) FROM source_records")
        .fetch_one(store.pool())
        .await
        .expect("source record count before preparation");
    let knowledge_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context_materials WHERE starts_with(source_path,'personal/business/knowledge/')",
    )
    .fetch_one(store.pool())
    .await
    .expect("initial business Knowledge source count");
    assert_eq!(knowledge_count, 0);
    for scenario in ["absent", "stale-file", "stale-directory"] {
        let view = harness_fixture::view();
        let root = view.path().canonicalize().expect("private context view");
        let parent = format!("vault/personal/business/knowledge/{scenario}/nested");
        let target = format!("{parent}/new.md");
        let selected = target.clone();
        let parents = vec![
            "vault/personal/business/knowledge".to_owned(),
            format!("vault/personal/business/knowledge/{scenario}"),
            parent.clone(),
        ];
        let observed_parents = parents.clone();
        let identities = store
            .with_native_context(root.clone(), move |source| {
                let metadata = source
                    .metadata(Path::new(&selected))
                    .expect("DB Missing target");
                assert_eq!(metadata.kind, SourcePathKind::Missing);
                assert_eq!(
                    metadata
                        .stored_version
                        .expect("stored Missing binding")
                        .state,
                    StoredSourceState::Missing
                );
                let identities: Vec<_> = observed_parents
                    .iter()
                    .map(|path| staging_identity(&source.view_root().join(path)))
                    .collect();
                let stale = source
                    .view_root()
                    .join(&observed_parents[2])
                    .join("stale.md");
                private_staging_file(&stale, b"not a stored source");
                for repetitions in [1, 64] {
                    let queries = source.metadata_queries();
                    for _ in 0..repetitions {
                        for path in &observed_parents {
                            let metadata =
                                source.metadata(Path::new(path)).expect("repeat Missing");
                            assert_eq!(metadata.kind, SourcePathKind::Missing);
                            assert_eq!(
                                metadata.stored_version.expect("DB Missing binding").state,
                                StoredSourceState::Missing
                            );
                        }
                    }
                    assert_eq!(
                        source.metadata_queries() - queries,
                        repetitions * observed_parents.len() as u64,
                        "one metadata query per Missing request, with no retries or bodies"
                    );
                    assert_eq!(source.body_queries(), 0);
                    for (path, identity) in observed_parents.iter().zip(&identities) {
                        assert_eq!(staging_identity(&source.view_root().join(path)), *identity);
                    }
                }
                assert!(
                    !stale.exists(),
                    "stale files are removed without removing parents"
                );
                assert!(
                    source
                        .children(Path::new(&observed_parents[0]), 1)
                        .expect("DB-authoritative children")
                        .is_empty()
                );
                Ok(identities)
            })
            .await
            .expect("Missing metadata preserves all nested parent identities");
        if scenario == "stale-file" {
            private_staging_file(&root.join(&target), b"stale exact target");
        } else if scenario == "stale-directory" {
            let stale_parent = root.join(&target).join("deeper");
            private_staging_directory(&stale_parent);
            for index in 0..64 {
                private_staging_file(&stale_parent.join(format!("{index}.md")), b"stale child");
            }
        }
        let (artifacts, prepared) = harness_fixture::cli_prepared(
            &store,
            &root,
            &root,
            business_knowledge_request(&policy, &target),
        )
        .await;
        assert_eq!(
            prepared.plan.frozen_targets.targets[0].operation,
            TargetOperation::Create
        );
        assert!(
            prepared
                .plan
                .bound_source_versions()
                .versions
                .iter()
                .any(|version| {
                    version.logical_path == target && version.state == StoredSourceState::Missing
                })
        );
        assert!(
            !root.join(&target).exists(),
            "the exact Create target is absent"
        );
        for (path, identity) in parents.iter().zip(&identities) {
            assert_eq!(staging_identity(&root.join(path)), *identity);
        }
        if scenario == "stale-directory" {
            let nested = root.join(&parent);
            let displaced = nested.with_file_name("displaced-parent");
            fs::rename(&nested, &displaced).expect("retain the actual frozen parent");
            private_staging_directory(&nested);
            assert_ne!(staging_identity(&nested), staging_identity(&displaced));
            let rejected = harness_fixture::command(
                &store,
                &root,
                &root,
                "begin",
                &[
                    "--prepared-run".into(),
                    artifacts.path().join("prepared.json").display().to_string(),
                    "--run-id".into(),
                    "business-replaced-parent".into(),
                ],
            )
            .await;
            assert!(
                !rejected.status.success(),
                "frozen parent replacement must fail closed"
            );
            assert!(rejected.stdout.is_empty());
            fs::remove_dir(&nested).expect("remove only the empty replacement fixture");
            fs::rename(&displaced, &nested).expect("restore the exact frozen parent inode");
        }
        let begun = harness_fixture::command(
            &store,
            &root,
            &root,
            "begin",
            &[
                "--prepared-run".into(),
                artifacts.path().join("prepared.json").display().to_string(),
                "--run-id".into(),
                format!("business-{scenario}"),
            ],
        )
        .await;
        assert!(
            begun.status.success(),
            "native Create begin: {}",
            String::from_utf8_lossy(&begun.stderr)
        );
        let head: HarnessExecutionRecord =
            decode_current_json(&begun.stdout, "business Create begun record")
                .expect("actual CLI begin record");
        assert_eq!(head.state, HarnessExecutionState::Begun);
        assert!(
            head.ready_role_invocations
                .iter()
                .any(|role| role.role == HarnessRole::Writer)
        );
        for (path, identity) in parents.iter().zip(&identities) {
            assert_eq!(staging_identity(&root.join(path)), *identity);
        }
        store
            .with_native_context(root, move |source| {
                let version = source
                    .stored_versions(&[PathBuf::from(&target)])
                    .expect("prepared source state remains Missing");
                assert_eq!(version[0].state, StoredSourceState::Missing);
                assert_eq!(source.body_queries(), 0);
                assert!(!source.view_root().join(&target).exists());
                Ok(())
            })
            .await
            .expect("fresh native Missing after begin");
        assert_eq!(native_rows_fingerprint(&store).await, before);
        let sources_after: i64 = sqlx::query_scalar("SELECT count(*) FROM source_records")
            .fetch_one(store.pool())
            .await
            .expect("source record count after begin");
        assert_eq!(sources_after, sources_before);
    }
}

#[tokio::test]
async fn native_live_file_replaces_owned_stale_directory_without_changing_parents() {
    use std::io::Read;
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(1, b"# Live stored bytes\n");
    import(&store, &root).await;
    let before = native_rows_fingerprint(&store).await;
    for children in [1, 64] {
        let view = harness_fixture::view();
        let root = view.path().canonicalize().expect("private transition view");
        store
            .with_native_context(root, move |source| {
                let path = Path::new("vault/personal/0000.md");
                assert_eq!(
                    source.metadata(path).expect("live metadata").kind,
                    SourcePathKind::RegularFile
                );
                let parent = source.view_root().join("vault/personal");
                let identity = staging_identity(&parent);
                let stale = source.view_root().join(path).join("nested");
                private_staging_directory(&stale);
                for index in 0..children {
                    private_staging_file(&stale.join(format!("{index}.md")), b"stale projection");
                }
                let queries = source.metadata_queries();
                let mut file = source
                    .open_file(path, 65536)
                    .expect("directory-to-file transition");
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes).expect("exact current body");
                assert_eq!(bytes, b"# Live stored bytes\n");
                assert_eq!(source.metadata_queries() - queries, 1);
                assert_eq!(source.body_queries(), 1);
                assert_eq!(staging_identity(&parent), identity);
                let metadata = file.metadata().expect("materialized file metadata");
                assert!(metadata.is_file());
                assert_eq!(metadata.nlink(), 1);
                assert_eq!(metadata.mode() & 0o777, 0o600);
                Ok(())
            })
            .await
            .expect("secure stale-directory replacement");
        assert_eq!(native_rows_fingerprint(&store).await, before);
    }
}

#[tokio::test]
async fn native_literal_bracket_projections_refresh_without_source_or_classification_changes() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(0, b"");
    fs::create_dir_all(root.join("work/acme")).expect("synthetic work scope");
    for path in ["personal/[a].md", "personal/a.md", "work/acme/[회사].md"] {
        fs::write(root.join(path), format!("# {path}\nLiteral bracket marker"))
            .expect("exact source");
    }
    let selected = vec![scope(), "work/acme".parse().expect("scope")];
    let inventory = inventory(&root, &selected).expect("inventory");
    store
        .import_context(&root, &selected, &inventory.inventory_digest)
        .await
        .expect("import");
    let snapshot = "SELECT jsonb_build_object('identity',(SELECT to_jsonb(s) FROM context_store s),'originals',(SELECT jsonb_agg(to_jsonb(m) ORDER BY material_id) FROM context_materials m),'history',(SELECT jsonb_agg(to_jsonb(v) ORDER BY material_id,revision) FROM context_material_versions v),'subjects',(SELECT jsonb_agg(to_jsonb(s) ORDER BY id) FROM subjects s),'areas',(SELECT jsonb_agg(to_jsonb(a) ORDER BY entity_id,area) FROM entity_areas a),'topics',(SELECT jsonb_agg(to_jsonb(t) ORDER BY entity_id,topic_id) FROM entity_topics t))";
    let before: Value = sqlx::query_scalar(snapshot)
        .fetch_one(store.pool())
        .await
        .expect("canonical snapshot");
    // Reproduce the exact legacy exclusion written before the literal-path fix.
    sqlx::query("TRUNCATE context_projection_versions")
        .execute(store.pool())
        .await
        .expect("owned projection fixture");
    sqlx::query("INSERT INTO context_projection_versions(material_id,revision,payload,payload_digest) SELECT material_id,revision,p,encode(sha256(convert_to(p::text,'UTF8')),'hex') FROM context_materials CROSS JOIN LATERAL (SELECT jsonb_build_object('status','excluded','source_digest',content_digest,'terms','[]'::jsonb) AS p) x").execute(store.pool()).await.expect("legacy excluded rows");
    let status = store.projection_status(&selected).await.expect("status");
    assert_eq!(status["ready"], false);
    assert_eq!(status["counts"]["excluded"], 3);
    assert_eq!(
        store.semantic_context(&selected, "marker", 10, false).await,
        Err(Error::ContextProjectionUnavailable)
    );
    for (scope, path, title) in [
        (scope(), "[a].md", "personal/[a].md"),
        (
            "work/acme".parse().expect("scope"),
            "[회사].md",
            "work/acme/[회사].md",
        ),
    ] {
        let read = store
            .read_context_documents(&scope, &[path.into()])
            .await
            .expect("literal exact read");
        assert_eq!(read["documents"][0]["title"], title);
    }
    let old_manifest = status["manifest_digest"].as_str().expect("digest");
    let result = store
        .project_context(&selected, old_manifest)
        .await
        .expect("explicit derived refresh");
    assert_eq!(result["inserted"], 0);
    assert_ne!(result["manifest_digest"], status["manifest_digest"]);
    assert_eq!(
        store.project_context(&selected, old_manifest).await,
        Err(Error::Conflict)
    );
    let status = store
        .projection_status(&selected)
        .await
        .expect("refreshed status");
    assert_eq!(status["ready"], true);
    assert_eq!(status["counts"]["searchable"], 3);
    let current = status["manifest_digest"].as_str().expect("digest");
    let again = store
        .project_context(&selected, current)
        .await
        .expect("idempotent rebuild");
    assert_eq!(again["manifest_digest"], status["manifest_digest"]);
    assert_eq!(
        store
            .semantic_context(&selected, "marker", 10, false)
            .await
            .expect("searchable originals")["items"]
            .as_array()
            .expect("items")
            .len(),
        3
    );
    assert_eq!(
        sqlx::query_scalar::<_, Value>(snapshot)
            .fetch_one(store.pool())
            .await
            .expect("unchanged originals"),
        before
    );
    assert!(
        sqlx::query("UPDATE context_projection_versions SET payload=payload")
            .execute(store.pool())
            .await
            .is_err(),
        "refresh permission cannot survive the committed project transaction"
    );
}

#[tokio::test]
async fn native_current_refresh_guard_keeps_history_identity_source_and_transaction_boundaries() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(1, b"# First revision");
    import(&store, &root).await;
    let edited = store
        .edit_context(
            &scope(),
            "0000.md",
            1,
            &ontology::store::digest(b"# First revision"),
            "# Second revision",
        )
        .await
        .expect("existing manual-edit revision boundary");
    assert_eq!(edited.revision, 2);
    let revisions: i64 = sqlx::query_scalar("SELECT count(*) FROM context_material_versions")
        .fetch_one(store.pool())
        .await
        .expect("actual historical revision");
    assert_eq!(revisions, 2);
    let before: Value = sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(p) ORDER BY material_id,revision) FROM context_projection_versions p").fetch_one(store.pool()).await.expect("snapshot");
    for statement in [
        "UPDATE context_projection_versions SET payload=payload WHERE revision=2",
        "UPDATE context_projection_versions SET revision=1 WHERE revision=2",
        "UPDATE context_projection_versions SET material_id=gen_random_uuid() WHERE revision=2",
        "UPDATE context_projection_versions SET payload=payload WHERE revision=1",
        "DELETE FROM context_projection_versions WHERE revision=2",
        "UPDATE context_projection_versions SET payload=jsonb_set(payload,'{source_digest}',to_jsonb(repeat('a',64))),payload_digest=encode(sha256(convert_to(jsonb_set(payload,'{source_digest}',to_jsonb(repeat('a',64)))::text,'UTF8')),'hex') WHERE revision=2",
        "UPDATE context_projection_versions SET payload=jsonb_build_object('status','tombstone','source_digest',payload->>'source_digest','terms','[]'::jsonb),payload_digest=encode(sha256(convert_to(jsonb_build_object('status','tombstone','source_digest',payload->>'source_digest','terms','[]'::jsonb)::text,'UTF8')),'hex') WHERE revision=2",
    ] {
        let mut tx = store.pool().begin().await.expect("fixture transaction");
        if !statement
            .starts_with("UPDATE context_projection_versions SET payload=payload WHERE revision=2")
        {
            sqlx::query("SELECT set_config('ontology.projection_refresh','on',true)")
                .execute(&mut *tx)
                .await
                .expect("owned explicit refresh permission");
        }
        assert!(
            sqlx::query(statement).execute(&mut *tx).await.is_err(),
            "reject {statement}"
        );
        tx.rollback().await.expect("rollback rejected refresh");
    }
    for flag in ["", "off"] {
        let mut tx = store
            .pool()
            .begin()
            .await
            .expect("disabled refresh permission");
        sqlx::query("SELECT set_config('ontology.projection_refresh',$1,true)")
            .bind(flag)
            .execute(&mut *tx)
            .await
            .expect("disabled local flag");
        assert!(
            sqlx::query("UPDATE context_projection_versions SET payload=payload WHERE revision=2")
                .execute(&mut *tx)
                .await
                .is_err()
        );
        tx.rollback().await.expect("rollback disabled permission");
    }
    assert_eq!(sqlx::query_scalar::<_, Value>("SELECT jsonb_agg(to_jsonb(p) ORDER BY material_id,revision) FROM context_projection_versions p").fetch_one(store.pool()).await.expect("unchanged history"), before);
    let mut tx = store
        .pool()
        .begin()
        .await
        .expect("rollback permission test");
    sqlx::query("SELECT set_config('ontology.projection_refresh','on',true)")
        .execute(&mut *tx)
        .await
        .expect("transaction local");
    tx.rollback().await.expect("rollback");
    assert!(
        sqlx::query("UPDATE context_projection_versions SET payload=payload WHERE revision=2")
            .execute(store.pool())
            .await
            .is_err(),
        "rolled back permission cannot authorize another pooled query"
    );
}

#[tokio::test]
async fn native_migration_012_preserves_all_existing_rows_and_rejects_corrupt_refresh() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_fixture, root) = fixture(1, b"# Preserve original");
    import(&store, &root).await;
    sqlx::raw_sql("ALTER TABLE memory_history DROP COLUMN grouping_only; DELETE FROM ontology_migrations WHERE name='016-memory-grouping-history.sql'; DROP TABLE document_grouping; DELETE FROM ontology_migrations WHERE name='015-document-grouping.sql'; ALTER TABLE memory_grouping DROP COLUMN claim_token; DELETE FROM ontology_migrations WHERE name='014-grouping-claim-ownership.sql'; DROP TABLE document_subjects; DROP TABLE document_subject_history; DROP TABLE subject_history; ALTER TABLE subjects DROP COLUMN definition, DROP COLUMN revision; ALTER TABLE source_records DROP COLUMN reference_paths; DELETE FROM ontology_migrations WHERE name='013-knowledge-taxonomy.sql'; DROP TRIGGER context_projection_immutable ON context_projection_versions; CREATE TRIGGER context_projection_immutable BEFORE UPDATE OR DELETE ON context_projection_versions FOR EACH ROW EXECUTE FUNCTION context_immutable_record(); DROP TRIGGER context_projection_check ON context_projection_versions; CREATE TRIGGER context_projection_check BEFORE INSERT ON context_projection_versions FOR EACH ROW EXECUTE FUNCTION context_projection_validate(); DROP FUNCTION context_projection_refresh_guard(); DELETE FROM ontology_migrations WHERE name='012-context-projection-refresh.sql';").execute(store.pool()).await.expect("owned pre-012 schema");
    let snapshot = "SELECT jsonb_build_object('identity',(SELECT to_jsonb(s) FROM context_store s),'originals',(SELECT jsonb_agg(to_jsonb(m) ORDER BY material_id) FROM context_materials m),'history',(SELECT jsonb_agg(to_jsonb(v) ORDER BY material_id,revision) FROM context_material_versions v),'projections',(SELECT jsonb_agg(to_jsonb(p) ORDER BY material_id,revision) FROM context_projection_versions p))";
    let before: Value = sqlx::query_scalar(snapshot)
        .fetch_one(store.pool())
        .await
        .expect("pre-upgrade values");
    store.initialize().await.expect("actual 011 to 012 upgrade");
    assert_eq!(
        sqlx::query_scalar::<_, Value>(snapshot)
            .fetch_one(store.pool())
            .await
            .expect("post-upgrade values"),
        before
    );
    let status = store
        .projection_status(&[scope()])
        .await
        .expect("valid manifest");
    // Intentionally corrupt one derived checksum inside the owned test DB.
    sqlx::raw_sql("ALTER TABLE context_projection_versions DISABLE TRIGGER context_projection_immutable; DO $$ DECLARE n text; BEGIN SELECT conname INTO STRICT n FROM pg_constraint WHERE conrelid='context_projection_versions'::regclass AND contype='c' AND pg_get_constraintdef(oid) LIKE '%payload_digest%'; EXECUTE format('ALTER TABLE context_projection_versions DROP CONSTRAINT %I',n); END $$; UPDATE context_projection_versions SET payload_digest=repeat('a',64); ALTER TABLE context_projection_versions ADD CONSTRAINT native_test_projection_checksum CHECK(payload_digest=encode(sha256(convert_to(payload::text,'UTF8')),'hex')) NOT VALID; ALTER TABLE context_projection_versions ENABLE TRIGGER context_projection_immutable;").execute(store.pool()).await.expect("owned damaged checksum fixture");
    assert_eq!(
        store.projection_status(&[scope()]).await,
        Err(Error::Storage)
    );
    // The manifest changed, so stale requests fail before refresh. Even a current
    // manifest cannot repair self-digest corruption; that is a storage error.
    assert_eq!(
        store
            .project_context(
                &[scope()],
                status["manifest_digest"].as_str().expect("digest")
            )
            .await,
        Err(Error::Conflict)
    );
    let current_values: Value = sqlx::query_scalar("SELECT jsonb_agg(jsonb_build_array(m.material_id::text,m.revision,m.scope,m.path,m.content_digest,p.payload->>'status',p.payload_digest) ORDER BY m.material_id,m.revision,m.path) FROM context_materials m LEFT JOIN context_projection_versions p USING(material_id,revision)").fetch_one(store.pool()).await.expect("owned corrupt current manifest fields");
    let corrupt_manifest =
        ontology::store::digest(&serde_json::to_vec(&current_values).expect("manifest bytes"));
    assert_eq!(
        store.project_context(&[scope()], &corrupt_manifest).await,
        Err(Error::Storage)
    );
    let mut tx = store
        .pool()
        .begin()
        .await
        .expect("explicit corrupt refresh");
    sqlx::query("SELECT set_config('ontology.projection_refresh','on',true)")
        .execute(&mut *tx)
        .await
        .expect("owned refresh flag");
    assert!(sqlx::query("UPDATE context_projection_versions SET payload_digest=encode(sha256(convert_to(payload::text,'UTF8')),'hex')").execute(&mut *tx).await.is_err(), "even a valid new payload digest cannot hide corrupt old bytes");
    tx.rollback().await.expect("rollback corrupt refresh");
    sqlx::raw_sql("ALTER TABLE context_projection_versions DISABLE TRIGGER context_projection_immutable; UPDATE context_projection_versions SET payload_digest=encode(sha256(convert_to(payload::text,'UTF8')),'hex'); ALTER TABLE context_projection_versions VALIDATE CONSTRAINT native_test_projection_checksum; ALTER TABLE context_projection_versions ENABLE TRIGGER context_projection_immutable;").execute(store.pool()).await.expect("restore owned fixture");
}

#[tokio::test]
async fn native_pre_apply_close_checks_real_effects_and_only_terminates_drifted_run() {
    use context_core::harness::*;
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let view = harness_fixture::view();
    let root = view.path().canonicalize().unwrap();
    let (files, prepared) = composed_cli_fixture(
        &store,
        &root,
        "Create a fictional document; close this request before applying it.",
        false,
    )
    .await;
    let run = "native-preapply-close";
    let begun = harness_fixture::command(
        &store,
        &root,
        &root,
        "begin",
        &[
            "--prepared-run".into(),
            files.path().join("prepared.json").display().to_string(),
            "--run-id".into(),
            run.into(),
        ],
    )
    .await;
    assert!(
        begun.status.success(),
        "{}",
        String::from_utf8_lossy(&begun.stderr)
    );
    let original: HarnessExecutionRecord =
        decode_current_json(&begun.stdout, "begun fixture").unwrap();
    let head_path = root
        .join(".llm-context-vault-harness/runs")
        .join(run)
        .join("head.json");
    let head_bytes = fs::read(&head_path).unwrap();
    let snapshot: Value = sqlx::query_scalar("SELECT jsonb_agg(jsonb_build_array(material_id,revision,content_digest,origin_kind,source_digest) ORDER BY material_id) FROM context_materials").fetch_one(store.pool()).await.unwrap();
    let config = files.path().join("current-policy.json");
    let mut bytes = fs::read(context_fixture::configuration_path()).unwrap();
    bytes.push(b' ');
    fs::write(&config, bytes).unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
        .fetch_one(store.pool())
        .await
        .unwrap();
    async fn call(
        root: &Path,
        config: &Path,
        id: &str,
        run: &str,
        verb: &str,
        extra: &[&str],
    ) -> std::process::Output {
        tokio::process::Command::new(env!("CARGO_BIN_EXE_ontology"))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("TMPDIR", std::env::temp_dir())
            .env("DATABASE_URL", std::env::var("TEST_DATABASE_URL").unwrap())
            .args(["harness", verb, "--context-view"])
            .arg(root)
            .arg("--workspace-root")
            .arg(root)
            .arg("--policy-config")
            .arg(config)
            .args(["--store-id", id, "--run-id", run])
            .args(extra)
            .output()
            .await
            .unwrap()
    }
    let args = [
        "--close-before-apply",
        "--reason",
        "Stop the uncommitted fictional request",
    ];
    for extra in [vec!["--reason", "Stop"], vec!["--close-before-apply"]] {
        let out = call(&root, &config, &id, run, "recover", &extra).await;
        assert!(!out.status.success() && out.stdout.is_empty());
    }
    let ordinary = call(&root, &config, &id, run, "recover", &[]).await;
    assert!(
        !ordinary.status.success() && ordinary.stdout.is_empty(),
        "policy drift must still reject ordinary recovery"
    );
    for (state, matching_run, matching_digest) in [
        ("pending", false, false),
        ("committed", true, false),
        ("finalized", true, false),
        ("aborted", false, true),
    ] {
        let effect_run = if matching_run {
            run
        } else {
            "other-fictional-run"
        };
        let effect_digest = if matching_digest {
            prepared.prepared_run_digest.clone()
        } else {
            "a".repeat(64)
        };
        let is_committed = state == "committed" || state == "finalized";
        let apply: String = sqlx::query_scalar("INSERT INTO context_apply_batches(apply_id,store_id,core_run_id,prepared_run_digest,candidate_digest,expected_source_versions,context_targets,core_apply_attempt_id,expected_batch_id,expected_journal_locator,state,actual_batch_id,actual_journal_locator,commit_receipt,final_core_receipt_digest) VALUES(gen_random_uuid(),$1::uuid,$2,$3,$4,'{}','[{}]','fictional-attempt','fictional-batch','fictional-journal',$5,$6,$7,$8,$9) RETURNING apply_id::text")
            .bind(&id).bind(effect_run).bind(effect_digest).bind("b".repeat(64)).bind(state)
            .bind(is_committed.then_some("fictional-batch")).bind(is_committed.then_some("fictional-journal"))
            .bind(is_committed.then_some(json!({"fictional":"counterevidence, not a Core receipt"})))
            .bind((state == "finalized").then(|| "c".repeat(64)))
            .fetch_one(store.pool()).await.unwrap();
        let rejected = call(&root, &config, &id, run, "recover", &args).await;
        assert!(
            !rejected.status.success() && rejected.stdout.is_empty(),
            "reject observed {state} effect"
        );
        assert_eq!(fs::read(&head_path).unwrap(), head_bytes);
        assert!(!head_path.with_file_name("pre-apply-close.json").exists());
        sqlx::query("DELETE FROM context_apply_batches WHERE apply_id=$1::uuid")
            .bind(apply)
            .execute(store.pool())
            .await
            .unwrap();
    }
    let closed = call(&root, &config, &id, run, "recover", &args).await;
    assert!(
        closed.status.success(),
        "{}",
        String::from_utf8_lossy(&closed.stderr)
    );
    let closed: Value = serde_json::from_slice(&closed.stdout).unwrap();
    let head: HarnessExecutionRecord = decode_current_json(
        &serde_json::to_vec(&closed["head"]).unwrap(),
        "closed fixture",
    )
    .unwrap();
    assert_eq!(head.state, HarnessExecutionState::ClosedBeforeApply);
    assert_eq!(head.role_execution, original.role_execution);
    assert_eq!(head.sequence, original.sequence + 1);
    assert!(head.ready_role_invocations.is_empty() && head.evaluation.is_none());
    assert!(!head_path.with_file_name("apply-attempt.json").exists());
    let repeated = call(&root, &config, &id, run, "recover", &args).await;
    assert!(repeated.status.success());
    assert_eq!(
        closed,
        serde_json::from_slice::<Value>(&repeated.stdout).unwrap()
    );
    let after: Value = sqlx::query_scalar("SELECT jsonb_agg(jsonb_build_array(material_id,revision,content_digest,origin_kind,source_digest) ORDER BY material_id) FROM context_materials").fetch_one(store.pool()).await.unwrap();
    assert_eq!(
        snapshot, after,
        "closure changes no native material or origin"
    );
    let versions: i64 = sqlx::query_scalar("SELECT count(*) FROM context_material_versions")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(versions as usize, context_fixture::DOCUMENT_COUNT);
    for verb in ["evaluate", "validate", "apply"] {
        assert!(
            !call(&root, &config, &id, run, verb, &[])
                .await
                .status
                .success()
        );
    }
}
