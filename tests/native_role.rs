//! Optional transport exercises the real native CLI with owned synthetic processes.
#[allow(
    dead_code,
    reason = "The shared integration fixtures support both role and commit suites"
)]
#[path = "native_harness.rs"]
mod fixtures;
use ontology::store::Store;
use std::{fs, os::unix::fs::PermissionsExt, path::Path};
#[tokio::test]
async fn native_role_rejects_writer_frontier_before_launch() {
    let url = std::env::var("TEST_DATABASE_URL").expect("owned test database");
    assert!(
        ontology::config::database_options(&url)
            .expect("local URL")
            .get_database()
            .is_some_and(|name| name.starts_with("ontology_test_"))
    );
    let store = Store::connect(&url).await.expect("owned PG");
    store.initialize().await.expect("migrations");
    sqlx::query("TRUNCATE context_source_bindings,context_projection_versions,context_material_versions,context_manual_edits,context_materials,context_apply_batches").execute(store.pool()).await.expect("isolated reset");
    let policy = fixtures::seed(&store).await;
    let view = fixtures::view();
    let root = view.path().canonicalize().expect("view");
    let workspace = tempfile::tempdir().expect("workspace");
    let workspace = workspace.path().canonicalize().expect("external workspace");
    fs::write(workspace.join("document.md"), "# Synthetic document").expect("target");
    let request = fixtures::envelope(&policy, vec!["document.md".into()], vec![], false);
    let (artifacts, _) = fixtures::cli_prepared(&store, &root, &workspace, request).await;
    let binary = artifacts.path().join("fake-codex");
    let marker = artifacts.path().join("launched");
    fs::write(
        &binary,
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    )
    .expect("synthetic executable");
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).expect("mode");
    let output = fixtures::command(
        &store,
        &root,
        &workspace,
        "begin",
        &[
            "--prepared-run".into(),
            artifacts.path().join("prepared.json").display().to_string(),
            "--run-id".into(),
            "native-unsupported-writer".into(),
            "--codex-binary".into(),
            binary.display().to_string(),
        ],
    )
    .await;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported runtime"));
    assert!(!Path::new(&marker).exists());
}
