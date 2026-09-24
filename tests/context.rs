use meenseek_ontology::{
    context::{ContextCommand, ContextScope, MAX_FILE_BYTES, MAX_READ_BYTES, inventory},
    domain::Error,
    store::{Store, digest},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
};

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn scope(value: &str) -> ContextScope {
    value.parse().expect("valid synthetic scope")
}

#[tokio::test]
async fn manual_edit_preserves_original_and_records_a_separate_version() {
    let _lock = TEST_LOCK.lock().await;
    let store = store().await;
    let (_temp, root) = source();
    let personal = scope("personal");
    let before = b"---\ntitle: Original\n---\n\nBefore\n";
    let after = "---\ntitle: Changed\n---\n\nAfter\n";
    write(&root, "personal/note.md", before);
    let manifest = inventory(&root, std::slice::from_ref(&personal)).expect("synthetic inventory");
    store
        .import_context(
            &root,
            std::slice::from_ref(&personal),
            &manifest.inventory_digest,
        )
        .await
        .expect("synthetic import");
    let result = store
        .edit_context(&personal, "note.md", 1, &digest(before), after)
        .await
        .expect("manual edit");
    assert!(result.changed);
    assert_eq!(result.revision, 2);
    assert_eq!(result.content_digest, digest(after.as_bytes()));
    assert!(result.manual_edit_id.is_some());
    assert_eq!(
        store
            .read_context(&personal, "note.md", false)
            .await
            .expect("latest original"),
        after
    );
    assert_eq!(
        store
            .read_context_revision(&personal, "note.md", 1)
            .await
            .expect("prior version")["content"],
        String::from_utf8_lossy(before).as_ref()
    );
    let history = store
        .context_history(&personal, "note.md", None)
        .await
        .expect("history");
    assert_eq!(history["items"].as_array().unwrap().len(), 2);
    assert_eq!(history["items"][0]["change_kind"], "manual");
    assert_eq!(history["items"][1]["change_kind"], "import");
    let provenance: (String, String, i64) = sqlx::query_as("SELECT source_digest,source_path,revision FROM context_materials WHERE scope='personal' AND path='note.md'").fetch_one(store.pool()).await.expect("source provenance");
    assert_eq!(
        provenance,
        (digest(before), "personal/note.md".to_owned(), 2)
    );
    let version: (Option<String>, Option<String>) = sqlx::query_as("SELECT apply_id::text,manual_edit_id::text FROM context_material_versions WHERE material_id=(SELECT material_id FROM context_materials WHERE scope='personal' AND path='note.md') AND revision=2").fetch_one(store.pool()).await.expect("manual version");
    assert_eq!(version.0, None);
    assert_eq!(version.1, result.manual_edit_id);
    let projection: (String,) = sqlx::query_as("SELECT payload->>'source_digest' FROM context_projection_versions WHERE material_id=(SELECT material_id FROM context_materials WHERE scope='personal' AND path='note.md') AND revision=2").fetch_one(store.pool()).await.expect("matching projection");
    assert_eq!(projection.0, result.content_digest);
    assert_eq!(
        store
            .edit_context(&personal, "note.md", 1, &digest(before), "stale")
            .await,
        Err(Error::Conflict)
    );
    assert!(
        !store
            .edit_context(&personal, "note.md", 2, &result.content_digest, after)
            .await
            .expect("idempotent save")
            .changed
    );
    let profile = scope("profile");
    write(&root, "profile/rule.md", before);
    let large_before = "a".repeat(MAX_READ_BYTES + 1);
    write(&root, "profile/large.md", large_before.as_bytes());
    let manifest = inventory(&root, std::slice::from_ref(&profile)).expect("profile inventory");
    store
        .import_context(
            &root,
            std::slice::from_ref(&profile),
            &manifest.inventory_digest,
        )
        .await
        .expect("profile import");
    assert_eq!(
        store
            .edit_context(&profile, "rule.md", 1, &digest(before), after)
            .await,
        Err(Error::Invalid)
    );
    assert_eq!(
        store
            .read_context(&profile, "rule.md", false)
            .await
            .expect("current profile"),
        std::str::from_utf8(before).expect("fixture UTF-8")
    );
    let large_after = format!("{large_before}b");
    assert_eq!(
        store
            .edit_context(
                &personal,
                "note.md",
                2,
                &result.content_digest,
                &large_after
            )
            .await,
        Err(Error::Invalid)
    );
    assert_eq!(
        store
            .read_context_material(&profile, "large.md")
            .await
            .expect("large Markdown read")
            .content,
        large_before
    );
    assert_eq!(
        store
            .read_context_revision(&profile, "large.md", 1)
            .await
            .expect("large Markdown history")["content"],
        large_before
    );
    let large_projection: (String,) = sqlx::query_as("SELECT payload->>'status' FROM context_projection_versions WHERE material_id=(SELECT material_id FROM context_materials WHERE scope='profile' AND path='large.md') AND revision=1")
        .fetch_one(store.pool()).await.expect("large Markdown projection");
    assert_eq!(large_projection.0, "unavailable");
    assert_eq!(
        store
            .edit_context(&profile, "raw/rule.md", 1, &digest(before), after)
            .await,
        Err(Error::Invalid)
    );
}
fn source() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("isolated source");
    let root = temp
        .path()
        .canonicalize()
        .expect("canonical temporary path");
    (temp, root)
}
fn write(root: &Path, path: &str, bytes: &[u8]) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().expect("fixture has parent")).expect("fixture directory");
    fs::write(path, bytes).expect("synthetic bytes");
}
async fn store() -> Store {
    let url = std::env::var("TEST_DATABASE_URL").expect("explicit isolated DB");
    assert!(
        meenseek_ontology::config::database_options(&url)
            .expect("loopback DB")
            .get_database()
            .is_some_and(|name| name.starts_with("ontology_test_"))
    );
    let store = Store::connect(&url).await.expect("isolated PostgreSQL");
    store.initialize().await.expect("additive migrations");
    sqlx::query("TRUNCATE context_source_bindings,context_projection_versions, context_material_versions, context_manual_edits,context_materials, context_apply_batches")
        .execute(store.pool())
        .await
        .expect("reset synthetic materials only");
    store
}

#[test]
fn inventory_is_exact_scoped_and_rejects_unsafe_sources() {
    let (_temp, root) = source();
    let scopes = [scope("profile"), scope("personal"), scope("work/acme")];
    for selected in &scopes {
        fs::create_dir_all(root.join(selected.as_str())).expect("scope directory");
    }
    let body = "---\r\ntitle: 합성 원본\r\nexport: false\r\n---\r\n\r\n원문\n".as_bytes();
    write(&root, "personal/한글 문서.md", body);
    write(&root, "work/acme/same.md", b"company");
    write(
        &root,
        "personal/attachments/picture.bin",
        &[0, 255, 128, 10],
    );
    for path in [
        ".hidden/note.md",
        "journal/note.md",
        "raw/data",
        ".private.md",
    ] {
        write(&root, &format!("personal/{path}"), b"restricted");
    }
    let first = inventory(&root, &scopes).expect("complete bounded inventory");
    assert_eq!(first.entries.len(), 7);
    assert_eq!(
        first
            .entries
            .iter()
            .filter(|entry| entry.restricted)
            .count(),
        4
    );
    assert_eq!(
        first
            .entries
            .iter()
            .find(|entry| entry.path == "한글 문서.md")
            .expect("unicode path")
            .content_digest,
        digest(body)
    );
    assert_eq!(inventory(&root, &scopes).expect("repeat scan"), first);
    assert_eq!(
        inventory(&root, &[scopes[0].clone()])
            .expect("empty valid scope")
            .entries
            .len(),
        0
    );
    assert!(inventory(&root, &[]).is_err());
    assert!(inventory(&root, &[scopes[0].clone(), scopes[0].clone()]).is_err());
    assert!(inventory(&root, &[scope("work/absent")]).is_err());
    for bad in [
        "work",
        "all",
        "work/",
        "work/acme/other",
        "work/../personal",
        "work/Acme",
        "../personal",
        "personal/",
    ] {
        assert!(bad.parse::<ContextScope>().is_err(), "reject {bad}");
    }
    for command in [
        json!({"op":"read","scope":"work","path":"x"}),
        json!({"op":"read","scope":"personal","path":"x","extra":true}),
    ] {
        assert!(serde_json::from_value::<ContextCommand>(command).is_err());
    }
    let too_large = fs::File::create(root.join("profile/large.bin")).expect("size fixture");
    too_large
        .set_len(MAX_FILE_BYTES as u64 + 1)
        .expect("sparse oversized fixture");
    assert_eq!(inventory(&root, &scopes), Err(Error::Limit));
    fs::remove_file(root.join("profile/large.bin")).expect("remove fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let (_outside_temp, outside) = source();
        write(&outside, "secret", b"outside");
        symlink(outside.join("secret"), root.join("personal/link")).expect("file symlink fixture");
        assert!(inventory(&root, &scopes).is_err());
        fs::remove_file(root.join("personal/link")).expect("remove link");
        symlink(&outside, root.join("personal/link")).expect("directory symlink fixture");
        assert!(inventory(&root, &scopes).is_err());
        fs::remove_file(root.join("personal/link")).expect("remove link");
        fs::hard_link(outside.join("secret"), root.join("personal/linked"))
            .expect("hardlink fixture");
        assert!(inventory(&root, &scopes).is_err());
        fs::remove_file(root.join("personal/linked")).expect("remove linked fixture");
        symlink(&root, outside.join("alias")).expect("root alias");
        assert!(inventory(&outside.join("alias"), &scopes).is_err());
        assert!(inventory(&outside.join("alias/personal/.."), &scopes).is_err());
    }
    assert!(inventory(&root.join("personal/.."), &scopes).is_err());
    assert!(inventory(Path::new("relative"), &scopes).is_err());
}

#[tokio::test]
async fn postgres_transport_preserves_bytes_scope_archive_and_provenance() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_temp, root) = source();
    let scopes: Vec<_> = [
        "profile",
        "personal",
        "work/common",
        "work/alpha",
        "work/beta",
    ]
    .into_iter()
    .map(scope)
    .collect();
    for selected in &scopes {
        write(
            &root,
            &format!("{}/note.md", selected.as_str()),
            format!(
                "---\r\ntitle: 합성\r\n---\r\nmarker {}\n",
                selected.as_str()
            )
            .as_bytes(),
        );
    }
    let binary = [0, 255, 128, 1, 13, 10];
    write(&root, "personal/attachments/사진.bin", &binary);
    write(
        &root,
        "personal/journal/diary.md",
        b"marker restricted journal",
    );
    write(&root, "personal/raw/input.bin", b"marker restricted raw");
    write(&root, "personal/.hidden", b"marker restricted hidden");
    let manifest = inventory(&root, &scopes).expect("explicit inventory before import");
    let before = store.calls();
    let result = store
        .import_context(&root, &scopes, &manifest.inventory_digest)
        .await
        .expect("atomic original transport");
    assert_eq!(
        store.calls() - before,
        8,
        "one insert plus bounded projection validation/append"
    );
    assert_eq!(result["inserted"], 9);
    assert_eq!(result["verified"], 9);
    let saved: Value = sqlx::query_scalar(
        "SELECT jsonb_agg(to_jsonb(m) ORDER BY scope,path) FROM context_materials m",
    )
    .fetch_one(store.pool())
    .await
    .expect("snapshot of synthetic rows");
    let before = store.calls();
    let repeat = store
        .import_context(&root, &scopes, &manifest.inventory_digest)
        .await
        .expect("idempotent retry");
    assert_eq!(store.calls() - before, 6);
    assert_eq!(repeat["inserted"], 0);
    let unchanged: Value = sqlx::query_scalar(
        "SELECT jsonb_agg(to_jsonb(m) ORDER BY scope,path) FROM context_materials m",
    )
    .fetch_one(store.pool())
    .await
    .expect("unchanged rows");
    assert_eq!(
        saved, unchanged,
        "retry preserves original timestamps and provenance"
    );
    let restarted = Store::connect(&std::env::var("TEST_DATABASE_URL").expect("isolated URL"))
        .await
        .expect("new DB connection");
    assert_eq!(
        restarted
            .verify_context(&root, &scopes)
            .await
            .expect("persistent verified originals")["verified"],
        9
    );
    for selected in &scopes {
        let result = restarted
            .search_context(selected, "marker", None, 100)
            .await
            .expect("explicit scope search");
        assert_eq!(result["items"].as_array().expect("metadata").len(), 1);
        assert_eq!(result["items"][0]["scope"], selected.as_str());
        assert_eq!(
            result["items"][0]["source_root"],
            root.to_str().expect("UTF8 path")
        );
        assert_eq!(
            result["items"][0]["source_path"],
            format!("{}/note.md", selected.as_str())
        );
        assert!(result["items"][0].get("content").is_none());
        assert_eq!(
            restarted
                .read_context(selected, "note.md", false)
                .await
                .expect("selected exact original")
                .as_bytes(),
            fs::read(root.join(selected.as_str()).join("note.md")).expect("source bytes")
        );
    }
    assert_eq!(
        store
            .read_context(&scope("work/beta"), "attachments/사진.bin", false)
            .await,
        Err(Error::NotFound)
    );
    assert_eq!(
        store
            .read_context(&scope("personal"), "journal/diary.md", false)
            .await,
        Err(Error::NotFound)
    );
    assert_eq!(
        store
            .read_context(&scope("personal"), "journal/diary.md", true)
            .await
            .expect("explicit archive selection"),
        "marker restricted journal"
    );
    assert_eq!(
        store
            .read_context(&scope("personal"), "attachments/사진.bin", false)
            .await,
        Err(Error::Invalid)
    );
    let (_output_temp, output) = source();
    let paths = vec!["note.md".to_owned(), "attachments/사진.bin".to_owned()];
    let before = store.calls();
    assert_eq!(
        store
            .export_context(&scope("personal"), &paths, &output.join("export"), false)
            .await
            .expect("new safe export")["exported"],
        2
    );
    assert_eq!(store.calls() - before, 5);
    for path in &paths {
        assert_eq!(
            fs::read(output.join("export").join(path)).expect("export bytes"),
            fs::read(root.join("personal").join(path)).expect("source bytes")
        );
    }
    assert_eq!(
        store
            .export_context(&scope("personal"), &paths, &output.join("export"), false)
            .await,
        Err(Error::Conflict)
    );
    let archives = vec![
        "journal/diary.md".into(),
        "raw/input.bin".into(),
        ".hidden".into(),
    ];
    assert_eq!(
        store
            .export_context(
                &scope("personal"),
                &archives,
                &output.join("archive"),
                false
            )
            .await,
        Err(Error::NotFound)
    );
    assert!(!output.join("archive").exists());
    store
        .export_context(&scope("personal"), &archives, &output.join("archive"), true)
        .await
        .expect("explicit archive restore");
    for path in archives {
        assert_eq!(
            fs::read(output.join("archive").join(&path)).expect("archive bytes"),
            fs::read(root.join("personal").join(&path)).expect("source bytes")
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        assert_eq!(
            fs::metadata(output.join("export/note.md"))
                .expect("private output")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(output.join("export"))
                .expect("private directory")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        symlink(output.join("export"), output.join("alias")).expect("unsafe destination alias");
        assert!(
            store
                .export_context(&scope("personal"), &paths, &output.join("alias/new"), false)
                .await
                .is_err()
        );
        assert!(
            store
                .export_context(&scope("personal"), &paths, &output.join("alias"), false)
                .await
                .is_err()
        );
        assert!(!output.join("export/new").exists());
    }
}

#[tokio::test]
async fn source_drift_and_existing_ontology_edits_fail_atomically() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_temp, root) = source();
    let scopes = [scope("personal")];
    write(&root, "personal/note.md", b"original");
    let original = inventory(&root, &scopes).expect("original inventory");
    store
        .import_context(&root, &scopes, &original.inventory_digest)
        .await
        .expect("first import");
    write(&root, "personal/note.md", b"changed source");
    let before = store.calls();
    assert_eq!(
        store
            .import_context(&root, &scopes, &original.inventory_digest)
            .await,
        Err(Error::Conflict)
    );
    assert_eq!(
        store.calls() - before,
        4,
        "source scan happens only after the gate, then rolls back"
    );
    write(&root, "personal/new.md", b"must roll back");
    let changed = inventory(&root, &scopes).expect("new source scan");
    let before = store.calls();
    assert_eq!(
        store
            .import_context(&root, &scopes, &changed.inventory_digest)
            .await,
        Err(Error::Conflict)
    );
    assert_eq!(
        store.calls() - before,
        8,
        "one bounded transaction including projection validation/append and rollback"
    );
    assert_eq!(
        store
            .read_context(&scopes[0], "note.md", false)
            .await
            .expect("original retained"),
        "original"
    );
    assert_eq!(
        store.read_context(&scopes[0], "new.md", false).await,
        Err(Error::NotFound)
    );
    assert_eq!(
        store.verify_context(&root, &scopes).await,
        Err(Error::Conflict)
    );
    fs::remove_file(root.join("personal/new.md")).expect("restore fixture");
    write(&root, "personal/note.md", b"original");
    let (_other_temp, other) = source();
    write(&other, "personal/note.md", b"original");
    let other_manifest = inventory(&other, &scopes).expect("other provenance");
    assert_eq!(
        store
            .import_context(&other, &scopes, &other_manifest.inventory_digest)
            .await,
        Err(Error::Conflict)
    );
    let edited = b"ontology owned edit";
    let apply = synthetic_apply(&store).await;
    sqlx::query("UPDATE context_materials SET content=$1,content_digest=$2,byte_len=$3,search_text='ontology owned edit',last_apply_id=$4::uuid WHERE scope='personal' AND path='note.md'")
        .bind(edited.as_slice()).bind(digest(edited)).bind(edited.len() as i64).bind(&apply).execute(store.pool()).await.expect("synthetic future ontology edit");
    finish_synthetic_apply(&store, &apply).await;
    assert_eq!(
        store
            .import_context(&root, &scopes, &original.inventory_digest)
            .await,
        Err(Error::Conflict)
    );
    assert_eq!(
        store
            .read_context(&scopes[0], "note.md", false)
            .await
            .expect("edit retained")
            .as_bytes(),
        edited
    );
    assert_eq!(
        store.verify_context(&root, &scopes).await,
        Err(Error::Conflict)
    );
    let apply = synthetic_apply(&store).await;
    let result = sqlx::query(
        "UPDATE context_materials SET last_apply_id=$1::uuid,content='different'::bytea WHERE scope='personal'",
    )
    .bind(&apply)
    .execute(store.pool())
    .await;
    assert!(result.is_err(), "database rejects corrupt content digests");
    finish_synthetic_apply(&store, &apply).await;
}

#[tokio::test]
async fn cardinality_limits_and_call_counts_are_explicit() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let scopes = [scope("personal")];
    for size in [0usize, 1, 101] {
        sqlx::query("TRUNCATE context_source_bindings,context_projection_versions, context_material_versions, context_manual_edits,context_materials, context_apply_batches")
            .execute(store.pool())
            .await
            .expect("reset bounded fixture");
        let (_temp, root) = source();
        fs::create_dir(root.join("personal")).expect("empty scope allowed");
        for i in 0..size {
            write(&root, &format!("personal/{i:04}.md"), b"bounded marker");
        }
        let manifest = inventory(&root, &scopes).expect("bounded inventory");
        assert_eq!(
            manifest.total_bytes,
            (size * b"bounded marker".len()) as u64
        );
        assert!(serde_json::to_vec(&manifest).expect("metadata JSON").len() <= 512 + size * 256);
        for inserted in [size, 0] {
            let before = store.calls();
            let result = store
                .import_context(&root, &scopes, &manifest.inventory_digest)
                .await
                .expect("bounded import or retry");
            assert_eq!(result["inserted"], inserted);
            assert_eq!(
                store.calls() - before,
                5 + size.div_ceil(100) as u64 * if inserted == 0 { 1 } else { 3 },
                "two projection queries only for each inserted page"
            );
        }
        let before = store.calls();
        store
            .verify_context(&root, &scopes)
            .await
            .expect("full fresh digest scan");
        assert_eq!(store.calls() - before, 5);
        let before = store.calls();
        let page = store
            .search_context(&scopes[0], "marker", None, 100)
            .await
            .expect("bounded search");
        assert_eq!(store.calls() - before, 5);
        assert_eq!(
            page["items"].as_array().expect("metadata").len(),
            size.min(100)
        );
        assert!(serde_json::to_vec(&page).expect("page JSON").len() <= 512 + size.min(100) * 512);
        if size > 100 {
            let after = page["next_after"]
                .as_str()
                .expect("complete pagination cursor");
            let next = store
                .search_context(&scopes[0], "marker", Some(after), 100)
                .await
                .expect("last page");
            assert_eq!(next["items"].as_array().expect("last item").len(), 1);
            assert!(next["next_after"].is_null());
        } else {
            assert!(page["next_after"].is_null());
        }
        let before = store.calls();
        for bad in ["../x", "/x", "a/../x", "a//x", "a/./x", "a\\x", "", "x/"] {
            assert_eq!(
                store.read_context(&scopes[0], bad, false).await,
                Err(Error::Invalid)
            );
            assert_eq!(
                store
                    .export_context(&scopes[0], &[bad.to_owned()], &root.join("export"), false)
                    .await,
                Err(Error::Invalid)
            );
        }
        assert_eq!(
            store.search_context(&scopes[0], "", None, 101).await,
            Err(Error::Invalid)
        );
        assert_eq!(
            store.search_context(&scopes[0], "", None, 0).await,
            Err(Error::Invalid)
        );
        assert_eq!(
            store
                .search_context(&scopes[0], &"x".repeat(241), None, 1)
                .await,
            Err(Error::Invalid)
        );
        assert_eq!(store.calls() - before, 0);
        let before = store.calls();
        assert_eq!(
            store.read_context(&scopes[0], "absent.md", false).await,
            Err(Error::NotFound)
        );
        assert_eq!(store.calls() - before, 5);
    }
    let (_temp, root) = source();
    let bytes = vec![b'x'; MAX_READ_BYTES + 1];
    write(&root, "profile/large.txt", &bytes);
    let scopes = [scope("profile")];
    let manifest = inventory(&root, &scopes).expect("large attachment allowed");
    store
        .import_context(&root, &scopes, &manifest.inventory_digest)
        .await
        .expect("preserve large bytes");
    assert_eq!(
        store.read_context(&scopes[0], "large.txt", false).await,
        Err(Error::Limit)
    );
    store
        .export_context(
            &scopes[0],
            &["large.txt".into()],
            &root.join("export"),
            false,
        )
        .await
        .expect("large material exported exactly");
    assert_eq!(
        fs::read(root.join("export/large.txt")).expect("exported bytes"),
        bytes
    );
}

async fn cli(body: &str, database: bool) -> std::process::Output {
    use tokio::io::AsyncWriteExt;
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_meenseek-ontology"));
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .arg("context")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if database {
        command.env(
            "DATABASE_URL",
            std::env::var("TEST_DATABASE_URL").expect("isolated URL"),
        );
    }
    let mut child = command.spawn().expect("candidate CLI");
    let mut stdin = child.stdin.take().expect("piped stdin");
    let _ = stdin.write_all(body.as_bytes()).await;
    drop(stdin);
    child.wait_with_output().await.expect("CLI output")
}

#[tokio::test]
async fn cli_inventory_is_offline_and_selected_text_is_exact() {
    let _guard = TEST_LOCK.lock().await;
    let _store = store().await;
    let (_temp, root) = source();
    let bytes = "---\r\ntitle: 원문\r\n---\r\n그대로".as_bytes();
    write(&root, "personal/exact.md", bytes);
    let output = cli(
        &json!({"op":"inventory","root":root,"scopes":["personal"]}).to_string(),
        false,
    )
    .await;
    assert!(
        output.status.success(),
        "inventory needs no database or old Vault executable"
    );
    assert!(output.stderr.is_empty());
    let manifest: Value = serde_json::from_slice(&output.stdout).expect("inventory JSON");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("title: 원문"));
    let output = cli(&json!({"op":"import","root":root,"scopes":["personal"],"inventory_digest":manifest["inventory_digest"]}).to_string(), true).await;
    assert!(output.status.success());
    let output = cli(
        &json!({"op":"read","scope":"personal","path":"exact.md"}).to_string(),
        true,
    )
    .await;
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, bytes, "no JSON escaping or appended newline");
    for bad in [
        "not-json sensitive-marker".to_owned(),
        "sensitive-marker".repeat(3000),
        "{\"op\":\"read\",\"scope\":\"personal\",\"scope\":\"work/acme\",\"path\":\"exact.md\"}"
            .to_owned(),
        json!({"op":"read","scope":"work","path":"exact.md"}).to_string(),
    ] {
        let output = cli(&bad, true).await;
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(output.stderr.len() < 1024);
        assert!(!String::from_utf8_lossy(&output.stderr).contains("sensitive-marker"));
    }
}

#[tokio::test]
async fn context_discovery_and_selected_bytes_have_bounded_calls() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    for size in [0usize, 1, 101] {
        sqlx::query("TRUNCATE context_source_bindings,context_projection_versions, context_material_versions, context_manual_edits,context_materials, context_apply_batches")
            .execute(store.pool())
            .await
            .expect("isolated fixture reset");
        let (_temp, root) = source();
        fs::create_dir(root.join("personal")).expect("empty scope fixture");
        for number in 0..size {
            write(
                &root,
                &format!("personal/{number:04}.md"),
                b"metadata search marker",
            );
        }
        write(&root, "work/hidden/journal/only.md", b"excluded scope");
        let scopes = [scope("personal"), scope("work/hidden")];
        let manifest = inventory(&root, &scopes).expect("bounded synthetic originals");
        store
            .import_context(&root, &scopes, &manifest.inventory_digest)
            .await
            .expect("fixture import");
        let before = store.calls();
        let discovered = store.context_scopes().await.expect("metadata discovery");
        assert_eq!(
            store.calls() - before,
            5,
            "one gated distinct query for zero/one/many originals, no retries"
        );
        assert_eq!(
            discovered,
            if size == 0 {
                vec![]
            } else {
                vec![scope("personal")]
            }
        );
        let before = store.calls();
        let page = store
            .search_context(&scope("personal"), "marker", None, 20)
            .await
            .expect("metadata page");
        assert_eq!(store.calls() - before, 5);
        assert_eq!(
            page["items"].as_array().expect("bounded page").len(),
            size.min(20)
        );
        assert!(
            serde_json::to_vec(&page).expect("bounded transfer").len() <= 512 + size.min(20) * 512
        );
        let before = store.calls();
        let material = store.download_context(&scope("personal"), "0000.md").await;
        assert_eq!(
            store.calls() - before,
            5,
            "one gated selected byte query even for absence"
        );
        if size == 0 {
            assert!(matches!(material, Err(Error::NotFound)));
        } else {
            let material = material.expect("original file");
            assert_eq!(material.bytes, b"metadata search marker");
            assert_eq!(material.metadata.content_digest, digest(&material.bytes));
            assert_eq!(
                material.metadata.source_digest,
                Some(digest(&material.bytes))
            );
            assert_eq!(material.metadata.source_path, "personal/0000.md");
            assert_eq!(material.metadata.byte_len, material.bytes.len());
        }
        let before = store.calls();
        for path in ["../x", "/x", "x//y", "x\\y", ""] {
            assert!(matches!(
                store.download_context(&scope("personal"), path).await,
                Err(Error::Invalid)
            ));
        }
        for path in ["journal/only.md", "raw/data", ".hidden"] {
            assert!(matches!(
                store.download_context(&scope("work/hidden"), path).await,
                Err(Error::NotFound)
            ));
        }
        assert_eq!(
            store.calls() - before,
            12,
            "three restricted selections each pass the gate without fetching bytes"
        );
    }
    sqlx::query("TRUNCATE context_source_bindings,context_projection_versions, context_material_versions, context_manual_edits,context_materials, context_apply_batches")
        .execute(store.pool())
        .await
        .expect("isolated discovery bound");
    let (_temp, root) = source();
    let scopes: Vec<_> = (0..64)
        .map(|number| scope(&format!("work/scope-{number:02}")))
        .collect();
    for selected in &scopes {
        write(&root, &format!("{}/note.md", selected.as_str()), b"scope");
    }
    let manifest = inventory(&root, &scopes).expect("maximum scope import");
    store
        .import_context(&root, &scopes, &manifest.inventory_digest)
        .await
        .expect("64 scopes");
    let before = store.calls();
    assert_eq!(
        store
            .context_scopes()
            .await
            .expect("bounded discovery")
            .len(),
        64
    );
    assert_eq!(store.calls() - before, 5);
    write(&root, "work/extra/note.md", b"extra scope");
    let extra = [scope("work/extra")];
    let manifest = inventory(&root, &extra).expect("additional scoped source");
    store
        .import_context(&root, &extra, &manifest.inventory_digest)
        .await
        .expect("separate import");
    let before = store.calls();
    assert_eq!(store.context_scopes().await, Err(Error::Limit));
    assert_eq!(
        store.calls() - before,
        5,
        "overflow fails closed without another query"
    );
}

// Explicitly synthetic persistence fixtures. No helper represents Core acceptance.
async fn synthetic_apply(store: &Store) -> String {
    sqlx::query_scalar("WITH i AS (SELECT gen_random_uuid() AS id) INSERT INTO context_apply_batches(apply_id,store_id,core_run_id,prepared_run_digest,candidate_digest,expected_source_versions,context_targets,core_apply_attempt_id,expected_batch_id,expected_journal_locator,state) SELECT i.id,s.store_id,'synthetic-run',repeat('a',64),repeat('b',64),'{}','[{\"synthetic\":true}]','synthetic-attempt-'||i.id,'synthetic-batch-'||i.id,'synthetic-journal-'||i.id,'pending' FROM i CROSS JOIN context_store s RETURNING apply_id::text")
        .fetch_one(store.pool()).await.expect("explicit synthetic pending apply")
}
async fn finish_synthetic_apply(store: &Store, apply: &str) {
    sqlx::query("UPDATE context_apply_batches SET state='finalized',actual_batch_id=expected_batch_id,actual_journal_locator=expected_journal_locator,commit_receipt='{\"synthetic\":true}',final_core_receipt_digest=repeat('c',64) WHERE apply_id=$1::uuid")
        .bind(apply).execute(store.pool()).await.expect("synthetic terminal fixture, not a Core receipt");
}
async fn update_synthetic_material(store: &Store, path: &str, bytes: &[u8], deleted: bool) {
    let apply = synthetic_apply(store).await;
    sqlx::query("UPDATE context_materials SET content=$1,content_digest=$2,byte_len=$3,search_text=$4,deleted=$5,last_apply_id=$6::uuid WHERE scope='personal' AND path=$7")
        .bind(bytes).bind(digest(bytes)).bind(bytes.len() as i64)
        .bind(if deleted { None } else { std::str::from_utf8(bytes).ok() })
        .bind(deleted).bind(&apply).bind(path).execute(store.pool()).await.expect("synthetic material revision");
    finish_synthetic_apply(store, &apply).await;
}

#[tokio::test]
async fn native_identity_origins_and_continuous_history_are_enforced() {
    use sqlx::Row;
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_temp, root) = source();
    write(&root, "personal/imported.md", b"original");
    let scopes = [scope("personal")];
    let manifest = inventory(&root, &scopes).expect("synthetic original");
    store
        .import_context(&root, &scopes, &manifest.inventory_digest)
        .await
        .expect("imported version one");
    let origin: Value = sqlx::query_scalar("SELECT to_jsonb(m) - ARRAY['content','content_digest','byte_len','search_text','revision','deleted','last_apply_id'] FROM context_materials m WHERE path='imported.md'")
        .fetch_one(store.pool()).await.expect("immutable imported origin");
    for _ in 0..2 {
        store.initialize().await.expect("migration replay");
        store
            .import_context(&root, &scopes, &manifest.inventory_digest)
            .await
            .expect("idempotent import");
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM context_material_versions")
            .fetch_one(store.pool())
            .await
            .expect("version count");
        assert_eq!(count, 1);
    }
    update_synthetic_material(&store, "imported.md", b"changed", false).await;
    update_synthetic_material(&store, "imported.md", b"changed", false).await;
    update_synthetic_material(&store, "imported.md", b"", true).await;
    assert_eq!(
        store.read_context(&scopes[0], "imported.md", true).await,
        Err(Error::NotFound)
    );
    assert!(
        store
            .context_scopes()
            .await
            .expect("tombstone excluded")
            .is_empty()
    );
    assert!(
        store
            .search_context(&scopes[0], "", None, 20)
            .await
            .expect("current search")["items"]
            .as_array()
            .expect("items")
            .is_empty()
    );
    assert_eq!(
        store
            .export_context(
                &scopes[0],
                &["imported.md".into()],
                &root.join("deleted-export"),
                true
            )
            .await,
        Err(Error::NotFound)
    );
    update_synthetic_material(&store, "imported.md", b"recreated", false).await;
    let unchanged: Value = sqlx::query_scalar("SELECT to_jsonb(m) - ARRAY['content','content_digest','byte_len','search_text','revision','deleted','last_apply_id'] FROM context_materials m WHERE path='imported.md'")
        .fetch_one(store.pool()).await.expect("same imported identity after recreation");
    assert_eq!(origin, unchanged);
    let versions = sqlx::query("SELECT revision,content,content_digest,byte_len,deleted,apply_id::text FROM context_material_versions ORDER BY revision")
        .fetch_all(store.pool()).await.expect("exact history");
    for (index, (version, expected)) in versions
        .iter()
        .zip([
            b"original".as_slice(),
            b"changed",
            b"changed",
            b"",
            b"recreated",
        ])
        .enumerate()
    {
        assert_eq!(version.get::<i64, _>("revision"), index as i64 + 1);
        assert_eq!(version.get::<Vec<u8>, _>("content"), expected);
        assert_eq!(version.get::<String, _>("content_digest"), digest(expected));
        assert_eq!(version.get::<i64, _>("byte_len"), expected.len() as i64);
        assert_eq!(version.get::<bool, _>("deleted"), index == 3);
        assert_eq!(
            version.get::<Option<String>, _>("apply_id").is_none(),
            index == 0
        );
    }
    assert_eq!(versions.len(), 5);
    let apply = synthetic_apply(&store).await;
    sqlx::query("INSERT INTO context_materials(scope,path,source_root,source_path,source_digest,imported_at,origin_kind,content,content_digest,byte_len,restricted,search_text,last_apply_id) VALUES('personal','native.md',NULL,'personal/native.md',NULL,NULL,'native',$1,$2,6,false,'native',$3::uuid)")
        .bind(b"native".as_slice()).bind(digest(b"native")).bind(&apply).execute(store.pool()).await.expect("honest native origin");
    finish_synthetic_apply(&store, &apply).await;
    let native = store
        .read_context_material(&scopes[0], "native.md")
        .await
        .expect("native material");
    assert_eq!(
        native.metadata.origin_kind,
        meenseek_ontology::context::ContextOrigin::Native
    );
    assert_eq!(native.metadata.source_digest, None);
    assert_eq!(native.metadata.source_path, "personal/native.md");
    assert_eq!(native.metadata.revision, 1);
    let page = store
        .search_context(&scopes[0], "native", None, 20)
        .await
        .expect("native metadata");
    assert!(page["items"][0]["source_root"].is_null());
    assert!(page["items"][0]["source_digest"].is_null());
    assert_eq!(page["items"][0]["material_id"], native.metadata.material_id);
    update_synthetic_material(&store, "native.md", b"", true).await;
    update_synthetic_material(&store, "native.md", b"again", false).await;
    let recreated = store
        .read_context_material(&scopes[0], "native.md")
        .await
        .expect("native recreation");
    assert_eq!(recreated.metadata.material_id, native.metadata.material_id);
    assert_eq!(recreated.metadata.revision, 3);
    assert_eq!(recreated.metadata.origin_kind, native.metadata.origin_kind);
    assert_eq!(recreated.metadata.source_digest, None);
    // A later file import never overwrites a native or edited imported row.
    write(&root, "personal/native.md", b"file collision");
    let collision = inventory(&root, &scopes).expect("collision inventory");
    assert_eq!(
        store
            .import_context(&root, &scopes, &collision.inventory_digest)
            .await,
        Err(Error::Conflict)
    );
    assert_eq!(
        store
            .read_context(&scopes[0], "native.md", false)
            .await
            .expect("native retained"),
        "again"
    );
    let consistent: bool = sqlx::query_scalar("SELECT bool_and(ROW(m.content,m.content_digest,m.byte_len,m.restricted,m.search_text,m.deleted,m.last_apply_id) IS NOT DISTINCT FROM ROW(v.content,v.content_digest,v.byte_len,v.restricted,v.search_text,v.deleted,v.apply_id)) FROM context_materials m JOIN context_material_versions v USING(material_id,revision)")
        .fetch_one(store.pool()).await.expect("current and history consistency");
    assert!(consistent);
    let apply = synthetic_apply(&store).await;
    for assignment in [
        "material_id=gen_random_uuid()",
        "scope='profile'",
        "path='renamed.md'",
        "source_root='/different'",
        "source_path='personal/other.md'",
        "source_digest=repeat('d',64)",
        "origin_kind='native'",
        "imported_at=now()+interval '1 day'",
        "created_at=now()+interval '1 day'",
        "revision=7",
    ] {
        let mut query = sqlx::QueryBuilder::<sqlx::Postgres>::new("UPDATE context_materials SET ");
        query
            .push(assignment)
            .push(",last_apply_id=")
            .push_bind(&apply)
            .push("::uuid WHERE path='imported.md'");
        assert!(
            query.build().execute(store.pool()).await.is_err(),
            "reject immutable {assignment}"
        );
    }
    for sql in [
        "DELETE FROM context_materials",
        "UPDATE context_material_versions SET recorded_at=now()",
        "DELETE FROM context_material_versions",
        "UPDATE context_store SET store_id=gen_random_uuid()",
        "DELETE FROM context_store",
    ] {
        assert!(
            sqlx::query(sql).execute(store.pool()).await.is_err(),
            "reject {sql}"
        );
    }
    // Owned synthetic fixture only: reach bigint overflow without billions of revisions.
    let mut tx = store
        .pool()
        .begin()
        .await
        .expect("overflow fixture transaction");
    sqlx::raw_sql("ALTER TABLE context_materials DISABLE TRIGGER context_revision; ALTER TABLE context_materials DISABLE TRIGGER context_version; UPDATE context_materials SET revision=9223372036854775807 WHERE path='imported.md'; ALTER TABLE context_materials ENABLE TRIGGER context_revision; ALTER TABLE context_materials ENABLE TRIGGER context_version;")
        .execute(&mut *tx).await.expect("synthetic maximum revision");
    assert!(
        sqlx::query("UPDATE context_materials SET last_apply_id=$1::uuid WHERE path='imported.md'")
            .bind(&apply)
            .execute(&mut *tx)
            .await
            .is_err()
    );
    tx.rollback()
        .await
        .expect("discard synthetic overflow fixture entirely");
    finish_synthetic_apply(&store, &apply).await;
}

#[tokio::test]
async fn pending_and_committed_gate_every_store_read_before_sources_or_absence() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_temp, root) = source();
    write(&root, "personal/note.md", b"original");
    let scopes = [scope("personal")];
    let manifest = inventory(&root, &scopes).expect("synthetic originals");
    store
        .import_context(&root, &scopes, &manifest.inventory_digest)
        .await
        .expect("import");
    let apply = synthetic_apply(&store).await;
    for state in ["pending", "committed"] {
        if state == "committed" {
            sqlx::query("UPDATE context_apply_batches SET state='committed',actual_batch_id=expected_batch_id,actual_journal_locator=expected_journal_locator,commit_receipt='{\"synthetic\":true}' WHERE apply_id=$1::uuid")
                .bind(&apply).execute(store.pool()).await.expect("synthetic committed recovery record");
        }
        let before = store.calls();
        assert_eq!(store.context_scopes().await, Err(Error::ContextPending));
        assert_eq!(
            store.calls() - before,
            4,
            "one checkout/lock/check/rollback, no body query"
        );
        assert_eq!(
            store.search_context(&scopes[0], "", None, 20).await,
            Err(Error::ContextPending)
        );
        for path in ["note.md", "absent.md", "journal/private.md"] {
            assert_eq!(
                store.read_context(&scopes[0], path, false).await,
                Err(Error::ContextPending)
            );
            assert_eq!(
                store.read_context(&scopes[0], path, true).await,
                Err(Error::ContextPending)
            );
            assert!(matches!(
                store.read_context_material(&scopes[0], path).await,
                Err(Error::ContextPending)
            ));
            assert!(matches!(
                store.download_context(&scopes[0], path).await,
                Err(Error::ContextPending)
            ));
            assert_eq!(
                store
                    .export_context(&scopes[0], &[path.into()], &root.join("export"), true)
                    .await,
                Err(Error::ContextPending)
            );
        }
        assert!(!root.join("export").exists());
        for input in [&root, &root.join("unreadable-source")] {
            assert_eq!(
                store.verify_context(input, &scopes).await,
                Err(Error::ContextPending)
            );
            assert_eq!(
                store
                    .import_context(input, &scopes, &manifest.inventory_digest)
                    .await,
                Err(Error::ContextPending)
            );
        }
        assert!(
            inventory(&root, &scopes).is_ok(),
            "offline inventory is independent"
        );
        for body in [
            json!({"op":"search","scope":"personal"}),
            json!({"op":"read","scope":"personal","path":"absent.md"}),
            json!({"op":"verify","root":root,"scopes":["personal"]}),
            json!({"op":"import","root":root,"scopes":["personal"],"inventory_digest":manifest.inventory_digest}),
            json!({"op":"export","scope":"personal","paths":["note.md"],"destination":root.join("export")}),
        ] {
            let result = cli(&body.to_string(), true).await;
            assert!(!result.status.success());
            assert!(result.stdout.is_empty());
            assert!(
                String::from_utf8_lossy(&result.stderr)
                    .contains(&Error::ContextPending.to_string())
            );
        }
    }
    finish_synthetic_apply(&store, &apply).await;
    store
        .verify_context(&root, &scopes)
        .await
        .expect("finalized opens reads");
    let apply = synthetic_apply(&store).await;
    sqlx::query("UPDATE context_apply_batches SET state='aborted' WHERE apply_id=$1::uuid")
        .bind(&apply)
        .execute(store.pool())
        .await
        .expect("synthetic aborted preparation");
    assert_eq!(
        store
            .read_context(&scopes[0], "note.md", false)
            .await
            .expect("aborted opens reads"),
        "original"
    );
}

#[tokio::test]
async fn actual_connection_gates_serialize_readers_writers_and_pending_insertion() {
    use std::time::Duration;
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let (_temp, root) = source();
    write(&root, "personal/note.md", b"original");
    let scopes = [scope("personal")];
    let manifest = inventory(&root, &scopes).expect("synthetic original");
    store
        .import_context(&root, &scopes, &manifest.inventory_digest)
        .await
        .expect("fixture import");
    // With four of five connections held, each read must use its one gate connection.
    let mut held = Vec::new();
    for _ in 0..4 {
        held.push(store.pool().acquire().await.expect("reserve pool slot"));
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        store
            .context_scopes()
            .await
            .expect("same connection discovery");
        store
            .search_context(&scopes[0], "", None, 20)
            .await
            .expect("same connection search");
        store
            .read_context(&scopes[0], "note.md", false)
            .await
            .expect("same connection bytes");
        store
            .verify_context(&root, &scopes)
            .await
            .expect("same connection verify");
        store
            .export_context(&scopes[0], &["note.md".into()], &root.join("export"), false)
            .await
            .expect("same connection export");
        store
            .import_context(&root, &scopes, &manifest.inventory_digest)
            .await
            .expect("same connection import");
    })
    .await
    .expect("no second pool checkout at any current read");
    drop(held);
    let mut reader = store.pool().begin().await.expect("actual shared reader");
    sqlx::query("SELECT pg_advisory_xact_lock_shared(478310003)")
        .execute(&mut *reader)
        .await
        .expect("hold shared gate");
    store
        .context_scopes()
        .await
        .expect("concurrent reader allowed");
    assert_eq!(
        store
            .import_context(&root, &scopes, &manifest.inventory_digest)
            .await,
        Err(Error::ContextPending)
    );
    let insertion = sqlx::query("WITH i AS (SELECT gen_random_uuid() AS id) INSERT INTO context_apply_batches(apply_id,store_id,core_run_id,prepared_run_digest,candidate_digest,expected_source_versions,context_targets,core_apply_attempt_id,expected_batch_id,expected_journal_locator,state) SELECT i.id,s.store_id,'synthetic-race',repeat('a',64),repeat('b',64),'{}','[{}]',i.id::text,i.id::text,i.id::text,'pending' FROM i CROSS JOIN context_store s")
        .execute(store.pool()).await;
    assert!(
        insertion.is_err(),
        "pending insertion cannot pass a current reader"
    );
    let mut writer = store.pool().begin().await.expect("actual native writer");
    let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(478310003)")
        .fetch_one(&mut *writer)
        .await
        .expect("exclusive attempt");
    assert!(!acquired);
    reader.commit().await.expect("reader releases actual gate");
    let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(478310003)")
        .fetch_one(&mut *writer)
        .await
        .expect("exclusive retry");
    assert!(acquired);
    let before = store.calls();
    assert_eq!(store.context_scopes().await, Err(Error::ContextPending));
    assert_eq!(
        store.calls() - before,
        3,
        "contention ends before the pending query"
    );
    assert_eq!(
        store.verify_context(&root.join("missing"), &scopes).await,
        Err(Error::ContextPending)
    );
    writer.commit().await.expect("native gate released");
    let apply = synthetic_apply(&store).await;
    assert_eq!(store.context_scopes().await, Err(Error::ContextPending));
    finish_synthetic_apply(&store, &apply).await;
    assert_eq!(store.context_scopes().await.expect("resolved read"), scopes);
}

#[tokio::test]
async fn current_context_call_budget_is_constant_for_one_and_two_thousand_materials() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    for size in [1i64, 2000] {
        sqlx::query("TRUNCATE context_source_bindings,context_projection_versions, context_material_versions,context_manual_edits,context_materials,context_apply_batches")
            .execute(store.pool())
            .await
            .expect("complete synthetic reset");
        sqlx::query("INSERT INTO context_materials(scope,path,source_root,source_path,source_digest,content_digest,content,byte_len,restricted,search_text) SELECT 'personal',lpad(n::text,4,'0')||'.md','/synthetic','personal/'||lpad(n::text,4,'0')||'.md',encode(sha256(convert_to(repeat('marker ',512),'UTF8')),'hex'),encode(sha256(convert_to(repeat('marker ',512),'UTF8')),'hex'),convert_to(repeat('marker ',512),'UTF8'),3584,false,repeat('marker ',512) FROM generate_series(1,$1) n")
            .bind(size).execute(store.pool()).await.expect("batched synthetic canonical materials");
        let before = store.calls();
        assert_eq!(
            store.context_scopes().await.expect("bounded discovery"),
            vec![scope("personal")]
        );
        assert_eq!(store.calls() - before, 5);
        let before = store.calls();
        let page = store
            .search_context(&scope("personal"), "marker", None, 20)
            .await
            .expect("bounded metadata");
        assert_eq!(
            store.calls() - before,
            5,
            "one shared transaction and one page SQL independent of rows"
        );
        assert_eq!(
            page["items"].as_array().expect("items").len(),
            (size as usize).min(20)
        );
        for item in page["items"].as_array().expect("items") {
            assert!(item.get("content").is_none());
            assert!(item.get("search_text").is_none());
        }
        assert!(
            serde_json::to_vec(&page).expect("bounded response").len()
                <= 512 + 768 * (size as usize).min(20)
        );
        let before = store.calls();
        assert_eq!(
            store
                .download_context(&scope("personal"), "0001.md")
                .await
                .expect("one selected body")
                .bytes
                .len(),
            3584
        );
        assert_eq!(store.calls() - before, 5);
    }
}

#[tokio::test]
async fn read_holds_its_shared_gate_until_the_actual_current_query_finishes() {
    use std::time::Duration;
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let mut blocker = store
        .pool()
        .begin()
        .await
        .expect("owned synthetic table blocker");
    sqlx::query("LOCK TABLE context_materials IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await
        .expect("pause current query only");
    let reader = store.clone();
    let task = tokio::spawn(async move { reader.context_scopes().await });
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_locks l JOIN pg_stat_activity a USING(pid) WHERE l.locktype='advisory' AND l.classid=0 AND l.objid=478310003 AND l.mode='ShareLock' AND l.granted AND a.wait_event_type='Lock' AND a.query LIKE '%SELECT DISTINCT scope%')")
                .fetch_one(store.pool()).await.expect("observe actual query backend and shared gate");
            if active { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("current query waits on the same backend that holds the shared gate");
    let insertion = sqlx::query("WITH i AS (SELECT gen_random_uuid() AS id) INSERT INTO context_apply_batches(apply_id,store_id,core_run_id,prepared_run_digest,candidate_digest,expected_source_versions,context_targets,core_apply_attempt_id,expected_batch_id,expected_journal_locator,state) SELECT i.id,s.store_id,'synthetic-race',repeat('a',64),repeat('b',64),'{}','[{}]',i.id::text,i.id::text,i.id::text,'pending' FROM i CROSS JOIN context_store s")
        .execute(store.pool()).await;
    assert!(
        insertion.is_err(),
        "actual read prevents pending publication until its SQL ends"
    );
    blocker
        .commit()
        .await
        .expect("release synthetic query blocker");
    assert!(
        task.await
            .expect("reader task")
            .expect("unblocked read")
            .is_empty()
    );
    let apply = synthetic_apply(&store).await;
    assert_eq!(store.context_scopes().await, Err(Error::ContextPending));
    finish_synthetic_apply(&store, &apply).await;
}

#[tokio::test]
async fn storage_rejects_invalid_apply_shapes_and_unbound_native_origins() {
    let _guard = TEST_LOCK.lock().await;
    let store = store().await;
    let insert_native = "INSERT INTO context_materials(scope,path,source_root,source_path,source_digest,imported_at,origin_kind,content,content_digest,byte_len,restricted,search_text,last_apply_id) VALUES('personal','native.md',NULL,'personal/native.md',NULL,NULL,'native',''::bytea,encode(sha256(''::bytea),'hex'),0,false,'',$1::uuid)";
    assert!(
        sqlx::query(insert_native)
            .bind(None::<String>)
            .execute(store.pool())
            .await
            .is_err(),
        "native insert needs a pending apply"
    );
    let apply = synthetic_apply(&store).await;
    for assignment in [
        "state='accepted'",
        "state='committed'",
        "state='finalized'",
        "commit_receipt='{}'",
        "final_core_receipt_digest=repeat('c',64)",
        "context_targets='{}'",
        "context_targets='[]'",
        "expected_source_versions='[]'",
        "prepared_run_digest='invalid'",
        "candidate_digest='invalid'",
        "actual_batch_id='different'",
        "actual_journal_locator='different'",
        "core_apply_attempt_id=''",
        "expected_batch_id=''",
        "expected_journal_locator=''",
    ] {
        let mut query =
            sqlx::QueryBuilder::<sqlx::Postgres>::new("UPDATE context_apply_batches SET ");
        query
            .push(assignment)
            .push(" WHERE apply_id=")
            .push_bind(&apply)
            .push("::uuid");
        assert!(
            query.build().execute(store.pool()).await.is_err(),
            "invalid persistence shape: {assignment}"
        );
    }
    let second = sqlx::query("WITH i AS (SELECT gen_random_uuid() AS id) INSERT INTO context_apply_batches(apply_id,store_id,core_run_id,prepared_run_digest,candidate_digest,expected_source_versions,context_targets,core_apply_attempt_id,expected_batch_id,expected_journal_locator,state) SELECT i.id,s.store_id,'synthetic-second',repeat('a',64),repeat('b',64),'{}','[{}]',i.id::text,i.id::text,i.id::text,'pending' FROM i CROSS JOIN context_store s")
        .execute(store.pool()).await;
    assert!(second.is_err(), "only one unresolved apply per store");
    sqlx::query(insert_native)
        .bind(&apply)
        .execute(store.pool())
        .await
        .expect("native pending identity");
    assert!(
        sqlx::query("UPDATE context_materials SET last_apply_id=$1::uuid WHERE path='native.md'")
            .bind(&apply)
            .execute(store.pool())
            .await
            .is_err(),
        "same apply cannot reuse a material revision"
    );
    finish_synthetic_apply(&store, &apply).await;
    assert!(sqlx::query("INSERT INTO context_materials(scope,path,source_root,source_path,source_digest,imported_at,origin_kind,content,content_digest,byte_len,restricted,search_text,last_apply_id) VALUES('personal','other.md',NULL,'personal/other.md',NULL,NULL,'native',''::bytea,encode(sha256(''::bytea),'hex'),0,false,'',$1::uuid)")
        .bind(&apply).execute(store.pool()).await.is_err(), "terminal apply cannot create native material");
    assert!(sqlx::query("UPDATE context_materials SET content='x'::bytea,content_digest=encode(sha256('x'::bytea),'hex'),byte_len=1 WHERE path='native.md'").execute(store.pool()).await.is_err(), "raw update lacks a new pending apply");
}
