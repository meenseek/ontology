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
async fn native_role_rejects_writer_and_executes_independent_review_frontier() {
    let url = std::env::var("TEST_DATABASE_URL").expect("owned test database");
    assert!(
        ontology::config::database_options(&url)
            .expect("local URL")
            .get_database()
            .is_some_and(|name| name.starts_with("ontology_test_"))
    );
    let store = Store::connect(&url).await.expect("owned PG");
    store.initialize().await.expect("migrations");
    sqlx::query("TRUNCATE document_grouping,document_subjects,context_source_bindings,context_projection_versions,context_material_versions,context_manual_edits,context_materials,context_apply_batches").execute(store.pool()).await.expect("isolated reset");
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
    verify_independent_cli_frontier(&store, &root, &workspace, &policy).await;
}

// The real CLI must issue both independent processes; the start barrier fails
// when either role is executed sequentially, even if both outcomes are valid.
async fn verify_independent_cli_frontier(
    store: &Store,
    root: &Path,
    workspace: &Path,
    policy: &str,
) {
    use context_core::harness::*;
    let mut records = Vec::new();
    let statement = "Independently review the exact synthetic document.".to_owned();
    let request = RequestEnvelope {
        version: HARNESS_SCHEMA_VERSION,
        source: RequestSource::UserLanguage {
            statements: vec![UserStatement {
                identifier: "statement-0000000000000001".into(),
                text: statement.clone(),
            }],
        },
        draft: DraftTaskRequest::Review(DraftReviewContract {
            common: DraftTaskCommon {
                owner: fixtures::value(DataOwner::Personal, 1, &mut records),
                intent: fixtures::default(
                    HarnessIntent::General,
                    2,
                    PolicyDefaultRule::GeneralIntent,
                    policy,
                    &mut records,
                ),
                task_statement: fixtures::value(statement, 3, &mut records),
                execution_profile: fixtures::default(
                    HarnessExecutionProfile::Standard,
                    4,
                    PolicyDefaultRule::StandardExecutionProfile,
                    policy,
                    &mut records,
                ),
                context_grants: None,
                evidence_source_paths: None,
            },
            review_kind: fixtures::value(ReviewKind::Document, 5, &mut records),
            targets: fixtures::value(vec!["document.md".into()], 6, &mut records),
        }),
        decision_trace: DecisionTrace { records },
    };
    let (artifacts, prepared) = fixtures::cli_prepared(store, root, workspace, request).await;
    assert!(prepared.role_run.runtime_capabilities.max_concurrent_roles >= 2);
    let owned_root = root.to_path_buf();
    let owned_workspace = workspace.to_path_buf();
    let preview = prepared.clone();
    let invocations = store
        .with_native_context(owned_root, move |source| {
            let engine =
                HarnessEngine::with_source(source.view_root(), &owned_workspace, source.clone())
                    .expect("owned native engine");
            Ok(engine
                .begin_execution(&preview.plan.resolved_request, &preview.role_run, &[])
                .expect("Core-issued preview")
                .ready_role_invocations)
        })
        .await
        .expect("owned read-only source gate");
    assert_eq!(invocations.len(), 2);
    let binary = artifacts.path().join("fake-concurrent-codex");
    fs::write(&binary, r#"#!/bin/sh
set -eu
cat > "$0.stdin.$$"
role=$(python3 -c 'import json,sys; print(json.loads(json.load(open(sys.argv[1]))[0]["content"])["role"])' "$0.stdin.$$")
printf '%s\n' "$$" > "$0.pid.$role"
printf '%s\n' "$PWD" > "$0.cwd.$role"
printf '{"type":"thread.started","thread_id":"cli-%s"}\n' "$$"
i=0
while [ ! -f "$0.pid.verifier" ] || [ ! -f "$0.pid.reviewer" ]; do
  i=$((i + 1)); [ "$i" -lt 300 ] || exit 9
  sleep 0.01
done
if [ "$role" = reviewer ]; then sleep 0.04; else sleep 0.3; fi
cat "$0.events.$role"
"#).expect("owned transport");
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).expect("mode");
    for invocation in &invocations {
        let role = if invocation.role == HarnessRole::Reviewer {
            "reviewer"
        } else {
            "verifier"
        };
        let message =
            serde_json::json!({"outcome": fixtures::outcome(&prepared, invocation, "unused")})
                .to_string();
        let events = [
            serde_json::json!({"type":"turn.started"}),
            serde_json::json!({"type":"item.completed","item":{"id":"message","type":"agent_message","text":message}}),
            serde_json::json!({"type":"turn.completed","usage":{"input_tokens":1,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0}}),
        ];
        let bytes = events.iter().map(|e| format!("{e}\n")).collect::<String>();
        fs::write(format!("{}.events.{role}", binary.display()), bytes)
            .expect("exact Core outcome");
    }
    let output = fixtures::command(
        store,
        root,
        workspace,
        "begin",
        &[
            "--prepared-run".into(),
            artifacts.path().join("prepared.json").display().to_string(),
            "--run-id".into(),
            "native-independent-cli".into(),
            "--codex-binary".into(),
            binary.display().to_string(),
        ],
    )
    .await;
    assert!(
        output.status.success(),
        "native begin: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let head: HarnessExecutionRecord =
        decode_current_json(&output.stdout, "native concurrent head").expect("strict head");
    assert_eq!(head.state, HarnessExecutionState::Evaluated);
    assert_eq!(
        head.role_execution.accepted_role_order,
        [HarnessRole::Reviewer, HarnessRole::Verifier]
    );
    assert!(
        head.evaluation
            .as_ref()
            .expect("Core evaluation")
            .requirements
            .iter()
            .all(|r| r.passed)
    );
    let raw = fs::read(artifacts.path().join("prepared.json")).expect("exact prepared");
    head.validate(&prepared, &raw)
        .expect("Core validates exact issued results");
    let mut working_directories = Vec::new();
    for invocation in invocations {
        let role = if invocation.role == HarnessRole::Reviewer {
            "reviewer"
        } else {
            "verifier"
        };
        let pid = fs::read_to_string(format!("{}.pid.{role}", binary.display()))
            .expect("both roles started");
        let captured =
            fs::read(format!("{}.stdin.{}", binary.display(), pid.trim())).expect("exact stdin");
        assert_eq!(
            captured,
            serde_json::to_vec(&invocation.segments).expect("Core-issued segments")
        );
        let group: i32 = pid.trim().parse().expect("owned process group");
        // SAFETY: Signal zero only observes the positive group issued by this fixture.
        assert_eq!(unsafe { libc::kill(-group, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        let cwd = fs::read_to_string(format!("{}.cwd.{role}", binary.display())).expect("role cwd");
        assert!(
            !Path::new(cwd.trim()).exists(),
            "owned role cwd must be removed"
        );
        working_directories.push(cwd);
    }
    assert_ne!(working_directories[0], working_directories[1]);
    assert_eq!(
        fs::read_to_string(workspace.join("document.md")).expect("target"),
        "# Synthetic document"
    );
}
