//! Actual CLI configuration checks use only the owned isolated database.
#[allow(
    dead_code,
    reason = "shared CLI fixture supports other lifecycle suites"
)]
#[path = "native_harness.rs"]
mod fixtures;
use fixtures::{cli_prepared, context_fixture, envelope, seed, view};
use ontology::store::Store;
use std::fs;

#[tokio::test]
async fn composition_rejects_semantic_and_authority_errors_before_database_access() {
    let files = tempfile::tempdir().unwrap();
    for (variant, expected) in [
        ("unresolved", "clarification"),
        ("wrong-authority", "configured authority"),
        ("model-owner", "not authoritative"),
        ("unknown-owner-reference", "unknown composition reference"),
    ] {
        let mut request = serde_json::to_value(envelope(
            &"1".repeat(64),
            vec!["README.md".into()],
            vec![],
            false,
        ))
        .unwrap();
        for record in request["decision_trace"]["records"].as_array_mut().unwrap() {
            record["value_digest"] = "".into();
            if record["kind"] == "policy-default" {
                record["policy_content_digest"] = "".into();
            }
        }
        match variant {
            "unresolved" => {
                request["draft"]["common"]["owner"] =
                    serde_json::to_value(context_core::harness::DraftValue::<
                        context_core::harness::DataOwner,
                    >::Unresolved {
                        question: "Which owner did the user authorize?".into(),
                    })
                    .unwrap()
            }
            "wrong-authority" => {
                request["decision_trace"]["records"][1]["policy_identifier"] = "foundation".into()
            }
            "model-owner" => {
                let parent = request["decision_trace"]["records"][0]["identifier"].clone();
                request["decision_trace"]["records"][0]["value_digest"] = ontology::store::digest(
                    &serde_json::to_vec(&context_core::harness::DataOwner::Personal).unwrap(),
                )
                .into();
                request["decision_trace"]["records"].as_array_mut().unwrap().push(serde_json::json!({
                    "kind": "model-proposal", "identifier": "owner-proposal", "value_digest": "",
                    "based_on_record_identifiers": [parent]
                }));
                request["draft"]["common"]["owner"]["decision_identifier"] =
                    "owner-proposal".into();
            }
            "unknown-owner-reference" => {
                request["draft"]["common"]["owner"]["decision_identifier"] =
                    "missing-reference".into()
            }
            _ => unreachable!(),
        }
        let path = files.path().join(format!("{variant}.json"));
        fs::write(&path, serde_json::to_vec(&request).unwrap()).unwrap();
        let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_ontology"))
            .env_clear()
            .env("DATABASE_URL", "invalid://must-not-connect")
            .args([
                "harness",
                "resolve",
                "--compose-decisions",
                "--context-view",
                "/never-opened",
                "--store-id",
                "11111111-1111-4111-8111-111111111111",
                "--workspace-root",
                files.path().to_str().unwrap(),
                "--request-envelope",
                path.to_str().unwrap(),
                "--runtime-capabilities",
                "/never-opened",
                "--policy-config",
                context_fixture::configuration_path().to_str().unwrap(),
            ])
            .output()
            .await
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "{variant}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[tokio::test]
async fn native_harness_reloads_changed_configuration_before_replay_database_access() {
    let url = std::env::var("TEST_DATABASE_URL").expect("owned test DB");
    assert!(
        ontology::config::database_options(&url)
            .unwrap()
            .get_database()
            .is_some_and(|name| name.starts_with("ontology_test_"))
    );
    let store = Store::connect(&url).await.unwrap();
    store.initialize().await.unwrap();
    sqlx::query("TRUNCATE document_grouping,document_subjects,context_source_bindings,context_projection_versions,context_material_versions,context_manual_edits,context_materials,context_apply_batches").execute(store.pool()).await.unwrap();
    let sha = seed(&store).await;
    let source = view();
    let source_root = source.path().canonicalize().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let workspace_root = workspace.path().canonicalize().unwrap();
    fs::write(workspace.path().join("document.md"), "# Fixture\n").unwrap();
    let (artifacts, _) = cli_prepared(
        &store,
        &source_root,
        &workspace_root,
        envelope(&sha, vec!["document.md".into()], vec![], false),
    )
    .await;
    let config = artifacts.path().join("changed.json");
    let mut bytes = fs::read(context_fixture::configuration_path()).unwrap();
    bytes.push(b' ');
    fs::write(&config, bytes).unwrap();
    let id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
        .fetch_one(store.pool())
        .await
        .unwrap();
    for (verb, flag, file) in [
        ("prepare", "--plan", "plan.json"),
        ("replay", "--prepared-run", "prepared.json"),
    ] {
        let mut args = vec![
            "harness".to_owned(),
            verb.into(),
            "--context-view".into(),
            source_root.display().to_string(),
            "--workspace-root".into(),
            workspace_root.display().to_string(),
            "--store-id".into(),
            id.clone(),
            "--policy-config".into(),
            config.display().to_string(),
            flag.into(),
            artifacts.path().join(file).display().to_string(),
        ];
        if verb == "prepare" {
            args.extend([
                "--runtime-capabilities".into(),
                artifacts
                    .path()
                    .join("capabilities.json")
                    .display()
                    .to_string(),
                "--prepared-output".into(),
                artifacts
                    .path()
                    .join("never-created.json")
                    .display()
                    .to_string(),
            ]);
        }
        let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_ontology"))
            .env_clear()
            .env("DATABASE_URL", "invalid://must-not-connect")
            .args(args)
            .output()
            .await
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("resolved plan changed before validation"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!artifacts.path().join("never-created.json").exists());
    }
}

#[tokio::test]
async fn configured_binding_conflicts_are_rejected_before_database_access() {
    let files = tempfile::tempdir().unwrap();
    let request = files.path().join("request.json");
    fs::write(
        &request,
        serde_json::to_vec(&envelope(
            &"1".repeat(64),
            vec!["README.md".into()],
            vec![],
            false,
        ))
        .unwrap(),
    )
    .unwrap();
    let settings = || {
        serde_json::from_slice::<serde_json::Value>(
            &fs::read(context_fixture::configuration_path()).unwrap(),
        )
        .unwrap()
    };
    for variant in [
        "direct",
        "dependency",
        "alias",
        "flat-project",
        "index-project",
    ] {
        let mut config = settings();
        let expected = match variant {
            "direct" => {
                config["rules"][0]["documents"] = serde_json::json!(["control"]);
                "orchestrator policies"
            }
            "dependency" => {
                config["documents"][2]["dependencies"] = serde_json::json!(["control"]);
                "orchestrator policies"
            }
            "alias" => {
                config["documents"][2]["path"] = config["documents"][3]["path"].clone();
                "conflicting identifiers or paths"
            }
            "flat-project" | "index-project" => {
                let entrypoint = if variant == "flat-project" {
                    "sample.md"
                } else {
                    "sample/index.md"
                };
                config["documents"]
                    .as_array_mut()
                    .unwrap()
                    .push(serde_json::json!({
                        "key": "project-note", "id": "project-note",
                        "path": format!("vault/personal/projects/{entrypoint}"),
                        "dependencies": [], "project_entrypoint": false
                    }));
                config["rules"][0]["documents"]
                    .as_array_mut()
                    .unwrap()
                    .push("project-note".into());
                config["rules"][3]["owner_source"] = "primary".into();
                let mut input = envelope(&"1".repeat(64), vec!["README.md".into()], vec![], false);
                input.decision_trace.records.retain(|record| {
                    !matches!(record,
                    context_core::harness::DecisionRecord::UserStatement { identifier, .. }
                        if identifier == "decision-0000000000000001")
                });
                let context_core::harness::DraftTaskRequest::Write(ref mut draft) = input.draft
                else {
                    unreachable!()
                };
                draft.common.owner = fixtures::value(
                    context_core::harness::DataOwner::PersonalProject {
                        project: "sample".into(),
                    },
                    1,
                    &mut input.decision_trace.records,
                );
                fs::write(&request, serde_json::to_vec(&input).unwrap()).unwrap();
                "conflicting identifiers or paths"
            }
            _ => unreachable!(),
        };
        let config_path = files.path().join(format!("{variant}.json"));
        fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_ontology"))
            .env_clear()
            .env("DATABASE_URL", "invalid://must-not-connect")
            .args([
                "harness",
                "resolve",
                "--context-view",
                "/never-opened",
                "--store-id",
                "11111111-1111-4111-8111-111111111111",
                "--workspace-root",
                files.path().to_str().unwrap(),
                "--request-envelope",
                request.to_str().unwrap(),
                "--runtime-capabilities",
                "/never-opened",
                "--policy-config",
                config_path.to_str().unwrap(),
            ])
            .output()
            .await
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
