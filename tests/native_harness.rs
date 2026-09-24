//! CLI transfer ledger (49 original basenames, separate from the 399 Core library baseline):
//! The 33 Harness/transport cases retain their exact basenames in native_harness::tests.
//! exact_read_cli_writes_one_response_for_one_and_one_hundred_paths ->
//! native_read_documents_one_response_for_one_and_one_hundred_paths (accepted A proof).
//! exact_read_cli_has_no_partial_output_on_invalid_arguments_or_later_document_failure ->
//! native_read_documents_no_partial_output_on_invalid_or_later_failure (accepted A proof).
//! exact_read_cli_response_limit_failure_writes_nothing -> native_read_documents_response_limit_writes_nothing (accepted A proof).
//! Retired raw filesystem commands: init_command_creates_vault_directories,
//! save_command_creates_scoped_markdown, save_command_accepts_language_and_aliases,
//! save_command_accepts_export_false, delete_command_removes_scoped_markdown,
//! export_command_reads_scoped_markdown, save_without_body_does_not_create_file.
//! Their applicable data/serialization behavior is transferred to accepted Core writes,
//! native_commit_revision_and_scope_integrity, native_projection_search_semantics,
//! native_cli_exact_read_has_no_old_runtime, and the existing native export tests.
//! Retired SQLite freshness/CLI mechanisms: command_flow_saves_updates_searches_and_deletes_context,
//! ontology_command_refreshes_a_stale_source_projection, search_without_query_does_not_create_database,
//! search_without_scope_does_not_create_database, search_with_missing_database_does_not_create_database,
//! required_scope_rejects_unknown_scope. Native scope/input checks, explicit initialization,
//! current revisions and atomic projections retain the applicable capabilities.
//! This ledger declares the transfer destinations, not a claim that an unexecuted proof passed.
//! Real Core / PostgreSQL fixtures shared with native lifecycle proofs.
#[cfg(test)]
#[path = "context_fixture.rs"]
pub(crate) mod context_fixture;

use context_core::harness::*;
use ontology::{
    context::{ContextScope, inventory},
    store::{Store, digest},
};
use serde_json::Value;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
pub fn value<T: serde::Serialize>(
    value: T,
    n: u64,
    records: &mut Vec<DecisionRecord>,
) -> DraftValue<T> {
    let identifier = format!("decision-{n:016x}");
    records.push(DecisionRecord::UserStatement {
        identifier: identifier.clone(),
        value_digest: digest(&serde_json::to_vec(&value).expect("decision value")),
        statement_identifiers: vec!["statement-0000000000000001".into()],
    });
    DraftValue::Resolved {
        value,
        decision_identifier: identifier,
    }
}
pub fn default<T: serde::Serialize>(
    value: T,
    n: u64,
    rule: PolicyDefaultRule,
    sha: &str,
    records: &mut Vec<DecisionRecord>,
) -> DraftValue<T> {
    let identifier = format!("decision-{n:016x}");
    records.push(DecisionRecord::PolicyDefault {
        identifier: identifier.clone(),
        value_digest: digest(&serde_json::to_vec(&value).expect("default value")),
        policy_identifier: "agent-harness".into(),
        policy_content_digest: sha.into(),
        rule,
    });
    DraftValue::Resolved {
        value,
        decision_identifier: identifier,
    }
}
pub async fn seed(store: &Store) -> String {
    let fixture = context_fixture::build();
    let root = fixture
        .path()
        .canonicalize()
        .expect("synthetic fixture root");
    let scopes: Vec<ContextScope> = context_fixture::SCOPES
        .iter()
        .map(|s| s.parse().expect("declared synthetic scope"))
        .collect();
    let inventory = inventory(&root, &scopes).expect("synthetic fixture inventory");
    assert_eq!(inventory.entries.len(), context_fixture::DOCUMENT_COUNT);
    assert_eq!(inventory.total_bytes, context_fixture::TOTAL_BYTES);
    let mut bindings = String::new();
    for (path, expected) in context_fixture::documents() {
        let bytes = fs::read(root.join(path)).expect("declared synthetic fixture file");
        assert_eq!(bytes, expected.as_bytes(), "declared bytes for {path}");
        bindings.push_str(&format!("{path}\t{}\t{}\n", bytes.len(), digest(&bytes)));
    }
    assert_eq!(
        digest(bindings.as_bytes()),
        context_fixture::CONTRACT_DIGEST,
        "declared fixture paths, lengths, and content digests"
    );
    store
        .import_context(&root, &scopes, &inventory.inventory_digest)
        .await
        .expect("isolated fixture import");
    sqlx::query_scalar("SELECT content_digest FROM context_materials WHERE scope='profile' AND path='rules/agent-harness.md'").fetch_one(store.pool()).await.expect("actual policy metadata")
}
pub fn view() -> tempfile::TempDir {
    let view = tempfile::tempdir().expect("view");
    fs::set_permissions(view.path(), fs::Permissions::from_mode(0o700)).expect("private view");
    view
}
pub fn envelope(
    sha: &str,
    targets: Vec<String>,
    deletes: Vec<String>,
    curation: bool,
) -> RequestEnvelope {
    let mut records = Vec::new();
    let statement="Write the exact synthetic context candidate and independently verify the resulting changes.".to_owned();
    let draft = if curation {
        DraftTaskRequest::Curation(Box::new(DraftCurationContract {
            common: DraftOwnedTaskCommon {
                owner: value(DataOwner::Personal, 1, &mut records),
                task_statement: value(statement.clone(), 2, &mut records),
                execution_profile: default(
                    HarnessExecutionProfile::Standard,
                    3,
                    PolicyDefaultRule::StandardExecutionProfile,
                    sha,
                    &mut records,
                ),
            },
            curation_kind: value(CurationKind::Knowledge, 4, &mut records),
            target: value(targets[0].clone(), 5, &mut records),
            curation_sources: Some(value(
                vec!["vault/personal/index.md".to_owned()],
                8,
                &mut records,
            )),
            delete_target: deletes.first().map(|p| value(p.clone(), 6, &mut records)),
            promotion_handoff: None,
            confirmation: value(UserConfirmationStatus::Confirmed, 7, &mut records),
        }))
    } else {
        DraftTaskRequest::Write(DraftWriteContract {
            common: DraftTaskCommon {
                owner: value(DataOwner::Personal, 1, &mut records),
                intent: default(
                    HarnessIntent::General,
                    2,
                    PolicyDefaultRule::GeneralIntent,
                    sha,
                    &mut records,
                ),
                task_statement: value(statement.clone(), 3, &mut records),
                execution_profile: default(
                    HarnessExecutionProfile::Standard,
                    4,
                    PolicyDefaultRule::StandardExecutionProfile,
                    sha,
                    &mut records,
                ),
                context_grants: None,
                evidence_source_paths: None,
            },
            write_kind: value(WriteKind::Document, 5, &mut records),
            targets: value(targets, 6, &mut records),
            delete_targets: (!deletes.is_empty()).then(|| value(deletes, 7, &mut records)),
        })
    };
    RequestEnvelope {
        version: HARNESS_SCHEMA_VERSION,
        source: RequestSource::UserLanguage {
            statements: vec![UserStatement {
                identifier: "statement-0000000000000001".into(),
                text: statement,
            }],
        },
        draft,
        decision_trace: DecisionTrace { records },
    }
}
pub fn capabilities() -> HarnessRuntimeCapabilities {
    HarnessRuntimeCapabilities {
        version: HARNESS_SCHEMA_VERSION,
        available_roles: vec![
            HarnessRole::Writer,
            HarnessRole::Reviewer,
            HarnessRole::Verifier,
            HarnessRole::Specialist,
        ],
        max_concurrent_roles: 4,
        separate_contexts: true,
        file_reading: true,
        tool_execution: true,
        deterministic_validation: true,
        max_role_bundle_bytes: 8 * 1024 * 1024,
        max_role_invocation_bytes: 16 * 1024 * 1024,
        lifecycle: RoleLifecycleLimits {
            max_role_execution_millis: 60000,
            max_role_grace_millis: 1000,
            max_role_close_millis: 1000,
            max_total_role_millis: 62000,
        },
    }
}
pub fn outcome(
    prepared: &PreparedHarnessRun,
    invocation: &RoleInvocationContract,
    content: &str,
) -> RoleExecutionOutcome {
    let artifact_evidence = match &invocation.subject {
        EvaluationSubject::ProducedArtifact { artifact_digest } => {
            vec![ResultEvidenceReference::ProducedArtifact {
                artifact_digest: artifact_digest.clone(),
                locator: "entire exact candidate".into(),
            }]
        }
        _ => prepared
            .plan
            .resolved_request
            .plan
            .targets
            .iter()
            .filter_map(|target| match &target.state {
                TargetState::Existing { content_digest } => Some(ResultEvidenceReference::Target {
                    workspace_relative_path: target.workspace_relative_path.clone(),
                    content_digest: content_digest.clone(),
                    locator: "entire target".into(),
                }),
                _ => None,
            })
            .collect(),
    };
    let metadata = prepared
        .role_run
        .role_metadata
        .iter()
        .find(|m| m.role == invocation.role)
        .expect("issued role metadata");
    let mut result = match &metadata.task {
        RoleTaskContract::Writer { .. } => CompletedRoleResult::Writer {
            artifact: WriterArtifact::Changes {
                changes: prepared
                    .plan
                    .resolved_request
                    .plan
                    .targets
                    .iter()
                    .map(|target| match &target.state {
                        TargetState::Existing { content_digest } => {
                            if target.operation == TargetOperation::Delete {
                                FileChange::Delete {
                                    path: target.workspace_relative_path.clone(),
                                    expected_content_digest: content_digest.clone(),
                                }
                            } else {
                                FileChange::Update {
                                    path: target.workspace_relative_path.clone(),
                                    expected_content_digest: content_digest.clone(),
                                    content: content.into(),
                                }
                            }
                        }
                        _ => FileChange::Create {
                            path: target.workspace_relative_path.clone(),
                            content: content.into(),
                        },
                    })
                    .collect(),
            },
        },
        RoleTaskContract::Reviewer {
            verification_requirements,
            ..
        } => CompletedRoleResult::Reviewer {
            summary: "No Findings in synthetic fixture".into(),
            subject_evidence: artifact_evidence.clone(),
            requirement_results: verification_requirements
                .iter()
                .map(|r| RequirementResult {
                    unit: r.unit,
                    passed: true,
                    detail: "Verified synthetic candidate".into(),
                    evidence: artifact_evidence.clone(),
                })
                .collect(),
            blocking_findings: vec![],
            improvements: vec![],
            learning_candidates: vec![],
        },
        RoleTaskContract::Verifier {
            verification_requirements,
            ..
        } => CompletedRoleResult::Verifier {
            subject_evidence: artifact_evidence.clone(),
            requirement_results: verification_requirements
                .iter()
                .map(|r| RequirementResult {
                    unit: r.unit,
                    passed: true,
                    detail: "Verified synthetic candidate".into(),
                    evidence: artifact_evidence.clone(),
                })
                .collect(),
        },
        RoleTaskContract::Specialist {
            source_targets,
            verification_requirements,
        } => {
            let artifact = SpecialistArtifact {
                source_targets: source_targets
                    .iter()
                    .map(|target| target.workspace_relative_path.clone())
                    .collect(),
                context_bundle_digest: metadata.scope.context_bundle_digest.clone(),
                output: content.to_owned(),
            };
            let evidence = vec![ResultEvidenceReference::ProducedArtifact {
                artifact_digest: digest(
                    &serde_json::to_vec(&artifact).expect("Specialist artifact"),
                ),
                locator: "entire synthetic analysis".into(),
            }];
            let target = &source_targets[0];
            let TargetState::Existing { content_digest } = &target.state else {
                panic!("actual investigation source")
            };
            CompletedRoleResult::Specialist {
                artifact,
                requirement_results: verification_requirements
                    .iter()
                    .map(|r| RequirementResult {
                        unit: r.unit,
                        passed: true,
                        detail: "Verified synthetic source analysis".into(),
                        evidence: evidence.clone(),
                    })
                    .collect(),
                promotion_proposals: vec![PromotionProposal {
                    identifier: "proposal-0000000000000003".into(),
                    owner: DataOwner::Personal,
                    curation_kind: CurationKind::Knowledge,
                    title: "Native promotion".into(),
                    content:
                        "# Native promotion\nnativepromotionsearchtoken verified synthetic source"
                            .into(),
                    origin: PromotionProposalOrigin::SpecialistMemory {
                        claim_kind: PromotionClaimKind::ObservedFact,
                        evidence: vec![PromotionEvidenceReference {
                            source_kind: PromotionSourceKind::Target,
                            source_owner: DataOwner::Personal,
                            relative_path: target.workspace_relative_path.clone(),
                            content_digest: content_digest.clone(),
                            locator: "entire synthetic source".into(),
                        }],
                    },
                }],
            }
        }
    };
    if let CompletedRoleResult::Writer { artifact } = &mut result
        && prepared.plan.resolved_request.plan.action == HarnessAction::VaultCuration
    {
        let WriterArtifact::Changes { changes } = artifact else {
            panic!("fixture changes")
        };
        *artifact = WriterArtifact::Curation {
            curation_kind: prepared
                .plan
                .resolved_request
                .plan
                .curation_kind
                .expect("curation kind"),
            entries: changes
                .iter()
                .cloned()
                .map(|change| CurationEntry {
                    change,
                    provenance: vec!["confirmed synthetic user request".into()],
                    source_references: prepared.plan.resolved_request.plan.curation_sources.clone(),
                    promotion_handoff_digest: prepared
                        .plan
                        .resolved_request
                        .plan
                        .promotion_handoff
                        .as_ref()
                        .map(|h| h.handoff_digest.clone()),
                })
                .collect(),
        };
    }
    RoleExecutionOutcome::Completed { result }
}
pub fn role_event(
    prepared: &PreparedHarnessRun,
    invocation: &RoleInvocationContract,
    content: &str,
) -> HarnessExecutionEvent {
    let lifecycle = ReportedRoleLifecycle {
        role: invocation.role,
        context_id: format!(
            "fixture-{:?}-{}",
            invocation.role, invocation.invocation_digest
        ),
        started_at_millis: 1,
        context_ready_at_millis: 2,
        first_output_at_millis: Some(3),
        interrupt_requested_at_millis: None,
        grace_deadline_at_millis: None,
        terminal_at_millis: 4,
        closed_at_millis: 5,
        terminal_state: RoleTerminalState::Completed,
    };
    HarnessExecutionEvent::RoleResult {
        result: Box::new(
            invocation
                .bind_result(lifecycle, outcome(prepared, invocation, content))
                .expect("exact issued result binding"),
        ),
    }
}
pub async fn prepare(
    store: &Store,
    view: PathBuf,
    workspace: PathBuf,
    envelope: RequestEnvelope,
    run: String,
    content: String,
) -> PreparedHarnessRun {
    prepare_with_heads(store, view, workspace, envelope, run, content)
        .await
        .0
}
pub async fn prepare_with_heads(
    store: &Store,
    view: PathBuf,
    workspace: PathBuf,
    envelope: RequestEnvelope,
    run: String,
    content: String,
) -> (PreparedHarnessRun, Vec<HarnessExecutionRecord>) {
    store
        .with_native_context(view, move |source| {
            let engine = HarnessEngine::with_source(source.view_root(), &workspace, source.clone())
                .expect("native engine");
            let resolved = engine
                .resolve_envelope(&envelope, None)
                .expect("actual Core resolve");
            let capabilities = capabilities();
            let plan =
                HarnessPlan::from_resolved(resolved, &workspace, capabilities.lifecycle.clone())
                    .expect("actual Core plan");
            let raw = serde_json::to_vec(&plan).expect("exact plan");
            let prepared =
                PreparedHarnessRun::prepare(&engine, &raw, plan, capabilities, None, None)
                    .expect("actual Core prepare");
            let raw = serde_json::to_vec(&prepared).expect("exact prepared");
            let mut head = HarnessExecutionRecord::begin_durable(&engine, &raw, &prepared, &run)
                .expect("actual Core begin");
            let mut heads = vec![head.clone()];
            while let Some(invocation) = head.ready_role_invocations.first() {
                head = HarnessExecutionRecord::advance_durable(
                    &engine,
                    &run,
                    role_event(&prepared, invocation, &content),
                )
                .expect("Core accepts only bound fixture role");
                heads.push(head.clone());
            }
            assert!(head.ready_tool_invocation.is_none());
            HarnessExecutionRecord::evaluate_durable(&engine, &run, None).expect("Core evaluation");
            HarnessExecutionRecord::validate_durable(&engine, &run).expect("Core validation");
            Ok((prepared, heads))
        })
        .await
        .expect("prepare native run")
}
pub async fn apply(
    store: &Store,
    view: PathBuf,
    workspace: PathBuf,
    run: String,
    prepared: PreparedHarnessRun,
) -> (HarnessExecutionRecord, HarnessApplyAttemptReceipt) {
    let id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
        .fetch_one(store.pool())
        .await
        .expect("store identity");
    store
        .with_native_commit(view, id, move |session| {
            let source = session.fresh_source().expect("fresh gate");
            let engine = HarnessEngine::with_source(source.view_root(), &workspace, source.clone())
                .expect("native apply engine");
            let result = HarnessExecutionRecord::apply_durable(&engine, &run, Some(session))
                .expect("real PG commit and Core finalize");
            result.1.validate(&prepared).expect("real terminal attempt");
            Ok(result)
        })
        .await
        .expect("exclusive native session")
}
pub async fn command(
    store: &Store,
    view: &Path,
    workspace: &Path,
    verb: &str,
    extra: &[String],
) -> std::process::Output {
    let id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
        .fetch_one(store.pool())
        .await
        .expect("store identity");
    tokio::process::Command::new(env!("CARGO_BIN_EXE_ontology"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env(
            "DATABASE_URL",
            std::env::var("TEST_DATABASE_URL").expect("owned DB"),
        )
        .args(["harness", verb, "--context-view"])
        .arg(view)
        .arg("--workspace-root")
        .arg(workspace)
        .args(["--store-id", &id])
        .args(extra)
        .output()
        .await
        .expect("actual native CLI")
}
#[tokio::test]
async fn native_harness_rejects_removed_and_duplicate_options_without_initializing() {
    for verb in [
        "resolve",
        "prepare",
        "replay",
        "begin",
        "advance",
        "revise",
        "evaluate",
        "validate",
        "apply",
        "recover",
        "attest-career",
        "compose-career",
    ] {
        for flag in ["--vault-root", "--index-database", "--unknown"] {
            let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_ontology"))
                .env_clear()
                .args(["harness", verb, flag, "/never-opened"])
                .output()
                .await
                .expect("strict parser process");
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
            let error: Value =
                serde_json::from_slice(&output.stderr).expect("single structured error");
            assert!(
                error["error"]
                    .as_str()
                    .expect("error message")
                    .contains("unexpected Harness path argument")
            );
        }
    }
    let result = tokio::process::Command::new(env!("CARGO_BIN_EXE_ontology"))
        .env_clear()
        .args(["harness", "resolve", "--json", "--json"])
        .output()
        .await
        .expect("duplicate parser");
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("duplicate"));
}

pub async fn cli_prepared(
    store: &Store,
    root: &Path,
    workspace: &Path,
    request: RequestEnvelope,
) -> (tempfile::TempDir, PreparedHarnessRun) {
    cli_prepared_with_manifest(store, root, workspace, request, None).await
}

pub async fn cli_prepared_with_manifest(
    store: &Store,
    root: &Path,
    workspace: &Path,
    request: RequestEnvelope,
    manifest: Option<&CareerCompositionManifest>,
) -> (tempfile::TempDir, PreparedHarnessRun) {
    let artifacts = tempfile::tempdir().expect("CLI artifacts");
    let request_path = artifacts.path().join("request.json");
    let capabilities_path = artifacts.path().join("capabilities.json");
    let plan_path = artifacts.path().join("plan.json");
    let prepared_path = artifacts.path().join("prepared.json");
    fs::write(
        &request_path,
        serde_json::to_vec(&request).expect("request bytes"),
    )
    .expect("request artifact");
    fs::write(
        &capabilities_path,
        serde_json::to_vec(&capabilities()).expect("capabilities bytes"),
    )
    .expect("capabilities artifact");
    let mut arguments = vec![
        "--request-envelope".into(),
        request_path.display().to_string(),
        "--runtime-capabilities".into(),
        capabilities_path.display().to_string(),
    ];
    if let Some(manifest) = manifest {
        let path = artifacts.path().join("manifest.json");
        fs::write(&path, serde_json::to_vec(manifest).expect("exact manifest"))
            .expect("manifest artifact");
        arguments.extend(["--career-manifest".into(), path.display().to_string()]);
    }
    let resolved = command(store, root, workspace, "resolve", &arguments).await;
    assert!(
        resolved.status.success(),
        "native CLI resolve: {}",
        String::from_utf8_lossy(&resolved.stderr)
    );
    fs::write(&plan_path, &resolved.stdout).expect("exact raw CLI plan");
    let prepared = command(
        store,
        root,
        workspace,
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
        "native CLI prepare: {}",
        String::from_utf8_lossy(&prepared.stderr)
    );
    assert_eq!(
        fs::metadata(&prepared_path)
            .expect("prepared mode")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let prepared: PreparedHarnessRun =
        decode_current_json(&prepared.stdout, "native prepared output")
            .expect("strict prepared output");
    let replay = command(
        store,
        root,
        workspace,
        "replay",
        &["--prepared-run".into(), prepared_path.display().to_string()],
    )
    .await;
    assert!(
        replay.status.success(),
        "native CLI replay: {}",
        String::from_utf8_lossy(&replay.stderr)
    );
    (artifacts, prepared)
}
#[allow(
    clippy::too_many_arguments,
    reason = "CLI proof binds the store, view, workspace, issued artifacts, run, and role outcome"
)]
pub async fn cli_roles(
    store: &Store,
    root: &Path,
    workspace: &Path,
    artifacts: &Path,
    prepared: &PreparedHarnessRun,
    run: &str,
    content: &str,
    reject: bool,
) -> HarnessExecutionRecord {
    let begun = command(
        store,
        root,
        workspace,
        "begin",
        &[
            "--prepared-run".into(),
            artifacts.join("prepared.json").display().to_string(),
            "--run-id".into(),
            run.into(),
        ],
    )
    .await;
    assert!(
        begun.status.success(),
        "native CLI begin: {}",
        String::from_utf8_lossy(&begun.stderr)
    );
    let mut head: HarnessExecutionRecord =
        decode_current_json(&begun.stdout, "native begun record").expect("strict begun record");
    while let Some(invocation) = head.ready_role_invocations.first() {
        let mut event = role_event(prepared, invocation, content);
        if reject
            && let HarnessExecutionEvent::RoleResult { result } = &mut event
            && let RoleExecutionOutcome::Completed {
                result:
                    CompletedRoleResult::Reviewer {
                        requirement_results,
                        ..
                    },
            } = &mut result.outcome
        {
            requirement_results[0].passed = false;
            event = HarnessExecutionEvent::RoleResult {
                result: Box::new(
                    invocation
                        .bind_result(result.lifecycle.clone(), result.outcome.clone())
                        .expect("bound rejecting review"),
                ),
            };
        }
        let event_path = artifacts.join("event.json");
        fs::write(
            &event_path,
            serde_json::to_vec(&event).expect("event bytes"),
        )
        .expect("event artifact");
        let output = command(
            store,
            root,
            workspace,
            "advance",
            &[
                "--run-id".into(),
                run.into(),
                "--event".into(),
                event_path.display().to_string(),
            ],
        )
        .await;
        assert!(
            output.status.success(),
            "native CLI advance: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        head = decode_current_json(&output.stdout, "advanced native head")
            .expect("Core advanced record");
    }
    let output = command(
        store,
        root,
        workspace,
        "evaluate",
        &["--run-id".into(), run.into()],
    )
    .await;
    assert!(
        output.status.success(),
        "native CLI evaluate: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    decode_current_json(&output.stdout, "evaluated native head").expect("Core evaluated record")
}
