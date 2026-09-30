use super::*;

#[test]
fn current_json_decoder_rejects_duplicate_unknown_and_omitted_fields() {
    #[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
    struct Nested {
        active: bool,
    }

    #[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
    struct Artifact {
        nested: Nested,
        note: Option<String>,
    }

    let exact = br#"{"nested":{"active":true},"note":null}"#;
    assert!(decode_current_json::<Artifact>(exact, "test artifact").is_ok());

    let duplicate_root = br#"{"nested":{"active":true},"note":"first","note":"second"}"#;
    assert!(decode_current_json::<Artifact>(duplicate_root, "test artifact").is_err());

    let duplicate_nested = br#"{"nested":{"active":false,"active":true},"note":null}"#;
    assert!(decode_current_json::<Artifact>(duplicate_nested, "test artifact").is_err());

    let duplicate_escaped = br#"{"nested":{"active":true},"note":"first","no\u0074e":"second"}"#;
    assert!(decode_current_json::<Artifact>(duplicate_escaped, "test artifact").is_err());

    let nested_unknown = br#"{"nested":{"active":true,"retired":true},"note":null}"#;
    assert!(decode_current_json::<Artifact>(nested_unknown, "test artifact").is_err());

    let omitted_current_field = br#"{"nested":{"active":true}}"#;
    assert!(decode_current_json::<Artifact>(omitted_current_field, "test artifact").is_err());
}

struct TemporaryWorkspace(PathBuf);

impl TemporaryWorkspace {
    fn create() -> Self {
        static NEXT_WORKSPACE_ID: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(0);
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock must follow the Unix epoch")
            .as_nanos();
        let sequence = NEXT_WORKSPACE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir()
            .canonicalize()
            .expect("temporary root must resolve")
            .join(format!(
                "llm-context-vault-harness-{}-{unique}-{sequence}",
                std::process::id()
            ));
        fs::create_dir(&path).expect("temporary workspace must be created");
        Self(path)
    }
}

impl Drop for TemporaryWorkspace {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("temporary workspace must be removed");
    }
}

fn repository_root() -> PathBuf {
    thread_local! {
        static REPOSITORY: TemporaryWorkspace = synthetic_repository();
    }
    REPOSITORY.with(|repository| repository.0.clone())
}

// These documents model routing and byte bindings only; they are not private policy originals.
#[allow(
    clippy::too_many_lines,
    reason = "explicit synthetic routing documents keep the fixture independent of private originals"
)]
fn synthetic_repository() -> TemporaryWorkspace {
    let repository = TemporaryWorkspace::create();
    let write = |relative: &str, content: &str| {
        let path = repository.0.join(relative);
        fs::create_dir_all(path.parent().expect("fixture file has a parent"))
            .expect("owned fixture parents must be created");
        fs::write(path, content).expect("owned fixture must be written");
    };
    let policies = [
        PolicyCapability::AgentHarness,
        PolicyCapability::AgentOperatingPreferences,
        PolicyCapability::ContextScopeRouting,
        PolicyCapability::MandatoryPreflight,
        PolicyCapability::CommonReviewQuality,
        PolicyCapability::CommonCodeQuality,
        PolicyCapability::CodeReview,
        PolicyCapability::RustCodeStyle,
        PolicyCapability::Dependency,
        PolicyCapability::Issue,
        PolicyCapability::CommonDocumentQuality,
        PolicyCapability::ContextVaultOperatingModel,
        PolicyCapability::ContextDocumentStability,
        PolicyCapability::ProfileIndex,
        PolicyCapability::PersonalIndex,
        PolicyCapability::AiCollaborationValues,
        PolicyCapability::SoloMvpIdeaDiscovery,
        PolicyCapability::PersonalBusinessIndex,
        PolicyCapability::WorkCompanyRegistry,
        PolicyCapability::WorkAgentGuide,
        PolicyCapability::WorkAgentOperatingPreferences,
        PolicyCapability::CareerResumeSourceMap,
        PolicyCapability::CareerClaimTokenOutputSystem,
        PolicyCapability::CareerOutputAssembly,
        PolicyCapability::CareerContributionAudit,
        PolicyCapability::CareerPerspectiveAndPublicSafety,
        PolicyCapability::CareerPortfolioCasebook,
        PolicyCapability::CareerResumeCaseView,
        PolicyCapability::CareerResumePdfBaseline,
        PolicyCapability::CareerCaseDocumentContract,
    ];
    for policy in policies {
        let (id, path) = policy.binding_location();
        let scope = path.split('/').nth(1).expect("policy has a scope");
        write(
            &path,
            &format!(
                "---\ntitle: Synthetic {id}\nscope: {scope}\n---\n# Synthetic {id}\n\nOwned routing and binding fixture for {path}.\n"
            ),
        );
    }
    for path in [
        "vault/profile/preferences/synthetic-routing-policy.md",
        "vault/personal/profile.md",
        "vault/personal/facts/index.md",
        "vault/personal/knowledge/skills.md",
        "vault/personal/knowledge/index.md",
        "vault/personal/journal/index.md",
        "vault/personal/learning/index.md",
        "vault/personal/ontology/index.md",
        "vault/personal/ontology/schema.md",
        "vault/personal/projects/coupler.md",
        "vault/personal/projects/gluesql/index.md",
        "vault/personal/projects/ontology.md",
        "vault/personal/projects/ideas/idea-discovery-registry.md",
        "vault/personal/projects/ideas/solo-founder-validation-platform.md",
        "vault/work/cluml/index.md",
        "vault/work/tmaxcloud/index.md",
        "vault/work/cluml/projects/giganto.md",
        "vault/work/cluml/experience/case-studies/ci-runner-availability-monitoring.md",
        "vault/work/cluml/experience/case-studies/hog-detection-period-config-externalization.md",
        "vault/work/cluml/rules/aice-aimer-auth-data-flow.md",
        "vault/work/cluml/rules/security-architecture-principles.md",
        "vault/work/cluml/rules/cert.md",
        "vault/work/cluml/rules/rust-issue-hunting.md",
        "vault/work/common/rules/design-discussion-note.md",
    ] {
        let scope = path.split('/').nth(1).expect("fixture has a scope");
        write(
            path,
            &format!(
                "---\ntitle: Synthetic source\nscope: {scope}\n---\n# Synthetic source\n\n{path}: skills knowledge ontology project journal fixture.\n"
            ),
        );
    }
    write(
        "vault/work/common/router/company-registry.md",
        "# Synthetic company registry\n\n## Company Registry\n\n| Company | Company index |\n| --- | --- |\n| ClumL | `vault/work/cluml/index.md` |\n| TmaxCloud | `vault/work/tmaxcloud/index.md` |\n",
    );
    write(
        "vault/work/cluml/preferences/cluml-routing.md",
        r#"---
domain_routes:
  - signals: [auth, jwt, oidc, session, token, mTLS, step-ca, aice, aimer]
    actions: [code-write, code-review, document-write, document-review, investigation, design]
    target_kind: any
    policies: [vault/work/cluml/rules/aice-aimer-auth-data-flow.md]
  - signals: [secret, vault, tenant, encrypt, 암호화, audit, 감사 로그, 개인정보]
    actions: [code-write, code-review, document-write, document-review, investigation, design]
    target_kind: any
    policies: [vault/work/cluml/rules/security-architecture-principles.md]
  - signals: [cert, certificate, CN, pem]
    actions: [code-write, code-review, document-write, document-review, investigation, design]
    target_kind: any
    policies: [vault/work/cluml/rules/cert.md]
  - signals: [dependency, crate, package, 버전]
    actions: [code-write, code-review, document-write, document-review, investigation, design]
    target_kind: any
    policies: [vault/work/common/rules/dependency.md]
  - signals: [issue hunting, 이슈헌팅, 할만한 이슈, 개선점 찾아줘, bug candidate, bug fix candidate, refactor candidate, refactoring candidate, performance candidate, performance improvement candidate, stability candidate, stability improvement candidate]
    actions: [investigation]
    target_kind: rust
    policies: [vault/work/cluml/rules/rust-issue-hunting.md, vault/profile/rules/common-code-quality.md, vault/work/common/rules/code-review.md, vault/work/common/rules/rust-code-style.md, vault/work/common/rules/issue.md]
---
# Synthetic company routing
"#,
    );
    for (path, content) in [
        ("AGENTS.md", "# Owned infrastructure fixture\n"),
        ("README.md", "# Owned repository fixture\n"),
        (
            "Cargo.toml",
            "[package]\nname = \"synthetic-fixture\"\nversion = \"0.0.0\"\n",
        ),
        (
            "crates/context-core/src/lib.rs",
            "pub fn infrastructure_fixture() {}\n",
        ),
        (
            "crates/context-core/src/document.rs",
            "pub fn document_fixture() {}\n",
        ),
        (
            "crates/context-core/src/harness.rs",
            "pub fn harness_fixture() {}\n",
        ),
        (
            "crates/context-core/Cargo.toml",
            "[package]\nname = \"synthetic-core\"\nversion = \"0.0.0\"\n",
        ),
        (".github/workflows/ci.yml", "name: Synthetic CI\n"),
        ("docs/scope-model.md", "# Synthetic scope model\n"),
    ] {
        write(path, content);
    }
    fs::create_dir(repository.0.join("src")).expect("owned tool working directory must exist");
    repository
}

fn engine() -> HarnessEngine {
    HarnessEngine::open(repository_root(), repository_root()).expect("repository fixture must open")
}

fn strict_engine() -> HarnessEngine {
    let router = HarnessRouter::with_execution_profile(2, HarnessExecutionProfile::Strict)
        .expect("strict router must be valid");
    HarnessEngine::with_router(repository_root(), repository_root(), router)
        .expect("repository fixture must open with strict router")
}

fn external_engine() -> (TemporaryWorkspace, HarnessEngine) {
    let workspace = TemporaryWorkspace::create();
    let source = workspace.0.join("src");
    fs::create_dir(&source).expect("external source directory must be created");
    fs::write(source.join("lib.rs"), "pub fn fixture() {}\n")
        .expect("external Rust fixture must be written");
    fs::write(source.join("security.rs"), "pub fn security_fixture() {}\n")
        .expect("external Rust basename fixture must be written");
    fs::write(
        source.join("license-manager.rs"),
        "pub fn license_manager_fixture() {}\n",
    )
    .expect("external license basename fixture must be written");
    fs::write(workspace.0.join("README.md"), "# External project\n")
        .expect("external document fixture must be written");
    for dependency in [
        "constraints.txt",
        "Pipfile",
        "Pipfile.lock",
        "requirements.in",
        "requirements.txt",
    ] {
        fs::write(
            workspace.0.join(dependency),
            "synthetic dependency fixture\n",
        )
        .expect("external dependency fixture must be written");
    }
    let engine = HarnessEngine::open(repository_root(), &workspace.0)
        .expect("external workspace fixture must open");
    (workspace, engine)
}

fn external_engine_with_profile(
    execution_profile: HarnessExecutionProfile,
) -> (TemporaryWorkspace, HarnessEngine) {
    let (workspace, engine) = external_engine();
    drop(engine);
    let router = HarnessRouter::with_execution_profile(2, execution_profile)
        .expect("execution profile router must be valid");
    let engine = HarnessEngine::with_router(repository_root(), &workspace.0, router)
        .expect("external workspace fixture must open with the selected execution profile");
    (workspace, engine)
}

fn assert_batch_completion_receipt(root: &Path, relative_path: Option<&String>) {
    let relative_path = relative_path.expect("completed batch must expose its durable receipt");
    assert!(root.join(relative_path).is_file());
    assert!(!root.join(".llm-context-vault-harness/batches").exists());
}

fn copy_directory(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("fixture destination directory must be created");
    for entry in fs::read_dir(source).expect("fixture source directory must be readable") {
        let entry = entry.expect("fixture directory entry must be readable");
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry
            .file_type()
            .expect("fixture entry type must be readable")
            .is_dir()
        {
            copy_directory(&source_path, &destination_path);
        } else {
            fs::copy(&source_path, &destination_path).expect("fixture file must be copied");
        }
    }
}

fn copy_ai_collaboration_values_fixture(repository: &Path) {
    let destination = repository.join(AI_COLLABORATION_VALUES_PATH);
    fs::create_dir_all(
        destination
            .parent()
            .expect("AI collaboration policy fixture must have a parent"),
    )
    .expect("AI collaboration policy parent must be created");
    fs::copy(
        repository_root().join(AI_COLLABORATION_VALUES_PATH),
        destination,
    )
    .expect("AI collaboration policy fixture must be copied");
}

fn isolated_vault_external_engine() -> (TemporaryWorkspace, TemporaryWorkspace, HarnessEngine) {
    let vault_repository = TemporaryWorkspace::create();
    copy_directory(
        &repository_root().join("vault"),
        &vault_repository.0.join("vault"),
    );
    let workspace = TemporaryWorkspace::create();
    fs::write(workspace.0.join("README.md"), "# External project\n")
        .expect("external document fixture must be written");
    let engine = HarnessEngine::open(&vault_repository.0, &workspace.0)
        .expect("isolated Vault and external workspace must open");
    (vault_repository, workspace, engine)
}

fn complete_runtime_capabilities() -> RoleRuntimeCapabilities {
    RoleRuntimeCapabilities {
        available_roles: vec![
            HarnessRole::Writer,
            HarnessRole::Verifier,
            HarnessRole::Reviewer,
            HarnessRole::Specialist,
        ],
        max_concurrent_roles: 5,
        separate_contexts: true,
        file_reading: true,
        tool_execution: true,
        max_role_bundle_bytes: default_max_role_bundle_bytes(),
        max_role_invocation_bytes: default_max_role_invocation_bytes(),
        max_role_execution_millis: default_max_role_execution_millis(),
        max_role_grace_millis: default_max_role_grace_millis(),
    }
}

fn prepared_run(resolved: &ResolvedHarnessRequest) -> PreparedRoleRun {
    engine()
        .prepare(resolved, &complete_runtime_capabilities(), None)
        .expect("test runtime must prepare")
}

fn prepared_run_with(engine: &HarnessEngine, resolved: &ResolvedHarnessRequest) -> PreparedRoleRun {
    engine
        .prepare(resolved, &complete_runtime_capabilities(), None)
        .expect("test runtime must prepare")
}

fn resolve_vault_read(
    engine: &HarnessEngine,
    request: &HarnessRequest,
    execution_profile: HarnessExecutionProfile,
    query: &str,
    maximum_context_bytes: usize,
) -> ResolvedHarnessRequest {
    let envelope = RequestEnvelope::from_structured_vault_read(
        request.clone(),
        execution_profile,
        query.to_owned(),
        maximum_context_bytes,
        "Vault read test input",
    )
    .expect("Vault read envelope must build");
    engine
        .resolve_test_envelope(&envelope, None)
        .expect("Vault read envelope must resolve")
}

fn user_profile_code_envelope(statement_text: &str) -> RequestEnvelope {
    let statement_identifier = "statement-0000000000000001".to_owned();
    let mut records = Vec::new();
    let mut next_decision = 1_u64;
    let owner = user_envelope_value(
        DataOwner::Profile,
        &statement_identifier,
        &mut records,
        &mut next_decision,
    );
    let intent = user_envelope_value(
        HarnessIntent::PolicyMaintenance,
        &statement_identifier,
        &mut records,
        &mut next_decision,
    );
    let task_statement = user_envelope_value(
        "change the Harness request contract".to_owned(),
        &statement_identifier,
        &mut records,
        &mut next_decision,
    );
    let execution_profile = user_envelope_policy_value(
        HarnessExecutionProfile::Standard,
        PolicyDefaultRule::StandardExecutionProfile,
        &mut records,
        &mut next_decision,
    );
    let write_kind = user_envelope_value(
        WriteKind::Code,
        &statement_identifier,
        &mut records,
        &mut next_decision,
    );
    let targets = user_envelope_value(
        vec!["crates/context-core/src/harness.rs".to_owned()],
        &statement_identifier,
        &mut records,
        &mut next_decision,
    );
    RequestEnvelope {
        version: HARNESS_SCHEMA_VERSION,
        source: RequestSource::UserLanguage {
            statements: vec![UserStatement {
                identifier: statement_identifier,
                text: statement_text.to_owned(),
            }],
        },
        draft: DraftTaskRequest::Write(DraftWriteContract {
            common: DraftTaskCommon {
                owner,
                intent,
                task_statement,
                execution_profile,
                context_grants: None,
                evidence_source_paths: None,
            },
            write_kind,
            targets,
            delete_targets: None,
        }),
        decision_trace: DecisionTrace { records },
    }
}

fn user_envelope_value<T: Serialize>(
    value: T,
    statement_identifier: &str,
    records: &mut Vec<DecisionRecord>,
    next_decision: &mut u64,
) -> DraftValue<T> {
    let decision_identifier = format!("decision-{:016x}", *next_decision);
    *next_decision += 1;
    records.push(DecisionRecord::UserStatement {
        identifier: decision_identifier.clone(),
        value_digest: serialized_digest(&value).expect("test decision value must serialize"),
        statement_identifiers: vec![statement_identifier.to_owned()],
    });
    DraftValue::Resolved {
        value,
        decision_identifier,
    }
}

fn user_envelope_policy_value<T: Serialize>(
    value: T,
    rule: PolicyDefaultRule,
    records: &mut Vec<DecisionRecord>,
    next_decision: &mut u64,
) -> DraftValue<T> {
    let decision_identifier = format!("decision-{:016x}", *next_decision);
    *next_decision += 1;
    let repository_root = repository_root();
    let policy_path = repository_root.join("vault/profile/rules/agent-harness.md");
    let policy_file = File::open(&policy_path).expect("Agent Harness policy must open");
    let policy_content_digest =
        digest_file(policy_file, MAX_POLICY_BYTES, &policy_path).expect("policy must hash");
    records.push(DecisionRecord::PolicyDefault {
        identifier: decision_identifier.clone(),
        value_digest: serialized_digest(&value).expect("test decision value must serialize"),
        policy_identifier: "agent-harness".to_owned(),
        policy_content_digest,
        rule,
    });
    DraftValue::Resolved {
        value,
        decision_identifier,
    }
}

fn user_confirmed_model_value<T: Serialize>(
    value: T,
    based_on_decision_identifier: &str,
    statement_identifier: &str,
    records: &mut Vec<DecisionRecord>,
    next_decision: &mut u64,
    confirmed: bool,
) -> DraftValue<T> {
    let value_digest = serialized_digest(&value).expect("test model value must serialize");
    let proposal_identifier = format!("decision-{:016x}", *next_decision);
    *next_decision += 1;
    records.push(DecisionRecord::ModelProposal {
        identifier: proposal_identifier.clone(),
        value_digest: value_digest.clone(),
        based_on_record_identifiers: vec![based_on_decision_identifier.to_owned()],
    });
    if !confirmed {
        return DraftValue::Resolved {
            value,
            decision_identifier: proposal_identifier,
        };
    }
    let confirmation_identifier = format!("decision-{:016x}", *next_decision);
    *next_decision += 1;
    records.push(DecisionRecord::UserConfirmation {
        identifier: confirmation_identifier.clone(),
        value_digest,
        confirms_record_identifier: proposal_identifier,
        statement_identifiers: vec![statement_identifier.to_owned()],
    });
    DraftValue::Resolved {
        value,
        decision_identifier: confirmation_identifier,
    }
}

fn promotion_curation_envelope(
    handoff: PromotionHandoff,
    owner: DataOwner,
    curation_kind: CurationKind,
    target: &str,
    confirmed_handoff: bool,
) -> RequestEnvelope {
    let statement_identifier = "statement-0000000000000002".to_owned();
    let mut records = Vec::new();
    let mut next_decision = 1_u64;
    let owner = user_envelope_value(
        owner,
        &statement_identifier,
        &mut records,
        &mut next_decision,
    );
    let owner_decision_identifier = match &owner {
        DraftValue::Resolved {
            decision_identifier,
            ..
        } => decision_identifier.clone(),
        DraftValue::Unresolved { .. } => unreachable!("test owner must be resolved"),
    };
    let common = DraftOwnedTaskCommon {
        owner,
        task_statement: user_envelope_value(
            "승인한 기억 후보를 장기기억으로 저장한다".to_owned(),
            &statement_identifier,
            &mut records,
            &mut next_decision,
        ),
        execution_profile: user_envelope_policy_value(
            HarnessExecutionProfile::Standard,
            PolicyDefaultRule::StandardExecutionProfile,
            &mut records,
            &mut next_decision,
        ),
    };
    let curation_kind = user_envelope_value(
        curation_kind,
        &statement_identifier,
        &mut records,
        &mut next_decision,
    );
    let target = user_envelope_value(
        target.to_owned(),
        &statement_identifier,
        &mut records,
        &mut next_decision,
    );
    let curation_sources = Some(user_envelope_value(
        vec!["vault/personal/index.md".to_owned()],
        &statement_identifier,
        &mut records,
        &mut next_decision,
    ));
    let promotion_handoff = Some(user_confirmed_model_value(
        handoff,
        &owner_decision_identifier,
        &statement_identifier,
        &mut records,
        &mut next_decision,
        confirmed_handoff,
    ));
    let confirmation = user_envelope_value(
        UserConfirmationStatus::Confirmed,
        &statement_identifier,
        &mut records,
        &mut next_decision,
    );
    RequestEnvelope {
        version: HARNESS_SCHEMA_VERSION,
        source: RequestSource::UserLanguage {
            statements: vec![UserStatement {
                identifier: statement_identifier,
                text: "이 기억 후보를 장기기억으로 저장해도 좋다".to_owned(),
            }],
        },
        draft: DraftTaskRequest::Curation(Box::new(DraftCurationContract {
            common,
            curation_kind,
            target,
            curation_sources,
            delete_target: None,
            promotion_handoff,
            confirmation,
        })),
        decision_trace: DecisionTrace { records },
    }
}

fn promotion_source_evaluation(
    engine: &HarnessEngine,
    rejected: bool,
) -> (
    ResolvedHarnessRequest,
    PreparedRoleRun,
    HarnessTaskEvaluation,
) {
    let request = HarnessRequest {
        action: HarnessAction::Investigation,
        owner: DataOwner::Personal,
        targets: vec!["vault/personal/index.md".to_owned()],
        objective: "장기기억 후보가 될 사실을 조사한다".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = engine
        .resolve(&request)
        .expect("investigation must resolve");
    let prepared = prepared_run_with(engine, &resolved);
    let source = prepared
        .role_bundles
        .iter()
        .find(|bundle| bundle.role == HarnessRole::Specialist)
        .and_then(|bundle| {
            bundle
                .bound_documents()
                .find(|document| document.source == HarnessBoundDocumentSource::Target)
        })
        .cloned()
        .expect("investigation target must be bound to the Specialist");
    let mut step = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("investigation must begin");
    let specialist = step.ready_role_invocations[0].clone();
    let mut specialist_event = completed_role_event(
        &resolved,
        &prepared,
        &specialist,
        Vec::new(),
        "handoff-source",
    );
    let HarnessExecutionEvent::RoleResult { result } = &mut specialist_event else {
        unreachable!("Specialist event must contain a role result");
    };
    let RoleExecutionOutcome::Completed {
        result:
            CompletedRoleResult::Specialist {
                promotion_proposals,
                ..
            },
    } = &mut result.outcome
    else {
        unreachable!("Specialist result must be completed");
    };
    promotion_proposals.push(PromotionProposal {
        identifier: "proposal-0000000000000003".to_owned(),
        owner: DataOwner::Personal,
        curation_kind: CurationKind::Fact,
        title: "승인 가능한 장기기억 후보".to_owned(),
        content: "promotionloopsearchtoken 은 승인 후 검색되어야 한다.".to_owned(),
        origin: PromotionProposalOrigin::SpecialistMemory {
            claim_kind: PromotionClaimKind::ObservedFact,
            evidence: vec![PromotionEvidenceReference {
                source_kind: PromotionSourceKind::Target,
                source_owner: DataOwner::Personal,
                relative_path: source.relative_path,
                content_digest: source.content_digest,
                locator: "전체 색인 문서".to_owned(),
            }],
        },
    });
    refresh_role_event_digest(&mut specialist_event);
    step = engine
        .advance_execution(&resolved, &prepared, &[], &step.record, specialist_event)
        .expect("promotion-bearing Specialist result must advance");
    let reviewer = step.ready_role_invocations[0].clone();
    let blocking_findings = if rejected {
        vec![BlockingFinding {
            message: "후보 근거가 충분하지 않다".to_owned(),
            evidence: vec![subject_evidence(&resolved.plan, &reviewer.subject)],
        }]
    } else {
        Vec::new()
    };
    step = engine
        .advance_execution(
            &resolved,
            &prepared,
            &[],
            &step.record,
            completed_role_event(
                &resolved,
                &prepared,
                &reviewer,
                blocking_findings,
                "handoff-source",
            ),
        )
        .expect("Reviewer result must complete the investigation");
    let evaluation = engine
        .evaluate_execution(&resolved, &prepared, &[], &step.record)
        .expect("investigation must evaluate");
    (resolved, prepared, evaluation)
}

fn reviewer_learning_source_evaluation(
    engine: &HarnessEngine,
) -> (
    ResolvedHarnessRequest,
    PreparedRoleRun,
    HarnessTaskEvaluation,
) {
    let request = HarnessRequest {
        action: HarnessAction::Investigation,
        owner: DataOwner::Personal,
        targets: vec!["vault/personal/index.md".to_owned()],
        objective: "실패한 검증에서 재사용 가능한 학습을 제안한다".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = engine
        .resolve(&request)
        .expect("learning source investigation must resolve");
    let prepared = prepared_run_with(engine, &resolved);
    let mut step = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("learning source investigation must begin");
    let specialist = step.ready_role_invocations[0].clone();
    step = engine
        .advance_execution(
            &resolved,
            &prepared,
            &[],
            &step.record,
            completed_role_event(
                &resolved,
                &prepared,
                &specialist,
                Vec::new(),
                "reviewer-learning-source",
            ),
        )
        .expect("Specialist must complete before Reviewer");
    let reviewer = step.ready_role_invocations[0].clone();
    let mut reviewer_event = completed_role_event(
        &resolved,
        &prepared,
        &reviewer,
        Vec::new(),
        "reviewer-learning-source",
    );
    let HarnessExecutionEvent::RoleResult { result } = &mut reviewer_event else {
        unreachable!("Reviewer event must contain a role result");
    };
    let RoleExecutionOutcome::Completed {
        result:
            CompletedRoleResult::Reviewer {
                requirement_results,
                learning_candidates,
                ..
            },
    } = &mut result.outcome
    else {
        unreachable!("Reviewer result must be completed");
    };
    let failed = requirement_results
        .iter_mut()
        .find(|result| result.unit == VerificationUnit::CompletionContract)
        .expect("Reviewer must own the completion requirement");
    failed.passed = false;
    failed.detail = "완료 조건을 결과에 명시하지 못했다".to_owned();
    learning_candidates.push(ReviewerLearningCandidate {
        title: "완료 조건을 결과에 명시한다".to_owned(),
        guidance: "작업을 시작하기 전에 완료 조건을 고정하고 최종 결과에서 항목별로 확인한다."
            .to_owned(),
        failed_unit: failed.unit,
        requirement_result_digest: serialized_digest(failed)
            .expect("failed requirement result must serialize"),
        evidence: failed.evidence.clone(),
    });
    refresh_role_event_digest(&mut reviewer_event);
    step = engine
        .advance_execution(&resolved, &prepared, &[], &step.record, reviewer_event)
        .expect("failed Reviewer result with learning candidate must advance");
    let evaluation = engine
        .evaluate_execution(&resolved, &prepared, &[], &step.record)
        .expect("failed source task must evaluate");
    (resolved, prepared, evaluation)
}

fn complete_reviewer_learning_curation_record(
    engine: &HarnessEngine,
    resolved: &ResolvedHarnessRequest,
    prepared: &PreparedRoleRun,
    handoff: &PromotionHandoff,
    context_suffix: &str,
) -> RoleExecutionRecord {
    let mut step = engine.begin_execution(resolved, prepared, &[]).unwrap();
    while let Some(invocation) = step.ready_role_invocations.first().cloned() {
        let mut event =
            completed_role_event(resolved, prepared, &invocation, Vec::new(), context_suffix);
        if invocation.role == HarnessRole::Writer {
            let HarnessExecutionEvent::RoleResult { result } = &mut event else {
                unreachable!("Writer event must contain a role result");
            };
            let RoleExecutionOutcome::Completed {
                result:
                    CompletedRoleResult::Writer {
                        artifact: WriterArtifact::Curation { entries, .. },
                    },
            } = &mut result.outcome
            else {
                unreachable!("learning Writer must produce curation entries");
            };
            let FileChange::Create { content, .. } = &mut entries[0].change else {
                unreachable!("learning curation must create a document");
            };
            *content = canonical_learning_markdown(handoff).unwrap();
            refresh_role_event_digest(&mut event);
        }
        step = engine
            .advance_execution(resolved, prepared, &[], &step.record, event)
            .unwrap();
    }
    step.record
}

fn capabilities_for(roles: Vec<HarnessRole>, tool_execution: bool) -> RoleRuntimeCapabilities {
    RoleRuntimeCapabilities {
        max_concurrent_roles: roles.len().max(1),
        available_roles: roles,
        separate_contexts: true,
        file_reading: true,
        tool_execution,
        max_role_bundle_bytes: default_max_role_bundle_bytes(),
        max_role_invocation_bytes: default_max_role_invocation_bytes(),
        max_role_execution_millis: default_max_role_execution_millis(),
        max_role_grace_millis: default_max_role_grace_millis(),
    }
}

fn planned_roles(plan: &ResolvedHarnessPlan) -> Vec<HarnessRole> {
    plan.workflow.iter().map(|node| node.role).collect()
}

fn assert_apply_lifecycle_failure_contains<T>(
    result: HarnessResult<T>,
    stage: &str,
    message_fragment: &str,
) {
    let Err(HarnessError::ApplyLifecycle { receipt, .. }) = result else {
        panic!("expected an apply lifecycle failure");
    };
    assert!(
        receipt.orchestration_failures.iter().any(|failure| {
            failure.stage == stage && failure.message.contains(message_fragment)
        }),
        "expected lifecycle failure at `{stage}` containing `{message_fragment}`"
    );
}

fn profile_code_request() -> HarnessRequest {
    HarnessRequest {
        action: HarnessAction::CodeWrite,
        owner: DataOwner::Profile,
        targets: vec!["crates/context-core/src/lib.rs".to_owned()],
        objective: "implement a small change".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    }
}

fn personal_project_code_request() -> HarnessRequest {
    HarnessRequest {
        action: HarnessAction::CodeWrite,
        owner: DataOwner::PersonalProject {
            project: "coupler".to_owned(),
        },
        targets: vec!["src/lib.rs".to_owned()],
        objective: "implement a small change".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    }
}

fn complete_submission(resolved: &ResolvedHarnessRequest) -> HarnessSubmission {
    complete_submission_with(&engine(), resolved)
}

fn complete_submission_with(
    engine: &HarnessEngine,
    resolved: &ResolvedHarnessRequest,
) -> HarnessSubmission {
    let mut submission =
        HarnessEngine::submission_template(resolved, &prepared_run_with(engine, resolved))
            .expect("validated plan must produce a submission template");
    fill_reported_role_contexts(&mut submission, &resolved.plan, "1");
    for check in &mut submission.verification_checks {
        check.passed = true;
        check.detail = "checked".to_owned();
    }
    match &mut submission.artifact {
        SubmissionArtifact::Changes { changes } => {
            for change in changes {
                match change {
                    FileChange::Create { content, .. } | FileChange::Update { content, .. } => {
                        *content = "candidate".to_owned();
                    }
                    FileChange::Delete { .. } => {}
                }
            }
        }
        SubmissionArtifact::Review { summary, .. } => *summary = "reviewed".to_owned(),
        SubmissionArtifact::Analysis { output, .. } => *output = "analyzed".to_owned(),
        SubmissionArtifact::Curation {
            entries,
            reported_user_confirmation,
            ..
        } => {
            *reported_user_confirmation = true;
            for entry in entries {
                entry.provenance = vec!["user request".to_owned()];
                match &mut entry.change {
                    FileChange::Create { content, .. } | FileChange::Update { content, .. } => {
                        *content = "candidate".to_owned();
                    }
                    FileChange::Delete { .. } => {}
                }
            }
        }
    }
    submission
}

fn rejected_evaluation(
    engine: &HarnessEngine,
    resolved: &ResolvedHarnessRequest,
    prepared: &PreparedRoleRun,
    message: &str,
) -> HarnessEvaluationResult {
    let mut submission = complete_submission_with(engine, resolved);
    submission.findings = vec![Finding {
        severity: "P1".to_owned(),
        message: message.to_owned(),
    }];
    engine
        .evaluate_submission_with_history(resolved, prepared, &submission, &[])
        .expect("rejected submission must produce an evaluation receipt")
}

fn refresh_evaluation_receipt(result: &mut HarnessEvaluationResult) {
    result.validation_receipt_digest = serialized_digest(&(
        &result.candidate_digest,
        &result.reported_reviewer_context_id,
        &result.reported_role_contexts,
        &result.reported_role_lifecycles,
        &result.verification_checks,
        &result.findings,
        result.assurance,
        result.accepted,
    ))
    .expect("evaluation receipt fixture must be serializable");
}

fn fill_reported_role_contexts(
    submission: &mut HarnessSubmission,
    plan: &ResolvedHarnessPlan,
    context_suffix: &str,
) {
    for role_context in &mut submission.reported_role_contexts {
        role_context.context_id = role_context_id(role_context.role, context_suffix);
    }
    for (index, lifecycle) in submission.reported_role_lifecycles.iter_mut().enumerate() {
        let started = u64::try_from(index).expect("role index must fit u64") * 10 + 1;
        lifecycle.context_id = role_context_id(lifecycle.role, context_suffix);
        lifecycle.started_at_millis = started;
        lifecycle.context_ready_at_millis = started + 1;
        lifecycle.first_output_at_millis = Some(started + 2);
        lifecycle.interrupt_requested_at_millis = None;
        lifecycle.grace_deadline_at_millis = None;
        lifecycle.terminal_at_millis = started + 3;
        lifecycle.closed_at_millis = started + 4;
        lifecycle.terminal_state = RoleTerminalState::Completed;
    }
    submission.reported_producer_context_id =
        role_context_id(plan.primary_producer_role, context_suffix);
    submission.reported_reviewer_context_id = if plan.contains_role(HarnessRole::Reviewer) {
        role_context_id(HarnessRole::Reviewer, context_suffix)
    } else {
        String::new()
    };
}

fn role_context_id(role: HarnessRole, context_suffix: &str) -> String {
    let role_name = match role {
        HarnessRole::Writer => "writer",
        HarnessRole::Verifier => "verifier",
        HarnessRole::Reviewer => "reviewer",
        HarnessRole::Specialist => "specialist",
    };
    format!("{role_name}-{context_suffix}")
}

fn has_verification(plan: &ResolvedHarnessPlan, unit: &str) -> bool {
    plan.verification_requirements
        .iter()
        .any(|requirement| requirement.unit.as_str() == unit)
}

fn execution_context_id(role: HarnessRole, suffix: &str) -> String {
    format!(
        "{}-execution-{suffix}",
        match role {
            HarnessRole::Writer => "writer",
            HarnessRole::Verifier => "verifier",
            HarnessRole::Reviewer => "reviewer",
            HarnessRole::Specialist => "specialist",
        }
    )
}

fn completed_lifecycle(role: HarnessRole, context_id: &str) -> ReportedRoleLifecycle {
    ReportedRoleLifecycle {
        role,
        context_id: context_id.to_owned(),
        started_at_millis: 1,
        context_ready_at_millis: 2,
        first_output_at_millis: Some(3),
        interrupt_requested_at_millis: None,
        grace_deadline_at_millis: None,
        terminal_at_millis: 4,
        closed_at_millis: 5,
        terminal_state: RoleTerminalState::Completed,
    }
}

fn subject_evidence(
    plan: &ResolvedHarnessPlan,
    subject: &EvaluationSubject,
) -> ResultEvidenceReference {
    match subject {
        EvaluationSubject::ProducedArtifact { artifact_digest } => {
            ResultEvidenceReference::ProducedArtifact {
                artifact_digest: artifact_digest.clone(),
                locator: "entire produced artifact".to_owned(),
            }
        }
        EvaluationSubject::FrozenTargets { .. } => {
            let target = plan
                .targets
                .first()
                .expect("review subject must contain a target");
            let TargetState::Existing { content_digest } = &target.state else {
                panic!("review target must exist");
            };
            ResultEvidenceReference::Target {
                workspace_relative_path: target.workspace_relative_path.clone(),
                content_digest: content_digest.clone(),
                locator: "entire target".to_owned(),
            }
        }
        EvaluationSubject::TaskContract { .. } => {
            panic!("role verification cannot use a task contract as final evidence")
        }
    }
}

fn subject_evidence_coverage(
    plan: &ResolvedHarnessPlan,
    subject: &EvaluationSubject,
) -> Vec<ResultEvidenceReference> {
    match subject {
        EvaluationSubject::ProducedArtifact { .. } => vec![subject_evidence(plan, subject)],
        EvaluationSubject::FrozenTargets { .. } => plan
            .targets
            .iter()
            .map(|target| {
                let TargetState::Existing { content_digest } = &target.state else {
                    panic!("review target must exist");
                };
                ResultEvidenceReference::Target {
                    workspace_relative_path: target.workspace_relative_path.clone(),
                    content_digest: content_digest.clone(),
                    locator: "entire target".to_owned(),
                }
            })
            .collect(),
        EvaluationSubject::TaskContract { .. } => {
            panic!("role verification cannot use a task contract as final evidence")
        }
    }
}

fn passed_requirement_results(
    plan: &ResolvedHarnessPlan,
    requirements: &[VerificationRequirement],
    subject: &EvaluationSubject,
) -> Vec<RequirementResult> {
    requirements
        .iter()
        .map(|requirement| RequirementResult {
            unit: requirement.unit,
            passed: true,
            detail: "verified".to_owned(),
            evidence: vec![subject_evidence(plan, subject)],
        })
        .collect()
}

fn reviewer_improvement_observation(
    resolved: &ResolvedHarnessRequest,
    subject: &EvaluationSubject,
    message: &str,
) -> ImprovementOpportunity {
    let evidence = match subject {
        EvaluationSubject::ProducedArtifact { artifact_digest } => {
            ResultEvidenceReference::ProducedArtifact {
                artifact_digest: artifact_digest.clone(),
                locator: "reviewed artifact summary".to_owned(),
            }
        }
        EvaluationSubject::FrozenTargets { .. } => {
            let target = resolved
                .plan
                .targets
                .first()
                .expect("review target must exist");
            let TargetState::Existing { content_digest } = &target.state else {
                panic!("review target must be frozen");
            };
            ResultEvidenceReference::Target {
                workspace_relative_path: target.workspace_relative_path.clone(),
                content_digest: content_digest.clone(),
                locator: "reviewed artifact summary".to_owned(),
            }
        }
        EvaluationSubject::TaskContract { .. } => {
            panic!("Reviewer improvement must bind a concrete artifact")
        }
    };
    ImprovementOpportunity {
        message: message.to_owned(),
        evidence: vec![evidence],
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "role event orchestration and result digest ordering must remain auditable together"
)]
fn completed_role_event(
    resolved: &ResolvedHarnessRequest,
    prepared: &PreparedRoleRun,
    invocation: &RoleInvocationContract,
    blocking_findings: Vec<BlockingFinding>,
    context_suffix: &str,
) -> HarnessExecutionEvent {
    let metadata = prepared
        .role_metadata
        .iter()
        .find(|metadata| metadata.role == invocation.role)
        .expect("invoked role must have prepared metadata");
    let result = match &metadata.task {
        RoleTaskContract::Writer {
            targets,
            curation_kind,
        } => {
            let artifact = if resolved.plan.action == HarnessAction::VaultCuration {
                WriterArtifact::Curation {
                    curation_kind: curation_kind.expect("curation Writer requires a kind"),
                    entries: targets
                        .iter()
                        .map(|target| CurationEntry {
                            change: match (&target.operation, &target.state) {
                                (TargetOperation::Create, TargetState::Absent) => {
                                    FileChange::Create {
                                        path: target.workspace_relative_path.clone(),
                                        content: "candidate".to_owned(),
                                    }
                                }
                                (
                                    TargetOperation::Update,
                                    TargetState::Existing { content_digest },
                                ) => FileChange::Update {
                                    path: target.workspace_relative_path.clone(),
                                    expected_content_digest: content_digest.clone(),
                                    content: "candidate".to_owned(),
                                },
                                (
                                    TargetOperation::Delete,
                                    TargetState::Existing { content_digest },
                                ) => FileChange::Delete {
                                    path: target.workspace_relative_path.clone(),
                                    expected_content_digest: content_digest.clone(),
                                },
                                _ => panic!("unsupported Writer target"),
                            },
                            provenance: vec!["confirmed request decision".to_owned()],
                            source_references: resolved.plan.curation_sources.clone(),
                            promotion_handoff_digest: resolved
                                .plan
                                .promotion_handoff
                                .as_ref()
                                .map(|handoff| handoff.handoff_digest.clone()),
                        })
                        .collect(),
                }
            } else {
                WriterArtifact::Changes {
                    changes: targets
                        .iter()
                        .map(|target| match (&target.operation, &target.state) {
                            (TargetOperation::Create, TargetState::Absent) => FileChange::Create {
                                path: target.workspace_relative_path.clone(),
                                content: "candidate".to_owned(),
                            },
                            (TargetOperation::Update, TargetState::Existing { content_digest }) => {
                                FileChange::Update {
                                    path: target.workspace_relative_path.clone(),
                                    expected_content_digest: content_digest.clone(),
                                    content: "candidate".to_owned(),
                                }
                            }
                            (TargetOperation::Delete, TargetState::Existing { content_digest }) => {
                                FileChange::Delete {
                                    path: target.workspace_relative_path.clone(),
                                    expected_content_digest: content_digest.clone(),
                                }
                            }
                            _ => panic!("unsupported Writer target"),
                        })
                        .collect(),
                }
            };
            CompletedRoleResult::Writer { artifact }
        }
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
                output: "analysis result".to_owned(),
            };
            let subject = EvaluationSubject::ProducedArtifact {
                artifact_digest: serialized_digest(&artifact)
                    .expect("Specialist artifact must be serializable"),
            };
            CompletedRoleResult::Specialist {
                requirement_results: passed_requirement_results(
                    &resolved.plan,
                    verification_requirements,
                    &subject,
                ),
                artifact,
                promotion_proposals: Vec::new(),
            }
        }
        RoleTaskContract::Verifier {
            verification_requirements,
            ..
        } => CompletedRoleResult::Verifier {
            subject_evidence: subject_evidence_coverage(&resolved.plan, &invocation.subject),
            requirement_results: passed_requirement_results(
                &resolved.plan,
                verification_requirements,
                &invocation.subject,
            ),
        },
        RoleTaskContract::Reviewer {
            verification_requirements,
            ..
        } => CompletedRoleResult::Reviewer {
            summary: "reviewed".to_owned(),
            subject_evidence: subject_evidence_coverage(&resolved.plan, &invocation.subject),
            requirement_results: passed_requirement_results(
                &resolved.plan,
                verification_requirements,
                &invocation.subject,
            ),
            blocking_findings,
            improvements: Vec::new(),
            learning_candidates: Vec::new(),
        },
    };
    let context_id = execution_context_id(invocation.role, context_suffix);
    let lifecycle = completed_lifecycle(invocation.role, &context_id);
    let outcome = RoleExecutionOutcome::Completed { result };
    let result_digest = serialized_digest(&(
        invocation.role,
        &invocation.invocation_digest,
        &context_id,
        &lifecycle,
        &outcome,
    ))
    .expect("role result must be serializable");
    HarnessExecutionEvent::RoleResult {
        result: Box::new(RoleExecutionResult {
            role: invocation.role,
            invocation_digest: invocation.invocation_digest.clone(),
            context_id,
            lifecycle,
            outcome,
            result_digest,
        }),
    }
}

fn timed_out_role_event(
    invocation: &RoleInvocationContract,
    context_id: &str,
) -> HarnessExecutionEvent {
    let context_id = context_id.to_owned();
    let lifecycle = ReportedRoleLifecycle {
        role: invocation.role,
        context_id: context_id.clone(),
        started_at_millis: 1,
        context_ready_at_millis: 2,
        first_output_at_millis: None,
        interrupt_requested_at_millis: Some(3),
        grace_deadline_at_millis: Some(5),
        terminal_at_millis: 5,
        closed_at_millis: 6,
        terminal_state: RoleTerminalState::TimedOut,
    };
    let outcome = RoleExecutionOutcome::TimedOut;
    let result_digest = serialized_digest(&(
        invocation.role,
        &invocation.invocation_digest,
        &context_id,
        &lifecycle,
        &outcome,
    ))
    .expect("terminal result must serialize");
    HarnessExecutionEvent::RoleResult {
        result: Box::new(RoleExecutionResult {
            role: invocation.role,
            invocation_digest: invocation.invocation_digest.clone(),
            context_id,
            lifecycle,
            outcome,
            result_digest,
        }),
    }
}

fn missing_context_role_event(
    resolved: &ResolvedHarnessRequest,
    invocation: &RoleInvocationContract,
    candidate: MissingContextCandidate,
    context_id: &str,
) -> HarnessExecutionEvent {
    let blocked_verification_units = resolved
        .plan
        .verification_requirements
        .iter()
        .filter_map(|requirement| {
            matches!(
                requirement.owner,
                VerificationOwner::Role {
                    role: HarnessRole::Reviewer
                }
            )
            .then_some(requirement.unit)
        })
        .take(1)
        .collect::<Vec<_>>();
    let context_id = context_id.to_owned();
    let lifecycle = ReportedRoleLifecycle {
        role: invocation.role,
        context_id: context_id.clone(),
        started_at_millis: 1,
        context_ready_at_millis: 2,
        first_output_at_millis: Some(3),
        interrupt_requested_at_millis: None,
        grace_deadline_at_millis: None,
        terminal_at_millis: 4,
        closed_at_millis: 5,
        terminal_state: RoleTerminalState::MissingContext,
    };
    let outcome = RoleExecutionOutcome::MissingContext {
        request: MissingContextRequest {
            reason: "the prepared evidence cannot establish this requirement".to_owned(),
            blocked_verification_units,
            subject_evidence: subject_evidence_coverage(&resolved.plan, &invocation.subject),
            candidate,
        },
    };
    let result_digest = serialized_digest(&(
        invocation.role,
        &invocation.invocation_digest,
        &context_id,
        &lifecycle,
        &outcome,
    ))
    .expect("missing-context result must serialize");
    HarnessExecutionEvent::RoleResult {
        result: Box::new(RoleExecutionResult {
            role: invocation.role,
            invocation_digest: invocation.invocation_digest.clone(),
            context_id,
            lifecycle,
            outcome,
            result_digest,
        }),
    }
}

fn code_reviewer_frontier(
    engine: &HarnessEngine,
    resolved: &ResolvedHarnessRequest,
    prepared: &PreparedRoleRun,
    context_suffix: &str,
) -> (HarnessExecutionStep, RoleInvocationContract) {
    let begin = engine
        .begin_execution(resolved, prepared, &[])
        .expect("code execution must begin");
    let writer_event = completed_role_event(
        resolved,
        prepared,
        &begin.ready_role_invocations[0],
        Vec::new(),
        context_suffix,
    );
    let frontier = engine
        .advance_execution(resolved, prepared, &[], &begin.record, writer_event)
        .expect("Writer result must advance to the review frontier");
    let reviewer = frontier
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Reviewer)
        .expect("Reviewer must be ready")
        .clone();
    (frontier, reviewer)
}

fn missing_context_request_mut(event: &mut HarnessExecutionEvent) -> &mut MissingContextRequest {
    let HarnessExecutionEvent::RoleResult { result } = event else {
        panic!("missing-context event must contain a role result");
    };
    let RoleExecutionOutcome::MissingContext { request } = &mut result.outcome else {
        panic!("role result must request missing context");
    };
    request
}

fn refresh_role_event_digest(event: &mut HarnessExecutionEvent) {
    let HarnessExecutionEvent::RoleResult { result } = event else {
        panic!("event must contain a role result");
    };
    result.result_digest = serialized_digest(&(
        result.role,
        &result.invocation_digest,
        &result.context_id,
        &result.lifecycle,
        &result.outcome,
    ))
    .expect("role event must serialize");
}

fn curation_create_content_mut(event: &mut HarnessExecutionEvent) -> &mut String {
    let HarnessExecutionEvent::RoleResult { result } = event else {
        panic!("Writer event must contain a role result");
    };
    let RoleExecutionOutcome::Completed {
        result:
            CompletedRoleResult::Writer {
                artifact: WriterArtifact::Curation { entries, .. },
            },
    } = &mut result.outcome
    else {
        panic!("learning Writer must produce curation entries");
    };
    let FileChange::Create { content, .. } = &mut entries[0].change else {
        panic!("learning curation must create a new document");
    };
    content
}

fn refresh_role_result_digest(result: &mut RoleExecutionResult) {
    result.result_digest = serialized_digest(&(
        result.role,
        &result.invocation_digest,
        &result.context_id,
        &result.lifecycle,
        &result.outcome,
    ))
    .expect("role result must serialize");
}

fn refresh_role_execution_record_digest(record: &mut RoleExecutionRecord) {
    record.execution_record_digest = serialized_digest(&(
        record.version,
        &record.resolved_plan_digest,
        &record.prepared_role_run_digest,
        &record.revision_contract,
        &record.role_results,
        &record.accepted_role_order,
        record.tool_evidence_after_accepted_roles,
        &record.tool_evidence,
    ))
    .expect("role execution record must serialize");
}

fn refresh_role_invocation_digest(invocation: &mut RoleInvocationContract) {
    invocation.invocation_digest = serialized_digest(&(
        invocation.version,
        &invocation.resolved_plan_digest,
        &invocation.prepared_role_run_digest,
        &invocation.revision_contract_digest,
        invocation.role,
        &invocation.role_input_digest,
        &invocation.predecessor_results,
        &invocation.subject,
        &invocation.segments,
        invocation.total_context_bytes,
    ))
    .expect("role invocation must serialize");
}

fn assert_invocation_dynamic_context(
    invocation: &RoleInvocationContract,
    revision_contract: Option<&RevisionContract>,
) {
    let dynamic_segments = invocation
        .segments
        .iter()
        .filter_map(|segment| match segment {
            RoleInvocationSegment::DynamicContext {
                content,
                content_digest,
            } => Some((content, content_digest)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let [(content, content_digest)] = dynamic_segments.as_slice() else {
        panic!("invocation must contain exactly one dynamic context");
    };
    assert_eq!(**content_digest, byte_digest(content.as_bytes()));
    let dynamic: serde_json::Value =
        decode_current_json(content.as_bytes(), "role invocation dynamic context")
            .expect("dynamic context must be canonical JSON");
    assert_eq!(
        dynamic,
        serde_json::json!({
            "predecessor_results": invocation.predecessor_results,
            "subject": invocation.subject,
            "revision_contract": revision_contract,
        })
    );
}

fn invocation_with_changed_correction(
    prepared: &PreparedRoleRun,
    invocation: &RoleInvocationContract,
    revision_contract: &RevisionContract,
) -> RoleInvocationContract {
    let mut forged_contract = revision_contract.clone();
    let Some(RevisionCorrection::FailedRequirement { detail, .. }) =
        forged_contract.corrections.first_mut()
    else {
        panic!("revised Writer fixture must contain a failed requirement");
    };
    *detail = "forged correction detail".to_owned();
    forged_contract.revision_contract_digest = serialized_digest(&(
        forged_contract.revision,
        &forged_contract.previous_candidate_digest,
        &forged_contract.previous_task_evaluation_receipt_digest,
        &forged_contract.corrections,
    ))
    .expect("forged correction contract must serialize");
    let original =
        serde_json::to_string(revision_contract).expect("Core correction contract must serialize");
    let replacement =
        serde_json::to_string(&forged_contract).expect("forged correction contract must serialize");
    let mut changed = invocation.clone();
    let (content, content_digest) = changed
        .segments
        .iter_mut()
        .find_map(|segment| match segment {
            RoleInvocationSegment::DynamicContext {
                content,
                content_digest,
            } => Some((content, content_digest)),
            _ => None,
        })
        .expect("Writer invocation must contain a dynamic context");
    assert_eq!(content.matches(&original).count(), 1);
    *content = content.replacen(&original, &replacement, 1);
    *content_digest = byte_digest(content.as_bytes());
    changed.total_context_bytes = changed
        .total_context_bytes
        .checked_sub(original.len())
        .and_then(|bytes| bytes.checked_add(replacement.len()))
        .expect("changed correction context size must be representable");
    let metadata = prepared
        .role_metadata
        .iter()
        .find(|metadata| metadata.role == invocation.role)
        .expect("Writer metadata must exist");
    changed.role_input_digest =
        serialized_digest(&(metadata, &changed.segments, changed.total_context_bytes))
            .expect("changed role input must serialize");
    changed.revision_contract_digest = forged_contract.revision_contract_digest;
    refresh_role_invocation_digest(&mut changed);
    changed
}

fn assert_invocation_predecessor_results_rejected(
    plan: &ResolvedHarnessPlan,
    prepared: &PreparedRoleRun,
    record: &RoleExecutionRecord,
    invocation: &RoleInvocationContract,
) {
    assert!(validate_invocation_predecessor_results(plan, prepared, record, invocation).is_err());
}

fn invocation_with_changed_writer_content(
    invocation: &RoleInvocationContract,
) -> RoleInvocationContract {
    let mut changed = invocation.clone();
    let producer = changed
        .predecessor_results
        .iter_mut()
        .find(|result| result.role == HarnessRole::Writer)
        .expect("downstream invocation must contain the Writer result");
    let RoleExecutionOutcome::Completed {
        result:
            CompletedRoleResult::Writer {
                artifact: WriterArtifact::Changes { changes },
            },
    } = &mut producer.outcome
    else {
        panic!("fixture producer must contain Writer changes");
    };
    let FileChange::Update { content, .. } = &mut changes[0] else {
        panic!("fixture target must be updated");
    };
    *content = "forged candidate".to_owned();
    refresh_role_result_digest(producer);
    refresh_role_invocation_digest(&mut changed);
    changed
}

fn refresh_apply_receipt_digests(receipt: &mut HarnessApplyReceipt) {
    receipt.batch_receipt_digest =
        serialized_digest(&receipt.applied_batch).expect("applied batch must serialize");
    receipt.finalization_digest = serialized_digest(&(
        &receipt.finalization_targets,
        &receipt.final_workspace,
        &receipt.context_commit_receipt,
        &receipt.applied_batch,
        receipt.recovered,
        &receipt.frozen_identity_set_digest,
        &receipt.workspace_locator_map_digest,
    ))
    .expect("finalization evidence must serialize");
    receipt.apply_receipt_digest = serialized_digest(&(
        receipt.version,
        &receipt.harness_plan_digest,
        &receipt.prepared_run_digest,
        &receipt.candidate_digest,
        &receipt.validation_receipt_digest,
        &receipt.finalization_digest,
        &receipt.batch_receipt_digest,
        &receipt.batch_journal_relative_path,
        receipt.recovered,
        &receipt.frozen_identity_set_digest,
        &receipt.workspace_locator_map_digest,
        &receipt.applied_batch,
        &receipt.finalization_targets,
        &receipt.final_workspace,
        &receipt.context_commit_receipt,
    ))
    .expect("apply receipt must serialize");
}

fn complete_execution_record(
    engine: &HarnessEngine,
    resolved: &ResolvedHarnessRequest,
    prepared: &PreparedRoleRun,
    reviewer_findings: &[BlockingFinding],
    context_suffix: &str,
) -> RoleExecutionRecord {
    complete_execution_record_with_improvements(
        engine,
        resolved,
        prepared,
        reviewer_findings,
        &[],
        context_suffix,
    )
}

fn complete_execution_record_with_improvements(
    engine: &HarnessEngine,
    resolved: &ResolvedHarnessRequest,
    prepared: &PreparedRoleRun,
    reviewer_findings: &[BlockingFinding],
    reviewer_improvements: &[&str],
    context_suffix: &str,
) -> RoleExecutionRecord {
    let step = engine
        .begin_execution(resolved, prepared, &[])
        .expect("execution must begin");
    complete_execution_step_with_improvements(
        engine,
        resolved,
        prepared,
        &[],
        step,
        (reviewer_findings, reviewer_improvements),
        context_suffix,
    )
}

fn complete_execution_step_with_improvements(
    engine: &HarnessEngine,
    resolved: &ResolvedHarnessRequest,
    prepared: &PreparedRoleRun,
    revision_history: &[HarnessTaskEvaluation],
    mut step: HarnessExecutionStep,
    reviewer_observations: (&[BlockingFinding], &[&str]),
    context_suffix: &str,
) -> RoleExecutionRecord {
    let (reviewer_findings, reviewer_improvements) = reviewer_observations;
    loop {
        if let Some(invocation) = step.ready_role_invocations.first().cloned() {
            let findings = if invocation.role == HarnessRole::Reviewer {
                reviewer_findings.to_owned()
            } else {
                Vec::new()
            };
            let mut event =
                completed_role_event(resolved, prepared, &invocation, findings, context_suffix);
            if invocation.role == HarnessRole::Reviewer && !reviewer_improvements.is_empty() {
                let HarnessExecutionEvent::RoleResult { result } = &mut event else {
                    panic!("Reviewer event must contain a role result");
                };
                let RoleExecutionOutcome::Completed {
                    result:
                        CompletedRoleResult::Reviewer {
                            requirement_results,
                            improvements,
                            ..
                        },
                } = &mut result.outcome
                else {
                    panic!("Reviewer result must be completed");
                };
                for message in reviewer_improvements {
                    improvements.push(reviewer_improvement_observation(
                        resolved,
                        &invocation.subject,
                        message,
                    ));
                }
                if matches!(
                    resolved.plan.intent,
                    HarnessIntent::CareerArtifact {
                        surface: CareerOutputSurface::ProfessionalProfile
                    }
                ) {
                    let result = requirement_results
                        .iter_mut()
                        .find(|result| result.unit == VerificationUnit::ProfessionalProfileArtifact)
                        .expect("professional profile Reviewer must own its artifact requirement");
                    result.passed = false;
                    result.detail =
                        "professional-profile improvements remain unresolved".to_owned();
                }
                refresh_role_event_digest(&mut event);
            }
            step = engine
                .advance_execution(resolved, prepared, revision_history, &step.record, event)
                .expect("ready role result must advance");
            continue;
        }
        if let Some(invocation) = step.ready_tool_invocation.clone() {
            step = engine
                .advance_execution(
                    resolved,
                    prepared,
                    revision_history,
                    &step.record,
                    HarnessExecutionEvent::ToolEvidence {
                        evidence: passed_tool_evidence_set(&invocation),
                    },
                )
                .expect("ready tool evidence must advance");
            continue;
        }
        return step.record;
    }
}

fn passed_tool_evidence_set(invocation: &ToolInvocationContract) -> ToolEvidenceSet {
    let executions = invocation
        .requirements
        .iter()
        .map(|requirement| {
            let command = vec![
                "harness-verifier".to_owned(),
                requirement.unit.as_str().to_owned(),
            ];
            let termination = ToolTermination::Exited { code: 0 };
            let stdout_digest = serialized_digest(&(requirement.unit, "tool standard output"))
                .expect("tool standard output digest must serialize");
            let stderr_digest = serialized_digest(&(requirement.unit, ""))
                .expect("tool standard error digest must serialize");
            let evidence_digest = serialized_digest(&(
                &invocation.invocation_digest,
                requirement.unit,
                &command,
                &termination,
                &stdout_digest,
                &stderr_digest,
            ))
            .expect("tool execution evidence must serialize");
            ToolCommandEvidence {
                unit: requirement.unit,
                command,
                termination,
                stdout_digest,
                stderr_digest,
                evidence_digest,
            }
        })
        .collect::<Vec<_>>();
    let results = executions
        .iter()
        .map(|execution| RequirementResult {
            unit: execution.unit,
            passed: true,
            detail: "tool passed".to_owned(),
            evidence: vec![ResultEvidenceReference::ToolResult {
                unit: execution.unit,
                result_digest: execution.evidence_digest.clone(),
            }],
        })
        .collect::<Vec<_>>();
    let evidence_set_digest =
        serialized_digest(&(&invocation.invocation_digest, &results, &executions))
            .expect("tool evidence must serialize");
    ToolEvidenceSet {
        invocation_digest: invocation.invocation_digest.clone(),
        results,
        executions,
        evidence_set_digest,
    }
}

fn passed_tool_event(invocation: &ToolInvocationContract) -> HarnessExecutionEvent {
    HarnessExecutionEvent::ToolEvidence {
        evidence: passed_tool_evidence_set(invocation),
    }
}

fn career_review_request(targets: Vec<String>) -> HarnessRequest {
    HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Personal,
        targets,
        objective: "review a frozen career artifact set".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    }
}

fn selected_career_manifest(
    targets: Vec<String>,
    owner_sources: Vec<(DataOwner, Vec<String>)>,
) -> CareerCompositionManifest {
    CareerCompositionManifest {
        version: HARNESS_SCHEMA_VERSION,
        career_output_surface: CareerOutputSurface::Resume,
        artifact_targets: targets,
        coverage: CareerCoverageMode::Selected,
        complete_coverage_confirmation_reported: false,
        evidence_owners: owner_sources
            .iter()
            .map(|(owner, _)| owner.clone())
            .collect(),
        canonical_evidence_owners: Vec::new(),
        excluded_evidence_owners: Vec::new(),
        claim_lineage: owner_sources
            .into_iter()
            .enumerate()
            .map(
                |(index, (evidence_owner, evidence_source_paths))| CareerClaimLineage {
                    claim_id: format!("claim-{index}"),
                    evidence_owner,
                    evidence_source_paths,
                },
            )
            .collect(),
    }
}

fn career_review_receipt(
    engine: &HarnessEngine,
    request: &HarnessRequest,
    manifest: &CareerCompositionManifest,
    grant: Option<ContextGrant>,
    evidence_source_paths: &[String],
    context_suffix: &str,
) -> CareerReviewReceipt {
    let grants = grant.into_iter().collect::<Vec<_>>();
    let resolved = engine
        .resolve_with_career_composition(
            request,
            &grants,
            Some(CareerOutputSurface::Resume),
            evidence_source_paths,
            Some(manifest),
        )
        .expect("career review plan must resolve");
    let prepared = prepared_run_with(engine, &resolved);
    let mut submission = complete_submission_with(engine, &resolved);
    fill_reported_role_contexts(&mut submission, &resolved.plan, context_suffix);
    engine
        .attest_career_review(&resolved, &prepared, &submission)
        .expect("career review must attest")
}

fn career_execution_review_receipt(
    engine: &HarnessEngine,
    request: &HarnessRequest,
    manifest: &CareerCompositionManifest,
    grant: Option<ContextGrant>,
    evidence_source_paths: &[String],
    context_suffix: &str,
) -> CareerExecutionReviewReceipt {
    let grants = grant.into_iter().collect::<Vec<_>>();
    let resolved = engine
        .resolve_with_career_composition(
            request,
            &grants,
            Some(CareerOutputSurface::Resume),
            evidence_source_paths,
            Some(manifest),
        )
        .expect("career execution review plan must resolve");
    let prepared = prepared_run_with(engine, &resolved);
    let record = complete_execution_record(engine, &resolved, &prepared, &[], context_suffix);
    engine
        .attest_career_execution_review(&resolved, &prepared, &record)
        .expect("career execution review must attest")
}

fn personal_scope_request(action: HarnessAction) -> HarnessRequest {
    let is_code = matches!(action, HarnessAction::CodeWrite | HarnessAction::CodeReview);
    HarnessRequest {
        action,
        owner: if is_code {
            DataOwner::PersonalProject {
                project: "coupler".to_owned(),
            }
        } else {
            DataOwner::Personal
        },
        targets: match action {
            HarnessAction::CodeWrite | HarnessAction::CodeReview => vec!["src/lib.rs".to_owned()],
            HarnessAction::DocumentWrite | HarnessAction::DocumentReview => {
                vec!["README.md".to_owned()]
            }
            HarnessAction::Investigation | HarnessAction::Design => vec!["README.md".to_owned()],
            HarnessAction::Ideation | HarnessAction::VaultRead => Vec::new(),
            HarnessAction::VaultCuration => {
                vec!["vault/personal/knowledge/skills.md".to_owned()]
            }
        },
        objective: format!("exercise the {} personal contract", action.as_str()),
        curation_kind: matches!(
            action,
            HarnessAction::VaultRead | HarnessAction::VaultCuration
        )
        .then_some(CurationKind::Knowledge),
        explicit_user_confirmation_reported: action == HarnessAction::VaultCuration,
        curation_sources: if action == HarnessAction::VaultCuration {
            vec!["vault/personal/index.md".to_owned()]
        } else {
            Vec::new()
        },
        delete_targets: Vec::new(),
    }
}

fn resolve_personal_scope_action(
    engine: &HarnessEngine,
    request: &HarnessRequest,
) -> ResolvedHarnessRequest {
    if request.action == HarnessAction::VaultRead {
        resolve_vault_read(
            engine,
            request,
            HarnessExecutionProfile::Standard,
            "skills",
            64 * 1024,
        )
    } else {
        engine
            .resolve(request)
            .expect("personal action contract must resolve")
    }
}

fn assert_ai_collaboration_values_bound(engine: &HarnessEngine, resolved: &ResolvedHarnessRequest) {
    assert!(resolved.plan.required_policies.iter().any(|policy| {
        policy.id == "ai-collaboration-values"
            && policy.repository_relative_path == AI_COLLABORATION_VALUES_PATH
    }));
    assert!(resolved.plan.role_policy_bindings.iter().all(|binding| {
        binding.policies.iter().any(|policy| {
            policy.id == "ai-collaboration-values"
                && policy.repository_relative_path == AI_COLLABORATION_VALUES_PATH
        })
    }));
    let context_bundle = (resolved.plan.action == HarnessAction::VaultRead).then(|| {
        engine
            .retrieve_context(resolved, "skills", 64 * 1024)
            .expect("personal Vault read context must resolve")
    });
    let prepared = engine
        .prepare(
            resolved,
            &complete_runtime_capabilities(),
            context_bundle.as_ref(),
        )
        .expect("personal action role input must prepare");
    assert!(prepared.role_bundles.iter().all(|bundle| {
        bundle.bound_documents().any(|document| {
            document.source == HarnessBoundDocumentSource::Policy
                && document.relative_path == AI_COLLABORATION_VALUES_PATH
        })
    }));
}

fn assert_ai_collaboration_values_not_bound(
    engine: &HarnessEngine,
    resolved: &ResolvedHarnessRequest,
) {
    assert!(
        resolved
            .plan
            .required_policies
            .iter()
            .all(|policy| policy.id != "ai-collaboration-values")
    );
    let prepared = prepared_run_with(engine, resolved);
    assert!(prepared.role_bundles.iter().all(|bundle| {
        bundle
            .bound_documents()
            .all(|document| document.relative_path != AI_COLLABORATION_VALUES_PATH)
    }));
}

#[test]
fn every_personal_scope_action_binds_ai_collaboration_values_to_each_role() {
    let (_workspace, external) = external_engine();
    let vault = engine();
    for action in [
        HarnessAction::CodeWrite,
        HarnessAction::DocumentWrite,
        HarnessAction::CodeReview,
        HarnessAction::DocumentReview,
        HarnessAction::Investigation,
        HarnessAction::Design,
        HarnessAction::Ideation,
        HarnessAction::VaultRead,
        HarnessAction::VaultCuration,
    ] {
        let request = personal_scope_request(action);
        let current_engine = if matches!(
            action,
            HarnessAction::VaultRead | HarnessAction::VaultCuration
        ) {
            &vault
        } else {
            &external
        };
        let resolved = resolve_personal_scope_action(current_engine, &request);
        assert_ai_collaboration_values_bound(current_engine, &resolved);
    }
}

#[test]
fn every_execution_profile_binds_common_review_quality_only_to_review_roles() {
    for execution_profile in [
        HarnessExecutionProfile::Standard,
        HarnessExecutionProfile::Strict,
    ] {
        for action in [
            HarnessAction::CodeWrite,
            HarnessAction::DocumentWrite,
            HarnessAction::CodeReview,
            HarnessAction::DocumentReview,
            HarnessAction::Investigation,
            HarnessAction::Design,
            HarnessAction::Ideation,
            HarnessAction::VaultRead,
            HarnessAction::VaultCuration,
        ] {
            let request = personal_scope_request(action);
            if matches!(
                action,
                HarnessAction::VaultRead | HarnessAction::VaultCuration
            ) {
                let engine = match execution_profile {
                    HarnessExecutionProfile::Standard => engine(),
                    HarnessExecutionProfile::Strict => strict_engine(),
                };
                let resolved = if action == HarnessAction::VaultRead {
                    resolve_vault_read(&engine, &request, execution_profile, "skills", 64 * 1024)
                } else {
                    engine
                        .resolve(&request)
                        .expect("Vault curation contract must resolve")
                };
                assert_common_review_quality_binding(&engine, &resolved);
            } else {
                let (_workspace, engine) = external_engine_with_profile(execution_profile);
                let resolved = resolve_personal_scope_action(&engine, &request);
                assert_common_review_quality_binding(&engine, &resolved);
            }
        }
    }
}

fn assert_common_review_quality_binding(engine: &HarnessEngine, resolved: &ResolvedHarnessRequest) {
    let policy_path = "vault/profile/rules/common-review-quality.md";
    let has_planned_review_role = resolved
        .plan
        .role_policy_bindings
        .iter()
        .any(|binding| matches!(binding.role, HarnessRole::Verifier | HarnessRole::Reviewer));
    let is_required = resolved.plan.required_policies.iter().any(|policy| {
        policy.id == "common-review-quality" && policy.repository_relative_path == policy_path
    });
    assert_eq!(
        is_required, has_planned_review_role,
        "review policy requirement must match the planned roles for {:?} under {:?}",
        resolved.plan.action, resolved.plan.execution_profile
    );

    for binding in &resolved.plan.role_policy_bindings {
        let should_receive = matches!(binding.role, HarnessRole::Verifier | HarnessRole::Reviewer);
        let receives = binding.policies.iter().any(|policy| {
            policy.id == "common-review-quality" && policy.repository_relative_path == policy_path
        });
        assert_eq!(
            receives, should_receive,
            "review policy binding must match the role for {:?} under {:?}",
            resolved.plan.action, resolved.plan.execution_profile
        );
    }

    let context_bundle = (resolved.plan.action == HarnessAction::VaultRead).then(|| {
        engine
            .retrieve_context(resolved, "skills", 64 * 1024)
            .expect("Vault read context must resolve")
    });
    let prepared = engine
        .prepare(
            resolved,
            &complete_runtime_capabilities(),
            context_bundle.as_ref(),
        )
        .expect("review quality role input must prepare");
    for (metadata, bundle) in prepared
        .role_metadata
        .iter()
        .zip(prepared.role_bundles.iter())
    {
        let should_receive = matches!(metadata.role, HarnessRole::Verifier | HarnessRole::Reviewer);
        let receives = bundle.bound_documents().any(|document| {
            document.source == HarnessBoundDocumentSource::Policy
                && document.relative_path == policy_path
        });
        assert_eq!(
            receives, should_receive,
            "prepared review policy must match the role for {:?} under {:?}",
            resolved.plan.action, resolved.plan.execution_profile
        );
    }
}

#[test]
fn every_personal_owner_variant_selects_the_ai_collaboration_values_policy() {
    let (_workspace, engine) = external_engine();
    for owner in [
        DataOwner::PersonalBusiness,
        DataOwner::PersonalProject {
            project: "coupler".to_owned(),
        },
    ] {
        let request = HarnessRequest {
            action: HarnessAction::DocumentWrite,
            owner,
            targets: vec!["README.md".to_owned()],
            objective: "exercise a personal owner policy boundary".to_owned(),
            curation_kind: None,
            explicit_user_confirmation_reported: false,
            curation_sources: Vec::new(),
            delete_targets: Vec::new(),
        };
        let resolved = engine
            .resolve(&request)
            .expect("personal owner policy boundary must resolve");
        assert_ai_collaboration_values_bound(&engine, &resolved);
    }
}

#[test]
fn profile_and_company_owners_do_not_receive_personal_ai_collaboration_values() {
    let profile_engine = engine();
    let profile = profile_engine
        .resolve(&profile_code_request())
        .expect("profile request must resolve");
    assert_ai_collaboration_values_not_bound(&profile_engine, &profile);

    let (_workspace, company_engine) = external_engine();
    let company_request = HarnessRequest {
        action: HarnessAction::CodeReview,
        owner: DataOwner::CompanyProject {
            company: "cluml".to_owned(),
            project: "giganto".to_owned(),
        },
        targets: vec!["src/lib.rs".to_owned()],
        objective: "review a company change".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let company = company_engine
        .resolve(&company_request)
        .expect("company request must resolve");
    assert_ai_collaboration_values_not_bound(&company_engine, &company);
}

#[test]
fn personal_code_uses_common_quality_policy_and_denies_work() {
    let (_workspace, engine) = external_engine();
    let resolved = engine
        .resolve(&personal_project_code_request())
        .expect("personal code request must resolve");
    assert!(
        resolved
            .plan
            .required_policies
            .iter()
            .any(|policy| policy.id == "common-code-quality")
    );
    assert!(resolved.plan.orchestrator_policies.iter().any(|policy| {
        policy.repository_relative_path == "vault/profile/rules/mandatory-preflight.md"
    }));
    assert!(resolved.plan.role_policy_bindings.iter().all(|binding| {
        binding
            .policies
            .iter()
            .all(|policy| policy.id != "mandatory-preflight")
    }));
    assert!(
        resolved
            .plan
            .required_policies
            .iter()
            .any(|policy| policy.id == "rust-code-style")
    );
    assert!(!resolved.plan.required_policies.iter().any(|policy| {
        policy
            .repository_relative_path
            .starts_with("vault/work/cluml/")
            || policy
                .repository_relative_path
                .starts_with("vault/work/tmaxcloud/")
    }));
    assert!(
        !resolved
            .plan
            .allowed_context_roots
            .iter()
            .any(|root| { root.repository_relative_path.starts_with("vault/work") })
    );
    assert!(
        resolved
            .plan
            .denied_context_roots
            .iter()
            .any(|root| root.repository_relative_path == "vault/work")
    );
    assert!(
        resolved
            .plan
            .denied_context_roots
            .iter()
            .any(|root| root.repository_relative_path == "vault/personal/journal")
    );
    assert!(!resolved.plan.allowed_context_roots.iter().any(|root| {
        matches!(
            root.repository_relative_path.as_str(),
            "vault/personal/facts" | "vault/personal/knowledge"
        )
    }));
}

#[test]
fn engineering_actions_bind_common_quality_to_every_role_without_company_context() {
    let (_workspace, engine) = external_engine();
    for action in [
        HarnessAction::CodeWrite,
        HarnessAction::CodeReview,
        HarnessAction::Design,
    ] {
        let request = personal_scope_request(action);
        let resolved = resolve_personal_scope_action(&engine, &request);

        assert!(resolved.plan.role_policy_bindings.iter().all(|binding| {
            binding
                .policies
                .iter()
                .any(|policy| policy.id == "common-code-quality")
        }));
        assert!(resolved.plan.required_policies.iter().all(|policy| {
            !policy.repository_relative_path.starts_with("vault/work/")
                || policy
                    .repository_relative_path
                    .starts_with("vault/work/common/")
        }));
        assert!(
            resolved
                .plan
                .denied_context_roots
                .iter()
                .any(|root| root.repository_relative_path == "vault/work")
        );
        assert!(
            !resolved
                .plan
                .allowed_context_roots
                .iter()
                .any(|root| root.repository_relative_path.starts_with("vault/work"))
        );
    }
}

#[test]
fn company_route_binds_only_selected_company_root() {
    let (_workspace, engine) = external_engine();
    let request = HarnessRequest {
        action: HarnessAction::CodeReview,
        owner: DataOwner::CompanyProject {
            company: "cluml".to_owned(),
            project: "giganto".to_owned(),
        },
        targets: vec!["src/lib.rs".to_owned()],
        objective: "review a change".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = engine
        .resolve(&request)
        .expect("known company must resolve");
    let roots = resolved
        .plan
        .allowed_context_roots
        .iter()
        .map(|root| root.repository_relative_path.as_str())
        .collect::<Vec<_>>();
    assert!(roots.contains(&"vault/work/cluml/projects/giganto.md"));
    assert!(!roots.contains(&"vault/work/cluml"));
    assert!(!roots.contains(&"vault/work/tmaxcloud"));
    assert!(
        !resolved
            .plan
            .denied_context_roots
            .iter()
            .any(|root| { root.repository_relative_path.starts_with("vault/work/") })
    );
}

#[test]
fn cluml_domain_signals_bind_the_router_and_selected_policy_to_every_role() {
    let (_workspace, engine) = external_engine();
    for (objective, target, expected_policy) in [
        (
            "review jwt trust boundaries",
            "src/lib.rs",
            "vault/work/cluml/rules/aice-aimer-auth-data-flow.md",
        ),
        (
            "review audit retention",
            "src/lib.rs",
            "vault/work/cluml/rules/security-architecture-principles.md",
        ),
        (
            "review certificate renewal",
            "src/lib.rs",
            "vault/work/cluml/rules/cert.md",
        ),
        (
            "review dependency versions",
            "requirements.txt",
            "vault/work/common/rules/dependency.md",
        ),
    ] {
        for action in [
            HarnessAction::CodeReview,
            HarnessAction::DocumentWrite,
            HarnessAction::DocumentReview,
        ] {
            let target = if action == HarnessAction::CodeReview {
                target
            } else {
                "README.md"
            };
            let request = HarnessRequest {
                action,
                owner: DataOwner::CompanyProject {
                    company: "cluml".to_owned(),
                    project: "giganto".to_owned(),
                },
                targets: vec![target.to_owned()],
                objective: objective.to_owned(),
                curation_kind: None,
                explicit_user_confirmation_reported: false,
                curation_sources: Vec::new(),
                delete_targets: Vec::new(),
            };
            let resolved = engine
                .resolve(&request)
                .expect("ClumL domain route must resolve");

            for binding in &resolved.plan.role_policy_bindings {
                assert!(binding.policies.iter().any(|policy| {
                    policy.repository_relative_path == "vault/profile/rules/common-code-quality.md"
                }));
                assert!(binding.policies.iter().any(|policy| {
                    policy.id == "company-routing"
                        && policy.repository_relative_path
                            == "vault/work/cluml/preferences/cluml-routing.md"
                }));
                assert!(
                    binding
                        .policies
                        .iter()
                        .any(|policy| policy.repository_relative_path == expected_policy)
                );
            }
            let prepared = prepared_run_with(&engine, &resolved);
            for bundle in &prepared.role_bundles {
                for path in [
                    expected_policy,
                    "vault/profile/rules/common-code-quality.md",
                ] {
                    assert!(bundle.bound_documents().any(|document| {
                        document.source == HarnessBoundDocumentSource::Policy
                            && document.relative_path == path
                    }));
                }
            }
        }
    }
}

#[test]
fn company_documents_without_domain_signals_do_not_bind_engineering_policies() {
    let (_workspace, engine) = external_engine();
    for action in [HarnessAction::DocumentWrite, HarnessAction::DocumentReview] {
        let request = HarnessRequest {
            action,
            owner: DataOwner::Company {
                company: "cluml".to_owned(),
            },
            targets: vec!["README.md".to_owned()],
            objective: "edit the team meeting agenda".to_owned(),
            curation_kind: None,
            explicit_user_confirmation_reported: false,
            curation_sources: Vec::new(),
            delete_targets: Vec::new(),
        };
        let resolved = engine
            .resolve(&request)
            .expect("company document must resolve");
        assert!(!resolved.plan.required_policies.iter().any(|policy| {
            policy.id.starts_with("company-domain-rule-") || policy.id == "common-code-quality"
        }));
    }
}

#[test]
fn company_route_ascii_signals_require_word_boundaries() {
    assert!(contains_company_route_signal(
        "review aice-web certificate handling",
        "aice"
    ));
    assert!(!contains_company_route_signal(
        "review concurrent processing",
        "CN"
    ));
}

#[test]
fn cluml_issue_hunting_investigation_binds_its_full_policy_bundle() {
    let (_workspace, engine) = external_engine();
    let expected = [
        "vault/work/cluml/rules/rust-issue-hunting.md",
        "vault/profile/rules/common-code-quality.md",
        "vault/work/common/rules/code-review.md",
        "vault/work/common/rules/rust-code-style.md",
        "vault/work/common/rules/issue.md",
    ];
    for signal in ["bug", "refactor", "performance", "stability"] {
        let request = HarnessRequest {
            action: HarnessAction::Investigation,
            owner: DataOwner::CompanyProject {
                company: "cluml".to_owned(),
                project: "giganto".to_owned(),
            },
            targets: vec!["src/lib.rs".to_owned()],
            objective: format!("find a {signal} candidate"),
            curation_kind: None,
            explicit_user_confirmation_reported: false,
            curation_sources: Vec::new(),
            delete_targets: Vec::new(),
        };
        let resolved = engine
            .resolve(&request)
            .expect("ClumL issue hunting investigation must resolve");
        for binding in &resolved.plan.role_policy_bindings {
            for expected_policy in expected {
                assert!(
                    binding
                        .policies
                        .iter()
                        .any(|policy| policy.repository_relative_path == expected_policy)
                );
            }
        }
        let prepared = prepared_run_with(&engine, &resolved);
        for bundle in &prepared.role_bundles {
            for expected_policy in expected {
                assert!(bundle.bound_documents().any(|document| {
                    document.source == HarnessBoundDocumentSource::Policy
                        && document.relative_path == expected_policy
                }));
            }
        }
    }
}

#[test]
fn issue_hunting_short_signals_do_not_overbind_a_code_review() {
    let (_workspace, engine) = external_engine();
    let request = HarnessRequest {
        action: HarnessAction::CodeReview,
        owner: DataOwner::CompanyProject {
            company: "cluml".to_owned(),
            project: "giganto".to_owned(),
        },
        targets: vec!["src/lib.rs".to_owned()],
        objective: "review a bug fix for performance and stability".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };

    let resolved = engine
        .resolve(&request)
        .expect("ordinary ClumL code review must resolve");
    assert!(!resolved.plan.required_policies.iter().any(|policy| {
        policy.repository_relative_path == "vault/work/cluml/rules/rust-issue-hunting.md"
    }));
}

#[test]
fn company_route_search_text_strips_the_vault_repository_prefix() {
    let request = HarnessRequest {
        action: HarnessAction::Investigation,
        owner: DataOwner::CompanyProject {
            company: "cluml".to_owned(),
            project: "giganto".to_owned(),
        },
        targets: vec!["vault/work/cluml/projects/giganto.md".to_owned()],
        objective: "review the project routing".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };

    let searchable = company_route_searchable(&request, "cluml");
    assert!(!contains_company_route_signal(&searchable, "vault"));
    assert!(contains_company_route_signal(&searchable, "giganto"));
}

#[test]
fn company_domain_routes_reject_indexes_routers_and_unknown_shared_policies() {
    for policy in [
        "vault/work/cluml/index.md",
        "vault/work/cluml/preferences/cluml-routing.md",
        "vault/work/common/rules/design-discussion-note.md",
    ] {
        let content = format!(
            "---\ndomain_routes:\n  - signals: [sample]\n    actions: [code-review]\n    target_kind: any\n    policies: [\"{policy}\"]\n---\n# Routing\n"
        );
        assert!(
            parse_company_domain_routes(&content, "cluml").is_err(),
            "non-leaf or unsupported shared policy must be rejected: {policy}"
        );
    }
}

#[test]
fn company_domain_routes_require_unique_explicit_actions() {
    let missing = "---\ndomain_routes:\n  - signals: [sample]\n    target_kind: any\n    policies: [\"vault/work/cluml/rules/cert.md\"]\n---\n# Routing\n";
    assert!(parse_company_domain_routes(missing, "cluml").is_err());

    let repeated = "---\ndomain_routes:\n  - signals: [sample]\n    actions: [code-review, code-review]\n    target_kind: any\n    policies: [\"vault/work/cluml/rules/cert.md\"]\n---\n# Routing\n";
    let error = parse_company_domain_routes(repeated, "cluml")
        .expect_err("duplicate route actions must be rejected");
    assert!(error.to_string().contains("repeats an action"));
}

#[test]
fn personal_career_work_can_bind_one_read_only_company_evidence_grant() {
    let (_workspace, engine) = external_engine();
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Personal,
        targets: vec!["README.md".to_owned()],
        objective: "review career writing about jwt authentication against company evidence"
            .to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let grant = ContextGrant {
        owner: DataOwner::Company {
            company: "cluml".to_owned(),
        },
        purpose: ContextGrantPurpose::CareerWritingEvidence,
        access: ContextAccess::ReadOnly,
    };

    let resolved = engine
        .resolve_with_context_grants(&request, std::slice::from_ref(&grant))
        .expect("explicit company evidence grant must resolve");

    assert_eq!(resolved.plan.context_grants, vec![grant]);
    assert_eq!(
        resolved.plan.intent,
        HarnessIntent::CareerArtifact {
            surface: CareerOutputSurface::General,
        }
    );
    assert!(
        resolved
            .plan
            .allowed_context_roots
            .iter()
            .any(|root| { root.repository_relative_path == "vault/work/cluml/index.md" })
    );
    assert!(!resolved.plan.allowed_context_roots.iter().any(|root| {
        root.repository_relative_path
            .starts_with("vault/work/tmaxcloud")
    }));
    assert!(
        !resolved
            .plan
            .denied_context_roots
            .iter()
            .any(|root| { root.repository_relative_path == "vault/work" })
    );
    assert!(resolved.plan.required_policies.iter().any(|policy| {
        policy.id == "granted-company-index"
            && policy.repository_relative_path == "vault/work/cluml/index.md"
    }));
    assert!(!resolved.plan.required_policies.iter().any(|policy| {
        policy.id.starts_with("company-domain-rule-") || policy.id == "common-code-quality"
    }));
    for policy_id in ["career-claim-token-output-system", "career-output-assembly"] {
        assert!(
            resolved
                .plan
                .required_policies
                .iter()
                .any(|policy| policy.id == policy_id),
            "career review policy must be bound: {policy_id}"
        );
    }
    assert!(has_verification(
        &resolved.plan,
        "career-output-surface-selection"
    ));
}

#[test]
fn exact_career_evidence_sources_are_hashed_and_sent_only_to_the_owner_roles() {
    let (_workspace, engine) = external_engine();
    let source =
        "vault/work/cluml/experience/case-studies/ci-runner-availability-monitoring.md".to_owned();
    let owner = DataOwner::Company {
        company: "cluml".to_owned(),
    };
    let manifest = selected_career_manifest(
        vec!["README.md".to_owned()],
        vec![(owner.clone(), vec![source.clone()])],
    );
    let request = career_review_request(vec!["README.md".to_owned()]);
    let grant = ContextGrant {
        owner,
        purpose: ContextGrantPurpose::CareerWritingEvidence,
        access: ContextAccess::ReadOnly,
    };
    let resolved = engine
        .resolve_with_career_composition(
            &request,
            &[grant],
            Some(CareerOutputSurface::Resume),
            std::slice::from_ref(&source),
            Some(&manifest),
        )
        .expect("exact evidence source must resolve");

    assert_eq!(resolved.plan.version, HARNESS_SCHEMA_VERSION);
    assert_eq!(resolved.plan.evidence_sources.len(), 1);
    assert_eq!(
        resolved.plan.evidence_sources[0].repository_relative_path,
        source
    );
    assert_eq!(resolved.plan.evidence_sources[0].content_digest.len(), 64);
    assert!(resolved.plan.allowed_context_roots.iter().any(|root| {
        root.repository_relative_path == resolved.plan.evidence_sources[0].repository_relative_path
    }));
    assert!(!resolved.plan.allowed_context_roots.iter().any(|root| {
        root.repository_relative_path == "vault/work/cluml/experience"
            || root.repository_relative_path == "vault/work/cluml/projects"
    }));

    let prepared = prepared_run_with(&engine, &resolved);
    for metadata in &prepared.role_metadata {
        assert_eq!(
            metadata.scope.evidence_sources,
            resolved.plan.evidence_sources
        );
        assert_eq!(metadata.scope.career_claim_lineage.len(), 1);
        assert_eq!(
            metadata.scope.career_manifest_digest,
            resolved.plan.career_manifest_digest
        );
    }
    let step = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("career review execution must begin");
    let reviewer = step
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Reviewer)
        .expect("career Reviewer must be ready");
    let control_head = reviewer
        .segments
        .iter()
        .find_map(|segment| match segment {
            RoleInvocationSegment::ControlHead { content, .. } => Some(content),
            _ => None,
        })
        .expect("career Reviewer must receive a control head");
    let control: serde_json::Value =
        serde_json::from_str(control_head).expect("career control head must be valid JSON");
    assert_eq!(
        control["career_manifest_digest"],
        serde_json::to_value(&resolved.plan.career_manifest_digest).unwrap()
    );
    assert_eq!(
        control["career_claim_lineage"],
        serde_json::to_value(&manifest.claim_lineage).unwrap()
    );
}

#[test]
fn role_bundles_deduplicate_a_personal_project_source_used_as_policy_and_evidence() {
    let (_workspace, engine) = external_engine();
    let source = "vault/personal/projects/coupler.md".to_owned();
    let owner = DataOwner::PersonalProject {
        project: "coupler".to_owned(),
    };
    let manifest = selected_career_manifest(
        vec!["README.md".to_owned()],
        vec![(owner.clone(), vec![source.clone()])],
    );
    let request = career_review_request(vec!["README.md".to_owned()]);
    let resolved = engine
        .resolve_with_career_composition(
            &request,
            &[ContextGrant {
                owner,
                purpose: ContextGrantPurpose::CareerWritingEvidence,
                access: ContextAccess::ReadOnly,
            }],
            Some(CareerOutputSurface::Resume),
            std::slice::from_ref(&source),
            Some(&manifest),
        )
        .expect("personal project evidence must resolve");
    let prepared = prepared_run_with(&engine, &resolved);

    for bundle in prepared.role_bundles {
        assert_eq!(
            bundle
                .bound_documents()
                .filter(|document| document.relative_path == source)
                .count(),
            1
        );
    }
}

#[test]
fn career_review_attestation_requires_a_manifest_exact_source_bundle_and_review_action() {
    let (_workspace, engine) = external_engine();
    let source = "vault/personal/projects/coupler.md".to_owned();
    let owner = DataOwner::PersonalProject {
        project: "coupler".to_owned(),
    };
    let manifest = selected_career_manifest(
        vec!["README.md".to_owned()],
        vec![(owner.clone(), vec![source.clone()])],
    );
    let request = career_review_request(vec!["README.md".to_owned()]);
    let grant = ContextGrant {
        owner,
        purpose: ContextGrantPurpose::CareerWritingEvidence,
        access: ContextAccess::ReadOnly,
    };

    assert!(matches!(
        engine.resolve_with_career_composition(
            &request,
            std::slice::from_ref(&grant),
            Some(CareerOutputSurface::Resume),
            &[],
            Some(&manifest),
        ),
        Err(HarnessError::InvalidRequest(_))
    ));

    let undeclared_grant = ContextGrant {
        owner: DataOwner::Company {
            company: "cluml".to_owned(),
        },
        purpose: ContextGrantPurpose::CareerWritingEvidence,
        access: ContextAccess::ReadOnly,
    };
    for action in [HarnessAction::DocumentReview, HarnessAction::DocumentWrite] {
        let mut undeclared_request = request.clone();
        undeclared_request.action = action;
        assert!(matches!(
            engine.resolve_with_career_composition(
                &undeclared_request,
                std::slice::from_ref(&undeclared_grant),
                Some(CareerOutputSurface::Resume),
                &[],
                Some(&manifest),
            ),
            Err(HarnessError::InvalidRequest(_))
        ));
    }

    let manifest_bound = engine
        .resolve_with_career_composition(
            &request,
            std::slice::from_ref(&grant),
            Some(CareerOutputSurface::Resume),
            std::slice::from_ref(&source),
            Some(&manifest),
        )
        .expect("manifest-bound owner review must resolve");
    let manifest_prepared = prepared_run_with(&engine, &manifest_bound);
    let mut failed_check = complete_submission_with(&engine, &manifest_bound);
    failed_check.verification_checks[0].passed = false;
    assert_apply_lifecycle_failure_contains(
        engine.attest_career_review(&manifest_bound, &manifest_prepared, &failed_check),
        "career-review-attestation",
        "one or more required verification checks failed",
    );
    let mut findings = complete_submission_with(&engine, &manifest_bound);
    findings.findings.push(Finding {
        severity: "P1".to_owned(),
        message: "career claim needs correction".to_owned(),
    });
    assert_apply_lifecycle_failure_contains(
        engine.attest_career_review(&manifest_bound, &manifest_prepared, &findings),
        "career-review-attestation",
        "review contains 1 finding(s)",
    );

    let resolved = engine
        .resolve_with_career_composition(
            &request,
            &[grant],
            Some(CareerOutputSurface::Resume),
            &[source],
            None,
        )
        .expect("retired exact source binding may resolve without an attestation manifest");
    let prepared = prepared_run_with(&engine, &resolved);
    let submission = complete_submission_with(&engine, &resolved);
    assert_apply_lifecycle_failure_contains(
        engine.attest_career_review(&resolved, &prepared, &submission),
        "career-review-attestation",
        "requires a predeclared composition manifest",
    );
}

fn assert_tampered_career_receipts_are_rejected(
    engine: &HarnessEngine,
    manifest: &CareerCompositionManifest,
    holistic: &CareerReviewReceipt,
    cluml: &CareerReviewReceipt,
    coupler: &CareerReviewReceipt,
) {
    let mut tampered_resolved = coupler.clone();
    tampered_resolved
        .resolved
        .plan
        .route_id
        .push_str("-tampered");
    let mut tampered_prepared = coupler.clone();
    tampered_prepared.prepared.prepared_role_run_digest = "0".repeat(64);
    let mut tampered_submission = coupler.clone();
    if let SubmissionArtifact::Review { summary, .. } = &mut tampered_submission.submission.artifact
    {
        summary.push_str(" tampered");
    }
    let mut tampered_evaluation = coupler.clone();
    tampered_evaluation.evaluation.candidate_digest = "0".repeat(64);
    let mut tampered_bundle_digest = coupler.clone();
    tampered_bundle_digest.evidence_bundle_digest = Some("0".repeat(64));
    let mut tampered_receipt_digest = coupler.clone();
    tampered_receipt_digest.receipt_digest = "0".repeat(64);
    for tampered in vec![
        tampered_resolved,
        tampered_prepared,
        tampered_submission,
        tampered_evaluation,
        tampered_bundle_digest,
        tampered_receipt_digest,
    ]
    .into_boxed_slice()
    {
        assert!(
            engine
                .compose_career_reviews(manifest, holistic, &[cluml.clone(), tampered])
                .is_err(),
            "every embedded receipt component must be revalidated"
        );
    }
}

fn assert_reused_career_context_is_rejected(
    engine: &HarnessEngine,
    request: &HarnessRequest,
    manifest: &CareerCompositionManifest,
    holistic: &CareerReviewReceipt,
    cluml: &CareerReviewReceipt,
    coupler: &CareerReviewReceipt,
) {
    let reused_context = career_review_receipt(
        engine,
        request,
        manifest,
        Some(ContextGrant {
            owner: coupler.resolved.context_grants[0].owner.clone(),
            purpose: ContextGrantPurpose::CareerWritingEvidence,
            access: ContextAccess::ReadOnly,
        }),
        &coupler.resolved.evidence_source_paths,
        "holistic",
    );
    assert_apply_lifecycle_failure_contains(
        engine.compose_career_reviews(manifest, holistic, &[cluml.clone(), reused_context]),
        "career-review-composition",
        "career review role context IDs must be globally disjoint",
    );
}

#[test]
fn career_composition_requires_same_artifacts_exact_owner_coverage_and_disjoint_contexts() {
    let (workspace, engine) = external_engine();
    fs::write(workspace.0.join("CAREER.md"), "# Career detail\n")
        .expect("second career artifact fixture must be written");
    let cluml_owner = DataOwner::Company {
        company: "cluml".to_owned(),
    };
    let coupler_owner = DataOwner::PersonalProject {
        project: "coupler".to_owned(),
    };
    let cluml_source =
        "vault/work/cluml/experience/case-studies/ci-runner-availability-monitoring.md".to_owned();
    let coupler_source = "vault/personal/projects/coupler.md".to_owned();
    let targets = vec!["README.md".to_owned(), "CAREER.md".to_owned()];
    let manifest = selected_career_manifest(
        targets.clone(),
        vec![
            (cluml_owner.clone(), vec![cluml_source.clone()]),
            (coupler_owner.clone(), vec![coupler_source.clone()]),
        ],
    );
    let request = career_review_request(targets);
    let holistic = career_review_receipt(&engine, &request, &manifest, None, &[], "holistic");
    let cluml = career_review_receipt(
        &engine,
        &request,
        &manifest,
        Some(ContextGrant {
            owner: cluml_owner,
            purpose: ContextGrantPurpose::CareerWritingEvidence,
            access: ContextAccess::ReadOnly,
        }),
        &[cluml_source],
        "cluml",
    );
    let coupler = career_review_receipt(
        &engine,
        &request,
        &manifest,
        Some(ContextGrant {
            owner: coupler_owner,
            purpose: ContextGrantPurpose::CareerWritingEvidence,
            access: ContextAccess::ReadOnly,
        }),
        &[coupler_source],
        "coupler",
    );
    assert!(has_verification(
        &holistic.resolved.plan,
        "career-holistic-coherence"
    ));
    assert!(!has_verification(
        &holistic.resolved.plan,
        "career-evidence-lineage"
    ));
    assert!(has_verification(
        &cluml.resolved.plan,
        "career-evidence-lineage"
    ));

    let forward = engine
        .compose_career_reviews(&manifest, &holistic, &[cluml.clone(), coupler.clone()])
        .expect("complete owner coverage must compose");
    let reversed = engine
        .compose_career_reviews(&manifest, &holistic, &[coupler.clone(), cluml.clone()])
        .expect("receipt order must not affect composition");
    assert_eq!(forward, reversed);
    assert_eq!(forward.assurance, ExecutionAssurance::Advisory);
    assert!(forward.coverage_is_caller_attested);
    assert_eq!(forward.career_output_surface, CareerOutputSurface::Resume);
    assert_eq!(forward.evidence_reviews.len(), 2);
    assert_eq!(forward.artifact_targets.len(), 2);

    assert_tampered_career_receipts_are_rejected(&engine, &manifest, &holistic, &cluml, &coupler);

    assert_apply_lifecycle_failure_contains(
        engine.compose_career_reviews(&manifest, &holistic, std::slice::from_ref(&cluml)),
        "career-review-composition",
        "owner review receipts must exactly cover the manifest evidence owners",
    );

    assert_reused_career_context_is_rejected(
        &engine, &request, &manifest, &holistic, &cluml, &coupler,
    );
}

#[test]
fn career_composition_rejects_a_stale_final_artifact() {
    let (workspace, engine) = external_engine();
    let owner = DataOwner::PersonalProject {
        project: "coupler".to_owned(),
    };
    let source = "vault/personal/projects/coupler.md".to_owned();
    let manifest = selected_career_manifest(
        vec!["README.md".to_owned()],
        vec![(owner.clone(), vec![source.clone()])],
    );
    let request = career_review_request(vec!["README.md".to_owned()]);
    let holistic = career_review_receipt(&engine, &request, &manifest, None, &[], "holistic");
    let owner_review = career_review_receipt(
        &engine,
        &request,
        &manifest,
        Some(ContextGrant {
            owner,
            purpose: ContextGrantPurpose::CareerWritingEvidence,
            access: ContextAccess::ReadOnly,
        }),
        &[source],
        "coupler",
    );

    fs::write(workspace.0.join("README.md"), "# Changed after review\n")
        .expect("artifact drift fixture must be written");
    let result = engine.compose_career_reviews(&manifest, &holistic, &[owner_review]);
    assert_apply_lifecycle_failure_contains(
        result,
        "career-review-composition",
        "resolved plan changed before validation",
    );
}

#[test]
fn career_composition_rejects_a_stale_exact_evidence_source() {
    let (vault_repository, _workspace, engine) = isolated_vault_external_engine();
    let owner = DataOwner::PersonalProject {
        project: "coupler".to_owned(),
    };
    let source = "vault/personal/projects/coupler.md".to_owned();
    let manifest = selected_career_manifest(
        vec!["README.md".to_owned()],
        vec![(owner.clone(), vec![source.clone()])],
    );
    let request = career_review_request(vec!["README.md".to_owned()]);
    let holistic = career_review_receipt(&engine, &request, &manifest, None, &[], "holistic");
    let owner_review = career_review_receipt(
        &engine,
        &request,
        &manifest,
        Some(ContextGrant {
            owner,
            purpose: ContextGrantPurpose::CareerWritingEvidence,
            access: ContextAccess::ReadOnly,
        }),
        std::slice::from_ref(&source),
        "coupler",
    );

    let source_path = vault_repository.0.join(source);
    let mut content = fs::read_to_string(&source_path).expect("copied source must be readable");
    content.push_str("\nTemporary source drift.\n");
    fs::write(source_path, content).expect("copied source drift must be written");

    let result = engine.compose_career_reviews(&manifest, &holistic, &[owner_review]);
    assert_apply_lifecycle_failure_contains(
        result,
        "career-review-composition",
        "resolved plan changed before validation",
    );
}

#[test]
fn career_composition_rejects_career_policy_and_workspace_policy_drift() {
    let (vault_repository, workspace, engine) = isolated_vault_external_engine();
    fs::write(workspace.0.join("AGENTS.md"), "# Career workspace policy\n")
        .expect("workspace policy fixture must be written");
    let owner = DataOwner::PersonalProject {
        project: "coupler".to_owned(),
    };
    let source = "vault/personal/projects/coupler.md".to_owned();
    let manifest = selected_career_manifest(
        vec!["README.md".to_owned()],
        vec![(owner.clone(), vec![source.clone()])],
    );
    let request = career_review_request(vec!["README.md".to_owned()]);
    let holistic = career_review_receipt(&engine, &request, &manifest, None, &[], "holistic");
    let owner_review = career_review_receipt(
        &engine,
        &request,
        &manifest,
        Some(ContextGrant {
            owner,
            purpose: ContextGrantPurpose::CareerWritingEvidence,
            access: ContextAccess::ReadOnly,
        }),
        &[source],
        "coupler",
    );

    let policy_path = vault_repository
        .0
        .join("vault/personal/writing/career-technical-portfolio-strategy.md");
    let mut policy = fs::read_to_string(&policy_path).expect("copied policy must be readable");
    policy.push_str("\nTemporary policy drift.\n");
    fs::write(policy_path, policy).expect("copied policy drift must be written");
    assert_apply_lifecycle_failure_contains(
        engine.compose_career_reviews(&manifest, &holistic, std::slice::from_ref(&owner_review)),
        "career-review-composition",
        "resolved plan changed before validation",
    );

    let refreshed_holistic = career_review_receipt(
        &engine,
        &request,
        &manifest,
        None,
        &[],
        "holistic-refreshed",
    );
    let refreshed_owner = career_review_receipt(
        &engine,
        &request,
        &manifest,
        owner_review.resolved.context_grants.first().cloned(),
        &owner_review.resolved.evidence_source_paths,
        "coupler-refreshed",
    );
    fs::write(
        workspace.0.join("AGENTS.md"),
        "# Changed workspace policy\n",
    )
    .expect("workspace policy drift must be written");
    assert_apply_lifecycle_failure_contains(
        engine.compose_career_reviews(&manifest, &refreshed_holistic, &[refreshed_owner]),
        "career-review-composition",
        "resolved plan changed before validation",
    );
}

#[test]
fn complete_career_coverage_requires_explicit_canonical_exclusions() {
    let (_workspace, engine) = external_engine();
    let active = DataOwner::PersonalProject {
        project: "coupler".to_owned(),
    };
    let excluded = DataOwner::Company {
        company: "cluml".to_owned(),
    };
    let mut manifest = selected_career_manifest(
        vec!["README.md".to_owned()],
        vec![(
            active.clone(),
            vec!["vault/personal/projects/coupler.md".to_owned()],
        )],
    );
    manifest.coverage = CareerCoverageMode::Complete;
    manifest.complete_coverage_confirmation_reported = true;
    manifest.canonical_evidence_owners = vec![active, excluded.clone()];
    let request = career_review_request(vec!["README.md".to_owned()]);

    assert!(matches!(
        engine.resolve_with_career_composition(
            &request,
            &[],
            Some(CareerOutputSurface::Resume),
            &[],
            Some(&manifest),
        ),
        Err(HarnessError::InvalidRequest(_))
    ));

    manifest.excluded_evidence_owners = vec![CareerOwnerExclusion {
        owner: excluded,
        reason: "not selected for this artifact".to_owned(),
    }];
    engine
        .resolve_with_career_composition(
            &request,
            &[],
            Some(CareerOutputSurface::Resume),
            &[],
            Some(&manifest),
        )
        .expect("complete caller-attested coverage with exclusions must resolve");
}

#[test]
fn career_manifest_set_order_is_canonicalized_before_digesting() {
    let (workspace, engine) = external_engine();
    fs::write(workspace.0.join("CAREER.md"), "# Career detail\n")
        .expect("second career artifact fixture must be written");
    let cluml_owner = DataOwner::Company {
        company: "cluml".to_owned(),
    };
    let coupler_owner = DataOwner::PersonalProject {
        project: "coupler".to_owned(),
    };
    let cluml_sources = vec![
        "vault/work/cluml/experience/case-studies/ci-runner-availability-monitoring.md".to_owned(),
        "vault/work/cluml/index.md".to_owned(),
    ];
    let mut first = selected_career_manifest(
        vec!["README.md".to_owned(), "CAREER.md".to_owned()],
        vec![
            (cluml_owner, cluml_sources),
            (
                coupler_owner,
                vec!["vault/personal/projects/coupler.md".to_owned()],
            ),
        ],
    );
    let mut second = first.clone();
    second.artifact_targets.reverse();
    second.evidence_owners.reverse();
    second.claim_lineage.reverse();
    second.claim_lineage[1].evidence_source_paths.reverse();
    let request = career_review_request(vec!["CAREER.md".to_owned(), "README.md".to_owned()]);

    let first_resolved = engine
        .resolve_with_career_composition(
            &request,
            &[],
            Some(CareerOutputSurface::Resume),
            &[],
            Some(&first),
        )
        .expect("first manifest order must resolve");
    let second_resolved = engine
        .resolve_with_career_composition(
            &request,
            &[],
            Some(CareerOutputSurface::Resume),
            &[],
            Some(&second),
        )
        .expect("equivalent manifest order must resolve");

    assert_eq!(first_resolved, second_resolved);
    first.artifact_targets.sort();
    assert_eq!(
        first_resolved
            .career_composition_manifest
            .expect("normalized manifest must be retained")
            .artifact_targets,
        first.artifact_targets
    );
}

#[test]
fn explicit_career_surface_binds_existing_surface_policies_and_checks() {
    let (_workspace, engine) = external_engine();
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Personal,
        targets: vec!["README.md".to_owned()],
        objective: "review a resume".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };

    let resolved = engine
        .resolve_with_career_surface(&request, CareerOutputSurface::Resume)
        .expect("explicit resume surface must resolve");

    assert_eq!(resolved.plan.version, HARNESS_SCHEMA_VERSION);
    assert_eq!(
        resolved.intent,
        HarnessIntent::CareerArtifact {
            surface: CareerOutputSurface::Resume,
        }
    );
    for policy in [
        "career-resume-source-map",
        "career-claim-token-output-system",
        "career-output-assembly",
        "career-contribution-audit",
        "career-perspective-and-public-safety",
        "career-resume-case-view",
        "career-resume-pdf-baseline",
    ] {
        assert!(
            resolved
                .plan
                .required_policies
                .iter()
                .any(|binding| binding.id == policy),
            "career policy must be bound: {policy}"
        );
    }
    for check in [
        "career-evidence-lineage",
        "career-surface-contract",
        "career-perspective-routing",
        "career-public-safety",
        "resume-first-screen-and-artifact",
    ] {
        assert!(
            has_verification(&resolved.plan, check),
            "career verification must be required: {check}"
        );
    }

    let portfolio = engine
        .resolve_with_career_surface(&request, CareerOutputSurface::Portfolio)
        .expect("explicit portfolio surface must resolve");
    assert_ne!(
        resolved.resolved_plan_digest,
        portfolio.resolved_plan_digest
    );
    assert!(
        portfolio
            .plan
            .required_policies
            .iter()
            .any(|policy| policy.id == "career-portfolio-casebook")
    );
    assert!(has_verification(
        &portfolio.plan,
        "portfolio-local-build-and-public-copy"
    ));
}

#[test]
fn objective_text_does_not_infer_career_intent() {
    let (_workspace, engine) = external_engine();
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Personal,
        targets: vec!["README.md".to_owned()],
        objective: "review resume portfolio wording for a developer hiring process".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };

    let resolved = engine
        .resolve(&request)
        .expect("objective keywords must not change the default intent");

    assert_eq!(resolved.intent, HarnessIntent::General);
    assert!(
        resolved
            .plan
            .required_policies
            .iter()
            .all(|policy| !policy.id.starts_with("career-"))
    );
    assert!(
        resolved
            .plan
            .verification_requirements
            .iter()
            .all(|requirement| {
                !requirement.unit.as_str().starts_with("career-")
                    && !requirement.unit.as_str().starts_with("resume-")
            })
    );
}

#[test]
fn policy_maintenance_intent_excludes_career_artifact_contracts() {
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Personal,
        targets: vec!["vault/personal/writing/career-technical-portfolio-strategy.md".to_owned()],
        objective: "review the resume and portfolio policy itself".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };

    let resolved = engine()
        .resolve_with_intent(&request, HarnessIntent::PolicyMaintenance)
        .expect("policy maintenance must not be routed as a career artifact");

    assert_eq!(resolved.plan.intent, HarnessIntent::PolicyMaintenance);
    assert!(
        resolved
            .plan
            .required_policies
            .iter()
            .all(|policy| !policy.id.starts_with("career-"))
    );
    for policy_id in [
        "context-vault-operating-model",
        "context-document-stability",
    ] {
        assert!(
            resolved
                .plan
                .required_policies
                .iter()
                .any(|policy| policy.id == policy_id),
            "policy-maintenance must bind {policy_id}"
        );
    }
    assert!(
        resolved
            .plan
            .verification_requirements
            .iter()
            .all(|requirement| {
                !requirement.unit.as_str().starts_with("career-")
                    && !requirement.unit.as_str().starts_with("resume-")
            })
    );

    assert!(
        resolved
            .plan
            .orchestrator_policies
            .iter()
            .any(|policy| policy.id == "mandatory-preflight")
    );
    assert!(resolved.plan.role_policy_bindings.iter().all(|binding| {
        binding
            .policies
            .iter()
            .all(|policy| policy.id != "mandatory-preflight")
    }));
}

#[test]
fn external_personal_policy_maintenance_binds_operating_policies() {
    let (_workspace, engine) = external_engine();
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Personal,
        targets: vec!["README.md".to_owned()],
        objective: "review a personal policy outside the Vault repository".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };

    let resolved = engine
        .resolve_with_intent(&request, HarnessIntent::PolicyMaintenance)
        .expect("external personal policy maintenance must resolve");

    for policy_id in [
        "context-vault-operating-model",
        "context-document-stability",
    ] {
        assert!(
            resolved
                .plan
                .required_policies
                .iter()
                .any(|policy| policy.id == policy_id),
            "external policy maintenance must bind {policy_id}"
        );
    }
}

#[test]
fn output_adapter_intent_is_profile_scoped_without_career_evidence_policies() {
    let (_workspace, external_engine) = external_engine();
    let request = HarnessRequest {
        action: HarnessAction::CodeReview,
        owner: DataOwner::Profile,
        targets: vec!["src/lib.rs".to_owned()],
        objective: "review the portfolio output adapter".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let intent = HarnessIntent::OutputAdapter {
        surface: CareerOutputSurface::Portfolio,
    };

    let resolved = external_engine
        .resolve_with_intent(&request, intent)
        .expect("explicit output adapter code review must resolve");

    assert_eq!(resolved.plan.intent, intent);
    assert_eq!(intent.career_surface(), None);
    assert_eq!(
        intent.output_surface(),
        Some(CareerOutputSurface::Portfolio)
    );
    assert!(
        resolved
            .plan
            .required_policies
            .iter()
            .all(|policy| !policy.id.starts_with("career-"))
    );
    for policy_id in [
        "context-vault-operating-model",
        "context-document-stability",
    ] {
        assert!(
            resolved
                .plan
                .required_policies
                .iter()
                .any(|policy| policy.id == policy_id),
            "output-adapter must bind {policy_id}"
        );
    }
    assert!(has_verification(&resolved.plan, "output-adapter-contract"));
    assert!(has_verification(
        &resolved.plan,
        "portfolio-local-build-and-public-copy"
    ));
    assert!(
        resolved
            .plan
            .allowed_context_roots
            .iter()
            .all(|root| root.repository_relative_path.starts_with("vault/profile"))
    );
    assert!(
        resolved
            .plan
            .denied_context_roots
            .iter()
            .any(|root| root.repository_relative_path == "vault/personal")
    );
    assert!(
        resolved
            .plan
            .denied_context_roots
            .iter()
            .any(|root| root.repository_relative_path == "vault/work")
    );

    let document_request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        targets: vec!["README.md".to_owned()],
        ..request.clone()
    };
    external_engine
        .resolve_with_intent(&document_request, intent)
        .expect("explicit output adapter document review must resolve");
    assert!(matches!(
        external_engine.resolve_with_intent(&document_request, HarnessIntent::General),
        Err(HarnessError::InvalidRequest(message))
            if message.contains("explicit output-adapter intent")
    ));

    assert!(matches!(
        engine().resolve_with_intent(&profile_code_request(), intent),
        Err(HarnessError::InvalidRequest(message))
            if message == "output-adapter intent requires an external workspace"
    ));
}

#[test]
fn non_career_intents_reject_career_composition_manifests() {
    let (_workspace, engine) = external_engine();
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Profile,
        targets: vec!["README.md".to_owned()],
        objective: "review an output adapter contract".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let manifest = selected_career_manifest(
        vec!["README.md".to_owned()],
        vec![(
            DataOwner::PersonalProject {
                project: "coupler".to_owned(),
            },
            vec!["vault/personal/projects/coupler.md".to_owned()],
        )],
    );

    assert!(matches!(
        engine.resolve_with_intent_and_career_composition(
            &request,
            HarnessIntent::OutputAdapter {
                surface: CareerOutputSurface::Resume,
            },
            &[],
            &[],
            Some(&manifest),
        ),
        Err(HarnessError::InvalidRequest(message))
            if message == "career composition manifests require career-artifact intent"
    ));
}

#[test]
fn career_role_policy_bindings_are_minimal_and_have_an_exact_union() {
    let (_workspace, engine) = external_engine();
    let request = HarnessRequest {
        action: HarnessAction::DocumentWrite,
        owner: DataOwner::Personal,
        targets: vec!["README.md".to_owned()],
        objective: "update a resume".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = engine
        .resolve_with_intent(
            &request,
            HarnessIntent::CareerArtifact {
                surface: CareerOutputSurface::Resume,
            },
        )
        .expect("career document write must resolve");
    let writer = resolved
        .plan
        .role_policy_bindings
        .iter()
        .find(|binding| binding.role == HarnessRole::Writer)
        .expect("write plan must bind writer policies");
    let reviewer = resolved
        .plan
        .role_policy_bindings
        .iter()
        .find(|binding| binding.role == HarnessRole::Reviewer)
        .expect("write plan must bind reviewer policies");

    assert!(
        resolved
            .plan
            .orchestrator_policies
            .iter()
            .any(|policy| policy.id == "mandatory-preflight")
    );
    for policy_id in ["career-claim-token-output-system", "career-output-assembly"] {
        for binding in [writer, reviewer] {
            assert!(
                binding.policies.iter().any(|policy| policy.id == policy_id),
                "{:?} must bind canonical career policy: {policy_id}",
                binding.role
            );
        }
    }
    for policy_id in [
        "career-contribution-audit",
        "career-perspective-and-public-safety",
        "career-resume-case-view",
        "career-resume-pdf-baseline",
    ] {
        assert!(
            reviewer
                .policies
                .iter()
                .any(|policy| policy.id == policy_id)
        );
    }

    let plan_union = resolved
        .plan
        .required_policies
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut role_union = resolved
        .plan
        .role_policy_bindings
        .iter()
        .flat_map(|binding| binding.policies.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>();
    role_union.extend(resolved.plan.orchestrator_policies.iter().cloned());
    assert_eq!(plan_union, role_union);
}

#[test]
fn career_canonical_policies_bind_every_role_for_supported_actions_and_profiles() {
    for current_engine in [engine(), strict_engine()] {
        for action in [
            HarnessAction::DocumentWrite,
            HarnessAction::DocumentReview,
            HarnessAction::Investigation,
            HarnessAction::Design,
        ] {
            let request = HarnessRequest {
                action,
                owner: DataOwner::Personal,
                targets: vec![
                    "vault/personal/writing/career-technical-portfolio-strategy.md".to_owned(),
                ],
                objective: "check canonical career policy role bindings".to_owned(),
                curation_kind: None,
                explicit_user_confirmation_reported: false,
                curation_sources: Vec::new(),
                delete_targets: Vec::new(),
            };
            let resolved = current_engine
                .resolve_with_intent(
                    &request,
                    HarnessIntent::CareerArtifact {
                        surface: CareerOutputSurface::General,
                    },
                )
                .expect("supported career action must resolve");

            for binding in &resolved.plan.role_policy_bindings {
                for policy_id in ["career-claim-token-output-system", "career-output-assembly"] {
                    assert!(
                        binding.policies.iter().any(|policy| policy.id == policy_id),
                        "{action:?} {:?} must bind canonical career policy: {policy_id}",
                        binding.role
                    );
                }
            }
        }
    }
}

#[test]
fn prepare_seals_role_specific_bundles_and_enforces_serialized_size() {
    let (_workspace, engine) = external_engine();
    let request = HarnessRequest {
        action: HarnessAction::DocumentWrite,
        owner: DataOwner::Personal,
        targets: vec!["README.md".to_owned()],
        objective: "update a resume".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = engine
        .resolve_with_intent(
            &request,
            HarnessIntent::CareerArtifact {
                surface: CareerOutputSurface::Resume,
            },
        )
        .expect("career document write must resolve");
    let prepared = prepared_run_with(&engine, &resolved);

    assert_eq!(prepared.role_bundles.len(), resolved.plan.role_count());
    assert_eq!(prepared.role_metadata.len(), resolved.plan.role_count());
    for (bundle, metadata) in prepared.role_bundles.iter().zip(&prepared.role_metadata) {
        assert_eq!(bundle.role, metadata.role);
        assert_eq!(bundle.bundle_digest, metadata.role_bundle_digest);
        assert!(bundle.serialized_bytes >= bundle.total_content_bytes);
        assert!(bundle.bound_documents().any(|document| {
            document.source == HarnessBoundDocumentSource::Target
                && document.relative_path == "README.md"
        }));
        for orchestrator_policy in &resolved.plan.orchestrator_policies {
            assert!(!bundle.bound_documents().any(|document| {
                document.source == HarnessBoundDocumentSource::Policy
                    && document.relative_path == orchestrator_policy.repository_relative_path
            }));
        }
    }
    for bundle in &prepared.role_bundles {
        for policy_path in [
            "vault/personal/writing/claim-token-output-system.md",
            "vault/personal/writing/career-output-assembly.md",
        ] {
            assert!(
                bundle
                    .bound_documents()
                    .any(|document| document.relative_path == policy_path),
                "{:?} bundle must include canonical career policy: {policy_path}",
                bundle.role
            );
        }
    }

    let boundary = prepared
        .role_bundles
        .iter()
        .map(|bundle| bundle.serialized_bytes)
        .max()
        .expect("prepared run must contain a role bundle");
    let mut capabilities = complete_runtime_capabilities();
    capabilities.max_role_bundle_bytes = boundary;
    engine
        .prepare(&resolved, &capabilities, None)
        .expect("a serialized role bundle exactly at the runtime limit must prepare");
    capabilities.max_role_bundle_bytes = boundary - 1;
    assert!(matches!(
        engine.prepare(&resolved, &capabilities, None),
        Err(HarnessError::UnsupportedRuntime(message))
            if message.contains("serialized bytes")
    ));
}

#[test]
fn prepare_rejects_a_target_before_reading_past_the_role_bundle_budget() {
    let (workspace, engine) = external_engine();
    fs::write(workspace.0.join("LARGE.md"), "x".repeat(4_096))
        .expect("large target fixture must be written");
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Personal,
        targets: vec!["LARGE.md".to_owned()],
        objective: "review a bounded document".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = engine
        .resolve(&request)
        .expect("document review must resolve");
    let mut capabilities = complete_runtime_capabilities();
    capabilities.max_role_bundle_bytes = 256;
    assert!(matches!(
        engine.prepare(&resolved, &capabilities, None),
        Err(HarnessError::UnsupportedRuntime(message))
            if message.contains("LARGE.md")
                && message.contains("remaining role bundle content budget")
    ));
}

#[test]
fn prepare_rejects_a_canonical_policy_before_reading_past_the_bundle_budget() {
    let (workspace, engine) = external_engine();
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Personal,
        targets: vec!["README.md".to_owned()],
        objective: "review with the exact canonical policy".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = engine
        .resolve(&request)
        .expect("document review must resolve");
    let role = resolved.plan.workflow[0].role;
    let policy = resolved
        .plan
        .role_policy_bindings
        .iter()
        .find(|binding| binding.role == role)
        .and_then(|binding| binding.policies.first())
        .expect("review role must bind a canonical policy");
    let target_bytes = usize::try_from(
        fs::metadata(workspace.0.join("README.md"))
            .expect("target metadata must exist")
            .len(),
    )
    .expect("target fixture size must fit usize");
    let policy_bytes = usize::try_from(
        fs::metadata(repository_root().join(&policy.repository_relative_path))
            .expect("canonical policy metadata must exist")
            .len(),
    )
    .expect("policy fixture size must fit usize");
    let mut capabilities = complete_runtime_capabilities();
    capabilities.max_role_bundle_bytes = target_bytes + policy_bytes - 1;

    assert!(matches!(
        engine.prepare(&resolved, &capabilities, None),
        Err(HarnessError::UnsupportedRuntime(message))
            if message.contains(&policy.repository_relative_path)
                && message.contains("remaining role bundle content budget")
    ));
}

#[test]
fn incomplete_or_unsuccessful_role_lifecycles_are_not_empty_successes() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("profile code request must resolve");
    let prepared = prepared_run(&resolved);

    let mut missing_output = complete_submission(&resolved);
    missing_output.reported_role_lifecycles[0].first_output_at_millis = None;
    assert!(matches!(
        engine().validate_submission(&resolved, &prepared, &missing_output),
        Err(HarnessError::UnsupportedRuntime(message))
            if message.contains("produced no usable output")
    ));

    let mut timed_out = complete_submission(&resolved);
    timed_out.reported_role_lifecycles[0].terminal_state = RoleTerminalState::TimedOut;
    assert!(matches!(
        engine().validate_submission(&resolved, &prepared, &timed_out),
        Err(HarnessError::UnsupportedRuntime(message)) if message.contains("TimedOut")
    ));

    let mut completed_after_grace = complete_submission(&resolved);
    completed_after_grace.reported_role_lifecycles[0].interrupt_requested_at_millis = Some(3);
    completed_after_grace.reported_role_lifecycles[0].grace_deadline_at_millis = Some(4);
    completed_after_grace.reported_role_lifecycles[0].terminal_at_millis = 5;
    completed_after_grace.reported_role_lifecycles[0].closed_at_millis = 6;
    assert!(matches!(
        engine().validate_submission(&resolved, &prepared, &completed_after_grace),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("interrupt and grace timestamps are invalid")
    ));

    let mut completed_at_grace = complete_submission(&resolved);
    let lifecycle = &mut completed_at_grace.reported_role_lifecycles[0];
    lifecycle.interrupt_requested_at_millis = lifecycle.first_output_at_millis;
    lifecycle.grace_deadline_at_millis = Some(lifecycle.terminal_at_millis);
    engine()
        .validate_submission(&resolved, &prepared, &completed_at_grace)
        .expect("completion at the grace deadline must remain valid");
}

#[test]
fn runtime_lifecycle_limits_are_bound_to_preparation_and_revision_validation() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("profile code request must resolve");

    let mut execution_capabilities = complete_runtime_capabilities();
    execution_capabilities.max_role_execution_millis = 2;
    let execution_prepared = engine()
        .prepare(&resolved, &execution_capabilities, None)
        .expect("bounded execution runtime must prepare");
    let mut execution_submission = complete_submission(&resolved);
    execution_submission.prepared_role_run_digest =
        execution_prepared.prepared_role_run_digest.clone();
    assert!(matches!(
        engine().validate_submission(&resolved, &execution_prepared, &execution_submission),
        Err(HarnessError::UnsupportedRuntime(message))
            if message.contains("runtime execution limit")
    ));

    let mut grace_capabilities = complete_runtime_capabilities();
    grace_capabilities.max_role_grace_millis = 1;
    let grace_prepared = engine()
        .prepare(&resolved, &grace_capabilities, None)
        .expect("bounded grace runtime must prepare");
    let mut grace_submission = complete_submission(&resolved);
    grace_submission.prepared_role_run_digest = grace_prepared.prepared_role_run_digest.clone();
    let lifecycle = &mut grace_submission.reported_role_lifecycles[0];
    let interrupt = lifecycle
        .first_output_at_millis
        .expect("complete lifecycle must report first output");
    lifecycle.interrupt_requested_at_millis = Some(interrupt);
    lifecycle.grace_deadline_at_millis = Some(interrupt + 2);
    lifecycle.terminal_at_millis = interrupt + 1;
    lifecycle.closed_at_millis = interrupt + 2;
    assert!(matches!(
        engine().validate_submission(&resolved, &grace_prepared, &grace_submission),
        Err(HarnessError::UnsupportedRuntime(message))
            if message.contains("runtime grace limit")
    ));

    let lifecycle = &mut grace_submission.reported_role_lifecycles[0];
    lifecycle.grace_deadline_at_millis = Some(interrupt + 1);
    engine()
        .validate_submission(&resolved, &grace_prepared, &grace_submission)
        .expect("completion at the configured grace boundary must remain valid");

    let mut history_capabilities = complete_runtime_capabilities();
    history_capabilities.max_role_execution_millis = 10;
    let history_prepared = engine()
        .prepare(&resolved, &history_capabilities, None)
        .expect("bounded revision runtime must prepare");
    let mut first_submission = complete_submission(&resolved);
    first_submission.prepared_role_run_digest = history_prepared.prepared_role_run_digest.clone();
    first_submission.verification_checks[0].passed = false;
    first_submission.verification_checks[0].detail = "retry".to_owned();
    let mut first_result = engine()
        .evaluate_submission_with_history(&resolved, &history_prepared, &first_submission, &[])
        .expect("valid first revision must produce a receipt");
    let lifecycle = &mut first_result.reported_role_lifecycles[0];
    lifecycle.terminal_at_millis = lifecycle.started_at_millis + 11;
    lifecycle.closed_at_millis = lifecycle.terminal_at_millis + 1;
    let mut revision = complete_submission(&resolved);
    revision.prepared_role_run_digest = history_prepared.prepared_role_run_digest.clone();
    revision.revision = 1;
    revision.previous_candidate_digest = Some(first_result.candidate_digest.clone());
    assert!(matches!(
        engine().validate_submission_with_history(
            &resolved,
            &history_prepared,
            &revision,
            std::slice::from_ref(&first_result)
        ),
        Err(HarnessError::InvalidSubmission(message))
            if message == "revision history validation receipt digest is invalid"
    ));
    first_result.validation_receipt_digest = serialized_digest(&(
        &first_result.candidate_digest,
        &first_result.reported_reviewer_context_id,
        &first_result.reported_role_contexts,
        &first_result.reported_role_lifecycles,
        &first_result.verification_checks,
        &first_result.findings,
        first_result.assurance,
        first_result.accepted,
    ))
    .expect("tampered history receipt must be internally consistent");
    assert!(matches!(
        engine().validate_submission_with_history(
            &resolved,
            &history_prepared,
            &revision,
            &[first_result]
        ),
        Err(HarnessError::UnsupportedRuntime(message))
            if message.contains("runtime execution limit")
    ));
}

#[test]
fn runtime_lifecycle_capabilities_require_limits_and_reject_unbounded_values() {
    let missing_limits = serde_json::json!({
        "available_roles": ["writer", "reviewer"],
        "max_concurrent_roles": 2,
        "separate_contexts": true,
        "file_reading": true,
        "tool_execution": true,
        "max_role_bundle_bytes": 1024,
        "max_role_invocation_bytes": 1024
    });
    assert!(serde_json::from_value::<RoleRuntimeCapabilities>(missing_limits).is_err());

    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("profile code request must resolve");
    let mut unbounded = complete_runtime_capabilities();
    unbounded.max_role_grace_millis = u64::MAX;
    assert!(matches!(
        engine().prepare(&resolved, &unbounded, None),
        Err(HarnessError::UnsupportedRuntime(message))
            if message.contains("runtime role grace limit")
    ));
}

#[test]
fn professional_profile_surface_serializes_and_routes_through_both_intents() {
    let serialized = serde_json::to_string(&CareerOutputSurface::ProfessionalProfile)
        .expect("professional profile surface must serialize");
    assert_eq!(serialized, "\"professional-profile\"");
    assert_eq!(
        serde_json::from_str::<CareerOutputSurface>(&serialized)
            .expect("professional profile surface must deserialize"),
        CareerOutputSurface::ProfessionalProfile
    );

    let (_workspace, engine) = external_engine();
    let career_request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Personal,
        targets: vec!["README.md".to_owned()],
        objective: "review a professional profile".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let career = engine
        .resolve_with_career_surface(&career_request, CareerOutputSurface::ProfessionalProfile)
        .expect("professional profile career artifact must resolve");
    assert_eq!(
        career.plan.intent,
        HarnessIntent::CareerArtifact {
            surface: CareerOutputSurface::ProfessionalProfile,
        }
    );
    assert!(has_verification(
        &career.plan,
        "professional-profile-artifact"
    ));

    let adapter_request = HarnessRequest {
        action: HarnessAction::CodeReview,
        owner: DataOwner::Profile,
        targets: vec!["src/lib.rs".to_owned()],
        objective: "review a professional profile output contract".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let adapter = engine
        .resolve_with_intent(
            &adapter_request,
            HarnessIntent::OutputAdapter {
                surface: CareerOutputSurface::ProfessionalProfile,
            },
        )
        .expect("professional profile output-adapter verification must resolve");
    assert!(has_verification(&adapter.plan, "output-adapter-contract"));
    assert!(has_verification(
        &adapter.plan,
        "professional-profile-output-adapter-artifact"
    ));
    assert!(
        adapter
            .plan
            .required_policies
            .iter()
            .all(|policy| !policy.id.starts_with("career-"))
    );
    assert!(
        !serde_json::to_string(&adapter.plan)
            .expect("adapter plan must serialize")
            .to_ascii_lowercase()
            .contains("linkedin")
    );
}

#[test]
fn every_career_surface_is_bound_for_document_writes_without_an_evidence_grant() {
    let (_workspace, engine) = external_engine();
    let request = HarnessRequest {
        action: HarnessAction::DocumentWrite,
        owner: DataOwner::Personal,
        targets: vec!["README.md".to_owned()],
        objective: "write a resume from existing personal career sources".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };

    for (surface, surface_check) in [
        (
            CareerOutputSurface::General,
            "career-output-surface-selection",
        ),
        (
            CareerOutputSurface::Resume,
            "resume-first-screen-and-artifact",
        ),
        (
            CareerOutputSurface::CareerDescription,
            "career-description-case-structure",
        ),
        (
            CareerOutputSurface::Portfolio,
            "portfolio-local-build-and-public-copy",
        ),
        (
            CareerOutputSurface::ProfessionalProfile,
            "professional-profile-artifact",
        ),
    ] {
        let resolved = engine
            .resolve_with_career_surface(&request, surface)
            .expect("career document write must resolve without an evidence grant");

        assert!(resolved.plan.context_grants.is_empty());
        assert_eq!(
            resolved.plan.intent,
            HarnessIntent::CareerArtifact { surface }
        );
        assert!(resolved.plan.contains_role(HarnessRole::Writer));
        for policy_id in ["career-claim-token-output-system", "career-output-assembly"] {
            assert!(
                resolved
                    .plan
                    .required_policies
                    .iter()
                    .any(|policy| policy.id == policy_id),
                "{surface:?} must include canonical career policy: {policy_id}"
            );
        }
        assert!(has_verification(&resolved.plan, surface_check));
    }
}

#[test]
fn career_surface_rejects_non_personal_or_code_work() {
    let (_workspace, engine) = external_engine();
    let mut request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Company {
            company: "cluml".to_owned(),
        },
        targets: vec!["README.md".to_owned()],
        objective: "review a resume".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    assert!(matches!(
        engine.resolve_with_career_surface(&request, CareerOutputSurface::Resume),
        Err(HarnessError::InvalidRequest(_))
    ));

    request.action = HarnessAction::CodeReview;
    request.owner = DataOwner::PersonalProject {
        project: "coupler".to_owned(),
    };
    request.targets = vec!["src/lib.rs".to_owned()];
    assert!(matches!(
        engine.resolve_with_career_surface(&request, CareerOutputSurface::Resume),
        Err(HarnessError::InvalidRequest(_))
    ));
}

#[test]
fn personal_career_work_can_bind_one_read_only_personal_project_evidence_grant() {
    let (_workspace, engine) = external_engine();
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Personal,
        targets: vec!["README.md".to_owned()],
        objective: "review a resume against one personal project".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let grant = ContextGrant {
        owner: DataOwner::PersonalProject {
            project: "coupler".to_owned(),
        },
        purpose: ContextGrantPurpose::CareerWritingEvidence,
        access: ContextAccess::ReadOnly,
    };

    let resolved = engine
        .resolve_with_context_grants_and_career_surface(
            &request,
            std::slice::from_ref(&grant),
            Some(CareerOutputSurface::Resume),
        )
        .expect("selected personal project evidence must resolve");

    assert_eq!(resolved.plan.context_grants, vec![grant]);
    assert!(
        resolved
            .plan
            .allowed_context_roots
            .iter()
            .any(|root| { root.repository_relative_path == "vault/personal/projects/coupler.md" })
    );
    assert!(!resolved.plan.allowed_context_roots.iter().any(|root| {
        root.repository_relative_path
            .starts_with("vault/personal/projects/gluesql")
    }));
    assert!(
        !resolved
            .plan
            .allowed_context_roots
            .iter()
            .any(|root| { root.repository_relative_path == "vault/personal/projects/ideas" })
    );
    assert!(
        resolved
            .plan
            .denied_context_roots
            .iter()
            .any(|root| root.repository_relative_path == "vault/work")
    );
    assert!(resolved.plan.required_policies.iter().any(|policy| {
        policy.id == "granted-personal-project-source"
            && policy.repository_relative_path == "vault/personal/projects/coupler.md"
    }));
}

#[test]
fn current_career_receipts_bind_execution_and_global_context_separation() {
    let (_workspace, engine) = external_engine();
    let owner = DataOwner::Company {
        company: "cluml".to_owned(),
    };
    let source =
        "vault/work/cluml/experience/case-studies/ci-runner-availability-monitoring.md".to_owned();
    let manifest = selected_career_manifest(
        vec!["README.md".to_owned()],
        vec![(owner.clone(), vec![source.clone()])],
    );
    let request = career_review_request(vec!["README.md".to_owned()]);
    let holistic =
        career_execution_review_receipt(&engine, &request, &manifest, None, &[], "holistic");
    let owner_review = career_execution_review_receipt(
        &engine,
        &request,
        &manifest,
        Some(ContextGrant {
            owner,
            purpose: ContextGrantPurpose::CareerWritingEvidence,
            access: ContextAccess::ReadOnly,
        }),
        &[source],
        "owner",
    );

    assert_eq!(holistic.version, HARNESS_SCHEMA_VERSION);
    assert_eq!(
        holistic.evaluation.execution_record.resolved_plan_digest,
        holistic.evaluation.resolved_plan_digest
    );
    assert!(
        holistic
            .verification_bindings
            .iter()
            .any(|binding| binding.owner
                == VerificationOwner::Role {
                    role: HarnessRole::Reviewer,
                })
    );
    let composition = engine
        .compose_career_execution_reviews(&manifest, &holistic, &[owner_review])
        .expect("current career receipts must compose");
    assert_eq!(composition.version, HARNESS_SCHEMA_VERSION);
    assert_eq!(composition.assurance, ExecutionAssurance::Advisory);
}

#[test]
fn personal_project_evidence_grant_cannot_authorize_a_project_source_write() {
    let request = HarnessRequest {
        action: HarnessAction::DocumentWrite,
        owner: DataOwner::Personal,
        targets: vec!["vault/personal/projects/coupler.md".to_owned()],
        objective: "rewrite a career output from personal project evidence".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let grant = ContextGrant {
        owner: DataOwner::PersonalProject {
            project: "coupler".to_owned(),
        },
        purpose: ContextGrantPurpose::CareerWritingEvidence,
        access: ContextAccess::ReadOnly,
    };

    assert!(matches!(
        engine().resolve_with_context_grants_and_career_surface(
            &request,
            &[grant],
            Some(CareerOutputSurface::Resume),
        ),
        Err(HarnessError::InvalidRequest(_))
    ));
}

#[test]
fn read_only_evidence_grant_cannot_authorize_a_company_write_target() {
    let request = HarnessRequest {
        action: HarnessAction::DocumentWrite,
        owner: DataOwner::Personal,
        targets: vec!["vault/work/cluml/projects/giganto.md".to_owned()],
        objective: "rewrite a personal career document from company evidence".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let grant = ContextGrant {
        owner: DataOwner::Company {
            company: "cluml".to_owned(),
        },
        purpose: ContextGrantPurpose::CareerWritingEvidence,
        access: ContextAccess::ReadOnly,
    };

    assert!(matches!(
        engine().resolve_with_context_grants(&request, &[grant]),
        Err(HarnessError::InvalidRequest(_))
    ));
}

#[test]
fn resolved_plan_content_must_match_its_digest() {
    let mut resolved = engine()
        .resolve(&profile_code_request())
        .expect("request must resolve before tampering");
    resolved.plan.allowed_context_roots.push(ContextRoot {
        repository_relative_path: "vault/work".to_owned(),
    });

    assert!(matches!(
        engine().prepare(&resolved, &complete_runtime_capabilities(), None),
        Err(HarnessError::InvalidPlan(_))
    ));
}

#[test]
fn cross_scope_evidence_grants_reject_code_and_multiple_companies() {
    let grant = ContextGrant {
        owner: DataOwner::Company {
            company: "cluml".to_owned(),
        },
        purpose: ContextGrantPurpose::CareerWritingEvidence,
        access: ContextAccess::ReadOnly,
    };
    assert!(matches!(
        engine().resolve_with_context_grants(&profile_code_request(), std::slice::from_ref(&grant)),
        Err(HarnessError::InvalidRequest(_))
    ));

    let request = HarnessRequest {
        action: HarnessAction::Investigation,
        owner: DataOwner::Personal,
        targets: Vec::new(),
        objective: "compare evidence".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    assert!(matches!(
        engine().resolve_with_context_grants(&request, &[grant.clone(), grant]),
        Err(HarnessError::InvalidRequest(_))
    ));
}

#[test]
fn unknown_company_is_rejected() {
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Company {
            company: "unknown-company".to_owned(),
        },
        targets: vec!["AGENTS.md".to_owned()],
        objective: "review".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    assert!(engine().resolve(&request).is_err());
}

#[test]
fn company_registry_requires_an_exact_company_index_entry() {
    let registry = "## Company Registry\n\
                    | ClumL | `vault/work/cluml/index.md` | aicers |\n\
                    This prose mentions `vault/work/unregistered/index.md`.";

    let companies = parse_registered_company_slugs(registry).unwrap();
    assert!(companies.contains("cluml"));
    assert!(!companies.contains("unregistered"));
}

#[test]
fn vault_curation_keeps_data_owner_separate() {
    let request = HarnessRequest {
        action: HarnessAction::VaultCuration,
        owner: DataOwner::Personal,
        targets: vec!["vault/personal/knowledge/example.md".to_owned()],
        objective: "curate personal knowledge".to_owned(),
        curation_kind: Some(CurationKind::Knowledge),
        explicit_user_confirmation_reported: true,
        curation_sources: vec!["vault/personal/index.md".to_owned()],
        delete_targets: Vec::new(),
    };
    let resolved = engine().resolve(&request).expect("curation must resolve");
    assert_eq!(resolved.plan.owner, DataOwner::Personal);
    assert!(
        resolved
            .plan
            .allowed_context_roots
            .iter()
            .any(|root| root.repository_relative_path == "vault/personal/knowledge")
    );
}

#[test]
fn broad_personal_and_company_owners_cannot_route_code_work() {
    let mut request = profile_code_request();
    request.owner = DataOwner::Personal;
    assert!(matches!(
        engine().resolve(&request),
        Err(HarnessError::InvalidRequest(_))
    ));

    request.owner = DataOwner::Company {
        company: "cluml".to_owned(),
    };
    assert!(matches!(
        engine().resolve(&request),
        Err(HarnessError::InvalidRequest(_))
    ));

    request.owner = DataOwner::PersonalBusiness;
    assert!(matches!(
        engine().resolve(&request),
        Err(HarnessError::InvalidRequest(_))
    ));
}

#[test]
fn profile_code_work_is_limited_to_vault_repository_infrastructure() {
    let mut request = profile_code_request();
    request.owner = DataOwner::Profile;
    request.targets = vec!["crates/context-core/src/lib.rs".to_owned()];
    engine()
        .resolve(&request)
        .expect("Vault infrastructure code must resolve in profile scope");

    request.targets = vec!["vault/profile/index.md".to_owned()];
    assert!(matches!(
        engine().resolve(&request),
        Err(HarnessError::InvalidRequest(_))
    ));
}

#[test]
fn profile_dependency_infrastructure_uses_the_canonical_tooling_files() {
    let mut request = profile_code_request();
    request.owner = DataOwner::Profile;

    for target in [
        ".github/dependabot.yml",
        "pnpm-lock.yaml",
        "rust-toolchain.toml",
        "scripts/verify-latest-rust.sh",
    ] {
        request.targets = vec![target.to_owned()];
        let resolved = engine()
            .resolve(&request)
            .expect("canonical dependency infrastructure must resolve");
        assert!(
            resolved
                .plan
                .required_policies
                .iter()
                .any(|policy| policy.id == "dependency"),
            "dependency policy must be bound for {target}"
        );
    }

    for retired_lockfile in ["package-lock.json", "yarn.lock"] {
        request.targets = vec![retired_lockfile.to_owned()];
        assert!(
            matches!(
                engine().resolve(&request),
                Err(HarnessError::InvalidRequest(_))
            ),
            "retired repository lockfile must be rejected: {retired_lockfile}"
        );
    }
}

#[test]
fn vault_repository_infrastructure_targets_require_the_profile_owner() {
    let personal_document = HarnessRequest {
        action: HarnessAction::DocumentWrite,
        owner: DataOwner::Personal,
        targets: vec!["AGENTS.md".to_owned()],
        objective: "rewrite repository policy".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    assert!(matches!(
        engine().resolve(&personal_document),
        Err(HarnessError::InvalidRequest(_))
    ));

    let company_document = HarnessRequest {
        owner: DataOwner::Company {
            company: "cluml".to_owned(),
        },
        targets: vec!["Cargo.toml".to_owned()],
        ..personal_document.clone()
    };
    assert!(matches!(
        engine().resolve(&company_document),
        Err(HarnessError::InvalidRequest(_))
    ));

    let personal_code = personal_project_code_request();
    let personal_code = HarnessRequest {
        targets: vec!["crates/context-core/src/lib.rs".to_owned()],
        ..personal_code
    };
    assert!(matches!(
        engine().resolve(&personal_code),
        Err(HarnessError::InvalidRequest(_))
    ));
}

#[test]
fn profile_document_work_uses_an_explicit_repository_allowlist() {
    let request = HarnessRequest {
        action: HarnessAction::DocumentWrite,
        owner: DataOwner::Profile,
        targets: vec!["AGENTS.md".to_owned()],
        objective: "update repository policy".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = engine()
        .resolve_with_intent(&request, HarnessIntent::PolicyMaintenance)
        .expect("known profile infrastructure must resolve");
    let writer = resolved
        .plan
        .role_policy_bindings
        .iter()
        .find(|binding| binding.role == HarnessRole::Writer)
        .expect("profile document write must bind a writer");
    assert!(
        writer
            .policies
            .iter()
            .any(|policy| policy.id == "context-document-stability")
    );

    let unknown = HarnessRequest {
        targets: vec![".env".to_owned()],
        ..request.clone()
    };
    assert!(matches!(
        engine().resolve(&unknown),
        Err(HarnessError::InvalidRequest(_))
    ));
}

#[test]
fn profile_policy_maintenance_can_review_canonical_profile_sources() {
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Profile,
        targets: vec!["vault/profile/rules/agent-harness.md".to_owned()],
        objective: "review the canonical Harness policy".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = engine()
        .resolve_with_intent(&request, HarnessIntent::PolicyMaintenance)
        .expect("canonical profile policy review must resolve");
    assert_eq!(resolved.plan.intent, HarnessIntent::PolicyMaintenance);
    let document_write = HarnessRequest {
        action: HarnessAction::DocumentWrite,
        targets: vec!["vault/profile/rules/new-policy.md".to_owned()],
        objective: "create a canonical profile policy document".to_owned(),
        ..request.clone()
    };
    engine()
        .resolve_with_intent(&document_write, HarnessIntent::PolicyMaintenance)
        .expect("canonical Markdown policy writes must remain supported");
    assert!(matches!(
        engine().resolve_with_intent(&request, HarnessIntent::General),
        Err(HarnessError::InvalidRequest(message))
            if message.contains("requires policy-maintenance intent")
    ));

    let personal_request = HarnessRequest {
        owner: DataOwner::Personal,
        ..request.clone()
    };
    assert!(matches!(
        engine().resolve_with_intent(&personal_request, HarnessIntent::PolicyMaintenance),
        Err(HarnessError::InvalidRequest(_))
    ));

    let non_policy_source = HarnessRequest {
        action: HarnessAction::DocumentWrite,
        targets: vec!["vault/profile/facts/example.md".to_owned()],
        objective: "attempt to write a profile fact as policy maintenance".to_owned(),
        ..request.clone()
    };
    assert!(matches!(
        engine().resolve_with_intent(&non_policy_source, HarnessIntent::PolicyMaintenance),
        Err(HarnessError::InvalidRequest(message))
            if message.contains("canonical profile policy sources")
    ));

    for (action, target) in [
        (
            HarnessAction::CodeWrite,
            "vault/profile/rules/embedded-policy.rs",
        ),
        (
            HarnessAction::DocumentWrite,
            "vault/profile/rules/embedded-policy.txt",
        ),
    ] {
        let non_markdown_policy = HarnessRequest {
            action,
            targets: vec![target.to_owned()],
            objective: "attempt to create a non-Markdown profile policy source".to_owned(),
            ..request.clone()
        };
        assert!(matches!(
            engine().resolve_with_intent(
                &non_markdown_policy,
                HarnessIntent::PolicyMaintenance
            ),
            Err(HarnessError::InvalidRequest(message))
                if message.contains("canonical profile policy sources")
        ));
    }

    let mixed_design = HarnessRequest {
        action: HarnessAction::Design,
        targets: vec![
            "crates/context-core/src/harness.rs".to_owned(),
            "vault/profile/rules/agent-harness.md".to_owned(),
        ],
        objective: "design a coordinated Harness policy change".to_owned(),
        ..request
    };
    engine()
        .resolve_with_intent(&mixed_design, HarnessIntent::PolicyMaintenance)
        .expect("policy maintenance design may bind infrastructure and canonical policy targets");
}

#[test]
fn profile_repository_actions_must_match_the_target_kind() {
    let document_on_code = HarnessRequest {
        action: HarnessAction::DocumentWrite,
        owner: DataOwner::Profile,
        targets: vec!["crates/context-core/src/lib.rs".to_owned()],
        objective: "attempt to write Rust as a document".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    assert!(matches!(
        engine().resolve(&document_on_code),
        Err(HarnessError::InvalidRequest(_))
    ));

    let code_on_document = HarnessRequest {
        action: HarnessAction::CodeWrite,
        targets: vec!["README.md".to_owned()],
        objective: "attempt to write documentation as code".to_owned(),
        ..document_on_code
    };
    assert!(matches!(
        engine().resolve(&code_on_document),
        Err(HarnessError::InvalidRequest(_))
    ));

    let crate_document = HarnessRequest {
        action: HarnessAction::DocumentWrite,
        targets: vec!["crates/context-core/README.rst".to_owned()],
        objective: "write crate documentation".to_owned(),
        ..code_on_document
    };
    engine()
        .resolve(&crate_document)
        .expect("profile and generic target classification must agree");

    let workflow_document = HarnessRequest {
        targets: vec![".github/workflows/README.md".to_owned()],
        objective: "write workflow documentation".to_owned(),
        ..crate_document
    };
    engine()
        .resolve(&workflow_document)
        .expect("workflow documentation must remain a document target");
}

#[test]
fn external_workspace_actions_must_match_the_target_kind() {
    let (_workspace, engine) = external_engine();
    let document_on_code = HarnessRequest {
        action: HarnessAction::DocumentWrite,
        owner: DataOwner::PersonalProject {
            project: "coupler".to_owned(),
        },
        targets: vec!["src/lib.rs".to_owned()],
        objective: "attempt to write Rust as a document".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    assert!(matches!(
        engine.resolve(&document_on_code),
        Err(HarnessError::InvalidRequest(_))
    ));

    let code_on_document = HarnessRequest {
        action: HarnessAction::CodeWrite,
        targets: vec!["README.md".to_owned()],
        objective: "attempt to write documentation as code".to_owned(),
        ..document_on_code
    };
    assert!(matches!(
        engine.resolve(&code_on_document),
        Err(HarnessError::InvalidRequest(_))
    ));

    for source in ["src/license-manager.rs", "src/security.rs"] {
        let source_request = HarnessRequest {
            action: HarnessAction::CodeReview,
            targets: vec![source.to_owned()],
            objective: "review a Rust file with a document-like basename".to_owned(),
            ..code_on_document.clone()
        };
        engine
            .resolve(&source_request)
            .expect("a source extension must take precedence over its basename");
    }

    let document_on_source = HarnessRequest {
        action: HarnessAction::DocumentReview,
        targets: vec!["src/license-manager.rs".to_owned()],
        objective: "attempt to review Rust as prose".to_owned(),
        ..code_on_document
    };
    assert!(matches!(
        engine.resolve(&document_on_source),
        Err(HarnessError::InvalidRequest(_))
    ));
}

#[test]
fn dependency_targets_take_precedence_over_document_extensions() {
    let (_workspace, engine) = external_engine();
    for dependency in [
        "constraints.txt",
        "Pipfile",
        "Pipfile.lock",
        "requirements.in",
        "requirements.txt",
    ] {
        let code_review = HarnessRequest {
            action: HarnessAction::CodeReview,
            owner: DataOwner::PersonalProject {
                project: "coupler".to_owned(),
            },
            targets: vec![dependency.to_owned()],
            objective: "review dependency changes".to_owned(),
            curation_kind: None,
            explicit_user_confirmation_reported: false,
            curation_sources: Vec::new(),
            delete_targets: Vec::new(),
        };
        let resolved = engine
            .resolve(&code_review)
            .expect("dependency file must use the code review path");
        assert!(
            resolved
                .plan
                .required_policies
                .iter()
                .any(|policy| policy.id == "dependency")
        );
    }

    let document_review = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::PersonalProject {
            project: "coupler".to_owned(),
        },
        targets: vec!["constraints.txt".to_owned()],
        objective: "attempt to review dependency constraints as prose".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    assert!(matches!(
        engine.resolve(&document_review),
        Err(HarnessError::InvalidRequest(_))
    ));
}

#[test]
fn nested_vault_directories_cannot_be_used_as_workspace_roots() {
    let nested_roots = [
        repository_root().join("crates/context-core"),
        repository_root().join("vault/profile"),
        repository_root().join("vault/personal"),
    ];
    for nested_root in nested_roots {
        let nested_engine = HarnessEngine::open(repository_root(), nested_root)
            .expect("existing nested workspace must open before request validation");
        assert!(matches!(
            nested_engine.resolve(&personal_project_code_request()),
            Err(HarnessError::InvalidRequest(_))
        ));
    }
}

#[test]
fn a_parent_workspace_cannot_route_a_target_back_into_the_vault() {
    let root = repository_root()
        .canonicalize()
        .expect("repository fixture must canonicalize");
    let parent = root
        .parent()
        .expect("repository fixture must have a parent");
    let repository_name = root
        .file_name()
        .and_then(OsStr::to_str)
        .expect("repository fixture name must be UTF-8");
    let target = format!("{repository_name}/vault/profile/index.md");
    assert!(
        parent
            .join(&target)
            .canonicalize()
            .expect("parent-relative Vault target must exist")
            .starts_with(&root)
    );
    let parent_engine =
        HarnessEngine::open(&root, parent).expect("repository parent workspace must open");
    let request = HarnessRequest {
        action: HarnessAction::DocumentWrite,
        owner: DataOwner::Personal,
        targets: vec![target],
        objective: "attempt to route a parent workspace target into the Vault".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };

    assert!(matches!(
        parent_engine.resolve(&request),
        Err(HarnessError::InvalidRequest(_))
    ));
}

#[cfg(target_os = "macos")]
#[test]
fn a_parent_workspace_cannot_use_a_case_alias_to_target_the_vault() {
    let root = repository_root()
        .canonicalize()
        .expect("repository fixture must canonicalize");
    let parent = root
        .parent()
        .expect("repository fixture must have a parent");
    let repository_alias = root
        .file_name()
        .and_then(OsStr::to_str)
        .expect("repository fixture name must be UTF-8")
        .to_ascii_uppercase();
    let target = format!("{repository_alias}/vault/profile/index.md");
    assert!(
        parent
            .join(&target)
            .canonicalize()
            .expect("case-aliased Vault target must exist")
            .starts_with(&root)
    );
    let parent_engine =
        HarnessEngine::open(&root, parent).expect("repository parent workspace must open");
    let request = HarnessRequest {
        action: HarnessAction::DocumentWrite,
        owner: DataOwner::Personal,
        targets: vec![target],
        objective: "attempt to use a case alias to route into the Vault".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };

    assert!(matches!(
        parent_engine.resolve(&request),
        Err(HarnessError::InvalidRequest(_))
    ));
}

#[test]
fn profile_rust_work_binds_common_and_language_policy_digests() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("profile Rust request must resolve");
    let policy_ids = resolved
        .plan
        .required_policies
        .iter()
        .map(|policy| policy.id.as_str())
        .collect::<Vec<_>>();

    assert!(policy_ids.contains(&"common-code-quality"));
    assert!(policy_ids.contains(&"rust-code-style"));
    assert!(resolved.plan.required_policies.iter().all(|policy| {
        policy.content_digest.len() == 64
            && policy
                .content_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
    }));
}

#[test]
fn dependency_and_ci_targets_bind_the_dependency_policy() {
    for target in ["Cargo.toml", ".github/workflows/ci.yml"] {
        let request = HarnessRequest {
            targets: vec![target.to_owned()],
            ..profile_code_request()
        };
        let resolved = engine()
            .resolve(&request)
            .expect("profile dependency request must resolve");
        assert!(
            resolved
                .plan
                .required_policies
                .iter()
                .any(|policy| policy.id == "dependency")
        );
    }
}

#[test]
fn explicit_policy_bindings_do_not_widen_context_roots() {
    let (_workspace, engine) = external_engine();
    let resolved = engine
        .resolve(&personal_project_code_request())
        .expect("personal project request must resolve");

    assert!(
        resolved
            .plan
            .required_policies
            .iter()
            .any(|policy| policy.id == "rust-code-style")
    );
    assert!(
        resolved
            .plan
            .denied_context_roots
            .iter()
            .any(|root| root.repository_relative_path == "vault/work")
    );
    assert!(
        !resolved
            .plan
            .allowed_context_roots
            .iter()
            .any(|root| root.repository_relative_path.starts_with("vault/work"))
    );
}

#[test]
fn standard_execution_profile_uses_fewer_agent_roles() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("standard code write request must resolve");
    assert_eq!(
        resolved.plan.execution_profile,
        HarnessExecutionProfile::Standard
    );
    assert_eq!(
        planned_roles(&resolved.plan),
        vec![HarnessRole::Writer, HarnessRole::Reviewer]
    );
    assert_eq!(
        resolved.plan.workflow,
        vec![
            WorkflowRoleNode {
                role: HarnessRole::Writer,
                dependencies: Vec::new(),
                subject_source: WorkflowSubjectSource::TaskContract,
            },
            WorkflowRoleNode {
                role: HarnessRole::Reviewer,
                dependencies: vec![HarnessRole::Writer],
                subject_source: WorkflowSubjectSource::PrimaryProducer,
            },
        ]
    );
    assert_eq!(resolved.plan.primary_producer_role, HarnessRole::Writer);
    assert_eq!(resolved.plan.required_concurrent_roles, 1);

    let prepared = engine()
        .prepare(
            &resolved,
            &capabilities_for(vec![HarnessRole::Writer, HarnessRole::Reviewer], true),
            None,
        )
        .expect("standard code write must not require planner or verifier agent roles");
    assert_eq!(prepared.role_metadata.len(), 2);
}

#[test]
fn standard_execution_profile_keeps_review_split_and_fast_read_roles() {
    let mut review_request = profile_code_request();
    review_request.action = HarnessAction::CodeReview;
    let review = engine()
        .resolve(&review_request)
        .expect("standard code review request must resolve");
    assert_eq!(
        planned_roles(&review.plan),
        vec![HarnessRole::Verifier, HarnessRole::Reviewer]
    );
    assert!(review.plan.workflow.iter().all(|node| {
        node.dependencies.is_empty() && node.subject_source == WorkflowSubjectSource::FrozenTargets
    }));
    assert_eq!(review.plan.primary_producer_role, HarnessRole::Reviewer);
    assert_eq!(review.plan.required_concurrent_roles, 1);

    let investigation = HarnessRequest {
        action: HarnessAction::Investigation,
        owner: DataOwner::Personal,
        targets: Vec::new(),
        objective: "investigate the relevant context".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let investigation = engine()
        .resolve(&investigation)
        .expect("standard investigation request must resolve");
    assert_eq!(
        planned_roles(&investigation.plan),
        vec![HarnessRole::Specialist, HarnessRole::Reviewer]
    );
    assert_eq!(
        investigation.plan.workflow[1],
        WorkflowRoleNode {
            role: HarnessRole::Reviewer,
            dependencies: vec![HarnessRole::Specialist],
            subject_source: WorkflowSubjectSource::PrimaryProducer,
        }
    );
    assert_eq!(
        investigation.plan.primary_producer_role,
        HarnessRole::Specialist
    );
    assert_eq!(investigation.plan.required_concurrent_roles, 1);
    assert_eq!(investigation.plan.max_revisions, 0);

    let read = HarnessRequest {
        action: HarnessAction::VaultRead,
        owner: DataOwner::Personal,
        targets: Vec::new(),
        objective: "read personal knowledge".to_owned(),
        curation_kind: Some(CurationKind::Knowledge),
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let read = resolve_vault_read(
        &engine(),
        &read,
        HarnessExecutionProfile::Standard,
        "knowledge",
        64 * 1024,
    );
    assert_eq!(planned_roles(&read.plan), vec![HarnessRole::Specialist]);
    assert_eq!(read.plan.primary_producer_role, HarnessRole::Specialist);
    assert_eq!(read.plan.required_concurrent_roles, 1);
    assert_eq!(read.plan.max_revisions, 0);
}

#[test]
fn vault_read_rejects_file_targets() {
    let request = HarnessRequest {
        action: HarnessAction::VaultRead,
        owner: DataOwner::Personal,
        targets: vec!["vault/personal/knowledge/example.md".to_owned()],
        objective: "read personal knowledge".to_owned(),
        curation_kind: Some(CurationKind::Knowledge),
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };

    assert!(matches!(
        engine().resolve(&request),
        Err(HarnessError::InvalidRequest(message))
            if message == "Vault read uses bounded retrieval and does not accept file targets"
    ));
}

#[test]
fn strict_execution_profile_preserves_full_role_contract() {
    let resolved = strict_engine()
        .resolve(&profile_code_request())
        .expect("strict code write request must resolve");
    assert_eq!(
        resolved.plan.execution_profile,
        HarnessExecutionProfile::Strict
    );
    assert_eq!(
        planned_roles(&resolved.plan),
        vec![
            HarnessRole::Writer,
            HarnessRole::Verifier,
            HarnessRole::Reviewer,
        ]
    );
    assert_eq!(
        resolved.plan.workflow[2],
        WorkflowRoleNode {
            role: HarnessRole::Reviewer,
            dependencies: vec![HarnessRole::Writer],
            subject_source: WorkflowSubjectSource::PrimaryProducer,
        }
    );
    assert_eq!(resolved.plan.primary_producer_role, HarnessRole::Writer);
    assert_eq!(resolved.plan.required_concurrent_roles, 1);

    let mut review_request = profile_code_request();
    review_request.action = HarnessAction::CodeReview;
    let review = strict_engine()
        .resolve(&review_request)
        .expect("strict code review request must resolve");
    assert_eq!(
        planned_roles(&review.plan),
        vec![HarnessRole::Verifier, HarnessRole::Reviewer]
    );
    assert_eq!(review.plan.primary_producer_role, HarnessRole::Reviewer);
    assert_eq!(review.plan.required_concurrent_roles, 1);

    let read = HarnessRequest {
        action: HarnessAction::VaultRead,
        owner: DataOwner::Personal,
        targets: Vec::new(),
        objective: "read personal knowledge with strict planning".to_owned(),
        curation_kind: Some(CurationKind::Knowledge),
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let read = resolve_vault_read(
        &strict_engine(),
        &read,
        HarnessExecutionProfile::Strict,
        "knowledge",
        64 * 1024,
    );
    assert_eq!(
        planned_roles(&read.plan),
        vec![HarnessRole::Specialist, HarnessRole::Reviewer]
    );
    assert_eq!(read.plan.primary_producer_role, HarnessRole::Specialist);
    assert_eq!(read.plan.required_concurrent_roles, 1);
    assert_eq!(read.plan.max_revisions, 0);
}

#[test]
fn standard_code_work_still_requires_tool_execution() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("standard code write request must resolve");
    let mut capabilities =
        capabilities_for(vec![HarnessRole::Writer, HarnessRole::Reviewer], false);
    assert!(matches!(
        engine().prepare(&resolved, &capabilities, None),
        Err(HarnessError::UnsupportedRuntime(message))
            if message == "runtime cannot execute required verification tools"
    ));

    capabilities.tool_execution = true;
    engine()
        .prepare(&resolved, &capabilities, None)
        .expect("tool-capable runtime must prepare standard code write");
}

#[test]
fn standard_write_still_requires_separate_writer_reviewer_contexts() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("standard code write request must resolve");
    let mut capabilities = capabilities_for(vec![HarnessRole::Writer, HarnessRole::Reviewer], true);
    capabilities.separate_contexts = false;

    assert!(matches!(
        engine().prepare(&resolved, &capabilities, None),
        Err(HarnessError::UnsupportedRuntime(message))
            if message == "runtime cannot provide separate role contexts"
    ));
}

#[test]
fn strict_review_requires_verifier_and_separate_contexts() {
    let mut request = profile_code_request();
    request.action = HarnessAction::CodeReview;
    let resolved = strict_engine()
        .resolve(&request)
        .expect("strict code review request must resolve");

    let missing_verifier = capabilities_for(vec![HarnessRole::Reviewer], true);
    assert!(matches!(
        strict_engine().prepare(&resolved, &missing_verifier, None),
        Err(HarnessError::UnsupportedRuntime(message))
            if message.contains("Verifier")
    ));

    let mut shared_context =
        capabilities_for(vec![HarnessRole::Verifier, HarnessRole::Reviewer], true);
    shared_context.separate_contexts = false;
    assert!(matches!(
        strict_engine().prepare(&resolved, &shared_context, None),
        Err(HarnessError::UnsupportedRuntime(message))
            if message == "runtime cannot provide separate role contexts"
    ));
}

#[test]
fn review_can_prepare_with_sequential_role_execution() {
    let mut request = profile_code_request();
    request.action = HarnessAction::CodeReview;
    let resolved = engine()
        .resolve(&request)
        .expect("standard code review request must resolve");
    let mut capabilities =
        capabilities_for(vec![HarnessRole::Verifier, HarnessRole::Reviewer], true);
    capabilities.max_concurrent_roles = 1;

    engine()
        .prepare(&resolved, &capabilities, None)
        .expect("review roles may run sequentially when contexts stay separate");
}

#[test]
fn plan_rejects_five_entries_at_the_four_role_boundary() {
    let mut plan = engine()
        .resolve(&profile_code_request())
        .expect("standard code write request must resolve")
        .plan;
    plan.workflow = vec![
        WorkflowRoleNode {
            role: HarnessRole::Writer,
            dependencies: Vec::new(),
            subject_source: WorkflowSubjectSource::TaskContract,
        };
        5
    ];

    assert!(matches!(
        plan.validate(),
        Err(HarnessError::InvalidPlan(message))
            if message
                == format!(
                    "a plan must contain between one and {MAX_PLANNED_ROLES} roles"
                )
    ));
}

#[test]
fn workflow_rejects_duplicate_forward_unsorted_and_invalid_subject_bindings() {
    let standard = engine()
        .resolve(&profile_code_request())
        .expect("standard code write request must resolve")
        .plan;

    let mut duplicate = standard.clone();
    duplicate.workflow[1].role = HarnessRole::Writer;
    assert!(matches!(
        duplicate.validate(),
        Err(HarnessError::InvalidPlan(message))
            if message == "workflow roles must be unique"
    ));

    let mut forward = standard.clone();
    forward.workflow[0].dependencies = vec![HarnessRole::Reviewer];
    assert!(matches!(
        forward.validate(),
        Err(HarnessError::InvalidPlan(message))
            if message.contains("dependencies must occur earlier")
    ));

    let mut unplanned = standard.clone();
    unplanned.workflow[1].dependencies = vec![HarnessRole::Specialist];
    assert!(matches!(
        unplanned.validate(),
        Err(HarnessError::InvalidPlan(message))
            if message.contains("depends on an unplanned role")
    ));

    let mut invalid_subject = standard;
    invalid_subject.workflow[1].subject_source = WorkflowSubjectSource::FrozenTargets;
    assert!(matches!(
        invalid_subject.validate(),
        Err(HarnessError::InvalidPlan(message))
            if message.contains("has an invalid subject source")
    ));

    let mut strict = strict_engine()
        .resolve(&profile_code_request())
        .expect("strict code write request must resolve")
        .plan;
    strict.workflow[2].dependencies = vec![HarnessRole::Verifier, HarnessRole::Writer];
    assert!(matches!(
        strict.validate(),
        Err(HarnessError::InvalidPlan(message))
            if message.contains("dependencies must use workflow order")
    ));

    let mut unsupported_fan_in = strict_engine()
        .resolve(&profile_code_request())
        .expect("strict code write request must resolve")
        .plan;
    unsupported_fan_in.workflow[2].dependencies = vec![HarnessRole::Writer, HarnessRole::Verifier];
    assert!(matches!(
        unsupported_fan_in.validate(),
        Err(HarnessError::InvalidPlan(message))
            if message.contains("must depend only on the primary producer")
    ));

    let mut review_request = profile_code_request();
    review_request.action = HarnessAction::CodeReview;
    let mut dependent_review = engine()
        .resolve(&review_request)
        .expect("review request must resolve")
        .plan;
    dependent_review.workflow = vec![
        WorkflowRoleNode {
            role: HarnessRole::Reviewer,
            dependencies: Vec::new(),
            subject_source: WorkflowSubjectSource::FrozenTargets,
        },
        WorkflowRoleNode {
            role: HarnessRole::Verifier,
            dependencies: vec![HarnessRole::Reviewer],
            subject_source: WorkflowSubjectSource::PrimaryProducer,
        },
    ];
    assert!(matches!(
        dependent_review.validate(),
        Err(HarnessError::InvalidPlan(message))
            if message.contains("has an invalid subject source")
    ));
}

#[test]
fn workflow_drift_is_bound_by_plan_replay() {
    let engine = strict_engine();
    let mut resolved = engine
        .resolve(&profile_code_request())
        .expect("strict code write request must resolve");
    resolved.plan.workflow[2].dependencies = vec![HarnessRole::Writer, HarnessRole::Verifier];
    resolved.resolved_plan_digest =
        serialized_digest(&resolved.plan).expect("forged workflow plan must serialize");

    assert!(matches!(
        engine.prepare(&resolved, &complete_runtime_capabilities(), None),
        Err(HarnessError::PlanDrift { .. })
    ));
}

#[test]
fn current_plan_accepts_runtime_capacity_of_five() {
    let engine = engine();
    let resolved = engine
        .resolve(&profile_code_request())
        .expect("standard code write request must resolve");
    assert_eq!(resolved.plan.version, HARNESS_SCHEMA_VERSION);
    let mut capabilities = capabilities_for(planned_roles(&resolved.plan), true);
    capabilities.max_concurrent_roles = 5;

    engine
        .prepare(&resolved, &capabilities, None)
        .expect("current plan must accept runtime capacity of five");
}

#[test]
fn execution_profile_changes_plan_digest_and_prepared_role_run_digest() {
    let standard = engine()
        .resolve(&profile_code_request())
        .expect("standard code write request must resolve");
    let strict = strict_engine()
        .resolve(&profile_code_request())
        .expect("strict code write request must resolve");

    assert_ne!(standard.resolved_plan_digest, strict.resolved_plan_digest);

    let standard_prepared = engine()
        .prepare(&standard, &complete_runtime_capabilities(), None)
        .expect("standard code write must prepare");
    let strict_prepared = strict_engine()
        .prepare(&strict, &complete_runtime_capabilities(), None)
        .expect("strict code write must prepare");
    assert_ne!(
        standard_prepared.prepared_role_run_digest,
        strict_prepared.prepared_role_run_digest
    );
}

#[test]
fn request_envelope_json_uses_intent_not_purpose() {
    let envelope = user_profile_code_envelope("keep request terminology exact");
    let mut serialized =
        serde_json::to_value(&envelope).expect("request envelope must serialize to JSON");
    let common = serialized
        .get("draft")
        .and_then(|draft| draft.get("common"))
        .and_then(serde_json::Value::as_object)
        .expect("write request must contain common fields");
    assert!(common.contains_key("intent"));
    assert!(!common.contains_key("purpose"));

    let common = serialized
        .get_mut("draft")
        .and_then(|draft| draft.get_mut("common"))
        .and_then(serde_json::Value::as_object_mut)
        .expect("write request must contain mutable common fields");
    let intent = common
        .remove("intent")
        .expect("current request must contain intent");
    common.insert("purpose".to_owned(), intent);
    assert!(
        decode_current_json::<RequestEnvelope>(
            &serde_json::to_vec(&serialized).expect("legacy request shape must serialize"),
            "request envelope",
        )
        .is_err(),
        "the previous purpose field must not be accepted as an alias"
    );
}

#[test]
fn plan_json_requires_the_current_field_names() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("standard code write request must resolve");
    let mut serialized = serde_json::to_value(&resolved.plan).expect("plan must serialize to JSON");
    let object = serialized
        .as_object()
        .expect("serialized plan must be a JSON object");
    assert_eq!(
        object.get("version").and_then(serde_json::Value::as_u64),
        Some(u64::from(HARNESS_SCHEMA_VERSION))
    );
    assert_eq!(
        object
            .get("intent")
            .and_then(serde_json::Value::as_object)
            .and_then(|intent| intent.get("kind"))
            .and_then(serde_json::Value::as_str),
        Some("general")
    );
    assert!(object.contains_key("orchestrator_policies"));
    assert_eq!(
        object
            .get("required_concurrent_roles")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        object
            .get("primary_producer_role")
            .and_then(serde_json::Value::as_str),
        Some("writer")
    );
    assert!(!object.contains_key("max_parallel_roles"));

    let object = serialized
        .as_object_mut()
        .expect("serialized plan must remain a JSON object");
    let required = object
        .remove("required_concurrent_roles")
        .expect("serialized plan must include the new concurrency field");
    object.insert("max_parallel_roles".to_owned(), required);

    assert!(serde_json::from_value::<ResolvedHarnessPlan>(serialized).is_err());
}

#[test]
fn current_plan_binds_contract_trace_and_role_metadata() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("current request must resolve");
    let prepared = prepared_run(&resolved);

    assert_eq!(resolved.plan.version, HARNESS_SCHEMA_VERSION);
    assert_eq!(
        resolved.plan.contract_digest,
        serialized_digest(&resolved.contract).expect("contract must serialize")
    );
    assert_eq!(
        resolved.plan.decision_trace_digest,
        serialized_digest(&resolved.decision_trace).expect("decision trace must serialize")
    );
    assert_eq!(
        resolved.plan.request_provenance,
        resolved.request_provenance
    );
    assert!(prepared.role_metadata.iter().all(|metadata| {
        metadata.contract_digest == resolved.plan.contract_digest
            && metadata.decision_trace_digest == resolved.plan.decision_trace_digest
            && metadata.request_provenance == resolved.plan.request_provenance
    }));
}

#[test]
fn current_execution_binds_the_exact_predecessor_result_and_check_owners() {
    let engine = engine();
    let resolved = engine
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let mut step = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("execution must begin");

    assert_eq!(step.ready_role_invocations.len(), 1);
    assert_eq!(step.ready_role_invocations[0].role, HarnessRole::Writer);
    assert!(
        step.ready_role_invocations[0]
            .predecessor_results
            .is_empty()
    );
    assert!(step.ready_tool_invocation.is_none());
    assert!(
        resolved
            .plan
            .verification_requirements
            .iter()
            .any(|requirement| {
                requirement.unit == VerificationUnit::TestsAndStaticAnalysis
                    && requirement.owner == VerificationOwner::Tool
            })
    );
    assert!(
        resolved
            .plan
            .verification_requirements
            .iter()
            .any(|requirement| {
                requirement.unit == VerificationUnit::CodeCorrectness
                    && requirement.owner
                        == VerificationOwner::Role {
                            role: HarnessRole::Reviewer,
                        }
            })
    );

    let writer_invocation = step.ready_role_invocations[0].clone();
    let writer_event =
        completed_role_event(&resolved, &prepared, &writer_invocation, Vec::new(), "0");
    let writer_result = match &writer_event {
        HarnessExecutionEvent::RoleResult { result } => (**result).clone(),
        HarnessExecutionEvent::ToolEvidence { .. } => panic!("Writer must produce a role result"),
    };
    step = engine
        .advance_execution(&resolved, &prepared, &[], &step.record, writer_event)
        .expect("Writer result must advance");

    let reviewer_invocation = step
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Reviewer)
        .expect("Reviewer must become ready")
        .clone();
    assert_eq!(reviewer_invocation.predecessor_results, vec![writer_result]);
    assert!(matches!(
        reviewer_invocation.subject,
        EvaluationSubject::ProducedArtifact { .. }
    ));
    let tool_invocation = step
        .ready_tool_invocation
        .clone()
        .expect("code work requires tool evidence");
    step = engine
        .advance_execution(
            &resolved,
            &prepared,
            &[],
            &step.record,
            passed_tool_event(&tool_invocation),
        )
        .expect("tool evidence must advance");
    step = engine
        .advance_execution(
            &resolved,
            &prepared,
            &[],
            &step.record,
            completed_role_event(&resolved, &prepared, &reviewer_invocation, Vec::new(), "0"),
        )
        .expect("Reviewer result must advance");
    assert!(step.ready_role_invocations.is_empty());
    assert!(step.ready_tool_invocation.is_none());

    let evaluation = engine
        .evaluate_execution(&resolved, &prepared, &[], &step.record)
        .expect("terminal execution must evaluate");
    assert_eq!(evaluation.execution_status, ExecutionStatus::Completed);
    assert_eq!(evaluation.subject_status, SubjectStatus::Accepted);
    assert_eq!(
        evaluation.completion_state,
        HarnessCompletionState::ValidatedPendingApply
    );
    assert_eq!(evaluation.assurance, ExecutionAssurance::Advisory);
    assert!(evaluation.candidate_digest.is_some());
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "terminal, receipt, and forbidden follow-up assertions preserve one complete scenario"
)]
fn reviewer_missing_context_is_receipt_bound_and_halts_the_whole_run() {
    let engine = engine();
    let resolved = engine
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let (frontier, reviewer) =
        code_reviewer_frontier(&engine, &resolved, &prepared, "missing-context-writer");
    let pending_tool = frontier
        .ready_tool_invocation
        .clone()
        .expect("code work must have pending tool evidence");
    let event = missing_context_role_event(
        &resolved,
        &reviewer,
        MissingContextCandidate::AdditionalWorkspaceTarget {
            workspace_relative_path: "crates/context-core/src/document.rs".to_owned(),
        },
        "missing-context-reviewer",
    );
    let expected_request = match &event {
        HarnessExecutionEvent::RoleResult { result } => match &result.outcome {
            RoleExecutionOutcome::MissingContext { request } => request.clone(),
            _ => panic!("Reviewer must request missing context"),
        },
        HarnessExecutionEvent::ToolEvidence { .. } => {
            panic!("Reviewer must return a role result")
        }
    };
    let terminal = engine
        .advance_execution(&resolved, &prepared, &[], &frontier.record, event)
        .expect("valid Reviewer missing-context result must persist");

    assert!(terminal.ready_role_invocations.is_empty());
    assert!(terminal.ready_tool_invocation.is_none());
    assert!(matches!(
        engine.advance_execution(
            &resolved,
            &prepared,
            &[],
            &terminal.record,
            passed_tool_event(&pending_tool),
        ),
        Err(HarnessError::InvalidSubmission(_))
    ));

    let evaluation = engine
        .evaluate_execution(&resolved, &prepared, &[], &terminal.record)
        .expect("missing-context terminal record must evaluate");
    assert_eq!(evaluation.execution_status, ExecutionStatus::MissingContext);
    assert_eq!(
        evaluation.completion_state,
        HarnessCompletionState::MissingContext
    );
    assert_eq!(evaluation.subject_status, SubjectStatus::NotApplicable);
    assert_eq!(evaluation.missing_context, Some(expected_request));
    assert!(evaluation.candidate_digest.is_none());
    assert!(evaluation.artifact.is_none());
    assert!(evaluation.requirements.is_empty());
    assert!(matches!(
        engine.validate_execution(&resolved, &prepared, &[], &terminal.record),
        Err(HarnessError::UnsupportedRuntime(message))
            if message.contains("MissingContext")
    ));
    assert!(matches!(
        engine.apply_validated_execution(&resolved, &prepared, &[], &terminal.record),
        Err(HarnessError::UnsupportedRuntime(message))
            if message.contains("MissingContext")
    ));
    assert!(matches!(
        engine.begin_execution(&resolved, &prepared, std::slice::from_ref(&evaluation)),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("only a completed rejected write candidate may be revised")
    ));

    let mut forged_record = terminal.record.clone();
    let forged_result = forged_record
        .role_results
        .iter_mut()
        .find(|result| result.role == HarnessRole::Reviewer)
        .expect("terminal record must contain the Reviewer result");
    let RoleExecutionOutcome::MissingContext { request } = &mut forged_result.outcome else {
        panic!("Reviewer result must request missing context");
    };
    request.candidate = MissingContextCandidate::AdditionalWorkspaceTarget {
        workspace_relative_path: "crates/context-core/src/not-present.rs".to_owned(),
    };
    refresh_role_result_digest(forged_result);
    refresh_role_execution_record_digest(&mut forged_record);
    assert!(matches!(
        engine.evaluate_execution(&resolved, &prepared, &[], &forged_record),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("missing context candidate is invalid")
    ));

    let mut tampered = evaluation.clone();
    tampered
        .missing_context
        .as_mut()
        .expect("evaluation must expose missing context")
        .reason = "tampered reason".to_owned();
    assert!(matches!(
        engine.begin_execution(&resolved, &prepared, &[tampered]),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("task evaluation receipt digest is invalid")
    ));

    let mut encoded =
        serde_json::to_value(&evaluation).expect("task evaluation must serialize to JSON");
    encoded
        .as_object_mut()
        .expect("task evaluation must be an object")
        .remove("missing_context");
    let bytes = serde_json::to_vec(&encoded).expect("tampered task evaluation must encode");
    assert!(decode_current_json::<HarnessTaskEvaluation>(&bytes, "task evaluation").is_err());
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "all malformed MissingContext dimensions remain together as one contract matrix"
)]
fn missing_context_contract_rejects_wrong_role_scope_evidence_and_lifecycle() {
    let engine = engine();
    let resolved = engine
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let begin = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("execution must begin");
    let (frontier, reviewer) =
        code_reviewer_frontier(&engine, &resolved, &prepared, "invalid-missing-writer");
    let base = missing_context_role_event(
        &resolved,
        &reviewer,
        MissingContextCandidate::AdditionalWorkspaceTarget {
            workspace_relative_path: "crates/context-core/src/document.rs".to_owned(),
        },
        "invalid-missing-reviewer",
    );
    let assert_invalid = |event| {
        assert!(matches!(
            engine.advance_execution(&resolved, &prepared, &[], &frontier.record, event),
            Err(HarnessError::InvalidSubmission(_))
        ));
    };

    let mut empty_units = base.clone();
    missing_context_request_mut(&mut empty_units)
        .blocked_verification_units
        .clear();
    refresh_role_event_digest(&mut empty_units);
    assert_invalid(empty_units);

    let mut duplicate_units = base.clone();
    let unit = missing_context_request_mut(&mut duplicate_units).blocked_verification_units[0];
    missing_context_request_mut(&mut duplicate_units)
        .blocked_verification_units
        .push(unit);
    refresh_role_event_digest(&mut duplicate_units);
    assert_invalid(duplicate_units);

    let mut wrong_owner = base.clone();
    let non_reviewer_unit = resolved
        .plan
        .verification_requirements
        .iter()
        .find(|requirement| {
            !matches!(
                requirement.owner,
                VerificationOwner::Role {
                    role: HarnessRole::Reviewer
                }
            )
        })
        .expect("code plan must contain a non-Reviewer requirement")
        .unit;
    missing_context_request_mut(&mut wrong_owner).blocked_verification_units =
        vec![non_reviewer_unit];
    refresh_role_event_digest(&mut wrong_owner);
    assert_invalid(wrong_owner);

    let mut missing_evidence = base.clone();
    missing_context_request_mut(&mut missing_evidence)
        .subject_evidence
        .clear();
    refresh_role_event_digest(&mut missing_evidence);
    assert_invalid(missing_evidence);

    let mut wrong_lifecycle = base.clone();
    let HarnessExecutionEvent::RoleResult { result } = &mut wrong_lifecycle else {
        panic!("event must contain a role result");
    };
    result.lifecycle.terminal_state = RoleTerminalState::Completed;
    refresh_role_event_digest(&mut wrong_lifecycle);
    assert_invalid(wrong_lifecycle);

    let mut missing_output = base.clone();
    let HarnessExecutionEvent::RoleResult { result } = &mut missing_output else {
        panic!("event must contain a role result");
    };
    result.lifecycle.first_output_at_millis = None;
    refresh_role_event_digest(&mut missing_output);
    assert_invalid(missing_output);

    let mut already_bound = base.clone();
    missing_context_request_mut(&mut already_bound).candidate =
        MissingContextCandidate::AdditionalWorkspaceTarget {
            workspace_relative_path: "crates/context-core/src/lib.rs".to_owned(),
        };
    refresh_role_event_digest(&mut already_bound);
    assert_invalid(already_bound);

    let mut missing_file = base.clone();
    missing_context_request_mut(&mut missing_file).candidate =
        MissingContextCandidate::AdditionalWorkspaceTarget {
            workspace_relative_path: "crates/context-core/src/not-present.rs".to_owned(),
        };
    refresh_role_event_digest(&mut missing_file);
    assert_invalid(missing_file);

    let mut outside_scope = base.clone();
    missing_context_request_mut(&mut outside_scope).candidate =
        MissingContextCandidate::VaultEvidence {
            repository_relative_path: "docs/architecture.md".to_owned(),
        };
    refresh_role_event_digest(&mut outside_scope);
    assert_invalid(outside_scope);

    let mut denied_scope = base.clone();
    missing_context_request_mut(&mut denied_scope).candidate =
        MissingContextCandidate::VaultEvidence {
            repository_relative_path: "vault/work/common/router/company-registry.md".to_owned(),
        };
    refresh_role_event_digest(&mut denied_scope);
    assert_invalid(denied_scope);

    let no_grant_candidate = "vault/profile/preferences/synthetic-routing-policy.md";
    assert!(!prepared.role_bundles.iter().any(|bundle| {
        bundle
            .bound_documents()
            .any(|document| document.relative_path == no_grant_candidate)
    }));
    let mut no_grant_vault_evidence = base.clone();
    missing_context_request_mut(&mut no_grant_vault_evidence).candidate =
        MissingContextCandidate::VaultEvidence {
            repository_relative_path: no_grant_candidate.to_owned(),
        };
    refresh_role_event_digest(&mut no_grant_vault_evidence);
    assert!(matches!(
        engine.advance_execution(
            &resolved,
            &prepared,
            &[],
            &frontier.record,
            no_grant_vault_evidence,
        ),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("requires exactly one existing evidence grant")
    ));

    let mut wrong_role = base;
    let HarnessExecutionEvent::RoleResult { result } = &mut wrong_role else {
        panic!("event must contain a role result");
    };
    let writer = &begin.ready_role_invocations[0];
    result.role = HarnessRole::Writer;
    result.invocation_digest = writer.invocation_digest.clone();
    result.lifecycle.role = HarnessRole::Writer;
    refresh_role_event_digest(&mut wrong_role);
    assert!(matches!(
        engine.advance_execution(&resolved, &prepared, &[], &begin.record, wrong_role),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("only a ready Reviewer")
    ));
}

#[test]
fn missing_context_rejects_cross_tag_rebinding_of_the_same_file() {
    let engine = engine();
    let resolved = engine
        .resolve(&profile_code_request())
        .expect("code request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let (frontier, reviewer) =
        code_reviewer_frontier(&engine, &resolved, &prepared, "policy-rebind-writer");
    let policy_path = "vault/profile/rules/common-code-quality.md";
    assert!(prepared.role_bundles.iter().any(|bundle| {
        bundle
            .bound_documents()
            .any(|document| document.relative_path == policy_path)
    }));
    let policy_as_target = missing_context_role_event(
        &resolved,
        &reviewer,
        MissingContextCandidate::AdditionalWorkspaceTarget {
            workspace_relative_path: policy_path.to_owned(),
        },
        "policy-rebind-reviewer",
    );
    assert!(matches!(
        engine.advance_execution(
            &resolved,
            &prepared,
            &[],
            &frontier.record,
            policy_as_target,
        ),
        Err(HarnessError::InvalidSubmission(_))
    ));

    let target_path = "vault/profile/preferences/synthetic-routing-policy.md";
    let review_request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Profile,
        targets: vec![target_path.to_owned()],
        objective: "review one profile routing document".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let review_resolved = engine
        .resolve_with_intent(&review_request, HarnessIntent::PolicyMaintenance)
        .expect("profile document review must resolve");
    let review_prepared = prepared_run_with(&engine, &review_resolved);
    let begin = engine
        .begin_execution(&review_resolved, &review_prepared, &[])
        .expect("profile review must begin");
    let review_invocation = begin
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Reviewer)
        .expect("Reviewer must be ready");
    let target_as_vault_evidence = missing_context_role_event(
        &review_resolved,
        review_invocation,
        MissingContextCandidate::VaultEvidence {
            repository_relative_path: target_path.to_owned(),
        },
        "target-rebind-reviewer",
    );
    assert!(matches!(
        engine.advance_execution(
            &review_resolved,
            &review_prepared,
            &[],
            &begin.record,
            target_as_vault_evidence,
        ),
        Err(HarnessError::InvalidSubmission(_))
    ));
}

#[cfg(unix)]
#[test]
fn missing_context_rejects_a_workspace_hard_link_candidate() {
    let (workspace, engine) = external_engine();
    let resolved = engine
        .resolve(&personal_project_code_request())
        .expect("external request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let (frontier, reviewer) =
        code_reviewer_frontier(&engine, &resolved, &prepared, "hard-link-writer");
    fs::hard_link(
        workspace.0.join("src/security.rs"),
        workspace.0.join("src/security-alias.rs"),
    )
    .expect("hard-link candidate fixture must be created");
    let event = missing_context_role_event(
        &resolved,
        &reviewer,
        MissingContextCandidate::AdditionalWorkspaceTarget {
            workspace_relative_path: "src/security-alias.rs".to_owned(),
        },
        "hard-link-reviewer",
    );

    assert!(matches!(
        engine.advance_execution(&resolved, &prepared, &[], &frontier.record, event),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("must have exactly one hard link")
    ));
}

#[test]
fn missing_context_rejects_denied_vault_paths_before_existence_checks() {
    let engine = engine();
    let resolved = engine
        .resolve(&profile_code_request())
        .expect("profile request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let (frontier, reviewer) =
        code_reviewer_frontier(&engine, &resolved, &prepared, "denied-scope-writer");
    let rejection_for = |repository_relative_path: &str| {
        let event = missing_context_role_event(
            &resolved,
            &reviewer,
            MissingContextCandidate::VaultEvidence {
                repository_relative_path: repository_relative_path.to_owned(),
            },
            "denied-scope-reviewer",
        );
        match engine.advance_execution(&resolved, &prepared, &[], &frontier.record, event) {
            Err(HarnessError::InvalidSubmission(message)) => message,
            other => panic!("denied Vault candidate must be rejected uniformly: {other:?}"),
        }
    };

    let existing = rejection_for("vault/personal/index.md");
    let absent = rejection_for("vault/personal/not-present.md");
    assert_eq!(existing, absent);
    assert!(existing.contains("denied by the current owner and grant scope"));
}

#[cfg(unix)]
#[test]
fn missing_context_and_fresh_binding_reject_vault_hard_link_scope_aliases() {
    let (vault_repository, _workspace, engine) = isolated_vault_external_engine();
    let denied = vault_repository.0.join("vault/personal/journal/secret.md");
    fs::create_dir_all(denied.parent().expect("secret fixture must have a parent"))
        .expect("denied fixture directory must be created");
    fs::write(&denied, "# Secret\nprivate journal bytes\n")
        .expect("denied fixture must be written");
    let candidate = "vault/work/cluml/experience/missing-context-denied-hard-link-alias.md";
    fs::hard_link(&denied, vault_repository.0.join(candidate))
        .expect("cross-scope hard-link fixture must be created");
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Personal,
        targets: vec!["README.md".to_owned()],
        objective: "review one personal document against explicit company evidence".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let grant = ContextGrant {
        owner: DataOwner::Company {
            company: "cluml".to_owned(),
        },
        purpose: ContextGrantPurpose::CareerWritingEvidence,
        access: ContextAccess::ReadOnly,
    };
    assert!(matches!(
        engine.resolve_with_career_composition(
            &request,
            std::slice::from_ref(&grant),
            Some(CareerOutputSurface::General),
            &[candidate.to_owned()],
            None,
        ),
        Err(HarnessError::InvalidRequest(message))
            if message.contains("must have exactly one hard link")
    ));

    let resolved = engine
        .resolve_with_context_grants(&request, std::slice::from_ref(&grant))
        .expect("granted review without explicit evidence must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let begin = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("review must begin");
    let reviewer = begin
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Reviewer)
        .expect("Reviewer must be ready");
    let event = missing_context_role_event(
        &resolved,
        reviewer,
        MissingContextCandidate::VaultEvidence {
            repository_relative_path: candidate.to_owned(),
        },
        "hard-link-scope-reviewer",
    );
    assert!(matches!(
        engine.advance_execution(&resolved, &prepared, &[], &begin.record, event),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("must have exactly one hard link")
    ));
}

#[cfg(unix)]
#[test]
fn missing_context_and_fresh_target_reject_external_vault_hard_link_aliases() {
    let (vault_repository, workspace, engine) = isolated_vault_external_engine();
    let denied = vault_repository
        .0
        .join("vault/personal/journal/external-target-secret.md");
    fs::create_dir_all(
        denied
            .parent()
            .expect("external target secret must have a parent"),
    )
    .expect("external target secret directory must be created");
    fs::write(&denied, "# External target secret\n")
        .expect("external target secret must be written");
    let candidate = "vault-secret-alias.md";
    fs::hard_link(&denied, workspace.0.join(candidate))
        .expect("external Vault hard-link alias must be created");
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Personal,
        targets: vec!["README.md".to_owned()],
        objective: "review one external personal document".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let mut fresh_request = request.clone();
    fresh_request.targets.push(candidate.to_owned());
    fresh_request.targets.sort();
    assert!(matches!(
        engine.resolve(&fresh_request),
        Err(HarnessError::InvalidRequest(message))
            if message.contains("must have exactly one hard link")
    ));

    let resolved = engine.resolve(&request).expect("base review must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let begin = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("base review must begin");
    let reviewer = begin
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Reviewer)
        .expect("Reviewer must be ready");
    let event = missing_context_role_event(
        &resolved,
        reviewer,
        MissingContextCandidate::AdditionalWorkspaceTarget {
            workspace_relative_path: candidate.to_owned(),
        },
        "external-vault-hard-link-reviewer",
    );
    assert!(matches!(
        engine.advance_execution(&resolved, &prepared, &[], &begin.record, event),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("must have exactly one hard link")
    ));
}

#[cfg(unix)]
#[test]
fn external_workspace_policy_rejects_a_vault_hard_link_alias() {
    let (vault_repository, workspace, engine) = isolated_vault_external_engine();
    let denied = vault_repository
        .0
        .join("vault/personal/journal/external-policy-secret.md");
    fs::create_dir_all(
        denied
            .parent()
            .expect("external policy secret must have a parent"),
    )
    .expect("external policy secret directory must be created");
    fs::write(&denied, "# External policy secret\n")
        .expect("external policy secret must be written");
    fs::hard_link(&denied, workspace.0.join("AGENTS.md"))
        .expect("external workspace policy alias must be created");
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Personal,
        targets: vec!["README.md".to_owned()],
        objective: "review one external personal document".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };

    assert!(matches!(
        engine.resolve(&request),
        Err(HarnessError::InvalidRequest(message))
            if message.contains("must have exactly one hard link")
    ));
}

#[cfg(unix)]
#[test]
fn missing_context_rejects_a_candidate_that_discovers_an_invalid_workspace_policy() {
    let (vault_repository, workspace, engine) = isolated_vault_external_engine();
    let denied = vault_repository
        .0
        .join("vault/personal/journal/nested-policy-secret.md");
    fs::create_dir_all(
        denied
            .parent()
            .expect("nested policy secret must have a parent"),
    )
    .expect("nested policy secret directory must be created");
    fs::write(&denied, "# Nested policy secret\n").expect("nested policy secret must be written");
    fs::create_dir_all(workspace.0.join("nested"))
        .expect("nested target directory must be created");
    let candidate = "nested/candidate.md";
    fs::write(workspace.0.join(candidate), "# Candidate\n")
        .expect("nested candidate must be written");
    fs::hard_link(&denied, workspace.0.join("nested/AGENTS.md"))
        .expect("nested workspace policy alias must be created");
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Personal,
        targets: vec!["README.md".to_owned()],
        objective: "review one external personal document".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let mut fresh_request = request.clone();
    fresh_request.targets.push(candidate.to_owned());
    fresh_request.targets.sort();
    assert!(matches!(
        engine.resolve(&fresh_request),
        Err(HarnessError::InvalidRequest(message))
            if message.contains("must have exactly one hard link")
    ));

    let resolved = engine.resolve(&request).expect("base review must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let begin = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("base review must begin");
    let reviewer = begin
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Reviewer)
        .expect("Reviewer must be ready");
    let event = missing_context_role_event(
        &resolved,
        reviewer,
        MissingContextCandidate::AdditionalWorkspaceTarget {
            workspace_relative_path: candidate.to_owned(),
        },
        "nested-policy-reviewer",
    );
    assert!(matches!(
        engine.advance_execution(&resolved, &prepared, &[], &begin.record, event),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("must have exactly one hard link")
    ));
}

#[cfg(unix)]
#[test]
fn missing_context_rejects_a_candidate_that_activates_an_invalid_vault_policy() {
    let (vault_repository, workspace, engine) = isolated_vault_external_engine();
    fs::create_dir_all(workspace.0.join("src")).expect("code fixture directory must be created");
    fs::write(workspace.0.join("src/main.js"), "export const value = 1;\n")
        .expect("non-Rust code fixture must be written");
    let candidate = "src/candidate.rs";
    fs::write(workspace.0.join(candidate), "pub fn candidate() {}\n")
        .expect("Rust candidate fixture must be written");
    let denied = vault_repository
        .0
        .join("vault/personal/journal/rust-policy-secret.md");
    fs::create_dir_all(
        denied
            .parent()
            .expect("Rust policy secret must have a parent"),
    )
    .expect("Rust policy secret directory must be created");
    fs::write(&denied, "# Rust policy secret\n").expect("Rust policy secret must be written");
    let rust_policy = vault_repository
        .0
        .join("vault/work/common/rules/rust-code-style.md");
    fs::remove_file(&rust_policy).expect("copied Rust policy must be replaced");
    fs::hard_link(&denied, &rust_policy).expect("Rust policy alias must be created");
    let request = HarnessRequest {
        action: HarnessAction::CodeReview,
        owner: DataOwner::PersonalProject {
            project: "coupler".to_owned(),
        },
        targets: vec!["src/main.js".to_owned()],
        objective: "review one non-Rust source file".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let mut fresh_request = request.clone();
    fresh_request.targets.push(candidate.to_owned());
    fresh_request.targets.sort();
    assert!(matches!(
        engine.resolve(&fresh_request),
        Err(HarnessError::InvalidRequest(message))
            if message.contains("must have exactly one hard link")
    ));

    let resolved = engine
        .resolve(&request)
        .expect("non-Rust base review must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let begin = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("non-Rust review must begin");
    let reviewer = begin
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Reviewer)
        .expect("Reviewer must be ready");
    let event = missing_context_role_event(
        &resolved,
        reviewer,
        MissingContextCandidate::AdditionalWorkspaceTarget {
            workspace_relative_path: candidate.to_owned(),
        },
        "rust-policy-reviewer",
    );
    assert!(matches!(
        engine.advance_execution(&resolved, &prepared, &[], &begin.record, event),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("must have exactly one hard link")
    ));
}

#[test]
fn missing_context_rejects_a_candidate_outside_the_profile_source_intent() {
    let vault_repository = TemporaryWorkspace::create();
    copy_directory(
        &repository_root().join("vault"),
        &vault_repository.0.join("vault"),
    );
    let candidate = "vault/profile/facts/missing-context-profile-source.md";
    fs::create_dir_all(
        vault_repository
            .0
            .join(candidate)
            .parent()
            .expect("profile source candidate must have a parent"),
    )
    .expect("profile source candidate directory must be created");
    fs::write(vault_repository.0.join(candidate), "# Profile fact\n")
        .expect("profile source candidate must be written");
    let engine = HarnessEngine::open(&vault_repository.0, &vault_repository.0)
        .expect("isolated Vault workspace must open");
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Profile,
        targets: vec!["vault/profile/preferences/synthetic-routing-policy.md".to_owned()],
        objective: "review one canonical profile policy source".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let mut fresh_request = request.clone();
    fresh_request.targets.push(candidate.to_owned());
    fresh_request.targets.sort();
    assert!(
        engine
            .resolve_with_intent(&fresh_request, HarnessIntent::PolicyMaintenance)
            .is_err()
    );

    let resolved = engine
        .resolve_with_intent(&request, HarnessIntent::PolicyMaintenance)
        .expect("base profile review must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let begin = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("base profile review must begin");
    let reviewer = begin
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Reviewer)
        .expect("Reviewer must be ready");
    let event = missing_context_role_event(
        &resolved,
        reviewer,
        MissingContextCandidate::AdditionalWorkspaceTarget {
            workspace_relative_path: candidate.to_owned(),
        },
        "profile-source-intent-reviewer",
    );
    assert!(matches!(
        engine.advance_execution(&resolved, &prepared, &[], &begin.record, event),
        Err(HarnessError::InvalidSubmission(_))
    ));
}

#[cfg(unix)]
#[test]
fn vault_workspace_policy_rejects_a_denied_hard_link_alias() {
    let vault_repository = TemporaryWorkspace::create();
    copy_directory(
        &repository_root().join("vault"),
        &vault_repository.0.join("vault"),
    );
    let target = "vault/profile/preferences/workspace-policy-review-target.md";
    fs::write(vault_repository.0.join(target), "# Review target\n")
        .expect("Vault workspace review target must be written");
    let denied = vault_repository
        .0
        .join("vault/personal/journal/workspace-policy-secret.md");
    fs::create_dir_all(
        denied
            .parent()
            .expect("Vault workspace policy secret must have a parent"),
    )
    .expect("Vault workspace policy secret directory must be created");
    fs::write(&denied, "# Vault workspace policy secret\n")
        .expect("Vault workspace policy secret must be written");
    fs::hard_link(&denied, vault_repository.0.join("AGENTS.md"))
        .expect("Vault workspace policy alias must be created");
    let engine = HarnessEngine::open(&vault_repository.0, &vault_repository.0)
        .expect("isolated Vault workspace must open");
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Profile,
        targets: vec![target.to_owned()],
        objective: "review one profile preference document".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };

    assert!(matches!(
        engine.resolve_with_intent(&request, HarnessIntent::PolicyMaintenance),
        Err(HarnessError::InvalidRequest(message))
            if message.contains("must have exactly one hard link")
    ));
}

#[cfg(unix)]
#[test]
fn missing_context_and_fresh_target_reject_vault_workspace_hard_link_aliases() {
    let vault_repository = TemporaryWorkspace::create();
    copy_directory(
        &repository_root().join("vault"),
        &vault_repository.0.join("vault"),
    );
    let review_target = "vault/profile/preferences/hard-link-review-target.md";
    fs::write(vault_repository.0.join(review_target), "# Review target\n")
        .expect("profile review target must be written");
    let denied = vault_repository
        .0
        .join("vault/personal/journal/target-secret.md");
    fs::create_dir_all(
        denied
            .parent()
            .expect("target secret fixture must have a parent"),
    )
    .expect("target secret directory must be created");
    fs::write(&denied, "# Secret target bytes\n").expect("target secret fixture must be written");
    let candidate = "vault/profile/preferences/target-secret-alias.md";
    fs::hard_link(&denied, vault_repository.0.join(candidate))
        .expect("Vault-workspace cross-scope alias must be created");
    let engine = HarnessEngine::open(&vault_repository.0, &vault_repository.0)
        .expect("isolated Vault workspace must open");
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Profile,
        targets: vec![review_target.to_owned()],
        objective: "review one profile preference document".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let mut fresh_request = request.clone();
    fresh_request.targets.push(candidate.to_owned());
    fresh_request.targets.sort();
    assert!(matches!(
        engine.resolve_with_intent(&fresh_request, HarnessIntent::PolicyMaintenance),
        Err(HarnessError::InvalidRequest(message))
            if message.contains("must have exactly one hard link")
    ));

    let resolved = engine
        .resolve_with_intent(&request, HarnessIntent::PolicyMaintenance)
        .expect("base profile review must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let begin = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("profile review must begin");
    let reviewer = begin
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Reviewer)
        .expect("Reviewer must be ready");
    let event = missing_context_role_event(
        &resolved,
        reviewer,
        MissingContextCandidate::AdditionalWorkspaceTarget {
            workspace_relative_path: candidate.to_owned(),
        },
        "vault-workspace-hard-link-reviewer",
    );
    assert!(matches!(
        engine.advance_execution(&resolved, &prepared, &[], &begin.record, event),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("must have exactly one hard link")
    ));
}

#[test]
fn unsupported_and_missing_context_remain_distinct_terminal_outcomes() {
    let engine = engine();
    let resolved = engine
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let (frontier, reviewer) =
        code_reviewer_frontier(&engine, &resolved, &prepared, "unsupported-writer");
    let mut event = missing_context_role_event(
        &resolved,
        &reviewer,
        MissingContextCandidate::AdditionalWorkspaceTarget {
            workspace_relative_path: "crates/context-core/src/document.rs".to_owned(),
        },
        "unsupported-reviewer",
    );
    let HarnessExecutionEvent::RoleResult { result } = &mut event else {
        panic!("event must contain a role result");
    };
    result.outcome = RoleExecutionOutcome::Unsupported {
        reason: "runtime cannot name a valid context path".to_owned(),
    };
    result.lifecycle.first_output_at_millis = None;
    result.lifecycle.terminal_state = RoleTerminalState::Unsupported;
    refresh_role_event_digest(&mut event);
    let terminal = engine
        .advance_execution(&resolved, &prepared, &[], &frontier.record, event)
        .expect("unsupported result must remain a valid terminal outcome");
    let evaluation = engine
        .evaluate_execution(&resolved, &prepared, &[], &terminal.record)
        .expect("unsupported terminal record must evaluate");

    assert_eq!(evaluation.execution_status, ExecutionStatus::Unsupported);
    assert_eq!(
        evaluation.completion_state,
        HarnessCompletionState::ExecutionHalted
    );
    assert!(evaluation.missing_context.is_none());
}

#[test]
fn reviewer_can_name_one_unbound_granted_vault_evidence_file() {
    let (_workspace, engine) = external_engine();
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Personal,
        targets: vec!["README.md".to_owned()],
        objective: "review career writing against one company source".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let grant = ContextGrant {
        owner: DataOwner::Company {
            company: "cluml".to_owned(),
        },
        purpose: ContextGrantPurpose::CareerWritingEvidence,
        access: ContextAccess::ReadOnly,
    };
    let resolved = engine
        .resolve_with_context_grants(&request, std::slice::from_ref(&grant))
        .expect("granted review must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let begin = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("review must begin");
    let reviewer = begin
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Reviewer)
        .expect("Reviewer must be ready")
        .clone();
    let candidate =
        "vault/work/cluml/experience/case-studies/hog-detection-period-config-externalization.md";
    assert!(!prepared.role_bundles.iter().any(|bundle| {
        bundle
            .bound_documents()
            .any(|document| document.relative_path == candidate)
    }));
    let event = missing_context_role_event(
        &resolved,
        &reviewer,
        MissingContextCandidate::VaultEvidence {
            repository_relative_path: candidate.to_owned(),
        },
        "granted-vault-evidence-reviewer",
    );
    let terminal = engine
        .advance_execution(&resolved, &prepared, &[], &begin.record, event)
        .expect("unbound evidence inside the exact grant must be accepted");
    let evaluation = engine
        .evaluate_execution(&resolved, &prepared, &[], &terminal.record)
        .expect("missing granted evidence must evaluate");

    assert_eq!(evaluation.execution_status, ExecutionStatus::MissingContext);
    assert!(matches!(
        evaluation
            .missing_context
            .expect("evaluation must expose the request")
            .candidate,
        MissingContextCandidate::VaultEvidence {
            repository_relative_path
        } if repository_relative_path == candidate
    ));

    let rebound = engine
        .resolve_with_career_composition(
            &request,
            std::slice::from_ref(&grant),
            Some(CareerOutputSurface::General),
            &[candidate.to_owned()],
            None,
        )
        .expect("the exact candidate must be representable in a fresh request");
    assert!(
        rebound
            .plan
            .evidence_sources
            .iter()
            .any(|source| { source.repository_relative_path == candidate })
    );
    let rebound_prepared = prepared_run_with(&engine, &rebound);
    assert!(rebound_prepared.role_bundles.iter().any(|bundle| {
        bundle.bound_documents().any(|document| {
            document.source == HarnessBoundDocumentSource::Evidence
                && document.relative_path == candidate
        })
    }));
}

#[test]
fn missing_context_rejects_vault_evidence_that_would_exceed_the_fresh_request_limit() {
    let (vault_repository, _workspace, engine) = isolated_vault_external_engine();
    let evidence_root = vault_repository.0.join("vault/work/cluml/experience");
    let evidence_paths = (0..=MAX_TARGETS)
        .map(|index| {
            let relative =
                format!("vault/work/cluml/experience/missing-context-limit-{index:02}.md");
            fs::write(
                vault_repository.0.join(&relative),
                format!("# Evidence {index}\n"),
            )
            .expect("bounded evidence fixture must be written");
            relative
        })
        .collect::<Vec<_>>();
    assert!(evidence_root.is_dir());
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Personal,
        targets: vec!["README.md".to_owned()],
        objective: "review one document at the exact evidence-source limit".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let grant = ContextGrant {
        owner: DataOwner::Company {
            company: "cluml".to_owned(),
        },
        purpose: ContextGrantPurpose::CareerWritingEvidence,
        access: ContextAccess::ReadOnly,
    };
    let resolved = engine
        .resolve_with_career_composition(
            &request,
            std::slice::from_ref(&grant),
            Some(CareerOutputSurface::General),
            &evidence_paths[..MAX_TARGETS],
            None,
        )
        .expect("request at the exact evidence-source limit must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let begin = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("review must begin");
    let reviewer = begin
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Reviewer)
        .expect("Reviewer must be ready");
    let event = missing_context_role_event(
        &resolved,
        reviewer,
        MissingContextCandidate::VaultEvidence {
            repository_relative_path: evidence_paths[MAX_TARGETS].clone(),
        },
        "evidence-limit-reviewer",
    );

    assert!(matches!(
        engine.advance_execution(&resolved, &prepared, &[], &begin.record, event),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("between one and 64 paths")
    ));
}

#[cfg(unix)]
#[test]
fn missing_context_candidate_rejects_a_workspace_symlink_escape() {
    use std::os::unix::fs::symlink;

    let (workspace, engine) = external_engine();
    let outside = TemporaryWorkspace::create();
    fs::write(outside.0.join("outside.rs"), "pub fn outside() {}\n")
        .expect("outside fixture must be written");
    symlink(
        outside.0.join("outside.rs"),
        workspace.0.join("src/escape.rs"),
    )
    .expect("escape symlink must be created");
    let resolved = engine
        .resolve(&personal_project_code_request())
        .expect("external request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let (frontier, reviewer) =
        code_reviewer_frontier(&engine, &resolved, &prepared, "symlink-writer");
    let event = missing_context_role_event(
        &resolved,
        &reviewer,
        MissingContextCandidate::AdditionalWorkspaceTarget {
            workspace_relative_path: "src/escape.rs".to_owned(),
        },
        "symlink-reviewer",
    );

    assert!(matches!(
        engine.advance_execution(&resolved, &prepared, &[], &frontier.record, event),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("missing context candidate is invalid")
    ));
}

#[test]
fn strict_workflow_fans_out_from_writer_to_independent_checks() {
    let engine = strict_engine();
    let resolved = engine
        .resolve(&profile_code_request())
        .expect("strict code request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let begin = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("strict execution must begin");
    let writer_event = completed_role_event(
        &resolved,
        &prepared,
        &begin.ready_role_invocations[0],
        Vec::new(),
        "strict-producer",
    );
    let writer_result = match &writer_event {
        HarnessExecutionEvent::RoleResult { result } => (**result).clone(),
        HarnessExecutionEvent::ToolEvidence { .. } => panic!("Writer must return a role result"),
    };
    let step = engine
        .advance_execution(&resolved, &prepared, &[], &begin.record, writer_event)
        .expect("Writer result must advance");

    assert_eq!(step.ready_role_invocations.len(), 2);
    let verifier_invocation = step
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Verifier)
        .expect("Verifier must be issued from the producer frontier");
    let reviewer_invocation = step
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Reviewer)
        .expect("Reviewer must be issued independently from the producer frontier")
        .clone();
    assert_eq!(verifier_invocation.role, HarnessRole::Verifier);
    assert_eq!(
        verifier_invocation.predecessor_results,
        vec![writer_result.clone()]
    );
    assert_eq!(
        reviewer_invocation.predecessor_results,
        vec![writer_result.clone()]
    );
    let verifier_event = completed_role_event(
        &resolved,
        &prepared,
        verifier_invocation,
        Vec::new(),
        "strict-verifier",
    );
    let verifier_result = match &verifier_event {
        HarnessExecutionEvent::RoleResult { result } => (**result).clone(),
        HarnessExecutionEvent::ToolEvidence { .. } => panic!("Verifier must return a role result"),
    };
    let advanced = engine
        .advance_execution(&resolved, &prepared, &[], &step.record, verifier_event)
        .expect("Verifier result must advance");
    assert_eq!(
        advanced.ready_role_invocations,
        vec![reviewer_invocation.clone()]
    );
    let mut forged = reviewer_invocation;
    forged.predecessor_results.push(verifier_result);
    refresh_role_invocation_digest(&mut forged);
    assert_invocation_predecessor_results_rejected(
        &resolved.plan,
        &prepared,
        &advanced.record,
        &forged,
    );
}

#[test]
fn noncompleted_predecessor_closes_the_workflow_frontier() {
    let engine = strict_engine();
    let resolved = engine
        .resolve(&profile_code_request())
        .expect("strict code request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let begin = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("strict execution must begin");
    let writer_event = completed_role_event(
        &resolved,
        &prepared,
        &begin.ready_role_invocations[0],
        Vec::new(),
        "halt-writer",
    );
    let verifier_step = engine
        .advance_execution(&resolved, &prepared, &[], &begin.record, writer_event)
        .expect("Writer result must advance");
    let verifier = &verifier_step.ready_role_invocations[0];
    let context_id = "halt-verifier".to_owned();
    let lifecycle = ReportedRoleLifecycle {
        role: HarnessRole::Verifier,
        context_id: context_id.clone(),
        started_at_millis: 1,
        context_ready_at_millis: 2,
        first_output_at_millis: None,
        interrupt_requested_at_millis: Some(3),
        grace_deadline_at_millis: Some(5),
        terminal_at_millis: 5,
        closed_at_millis: 6,
        terminal_state: RoleTerminalState::TimedOut,
    };
    let outcome = RoleExecutionOutcome::TimedOut;
    let result_digest = serialized_digest(&(
        HarnessRole::Verifier,
        &verifier.invocation_digest,
        &context_id,
        &lifecycle,
        &outcome,
    ))
    .expect("timeout result must serialize");
    let terminal = engine
        .advance_execution(
            &resolved,
            &prepared,
            &[],
            &verifier_step.record,
            HarnessExecutionEvent::RoleResult {
                result: Box::new(RoleExecutionResult {
                    role: HarnessRole::Verifier,
                    invocation_digest: verifier.invocation_digest.clone(),
                    context_id,
                    lifecycle,
                    outcome,
                    result_digest,
                }),
            },
        )
        .expect("non-completed predecessor must close the frontier");

    assert!(terminal.ready_role_invocations.is_empty());
    assert!(terminal.ready_tool_invocation.is_none());
    assert!(
        !terminal
            .record
            .role_results
            .iter()
            .any(|result| result.role == HarnessRole::Reviewer)
    );
    let evaluation = engine
        .evaluate_execution(&resolved, &prepared, &[], &terminal.record)
        .expect("halted graph must evaluate");
    assert_eq!(evaluation.execution_status, ExecutionStatus::TimedOut);
    assert_eq!(
        evaluation.completion_state,
        HarnessCompletionState::ExecutionHalted
    );
}

#[test]
fn later_terminal_result_persists_after_an_independent_sibling_completed_first() {
    let engine = strict_engine();
    let resolved = engine
        .resolve(&profile_code_request())
        .expect("strict code request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let begin = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("strict execution must begin");
    let writer_event = completed_role_event(
        &resolved,
        &prepared,
        &begin.ready_role_invocations[0],
        Vec::new(),
        "out-of-order-writer",
    );
    let frontier = engine
        .advance_execution(&resolved, &prepared, &[], &begin.record, writer_event)
        .expect("producer result must issue the independent frontier");
    let reviewer = frontier
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Reviewer)
        .expect("Reviewer must be ready")
        .clone();
    let verifier = frontier
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Verifier)
        .expect("Verifier must be ready")
        .clone();
    let after_reviewer = engine
        .advance_execution(
            &resolved,
            &prepared,
            &[],
            &frontier.record,
            completed_role_event(
                &resolved,
                &prepared,
                &reviewer,
                Vec::new(),
                "out-of-order-reviewer",
            ),
        )
        .expect("the independently completed Reviewer result must persist");
    assert_eq!(
        after_reviewer.ready_role_invocations,
        vec![verifier.clone()]
    );

    let terminal = engine
        .advance_execution(
            &resolved,
            &prepared,
            &[],
            &after_reviewer.record,
            timed_out_role_event(&verifier, "out-of-order-verifier"),
        )
        .expect("the later terminal sibling must persist without erasing prior work");
    assert!(terminal.ready_role_invocations.is_empty());
    assert_eq!(terminal.record.role_results.len(), 3);
    assert_eq!(
        terminal.record.accepted_role_order,
        vec![
            HarnessRole::Writer,
            HarnessRole::Reviewer,
            HarnessRole::Verifier,
        ]
    );
    let evaluation = engine
        .evaluate_execution(&resolved, &prepared, &[], &terminal.record)
        .expect("the persisted terminal frontier must evaluate");
    assert_eq!(evaluation.execution_status, ExecutionStatus::TimedOut);
    assert_eq!(
        evaluation.completion_state,
        HarnessCompletionState::ExecutionHalted
    );

    let mut forged = terminal.record;
    forged.accepted_role_order = vec![
        HarnessRole::Writer,
        HarnessRole::Verifier,
        HarnessRole::Reviewer,
    ];
    refresh_role_execution_record_digest(&mut forged);
    assert!(matches!(
        engine.evaluate_execution(&resolved, &prepared, &[], &forged),
        Err(HarnessError::InvalidSubmission(message))
            if message == "no role result may be accepted after a terminal role result"
    ));
}

#[test]
fn aggregate_role_record_is_rejected_before_it_can_become_current() {
    let engine = engine();
    let resolved = engine
        .resolve(&profile_code_request())
        .expect("standard code request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let begin = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("execution must begin");
    let after_writer = engine
        .advance_execution(
            &resolved,
            &prepared,
            &[],
            &begin.record,
            completed_role_event(
                &resolved,
                &prepared,
                &begin.ready_role_invocations[0],
                Vec::new(),
                "aggregate-writer",
            ),
        )
        .expect("small producer result must advance");
    let reviewer = after_writer.ready_role_invocations[0].clone();
    let mut oversized = completed_role_event(
        &resolved,
        &prepared,
        &reviewer,
        Vec::new(),
        "aggregate-reviewer",
    );
    let HarnessExecutionEvent::RoleResult { result } = &mut oversized else {
        panic!("Reviewer event must contain a role result");
    };
    let RoleExecutionOutcome::Completed {
        result:
            CompletedRoleResult::Reviewer {
                summary,
                improvements,
                ..
            },
    } = &mut result.outcome
    else {
        panic!("Reviewer event must contain a completed Reviewer result");
    };
    *summary = "s".repeat(9 * 1024 * 1024);
    improvements.push(reviewer_improvement_observation(
        &resolved,
        &reviewer.subject,
        &"i".repeat(9 * 1024 * 1024),
    ));
    refresh_role_result_digest(result);

    assert!(matches!(
        engine.advance_execution(&resolved, &prepared, &[], &after_writer.record, oversized),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("role execution record exceeds")
                && message.contains("aggregate limit")
    ));
}

#[test]
fn analysis_reviewer_receives_the_exact_specialist_result() {
    let engine = engine();
    let mut request = profile_code_request();
    request.action = HarnessAction::Investigation;
    request.objective = "inspect the Harness implementation".to_owned();
    let resolved = engine
        .resolve(&request)
        .expect("investigation must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let begin = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("investigation must begin");
    assert_eq!(
        begin.ready_role_invocations[0].role,
        HarnessRole::Specialist
    );
    assert_invocation_dynamic_context(&begin.ready_role_invocations[0], None);
    let specialist_event = completed_role_event(
        &resolved,
        &prepared,
        &begin.ready_role_invocations[0],
        Vec::new(),
        "specialist-producer",
    );
    let specialist_result = match &specialist_event {
        HarnessExecutionEvent::RoleResult { result } => (**result).clone(),
        HarnessExecutionEvent::ToolEvidence { .. } => {
            panic!("Specialist must return a role result")
        }
    };
    let step = engine
        .advance_execution(&resolved, &prepared, &[], &begin.record, specialist_event)
        .expect("Specialist result must advance");
    let reviewer = step
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Reviewer)
        .expect("Reviewer must receive the Specialist result");

    assert_eq!(reviewer.predecessor_results, vec![specialist_result]);
    assert_invocation_dynamic_context(reviewer, None);
    assert!(matches!(
        reviewer.subject,
        EvaluationSubject::ProducedArtifact { .. }
    ));
}

#[test]
fn role_invocation_size_uses_the_final_canonical_serialization_boundary() {
    let engine = engine();
    let resolved = engine
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let begin = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("execution must begin");
    let writer_event = completed_role_event(
        &resolved,
        &prepared,
        &begin.ready_role_invocations[0],
        Vec::new(),
        "invocation-size-producer",
    );
    let step = engine
        .advance_execution(&resolved, &prepared, &[], &begin.record, writer_event)
        .expect("Writer result must produce a downstream invocation");
    let invocation = step
        .ready_role_invocations
        .iter()
        .find(|candidate| candidate.role == HarnessRole::Reviewer)
        .expect("Reviewer invocation must embed the Writer result");
    assert!(!invocation.predecessor_results.is_empty());
    assert!(matches!(
        invocation.segments.first(),
        Some(RoleInvocationSegment::ControlHead { .. })
    ));
    assert!(matches!(
        invocation.segments.last(),
        Some(RoleInvocationSegment::ControlTail { .. })
    ));
    assert!(matches!(
        invocation
            .segments
            .get(invocation.segments.len().saturating_sub(2)),
        Some(RoleInvocationSegment::DynamicContext { .. })
    ));
    assert_eq!(
        invocation
            .segments
            .iter()
            .filter(|segment| matches!(segment, RoleInvocationSegment::DynamicContext { .. }))
            .count(),
        1
    );
    let serialized_invocation =
        serde_json::to_string(invocation).expect("invocation must serialize");
    let duplicate_predecessor_results = serialized_invocation.replacen(
        "\"predecessor_results\":",
        "\"predecessor_results\":[],\"predecessor_results\":",
        1,
    );
    assert!(
        decode_current_json::<RoleInvocationContract>(
            duplicate_predecessor_results.as_bytes(),
            "role invocation",
        )
        .is_err(),
        "current role invocations must reject a duplicated lineage field before normalization"
    );
    let serialized_bytes = serde_json::to_vec(invocation)
        .expect("invocation must serialize")
        .len();
    let advance_with_limit = |limit, context_suffix: &str| {
        let mut capabilities = complete_runtime_capabilities();
        capabilities.max_role_invocation_bytes = limit;
        let limited_prepared = engine.prepare(&resolved, &capabilities, None)?;
        let limited_begin = engine.begin_execution(&resolved, &limited_prepared, &[])?;
        let limited_writer = completed_role_event(
            &resolved,
            &limited_prepared,
            &limited_begin.ready_role_invocations[0],
            Vec::new(),
            context_suffix,
        );
        engine.advance_execution(
            &resolved,
            &limited_prepared,
            &[],
            &limited_begin.record,
            limited_writer,
        )
    };
    let exact = advance_with_limit(serialized_bytes, "invocation-size-producer")
        .expect("the production path must accept an invocation exactly at the runtime limit");
    let exact_invocation = exact
        .ready_role_invocations
        .iter()
        .find(|candidate| candidate.role == HarnessRole::Reviewer)
        .expect("exact-limit execution must issue the Reviewer invocation");
    assert_eq!(
        serde_json::to_vec(exact_invocation).unwrap().len(),
        serialized_bytes
    );
    assert!(matches!(
        advance_with_limit(serialized_bytes - 1, "invocation-size-producer"),
        Err(HarnessError::UnsupportedRuntime(message))
            if message.contains("invocation is")
    ));
}

#[test]
fn role_invocation_context_budget_includes_dynamic_predecessor_material() {
    let engine = engine();
    let resolved = engine
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let reviewer_bundle_bound = prepared
        .role_bundles
        .iter()
        .find(|bundle| bundle.role == HarnessRole::Reviewer)
        .expect("Reviewer bundle must exist")
        .serialized_bytes;
    let mut context_limited_capabilities = complete_runtime_capabilities();
    context_limited_capabilities.max_role_bundle_bytes = reviewer_bundle_bound;
    let context_limited = engine
        .prepare(&resolved, &context_limited_capabilities, None)
        .expect("the prepared Reviewer bundle must fit exactly");
    let context_limited_begin = engine
        .begin_execution(&resolved, &context_limited, &[])
        .expect("context-limited execution must begin");
    let mut context_limited_writer = completed_role_event(
        &resolved,
        &context_limited,
        &context_limited_begin.ready_role_invocations[0],
        Vec::new(),
        "invocation-context-limit-producer",
    );
    let HarnessExecutionEvent::RoleResult { result } = &mut context_limited_writer else {
        panic!("Writer must return a role result");
    };
    let RoleExecutionOutcome::Completed {
        result:
            CompletedRoleResult::Writer {
                artifact: WriterArtifact::Changes { changes },
            },
    } = &mut result.outcome
    else {
        panic!("Writer must produce changes");
    };
    let FileChange::Update { content, .. } = &mut changes[0] else {
        panic!("Writer fixture must update its target");
    };
    *content = "x".repeat(reviewer_bundle_bound);
    refresh_role_result_digest(result);
    assert!(matches!(
        engine.advance_execution(
            &resolved,
            &context_limited,
            &[],
            &context_limited_begin.record,
            context_limited_writer,
        ),
        Err(HarnessError::UnsupportedRuntime(message))
            if message.contains("role-input limit")
    ));
}

#[test]
fn downstream_invocation_rejects_every_predecessor_lineage_tamper() {
    let engine = engine();
    let resolved = engine
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let begin = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("execution must begin");
    let writer_event = completed_role_event(
        &resolved,
        &prepared,
        &begin.ready_role_invocations[0],
        Vec::new(),
        "0",
    );
    let step = engine
        .advance_execution(&resolved, &prepared, &[], &begin.record, writer_event)
        .expect("Writer result must advance");
    let invocation = step
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Reviewer)
        .expect("Reviewer must be ready")
        .clone();
    validate_invocation_predecessor_results(&resolved.plan, &prepared, &step.record, &invocation)
        .expect("Core-issued invocation must bind the exact producer result");

    let mut missing = invocation.clone();
    missing.predecessor_results.clear();
    refresh_role_invocation_digest(&mut missing);
    assert_invocation_predecessor_results_rejected(
        &resolved.plan,
        &prepared,
        &step.record,
        &missing,
    );

    let changed_artifact = invocation_with_changed_writer_content(&invocation);
    assert_invocation_predecessor_results_rejected(
        &resolved.plan,
        &prepared,
        &step.record,
        &changed_artifact,
    );

    let mut changed_result_digest = invocation.clone();
    changed_result_digest
        .predecessor_results
        .first_mut()
        .expect("downstream invocation must contain a predecessor")
        .result_digest = "f".repeat(64);
    refresh_role_invocation_digest(&mut changed_result_digest);
    assert_invocation_predecessor_results_rejected(
        &resolved.plan,
        &prepared,
        &step.record,
        &changed_result_digest,
    );

    let mut changed_subject = invocation.clone();
    let EvaluationSubject::ProducedArtifact { artifact_digest } = &mut changed_subject.subject
    else {
        panic!("downstream invocation must review a produced artifact");
    };
    *artifact_digest = "e".repeat(64);
    refresh_role_invocation_digest(&mut changed_subject);
    assert_invocation_predecessor_results_rejected(
        &resolved.plan,
        &prepared,
        &step.record,
        &changed_subject,
    );

    let mut changed_role = invocation.clone();
    let producer = changed_role
        .predecessor_results
        .first_mut()
        .expect("downstream invocation must contain a predecessor");
    producer.role = HarnessRole::Reviewer;
    refresh_role_result_digest(producer);
    refresh_role_invocation_digest(&mut changed_role);
    assert_invocation_predecessor_results_rejected(
        &resolved.plan,
        &prepared,
        &step.record,
        &changed_role,
    );

    assert!(matches!(
        engine.advance_execution(
            &resolved,
            &prepared,
            &[],
            &step.record,
            completed_role_event(&resolved, &prepared, &missing, Vec::new(), "0"),
        ),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("Harness-issued invocation")
    ));
}

fn rejected_execution_with_corrections(
    engine: &HarnessEngine,
    resolved: &ResolvedHarnessRequest,
    prepared: &PreparedRoleRun,
) -> HarnessTaskEvaluation {
    let mut step = engine
        .begin_execution(resolved, prepared, &[])
        .expect("execution must begin");
    let writer_invocation = step.ready_role_invocations[0].clone();
    assert_invocation_dynamic_context(&writer_invocation, Some(&step.record.revision_contract));
    step = engine
        .advance_execution(
            resolved,
            prepared,
            &[],
            &step.record,
            completed_role_event(resolved, prepared, &writer_invocation, Vec::new(), "0"),
        )
        .expect("Writer result must advance");
    let tool_invocation = step
        .ready_tool_invocation
        .clone()
        .expect("tool evidence must be ready");
    step = engine
        .advance_execution(
            resolved,
            prepared,
            &[],
            &step.record,
            passed_tool_event(&tool_invocation),
        )
        .expect("tool evidence must advance");
    let reviewer_invocation = step
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Reviewer)
        .expect("Reviewer must be ready")
        .clone();
    let finding = BlockingFinding {
        message: "candidate requires correction".to_owned(),
        evidence: vec![subject_evidence(
            &resolved.plan,
            &reviewer_invocation.subject,
        )],
    };
    let mut reviewer_event =
        completed_role_event(resolved, prepared, &reviewer_invocation, vec![finding], "0");
    let HarnessExecutionEvent::RoleResult { result } = &mut reviewer_event else {
        panic!("Reviewer event must contain a role result");
    };
    let RoleExecutionOutcome::Completed {
        result:
            CompletedRoleResult::Reviewer {
                requirement_results,
                ..
            },
    } = &mut result.outcome
    else {
        panic!("Reviewer result must be completed");
    };
    let completion = requirement_results
        .iter_mut()
        .find(|result| result.unit == VerificationUnit::CompletionContract)
        .expect("Reviewer must own the completion requirement");
    completion.passed = false;
    completion.detail = "completion requirement failed".to_owned();
    refresh_role_event_digest(&mut reviewer_event);
    step = engine
        .advance_execution(resolved, prepared, &[], &step.record, reviewer_event)
        .expect("failed Reviewer check must still advance");
    let record = complete_execution_step_with_improvements(
        engine,
        resolved,
        prepared,
        &[],
        step,
        (&[], &[]),
        "0",
    );
    let rejected = engine
        .evaluate_execution(resolved, prepared, &[], &record)
        .expect("terminal record must evaluate");
    assert_eq!(rejected.subject_status, SubjectStatus::Rejected);
    assert_eq!(
        rejected.completion_state,
        HarnessCompletionState::RevisionRequired
    );
    rejected
}

#[test]
fn revision_contract_is_derived_from_the_previous_core_evaluation() {
    let engine = strict_engine();
    let resolved = engine
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let rejected = rejected_execution_with_corrections(&engine, &resolved, &prepared);
    let history = std::slice::from_ref(&rejected);
    let next = engine
        .begin_execution(&resolved, &prepared, history)
        .expect("rejected write candidate may start a revision");
    let contract = &next.record.revision_contract;
    assert_eq!(contract.revision, 1);
    assert!(contract.previous_candidate_digest.is_some());
    assert_eq!(
        contract.previous_candidate_digest,
        rejected.candidate_digest
    );
    assert_eq!(
        contract.previous_task_evaluation_receipt_digest.as_ref(),
        Some(&rejected.task_evaluation_receipt_digest)
    );
    let [
        RevisionCorrection::FailedRequirement { unit, detail },
        RevisionCorrection::BlockingFinding {
            finding_digest,
            finding,
        },
    ] = contract.corrections.as_slice()
    else {
        panic!("revision must retain the failed requirement and blocking finding in order");
    };
    assert_eq!(*unit, VerificationUnit::CompletionContract);
    assert_eq!(detail, "completion requirement failed");
    assert_eq!(rejected.blocking_findings, vec![finding.clone()]);
    assert_eq!(
        *finding_digest,
        serialized_digest(finding).expect("blocking finding must serialize")
    );
    let writer = next
        .ready_role_invocations
        .first()
        .expect("revised Writer must be ready");
    assert_eq!(writer.role, HarnessRole::Writer);
    assert_eq!(
        writer.revision_contract_digest,
        contract.revision_contract_digest
    );
    assert_invocation_dynamic_context(writer, Some(contract));
    let frontier = engine
        .advance_execution(
            &resolved,
            &prepared,
            history,
            &next.record,
            completed_role_event(&resolved, &prepared, writer, Vec::new(), "1"),
        )
        .expect("revised Writer must advance to independent checks");
    assert_eq!(
        frontier
            .ready_role_invocations
            .iter()
            .map(|invocation| invocation.role)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([HarnessRole::Reviewer, HarnessRole::Verifier])
    );
    for invocation in &frontier.ready_role_invocations {
        assert_invocation_dynamic_context(invocation, None);
    }
}

#[test]
fn revised_writer_rejects_changed_corrections_with_recomputed_public_hashes() {
    let engine = strict_engine();
    let resolved = engine
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let rejected = rejected_execution_with_corrections(&engine, &resolved, &prepared);
    let history = std::slice::from_ref(&rejected);
    let next = engine
        .begin_execution(&resolved, &prepared, history)
        .expect("rejected write candidate may start a revision");
    let writer = next
        .ready_role_invocations
        .first()
        .expect("revised Writer must be ready");
    assert!(
        validate_invocation_predecessor_results(&resolved.plan, &prepared, &next.record, writer)
            .is_ok()
    );
    let changed =
        invocation_with_changed_correction(&prepared, writer, &next.record.revision_contract);
    assert_ne!(changed.role_input_digest, writer.role_input_digest);
    assert_ne!(changed.invocation_digest, writer.invocation_digest);
    assert!(matches!(
        validate_invocation_predecessor_results(
            &resolved.plan,
            &prepared,
            &next.record,
            &changed,
        ),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("exact bounded segment sequence")
    ));
    assert!(matches!(
        engine.advance_execution(
            &resolved,
            &prepared,
            history,
            &next.record,
            completed_role_event(&resolved, &prepared, &changed, Vec::new(), "forged"),
        ),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("Harness-issued invocation")
    ));
}

#[test]
fn professional_profile_reviewer_rejects_passed_requirement_with_improvements() {
    let (_workspace, engine) = external_engine();
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Personal,
        targets: vec!["README.md".to_owned()],
        objective: "review a professional profile".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = engine
        .resolve_with_career_surface(&request, CareerOutputSurface::ProfessionalProfile)
        .expect("professional profile review must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let step = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("professional profile review must begin");
    let reviewer = step
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Reviewer)
        .expect("Reviewer must be ready");
    let mut event = completed_role_event(
        &resolved,
        &prepared,
        reviewer,
        Vec::new(),
        "professional-profile-contradiction",
    );
    let HarnessExecutionEvent::RoleResult { result } = &mut event else {
        panic!("Reviewer event must contain a role result");
    };
    let RoleExecutionOutcome::Completed {
        result:
            CompletedRoleResult::Reviewer {
                requirement_results,
                improvements,
                ..
            },
    } = &mut result.outcome
    else {
        panic!("Reviewer result must be completed");
    };
    assert!(
        requirement_results
            .iter()
            .find(|result| result.unit == VerificationUnit::ProfessionalProfileArtifact)
            .expect("professional profile requirement must exist")
            .passed
    );
    improvements.push(reviewer_improvement_observation(
        &resolved,
        &reviewer.subject,
        "clarify the scope",
    ));
    refresh_role_event_digest(&mut event);

    assert!(matches!(
        engine.advance_execution(&resolved, &prepared, &[], &step.record, event),
        Err(HarnessError::InvalidSubmission(message))
            if message
                == "professional-profile improvements require a failed ProfessionalProfileArtifact result"
    ));
}

#[test]
fn professional_profile_improvement_uses_the_existing_failed_requirement() {
    let (workspace, engine) = external_engine();
    let request = HarnessRequest {
        action: HarnessAction::DocumentWrite,
        owner: DataOwner::Personal,
        targets: vec!["README.md".to_owned()],
        objective: "update a professional profile".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = engine
        .resolve_with_career_surface(&request, CareerOutputSurface::ProfessionalProfile)
        .expect("professional profile write must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let record = complete_execution_record_with_improvements(
        &engine,
        &resolved,
        &prepared,
        &[],
        &["make the opening summary more specific"],
        "professional-profile-rejected",
    );
    let rejected = engine
        .evaluate_execution(&resolved, &prepared, &[], &record)
        .expect("professional profile candidate must evaluate");

    assert_eq!(
        rejected
            .requirements
            .iter()
            .filter(|result| !result.passed)
            .map(|result| result.requirement.unit)
            .collect::<Vec<_>>(),
        vec![VerificationUnit::ProfessionalProfileArtifact]
    );
    assert!(rejected.blocking_findings.is_empty());
    assert_eq!(rejected.improvements.len(), 1);
    assert_eq!(rejected.subject_status, SubjectStatus::Rejected);
    assert_eq!(
        rejected.completion_state,
        HarnessCompletionState::RevisionRequired
    );
    assert!(matches!(
        engine.validate_execution(&resolved, &prepared, &[], &record),
        Err(HarnessError::InvalidSubmission(message))
            if message == "execution result does not satisfy every verification requirement"
    ));
    let original = fs::read_to_string(workspace.0.join("README.md"))
        .expect("professional profile fixture must be readable");
    assert!(matches!(
        engine.apply_validated_execution(&resolved, &prepared, &[], &record),
        Err(HarnessError::InvalidSubmission(message))
            if message == "execution result does not satisfy every verification requirement"
    ));
    assert_eq!(
        fs::read_to_string(workspace.0.join("README.md"))
            .expect("rejected professional profile fixture must remain readable"),
        original
    );

    let revision_history = std::slice::from_ref(&rejected);
    let next = engine
        .begin_execution(&resolved, &prepared, revision_history)
        .expect("rejected professional profile must start a revision");
    assert!(matches!(
        next.record.revision_contract.corrections.as_slice(),
        [RevisionCorrection::FailedRequirement {
            unit: VerificationUnit::ProfessionalProfileArtifact,
            ..
        }]
    ));

    let accepted_record = complete_execution_step_with_improvements(
        &engine,
        &resolved,
        &prepared,
        revision_history,
        next,
        (&[], &[]),
        "professional-profile-accepted",
    );
    let accepted = engine
        .evaluate_execution(&resolved, &prepared, revision_history, &accepted_record)
        .expect("resolved professional profile revision must evaluate");
    assert_eq!(accepted.revision, 1);
    assert!(accepted.improvements.is_empty());
    assert_eq!(accepted.subject_status, SubjectStatus::Accepted);
    assert_eq!(
        engine
            .validate_execution(&resolved, &prepared, revision_history, &accepted_record,)
            .expect("professional profile revision without improvements must validate"),
        accepted
    );
}

#[test]
fn professional_profile_multiple_improvements_keep_one_failed_requirement_correction() {
    let (_workspace, engine) = external_engine();
    let request = HarnessRequest {
        action: HarnessAction::DocumentWrite,
        owner: DataOwner::Personal,
        targets: vec!["README.md".to_owned()],
        objective: "update a professional profile".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = engine
        .resolve_with_career_surface(&request, CareerOutputSurface::ProfessionalProfile)
        .expect("professional profile write must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let record = complete_execution_record_with_improvements(
        &engine,
        &resolved,
        &prepared,
        &[],
        &[
            "make the opening summary more specific",
            "align the experience order across platform slots",
        ],
        "professional-profile-multiple-improvements",
    );
    let rejected = engine
        .evaluate_execution(&resolved, &prepared, &[], &record)
        .expect("professional profile improvements must evaluate");
    let next = engine
        .begin_execution(&resolved, &prepared, std::slice::from_ref(&rejected))
        .expect("rejected professional profile must start a revision");

    assert_eq!(
        rejected
            .requirements
            .iter()
            .filter(|result| !result.passed)
            .map(|result| result.requirement.unit)
            .collect::<Vec<_>>(),
        vec![VerificationUnit::ProfessionalProfileArtifact]
    );
    assert_eq!(rejected.improvements.len(), 2);
    assert!(matches!(
        next.record.revision_contract.corrections.as_slice(),
        [RevisionCorrection::FailedRequirement {
            unit: VerificationUnit::ProfessionalProfileArtifact,
            ..
        }]
    ));
}

#[test]
fn reviewer_improvements_remain_non_blocking_outside_professional_profile_artifacts() {
    let (_workspace, engine) = external_engine();
    let resume_request = HarnessRequest {
        action: HarnessAction::DocumentWrite,
        owner: DataOwner::Personal,
        targets: vec!["README.md".to_owned()],
        objective: "update a resume".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resume = engine
        .resolve_with_career_surface(&resume_request, CareerOutputSurface::Resume)
        .expect("resume write must resolve");
    let resume_prepared = prepared_run_with(&engine, &resume);
    let resume_record = complete_execution_record_with_improvements(
        &engine,
        &resume,
        &resume_prepared,
        &[],
        &["consider a shorter opening summary"],
        "resume-improvement",
    );
    let resume_evaluation = engine
        .validate_execution(&resume, &resume_prepared, &[], &resume_record)
        .expect("resume improvements must remain non-blocking");
    assert_eq!(resume_evaluation.subject_status, SubjectStatus::Accepted);
    assert_eq!(resume_evaluation.improvements.len(), 1);

    let adapter_request = HarnessRequest {
        action: HarnessAction::CodeWrite,
        owner: DataOwner::Profile,
        targets: vec!["src/lib.rs".to_owned()],
        objective: "update a professional profile output adapter contract".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let adapter = engine
        .resolve_with_intent(
            &adapter_request,
            HarnessIntent::OutputAdapter {
                surface: CareerOutputSurface::ProfessionalProfile,
            },
        )
        .expect("professional profile output adapter write must resolve");
    let adapter_prepared = prepared_run_with(&engine, &adapter);
    let adapter_record = complete_execution_record_with_improvements(
        &engine,
        &adapter,
        &adapter_prepared,
        &[],
        &["consider simplifying the adapter boundary"],
        "professional-profile-output-adapter-improvement",
    );
    let adapter_evaluation = engine
        .validate_execution(&adapter, &adapter_prepared, &[], &adapter_record)
        .expect("output-adapter improvements must remain non-blocking");
    assert_eq!(adapter_evaluation.subject_status, SubjectStatus::Accepted);
    assert_eq!(adapter_evaluation.improvements.len(), 1);
}

#[test]
fn professional_profile_review_attestation_rejects_unresolved_improvements() {
    let (_workspace, engine) = external_engine();
    let request = career_review_request(vec!["README.md".to_owned()]);
    let source = "vault/personal/projects/coupler.md".to_owned();
    let mut manifest = selected_career_manifest(
        vec!["README.md".to_owned()],
        vec![(
            DataOwner::PersonalProject {
                project: "coupler".to_owned(),
            },
            vec![source],
        )],
    );
    manifest.career_output_surface = CareerOutputSurface::ProfessionalProfile;
    let resolved = engine
        .resolve_with_career_composition(
            &request,
            &[],
            Some(CareerOutputSurface::ProfessionalProfile),
            &[],
            Some(&manifest),
        )
        .expect("professional profile review must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let record = complete_execution_record_with_improvements(
        &engine,
        &resolved,
        &prepared,
        &[],
        &["clarify the leadership scope"],
        "professional-profile-attestation",
    );
    let evaluation = engine
        .evaluate_execution(&resolved, &prepared, &[], &record)
        .expect("professional profile review must evaluate");
    assert_eq!(evaluation.subject_status, SubjectStatus::Rejected);
    assert_eq!(
        evaluation.artifact,
        Some(EvaluatedArtifact::Review {
            summary: "Rejected: see failed requirements and blocking findings.".to_owned(),
        })
    );

    assert_apply_lifecycle_failure_contains(
        engine.attest_career_execution_review(&resolved, &prepared, &record),
        "career-execution-attestation",
        "execution result does not satisfy every verification requirement",
    );
}

#[test]
fn review_roles_receive_independent_frozen_target_invocations() {
    let mut request = profile_code_request();
    request.action = HarnessAction::CodeReview;
    let engine = engine();
    let resolved = engine.resolve(&request).expect("review must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let step = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("review execution must begin");

    assert_eq!(resolved.plan.primary_producer_role, HarnessRole::Reviewer);
    assert_eq!(step.ready_role_invocations.len(), 2);
    assert!(
        step.ready_role_invocations
            .iter()
            .all(|invocation| invocation.predecessor_results.is_empty())
    );
    let subjects = step
        .ready_role_invocations
        .iter()
        .map(|invocation| invocation.subject.clone())
        .collect::<Vec<_>>();
    assert_eq!(subjects[0], subjects[1]);
    assert!(matches!(
        subjects[0],
        EvaluationSubject::FrozenTargets { .. }
    ));
}

#[test]
fn frozen_target_evidence_coverage_is_an_exact_normalized_set() {
    let (workspace, _) = external_engine();
    let router = HarnessRouter::with_execution_profile(2, HarnessExecutionProfile::Strict)
        .expect("strict router must be valid");
    let engine = HarnessEngine::with_router(repository_root(), &workspace.0, router)
        .expect("strict external engine must open");
    let mut request = personal_project_code_request();
    request.action = HarnessAction::CodeReview;
    request.targets = vec!["src/lib.rs".to_owned(), "src/security.rs".to_owned()];
    let resolved = engine.resolve(&request).expect("review must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let begin = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("review execution must begin");
    let verifier = begin
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Verifier)
        .expect("Verifier must be ready")
        .clone();
    let complete = subject_evidence_coverage(&resolved.plan, &verifier.subject);

    validate_frozen_target_evidence_coverage(&resolved.plan, &verifier.subject, &complete)
        .expect("complete target coverage must pass");
    let mut duplicate = complete.clone();
    duplicate.push(complete[0].clone());
    validate_frozen_target_evidence_coverage(&resolved.plan, &verifier.subject, &duplicate)
        .expect("duplicate target references are normalized");
    let mut with_policy_reference = duplicate.clone();
    with_policy_reference.push(ResultEvidenceReference::BoundDocument {
        relative_path: "vault/profile/rules/common-code-quality.md".to_owned(),
        content_digest: "a".repeat(64),
        locator: "policy context".to_owned(),
    });
    validate_frozen_target_evidence_coverage(
        &resolved.plan,
        &verifier.subject,
        &with_policy_reference,
    )
    .expect("non-target policy evidence does not change target coverage");
    assert!(
        validate_frozen_target_evidence_coverage(&resolved.plan, &verifier.subject, &complete[..1])
            .is_err()
    );
    let mut wrong_digest = complete.clone();
    let ResultEvidenceReference::Target { content_digest, .. } = &mut wrong_digest[0] else {
        panic!("coverage fixture must use target evidence");
    };
    *content_digest = "b".repeat(64);
    assert!(
        validate_frozen_target_evidence_coverage(&resolved.plan, &verifier.subject, &wrong_digest)
            .is_err()
    );
    let mut extra_target = complete.clone();
    extra_target.push(ResultEvidenceReference::Target {
        workspace_relative_path: "src/extra.rs".to_owned(),
        content_digest: "c".repeat(64),
        locator: "extra target".to_owned(),
    });
    assert!(
        validate_frozen_target_evidence_coverage(&resolved.plan, &verifier.subject, &extra_target)
            .is_err()
    );
}

#[test]
fn zero_requirement_document_verifier_still_requires_complete_target_coverage() {
    let (workspace, _) = external_engine();
    fs::write(workspace.0.join("CHANGELOG.md"), "# Changes\n")
        .expect("second document target must be written");
    let router = HarnessRouter::with_execution_profile(2, HarnessExecutionProfile::Strict)
        .expect("strict router must be valid");
    let engine = HarnessEngine::with_router(repository_root(), &workspace.0, router)
        .expect("strict external engine must open");
    let mut request = personal_project_code_request();
    request.action = HarnessAction::DocumentReview;
    request.targets = vec!["README.md".to_owned(), "CHANGELOG.md".to_owned()];
    let resolved = engine
        .resolve(&request)
        .expect("document review must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let verifier_metadata = prepared
        .role_metadata
        .iter()
        .find(|metadata| metadata.role == HarnessRole::Verifier)
        .expect("strict document review must prepare a Verifier");
    let RoleTaskContract::Verifier {
        verification_requirements,
        ..
    } = &verifier_metadata.task
    else {
        panic!("Verifier input must use the Verifier contract");
    };
    assert!(verification_requirements.is_empty());

    let begin = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("document review must begin");
    let verifier = begin
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Verifier)
        .expect("Verifier must be ready")
        .clone();
    let mut incomplete = completed_role_event(
        &resolved,
        &prepared,
        &verifier,
        Vec::new(),
        "zero-requirements",
    );
    let HarnessExecutionEvent::RoleResult { result } = &mut incomplete else {
        panic!("Verifier event must contain a role result");
    };
    let RoleExecutionOutcome::Completed {
        result: CompletedRoleResult::Verifier {
            subject_evidence, ..
        },
    } = &mut result.outcome
    else {
        panic!("Verifier result must be completed");
    };
    subject_evidence.truncate(1);
    refresh_role_result_digest(result);
    assert!(matches!(
        engine.advance_execution(&resolved, &prepared, &[], &begin.record, incomplete),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("every planned target")
    ));

    engine
        .advance_execution(
            &resolved,
            &prepared,
            &[],
            &begin.record,
            completed_role_event(
                &resolved,
                &prepared,
                &verifier,
                Vec::new(),
                "zero-requirements-complete",
            ),
        )
        .expect("the same zero-requirement Verifier must pass with complete target coverage");
}

#[test]
fn review_roles_cannot_compensate_for_incomplete_peer_target_coverage() {
    let (workspace, _) = external_engine();
    let router = HarnessRouter::with_execution_profile(2, HarnessExecutionProfile::Strict)
        .expect("strict router must be valid");
    let engine = HarnessEngine::with_router(repository_root(), &workspace.0, router)
        .expect("strict external engine must open");
    let mut request = personal_project_code_request();
    request.action = HarnessAction::CodeReview;
    request.targets = vec!["src/lib.rs".to_owned(), "src/security.rs".to_owned()];
    let resolved = engine.resolve(&request).expect("review must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let begin = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("review execution must begin");
    let verifier = begin
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Verifier)
        .expect("Verifier must be ready")
        .clone();
    let policy = prepared
        .role_bundles
        .iter()
        .find(|bundle| bundle.role == HarnessRole::Verifier)
        .and_then(|bundle| {
            bundle
                .bound_documents()
                .find(|document| document.source == HarnessBoundDocumentSource::Policy)
        })
        .expect("Verifier bundle must contain a policy document");
    let mut verifier_event = completed_role_event(
        &resolved,
        &prepared,
        &verifier,
        Vec::new(),
        "coverage-verifier",
    );
    let HarnessExecutionEvent::RoleResult { result } = &mut verifier_event else {
        panic!("Verifier event must contain a role result");
    };
    let RoleExecutionOutcome::Completed {
        result: CompletedRoleResult::Verifier {
            subject_evidence, ..
        },
    } = &mut result.outcome
    else {
        panic!("Verifier result must be completed");
    };
    subject_evidence.push(subject_evidence[0].clone());
    subject_evidence.push(ResultEvidenceReference::BoundDocument {
        relative_path: policy.relative_path.clone(),
        content_digest: policy.content_digest.clone(),
        locator: "quality policy".to_owned(),
    });
    refresh_role_result_digest(result);
    let after_verifier = engine
        .advance_execution(&resolved, &prepared, &[], &begin.record, verifier_event)
        .expect("complete Verifier coverage must advance");
    let reviewer = after_verifier
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Reviewer)
        .expect("Reviewer must remain independently ready")
        .clone();
    let mut incomplete_reviewer = completed_role_event(
        &resolved,
        &prepared,
        &reviewer,
        Vec::new(),
        "coverage-reviewer",
    );
    let HarnessExecutionEvent::RoleResult { result } = &mut incomplete_reviewer else {
        panic!("Reviewer event must contain a role result");
    };
    let RoleExecutionOutcome::Completed {
        result: CompletedRoleResult::Reviewer {
            subject_evidence, ..
        },
    } = &mut result.outcome
    else {
        panic!("Reviewer result must be completed");
    };
    subject_evidence.truncate(1);
    refresh_role_result_digest(result);
    assert!(matches!(
        engine.advance_execution(
            &resolved,
            &prepared,
            &[],
            &after_verifier.record,
            incomplete_reviewer,
        ),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("every planned target")
    ));
}

#[test]
fn accepted_review_summary_is_derived_from_structured_results() {
    let mut request = profile_code_request();
    request.action = HarnessAction::CodeReview;
    let engine = engine();
    let resolved = engine.resolve(&request).expect("review must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let record = complete_execution_record(&engine, &resolved, &prepared, &[], "review-accepted");
    let evaluation = engine
        .evaluate_execution(&resolved, &prepared, &[], &record)
        .expect("completed review must evaluate");

    assert_eq!(evaluation.execution_status, ExecutionStatus::Completed);
    assert_eq!(evaluation.subject_status, SubjectStatus::Accepted);
    assert_eq!(
        evaluation.completion_state,
        HarnessCompletionState::ReviewComplete
    );
    assert_eq!(
        evaluation.artifact,
        Some(EvaluatedArtifact::Review {
            summary: "No Findings".to_owned(),
        })
    );
    assert!(
        evaluation
            .execution_record
            .role_results
            .iter()
            .any(|result| matches!(
                &result.outcome,
                RoleExecutionOutcome::Completed {
                    result: CompletedRoleResult::Reviewer { summary, .. },
                } if summary == "reviewed"
            ))
    );
    let evaluated_requirements = evaluation
        .requirements
        .iter()
        .map(|requirement| requirement.requirement)
        .collect::<Vec<_>>();
    assert_eq!(
        evaluated_requirements, resolved.plan.verification_requirements,
        "evaluation must preserve the exact planned unit, owner, subject, and order"
    );
    assert_eq!(
        evaluated_requirements
            .iter()
            .map(|requirement| requirement.unit)
            .collect::<BTreeSet<_>>()
            .len(),
        evaluated_requirements.len(),
        "evaluation must contain exactly one result for each planned unit"
    );
    let validated = engine
        .validate_execution(&resolved, &prepared, &[], &record)
        .expect("accepted review must validate");
    assert_eq!(
        validated, evaluation,
        "trust-boundary validation must not add a requirement, result, vote, or assurance"
    );
}

#[test]
fn failed_review_requirement_rejects_subject_without_blocking_findings() {
    let mut request = profile_code_request();
    request.action = HarnessAction::CodeReview;
    let engine = engine();
    let resolved = engine.resolve(&request).expect("review must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let mut step = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("review execution must begin");
    let verifier = step
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Verifier)
        .cloned()
        .expect("Verifier must be ready");
    step = engine
        .advance_execution(
            &resolved,
            &prepared,
            &[],
            &step.record,
            completed_role_event(
                &resolved,
                &prepared,
                &verifier,
                Vec::new(),
                "review-requirement-failed",
            ),
        )
        .expect("Verifier must complete");
    let reviewer = step
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Reviewer)
        .cloned()
        .expect("Reviewer must be ready");
    let mut reviewer_event = completed_role_event(
        &resolved,
        &prepared,
        &reviewer,
        Vec::new(),
        "review-requirement-failed",
    );
    let HarnessExecutionEvent::RoleResult { result } = &mut reviewer_event else {
        panic!("Reviewer event must contain a role result");
    };
    let RoleExecutionOutcome::Completed {
        result:
            CompletedRoleResult::Reviewer {
                requirement_results,
                blocking_findings,
                ..
            },
    } = &mut result.outcome
    else {
        panic!("Reviewer result must be completed");
    };
    assert!(blocking_findings.is_empty());
    requirement_results[0].passed = false;
    requirement_results[0].detail = "review completion requirement failed".to_owned();
    refresh_role_event_digest(&mut reviewer_event);
    step = engine
        .advance_execution(&resolved, &prepared, &[], &step.record, reviewer_event)
        .expect("failed Reviewer requirement must advance");
    let tool = step
        .ready_tool_invocation
        .clone()
        .expect("review tool evidence must be ready");
    step = engine
        .advance_execution(
            &resolved,
            &prepared,
            &[],
            &step.record,
            passed_tool_event(&tool),
        )
        .expect("tool evidence must complete review execution");
    let evaluation = engine
        .evaluate_execution(&resolved, &prepared, &[], &step.record)
        .expect("completed review must evaluate");

    assert!(evaluation.blocking_findings.is_empty());
    assert_eq!(evaluation.subject_status, SubjectStatus::Rejected);
    assert_eq!(
        evaluation.artifact,
        Some(EvaluatedArtifact::Review {
            summary: "Rejected: see failed requirements and blocking findings.".to_owned(),
        })
    );
    assert_eq!(
        evaluation.completion_state,
        HarnessCompletionState::ReviewComplete
    );
    assert!(matches!(
        engine.validate_execution(&resolved, &prepared, &[], &step.record),
        Err(HarnessError::InvalidSubmission(_))
    ));
}

#[test]
fn completed_review_and_rejected_subject_are_reported_separately() {
    let mut request = profile_code_request();
    request.action = HarnessAction::CodeReview;
    let engine = engine();
    let resolved = engine.resolve(&request).expect("review must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let target = resolved
        .plan
        .targets
        .first()
        .expect("review target must exist");
    let TargetState::Existing { content_digest } = &target.state else {
        panic!("review target must be frozen");
    };
    let finding = BlockingFinding {
        message: "the frozen target has a blocking defect".to_owned(),
        evidence: vec![ResultEvidenceReference::Target {
            workspace_relative_path: target.workspace_relative_path.clone(),
            content_digest: content_digest.clone(),
            locator: "blocking target section".to_owned(),
        }],
    };
    let record =
        complete_execution_record(&engine, &resolved, &prepared, &[finding], "review-rejected");
    let evaluation = engine
        .evaluate_execution(&resolved, &prepared, &[], &record)
        .expect("completed review must evaluate");

    assert_eq!(evaluation.execution_status, ExecutionStatus::Completed);
    assert_eq!(evaluation.subject_status, SubjectStatus::Rejected);
    assert_eq!(
        evaluation.artifact,
        Some(EvaluatedArtifact::Review {
            summary: "Rejected: see failed requirements and blocking findings.".to_owned(),
        })
    );
    assert_eq!(
        evaluation.completion_state,
        HarnessCompletionState::ReviewComplete
    );
    assert!(matches!(
        engine.validate_execution(&resolved, &prepared, &[], &record),
        Err(HarnessError::InvalidSubmission(_))
    ));
}

#[test]
fn learned_facts_remain_promotion_proposals_until_a_separate_curation_request() {
    let mut request = profile_code_request();
    request.action = HarnessAction::Investigation;
    request.objective = "inspect the implementation and report a fact".to_owned();
    let engine = engine();
    let resolved = engine
        .resolve(&request)
        .expect("investigation must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let mut step = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("investigation must begin");
    let specialist_invocation = step.ready_role_invocations[0].clone();
    let specialist_bundle = prepared
        .role_bundles
        .iter()
        .find(|bundle| bundle.role == HarnessRole::Specialist)
        .expect("Specialist bundle must exist");
    let source = specialist_bundle
        .bound_documents()
        .find(|document| document.source == HarnessBoundDocumentSource::Target)
        .expect("investigation target must be in the Specialist bundle");
    let mut specialist_event = completed_role_event(
        &resolved,
        &prepared,
        &specialist_invocation,
        Vec::new(),
        "promotion",
    );
    let HarnessExecutionEvent::RoleResult { result } = &mut specialist_event else {
        panic!("Specialist event must contain a role result");
    };
    let RoleExecutionOutcome::Completed {
        result:
            CompletedRoleResult::Specialist {
                promotion_proposals,
                ..
            },
    } = &mut result.outcome
    else {
        panic!("Specialist result must be completed");
    };
    promotion_proposals.push(PromotionProposal {
        identifier: "proposal-0000000000000001".to_owned(),
        owner: DataOwner::Profile,
        curation_kind: CurationKind::Fact,
        title: "Observed implementation fact".to_owned(),
        content: "The inspected target contains the observed implementation.".to_owned(),
        origin: PromotionProposalOrigin::SpecialistMemory {
            claim_kind: PromotionClaimKind::ObservedFact,
            evidence: vec![PromotionEvidenceReference {
                source_kind: PromotionSourceKind::Target,
                source_owner: DataOwner::Profile,
                relative_path: source.relative_path.clone(),
                content_digest: source.content_digest.clone(),
                locator: "entire bound target".to_owned(),
            }],
        },
    });
    result.result_digest = serialized_digest(&(
        result.role,
        &result.invocation_digest,
        &result.context_id,
        &result.lifecycle,
        &result.outcome,
    ))
    .expect("promotion-bearing Specialist result must serialize");
    step = engine
        .advance_execution(&resolved, &prepared, &[], &step.record, specialist_event)
        .expect("promotion proposal must remain a valid transient result");
    let reviewer_invocation = step.ready_role_invocations[0].clone();
    step = engine
        .advance_execution(
            &resolved,
            &prepared,
            &[],
            &step.record,
            completed_role_event(
                &resolved,
                &prepared,
                &reviewer_invocation,
                Vec::new(),
                "promotion",
            ),
        )
        .expect("Reviewer must complete the analysis");
    let evaluation = engine
        .evaluate_execution(&resolved, &prepared, &[], &step.record)
        .expect("analysis must evaluate");

    assert_eq!(
        evaluation.completion_state,
        HarnessCompletionState::AnalysisComplete
    );
    assert_eq!(evaluation.subject_status, SubjectStatus::Accepted);
    assert_single_observed_fact_promotion(&evaluation);
    assert!(!resolved.plan.source_write_allowed);
}

fn assert_single_observed_fact_promotion(evaluation: &HarnessTaskEvaluation) {
    assert!(matches!(
        evaluation.promotion_state,
        PromotionState::RequiresSeparateCuration { ref proposals }
            if proposals.len() == 1
                && matches!(
                    proposals[0].origin,
                    PromotionProposalOrigin::SpecialistMemory {
                        claim_kind: PromotionClaimKind::ObservedFact,
                        ..
                    }
                )
    ));
}

#[test]
fn promotion_handoff_factory_accepts_only_an_exact_accepted_proposal() {
    let repository = TemporaryWorkspace::create();
    copy_directory(
        &repository_root().join("vault"),
        &repository.0.join("vault"),
    );
    let engine = HarnessEngine::open(&repository.0, &repository.0).unwrap();
    let (resolved, prepared, evaluation) = promotion_source_evaluation(&engine, false);
    let handoff = PromotionHandoff::from_evaluation(
        &resolved.plan,
        &prepared,
        &[],
        &evaluation,
        "proposal-0000000000000003",
    )
    .expect("an exact accepted proposal must create a same-turn handoff");
    assert_eq!(
        handoff.source_resolved_plan_digest,
        resolved.resolved_plan_digest
    );
    assert_eq!(
        handoff.source_task_evaluation_receipt_digest,
        evaluation.task_evaluation_receipt_digest
    );
    assert_eq!(handoff.proposal.identifier, "proposal-0000000000000003");
    assert!(matches!(
        PromotionHandoff::from_evaluation(
            &resolved.plan,
            &prepared,
            &[],
            &evaluation,
            "proposal-ffffffffffffffff"
        ),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("is not part of the current task evaluation")
    ));

    let (rejected_resolved, rejected_prepared, rejected_evaluation) =
        promotion_source_evaluation(&engine, true);
    assert_eq!(rejected_evaluation.subject_status, SubjectStatus::Rejected);
    let rejected_handoff = PromotionHandoff::from_evaluation(
        &rejected_resolved.plan,
        &rejected_prepared,
        &[],
        &rejected_evaluation,
        "proposal-0000000000000003",
    )
    .unwrap_err();
    assert!(
        matches!(
        &rejected_handoff,
        HarnessError::InvalidSubmission(message)
            if message.contains("task evaluation has no promotion proposals")
        ),
        "{rejected_handoff:?}"
    );
}

#[test]
fn failed_reviewer_requirement_derives_an_exact_learning_promotion() {
    let repository = TemporaryWorkspace::create();
    copy_directory(
        &repository_root().join("vault"),
        &repository.0.join("vault"),
    );
    let engine = HarnessEngine::open(&repository.0, &repository.0).unwrap();
    let (resolved, prepared, evaluation) = reviewer_learning_source_evaluation(&engine);
    assert_eq!(evaluation.subject_status, SubjectStatus::Rejected);
    let PromotionState::RequiresSeparateCuration { proposals } = &evaluation.promotion_state else {
        panic!("failed Reviewer learning must produce a separate curation proposal");
    };
    let [proposal] = proposals.as_slice() else {
        panic!("one failed unit must produce one learning proposal");
    };
    let PromotionProposalOrigin::ReviewerLearning {
        learning_id,
        source_action,
        source_intent,
        failed_unit,
        requirement_result_digest,
        evidence,
    } = &proposal.origin
    else {
        panic!("Reviewer candidate must derive a Reviewer learning proposal");
    };
    assert!(proposal.identifier.starts_with("proposal-"));
    assert!(learning_id.starts_with("learning-"));
    assert_eq!(*source_action, HarnessAction::Investigation);
    assert_eq!(*source_intent, HarnessIntent::General);
    assert_eq!(*failed_unit, VerificationUnit::CompletionContract);
    assert_eq!(requirement_result_digest.len(), 64);
    assert_eq!(
        evidence,
        &evaluation
            .requirements
            .iter()
            .find(|result| result.requirement.unit == *failed_unit)
            .expect("failed evaluated requirement must exist")
            .evidence
    );
    let handoff = PromotionHandoff::from_evaluation(
        &resolved.plan,
        &prepared,
        &[],
        &evaluation,
        &proposal.identifier,
    )
    .expect("exact Reviewer learning may hand off from a rejected task");
    assert_eq!(handoff.proposal, proposal.clone());

    let mut tampered = evaluation.clone();
    let PromotionState::RequiresSeparateCuration { proposals } = &mut tampered.promotion_state
    else {
        unreachable!("fixture must contain a promotion");
    };
    proposals[0].content.push_str(" tampered");
    assert!(matches!(
        PromotionHandoff::from_evaluation(
            &resolved.plan,
            &prepared,
            &[],
            &tampered,
            &proposal.identifier
        ),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("task evaluation receipt digest is invalid")
    ));
}

#[test]
fn reviewer_learning_rejects_a_passed_or_tampered_requirement_result() {
    let engine = engine();
    let request = HarnessRequest {
        action: HarnessAction::Investigation,
        owner: DataOwner::Personal,
        targets: vec!["vault/personal/index.md".to_owned()],
        objective: "통과한 결과를 학습 실패로 위조할 수 없어야 한다".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = engine.resolve(&request).unwrap();
    let prepared = prepared_run_with(&engine, &resolved);
    let mut step = engine.begin_execution(&resolved, &prepared, &[]).unwrap();
    let specialist = step.ready_role_invocations[0].clone();
    step = engine
        .advance_execution(
            &resolved,
            &prepared,
            &[],
            &step.record,
            completed_role_event(
                &resolved,
                &prepared,
                &specialist,
                Vec::new(),
                "invalid-reviewer-learning",
            ),
        )
        .unwrap();
    let reviewer = step.ready_role_invocations[0].clone();
    let mut event = completed_role_event(
        &resolved,
        &prepared,
        &reviewer,
        Vec::new(),
        "invalid-reviewer-learning",
    );
    let HarnessExecutionEvent::RoleResult { result } = &mut event else {
        unreachable!("Reviewer event must contain a role result");
    };
    let RoleExecutionOutcome::Completed {
        result:
            CompletedRoleResult::Reviewer {
                requirement_results,
                learning_candidates,
                ..
            },
    } = &mut result.outcome
    else {
        unreachable!("Reviewer result must be completed");
    };
    let passed = requirement_results
        .iter()
        .find(|result| result.unit == VerificationUnit::CompletionContract)
        .expect("Reviewer must own the completion requirement");
    learning_candidates.push(ReviewerLearningCandidate {
        title: "위조된 학습".to_owned(),
        guidance: "통과한 결과를 실패라고 기록한다".to_owned(),
        failed_unit: passed.unit,
        requirement_result_digest: serialized_digest(passed).unwrap(),
        evidence: passed.evidence.clone(),
    });
    refresh_role_event_digest(&mut event);
    assert!(matches!(
        engine.advance_execution(&resolved, &prepared, &[], &step.record, event),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("require a failed requirement result")
    ));
}

#[test]
fn reviewer_learning_requires_a_new_canonical_knowledge_document() {
    let repository = TemporaryWorkspace::create();
    copy_directory(
        &repository_root().join("vault"),
        &repository.0.join("vault"),
    );
    let engine = HarnessEngine::open(&repository.0, &repository.0).unwrap();
    let (source_resolved, source_prepared, source_evaluation) =
        reviewer_learning_source_evaluation(&engine);
    let PromotionState::RequiresSeparateCuration { proposals } = &source_evaluation.promotion_state
    else {
        panic!("learning source must contain a promotion");
    };
    let handoff = PromotionHandoff::from_evaluation(
        &source_resolved.plan,
        &source_prepared,
        &[],
        &source_evaluation,
        &proposals[0].identifier,
    )
    .unwrap();
    let target = "vault/personal/knowledge/reviewer-learning.md";
    let envelope = promotion_curation_envelope(
        handoff.clone(),
        DataOwner::Personal,
        CurationKind::Knowledge,
        target,
        true,
    );
    let resolved = engine
        .resolve_envelope(&envelope, None)
        .expect("confirmed Reviewer learning must resolve as Knowledge Create");
    assert!(matches!(
        resolved.plan.targets.as_slice(),
        [TargetBinding {
            operation: TargetOperation::Create,
            ..
        }]
    ));
    let prepared = prepared_run_with(&engine, &resolved);
    let step = engine.begin_execution(&resolved, &prepared, &[]).unwrap();
    let writer = step.ready_role_invocations[0].clone();
    let mut invalid = completed_role_event(
        &resolved,
        &prepared,
        &writer,
        Vec::new(),
        "learning-markdown",
    );
    *curation_create_content_mut(&mut invalid) = "# not canonical".to_owned();
    refresh_role_event_digest(&mut invalid);
    assert!(matches!(
        engine.advance_execution(&resolved, &prepared, &[], &step.record, invalid),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("canonical learning Markdown")
    ));

    let mut valid = completed_role_event(
        &resolved,
        &prepared,
        &writer,
        Vec::new(),
        "learning-markdown",
    );
    let content = curation_create_content_mut(&mut valid);
    *content = canonical_learning_markdown(&handoff).unwrap();
    assert!(content.contains("learning_id: learning-"));
    assert!(content.contains("source_task_evaluation_receipt_digest:"));
    assert!(content.contains("promotion_handoff_digest:"));
    refresh_role_event_digest(&mut valid);
    engine
        .advance_execution(&resolved, &prepared, &[], &step.record, valid)
        .expect("exact canonical learning Markdown must advance");

    fs::write(repository.0.join(target), "existing learning target\n")
        .expect("existing target fixture must be written");
    assert!(matches!(
        engine.resolve_envelope(&envelope, None),
        Err(HarnessError::InvalidPlan(message))
            if message.contains("requires a new Knowledge target")
    ));
}

fn assert_learning_bound_only_to_primary(
    resolved: &ResolvedHarnessRequest,
    prepared: &PreparedRoleRun,
) {
    for (metadata, bundle) in prepared.role_metadata.iter().zip(&prepared.role_bundles) {
        let learning_documents = bundle
            .bound_documents()
            .filter(|document| document.source == HarnessBoundDocumentSource::Learning)
            .collect::<Vec<_>>();
        if metadata.role == resolved.plan.primary_producer_role {
            assert_eq!(
                metadata.scope.learning_sources,
                resolved.plan.learning_sources
            );
            assert_eq!(learning_documents.len(), 1);
            let HarnessRoleSegment::ControlHead { content, .. } = &bundle.segments[0] else {
                unreachable!("first segment must be the control head");
            };
            assert!(content.contains("Approved learning is advisory evidence"));
        } else {
            assert!(metadata.scope.learning_sources.is_empty());
            assert!(learning_documents.is_empty());
        }
    }
}

fn fail_reviewer_completion_requirement(record: &mut RoleExecutionRecord) {
    let reviewer = record
        .role_results
        .iter_mut()
        .find(|result| result.role == HarnessRole::Reviewer)
        .expect("follow-up must contain Reviewer evidence");
    let RoleExecutionOutcome::Completed {
        result:
            CompletedRoleResult::Reviewer {
                requirement_results,
                ..
            },
    } = &mut reviewer.outcome
    else {
        unreachable!("Reviewer result must be completed");
    };
    let completion = requirement_results
        .iter_mut()
        .find(|result| result.unit == VerificationUnit::CompletionContract)
        .expect("Reviewer must report the learned unit");
    completion.passed = false;
    completion.detail = "후속 실행에서도 완료 조건을 놓쳤다".to_owned();
    refresh_role_result_digest(reviewer);
    refresh_role_execution_record_digest(record);
}

fn assert_passed_learning_observation(
    evaluation: &HarnessTaskEvaluation,
    resolved: &ResolvedHarnessRequest,
    target: &str,
) {
    assert_eq!(
        evaluation.learning_observations,
        vec![LearningObservation {
            learning_id: resolved.plan.learning_sources[0]
                .metadata
                .learning_id
                .clone(),
            repository_relative_path: target.to_owned(),
            content_digest: resolved.plan.learning_sources[0].content_digest.clone(),
            verification_unit: VerificationUnit::CompletionContract,
            outcome: LearningObservationOutcome::Passed,
        }]
    );
}

#[test]
fn approved_learning_binds_only_to_the_primary_producer_as_advisory_context() {
    let repository = TemporaryWorkspace::create();
    copy_directory(
        &repository_root().join("vault"),
        &repository.0.join("vault"),
    );
    let engine = HarnessEngine::open(&repository.0, &repository.0).unwrap();
    let (source_resolved, source_prepared, source_evaluation) =
        reviewer_learning_source_evaluation(&engine);
    let PromotionState::RequiresSeparateCuration { proposals } = &source_evaluation.promotion_state
    else {
        panic!("learning source must contain a promotion");
    };
    let handoff = PromotionHandoff::from_evaluation(
        &source_resolved.plan,
        &source_prepared,
        &[],
        &source_evaluation,
        &proposals[0].identifier,
    )
    .unwrap();
    let target = "vault/personal/knowledge/reviewer-learning.md";
    fs::write(
        repository.0.join(target),
        canonical_learning_markdown(&handoff).unwrap(),
    )
    .unwrap();

    let resolved = engine.resolve(&source_resolved.request).unwrap();
    assert_eq!(resolved.plan.learning_sources.len(), 1);
    assert_eq!(
        resolved.plan.learning_sources[0].repository_relative_path,
        target
    );
    let prepared = prepared_run_with(&engine, &resolved);
    let begin = engine.begin_execution(&resolved, &prepared, &[]).unwrap();
    let producer = begin.ready_role_invocations[0].clone();
    let frontier = engine
        .advance_execution(
            &resolved,
            &prepared,
            &[],
            &begin.record,
            completed_role_event(
                &resolved,
                &prepared,
                &producer,
                Vec::new(),
                "approved-learning-producer",
            ),
        )
        .unwrap();
    let reviewer = frontier
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Reviewer)
        .expect("Reviewer must be ready");
    let already_bound = missing_context_role_event(
        &resolved,
        reviewer,
        MissingContextCandidate::AdditionalWorkspaceTarget {
            workspace_relative_path: target.to_owned(),
        },
        "approved-learning-already-bound",
    );
    assert!(matches!(
        engine.advance_execution(
            &resolved,
            &prepared,
            &[],
            &frontier.record,
            already_bound
        ),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("already bound")
    ));
    assert_learning_bound_only_to_primary(&resolved, &prepared);
    let record = complete_execution_record(
        &engine,
        &resolved,
        &prepared,
        &[],
        "approved-learning-follow-up",
    );
    let evaluation = engine
        .evaluate_execution(&resolved, &prepared, &[], &record)
        .unwrap();
    assert_passed_learning_observation(&evaluation, &resolved, target);
    let mut failed_record = record;
    fail_reviewer_completion_requirement(&mut failed_record);
    let failed = engine
        .evaluate_execution(&resolved, &prepared, &[], &failed_record)
        .unwrap();
    assert_eq!(
        failed.learning_observations[0].outcome,
        LearningObservationOutcome::Failed
    );
}

#[test]
fn learning_discovery_ignores_ordinary_knowledge_and_rejects_malformed_or_duplicate_learning() {
    let repository = TemporaryWorkspace::create();
    copy_directory(
        &repository_root().join("vault"),
        &repository.0.join("vault"),
    );
    let engine = HarnessEngine::open(&repository.0, &repository.0).unwrap();
    let request = HarnessRequest {
        action: HarnessAction::Investigation,
        owner: DataOwner::Personal,
        targets: vec!["vault/personal/index.md".to_owned()],
        objective: "학습 문서 탐색 경계를 검증한다".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let knowledge = repository.0.join("vault/personal/knowledge");
    fs::write(
        knowledge.join("ordinary.md"),
        "---\ntitle: Ordinary\nscope: personal\nexport: false\n---\n# Ordinary\n",
    )
    .unwrap();
    assert!(
        engine
            .resolve(&request)
            .unwrap()
            .plan
            .learning_sources
            .is_empty()
    );

    fs::write(
        knowledge.join("unterminated-inline.md"),
        "---\ntitle: Broken Inline\nlearning: { learning_id: learning-0000000000000000 }\n",
    )
    .unwrap();
    assert!(matches!(
        engine.resolve(&request),
        Err(HarnessError::InvalidRepository(message))
            if message.contains("unterminated-inline.md")
                && message.contains("unterminated frontmatter")
    ));
    fs::remove_file(knowledge.join("unterminated-inline.md")).unwrap();

    fs::write(
        knowledge.join("malformed.md"),
        "---\ntitle: Broken\nscope: personal\nexport: false\nlearning:\n  learning_id: nope\n---\n# Broken\n",
    )
    .unwrap();
    assert!(matches!(
        engine.resolve(&request),
        Err(HarnessError::InvalidRepository(message))
            if message.contains("malformed.md") && message.contains("invalid")
    ));
    fs::remove_file(knowledge.join("malformed.md")).unwrap();

    let (source_resolved, source_prepared, source_evaluation) =
        reviewer_learning_source_evaluation(&engine);
    let PromotionState::RequiresSeparateCuration { proposals } = &source_evaluation.promotion_state
    else {
        unreachable!("fixture must contain a proposal");
    };
    let handoff = PromotionHandoff::from_evaluation(
        &source_resolved.plan,
        &source_prepared,
        &[],
        &source_evaluation,
        &proposals[0].identifier,
    )
    .unwrap();
    let canonical = canonical_learning_markdown(&handoff).unwrap();
    fs::write(knowledge.join("duplicate-a.md"), &canonical).unwrap();
    fs::write(knowledge.join("duplicate-b.md"), canonical).unwrap();
    assert!(matches!(
        engine.resolve(&request),
        Err(HarnessError::InvalidRepository(message))
            if message.contains("duplicated by")
    ));
}

#[test]
#[cfg(unix)]
fn learning_discovery_rejects_hard_links_and_prepare_rejects_source_drift() {
    let repository = TemporaryWorkspace::create();
    copy_directory(
        &repository_root().join("vault"),
        &repository.0.join("vault"),
    );
    let engine = HarnessEngine::open(&repository.0, &repository.0).unwrap();
    let (source_resolved, source_prepared, source_evaluation) =
        reviewer_learning_source_evaluation(&engine);
    let PromotionState::RequiresSeparateCuration { proposals } = &source_evaluation.promotion_state
    else {
        unreachable!("fixture must contain a learning proposal");
    };
    let handoff = PromotionHandoff::from_evaluation(
        &source_resolved.plan,
        &source_prepared,
        &[],
        &source_evaluation,
        &proposals[0].identifier,
    )
    .unwrap();
    let knowledge = repository.0.join("vault/personal/knowledge");
    let source = knowledge.join("secure-learning.md");
    let canonical = canonical_learning_markdown(&handoff).unwrap();
    fs::write(&source, &canonical).unwrap();
    let resolved = engine.resolve(&source_resolved.request).unwrap();
    fs::write(&source, format!("{canonical}\n후속 변조\n")).unwrap();
    assert!(matches!(
        engine.prepare(&resolved, &complete_runtime_capabilities(), None),
        Err(HarnessError::PlanDrift { .. })
    ));

    fs::write(&source, canonical).unwrap();
    fs::hard_link(&source, knowledge.join("secure-learning-alias.md")).unwrap();
    let hard_link_error = engine.resolve(&source_resolved.request).unwrap_err();
    assert!(
        matches!(
            &hard_link_error,
        HarnessError::InvalidRequest(message)
                if message.contains("exactly one hard link")
        ),
        "{hard_link_error:?}"
    );
}

#[test]
fn concurrent_learning_promotions_recheck_the_identifier_before_source_mutation() {
    let repository = TemporaryWorkspace::create();
    copy_directory(
        &repository_root().join("vault"),
        &repository.0.join("vault"),
    );
    let engine = HarnessEngine::open(&repository.0, &repository.0).unwrap();
    let (source_resolved, source_prepared, source_evaluation) =
        reviewer_learning_source_evaluation(&engine);
    let PromotionState::RequiresSeparateCuration { proposals } = &source_evaluation.promotion_state
    else {
        unreachable!("fixture must contain a learning proposal");
    };
    let handoff = PromotionHandoff::from_evaluation(
        &source_resolved.plan,
        &source_prepared,
        &[],
        &source_evaluation,
        &proposals[0].identifier,
    )
    .unwrap();
    let first_target = "vault/personal/knowledge/concurrent-learning-a.md";
    let second_target = "vault/personal/knowledge/concurrent-learning-b.md";
    let first_resolved = engine
        .resolve_envelope(
            &promotion_curation_envelope(
                handoff.clone(),
                DataOwner::Personal,
                CurationKind::Knowledge,
                first_target,
                true,
            ),
            None,
        )
        .unwrap();
    let second_resolved = engine
        .resolve_envelope(
            &promotion_curation_envelope(
                handoff.clone(),
                DataOwner::Personal,
                CurationKind::Knowledge,
                second_target,
                true,
            ),
            None,
        )
        .unwrap();
    let first_prepared = prepared_run_with(&engine, &first_resolved);
    let second_prepared = prepared_run_with(&engine, &second_resolved);
    let first_record = complete_reviewer_learning_curation_record(
        &engine,
        &first_resolved,
        &first_prepared,
        &handoff,
        "learning-race-a",
    );
    let second_record = complete_reviewer_learning_curation_record(
        &engine,
        &second_resolved,
        &second_prepared,
        &handoff,
        "learning-race-b",
    );
    engine
        .apply_validated_execution(&first_resolved, &first_prepared, &[], &first_record)
        .expect("first approved learning must apply");
    let second =
        engine.apply_validated_execution(&second_resolved, &second_prepared, &[], &second_record);
    assert!(matches!(
        second,
        Err(HarnessError::BatchApply { receipt, .. })
            if receipt.targets.is_empty()
                && receipt.orchestration_failures.iter().any(|failure| {
                    failure.stage == "batch-preflight"
                        && failure.message.contains("learning identifier already exists")
                })
    ));
    assert!(!repository.0.join(second_target).exists());
}

#[test]
fn promotion_handoff_requires_exact_user_confirmation_and_matching_scope() {
    let repository = TemporaryWorkspace::create();
    copy_directory(
        &repository_root().join("vault"),
        &repository.0.join("vault"),
    );
    let engine = HarnessEngine::open(&repository.0, &repository.0).unwrap();
    let (source_resolved, source_prepared, source_evaluation) =
        promotion_source_evaluation(&engine, false);
    let handoff = PromotionHandoff::from_evaluation(
        &source_resolved.plan,
        &source_prepared,
        &[],
        &source_evaluation,
        "proposal-0000000000000003",
    )
    .unwrap();

    let unconfirmed = promotion_curation_envelope(
        handoff.clone(),
        DataOwner::Personal,
        CurationKind::Fact,
        "vault/personal/facts/promotion-flow.md",
        false,
    );
    assert!(matches!(
        engine.resolve_envelope(&unconfirmed, None),
        Err(HarnessError::InvalidRequest(message))
            if message.contains("exact-value user confirmation")
    ));

    let mut structured = promotion_curation_envelope(
        handoff.clone(),
        DataOwner::Personal,
        CurationKind::Fact,
        "vault/personal/facts/promotion-flow.md",
        true,
    );
    structured.source = RequestSource::StructuredCaller {
        description: "attempted internal promotion".to_owned(),
    };
    assert!(matches!(
        engine.resolve_test_envelope(&structured, None),
        Err(HarnessError::InvalidRequest(message))
            if message.contains("cannot be supplied by a structured caller")
    ));

    let mut tampered = handoff.clone();
    tampered.handoff_digest = "0".repeat(64);
    assert!(
        engine
            .resolve_envelope(
                &promotion_curation_envelope(
                    tampered,
                    DataOwner::Personal,
                    CurationKind::Fact,
                    "vault/personal/facts/promotion-flow.md",
                    true,
                ),
                None,
            )
            .is_err()
    );

    for (owner, kind, target) in [
        (
            DataOwner::Profile,
            CurationKind::Fact,
            "vault/profile/facts/promotion-flow.md",
        ),
        (
            DataOwner::Personal,
            CurationKind::Knowledge,
            "vault/personal/knowledge/promotion-flow.md",
        ),
    ] {
        assert!(matches!(
            engine.resolve_envelope(
                &promotion_curation_envelope(handoff.clone(), owner, kind, target, true),
                None,
            ),
            Err(HarnessError::InvalidRequest(message))
                if message.contains("owner and curation kind")
        ));
    }
}

#[test]
fn promotion_handoff_is_visible_to_roles_and_required_on_curation_entries() {
    let repository = TemporaryWorkspace::create();
    copy_directory(
        &repository_root().join("vault"),
        &repository.0.join("vault"),
    );
    let engine = HarnessEngine::open(&repository.0, &repository.0).unwrap();
    let (source_resolved, source_prepared, source_evaluation) =
        promotion_source_evaluation(&engine, false);
    let handoff = PromotionHandoff::from_evaluation(
        &source_resolved.plan,
        &source_prepared,
        &[],
        &source_evaluation,
        "proposal-0000000000000003",
    )
    .unwrap();
    let envelope = promotion_curation_envelope(
        handoff.clone(),
        DataOwner::Personal,
        CurationKind::Fact,
        "vault/personal/facts/promotion-flow.md",
        true,
    );
    let resolved = engine.resolve_envelope(&envelope, None).unwrap();
    assert_eq!(resolved.plan.promotion_handoff, Some(handoff.clone()));
    let re_resolved = engine
        .resolve_contract(
            resolved.contract.clone(),
            resolved.decision_trace.clone(),
            resolved.request_provenance.clone(),
            None,
        )
        .unwrap();
    assert_eq!(
        re_resolved, resolved,
        "promotion-backed resolution must be stable before prepare"
    );
    let prepared = prepared_run_with(&engine, &resolved);
    assert!(
        prepared
            .role_metadata
            .iter()
            .all(|metadata| { metadata.scope.promotion_handoff.as_ref() == Some(&handoff) })
    );
    assert!(prepared.role_bundles.iter().all(|bundle| {
        bundle.segments.iter().any(|segment| {
            matches!(
                segment,
                HarnessRoleSegment::ControlHead { content, .. }
                    if content.contains(&handoff.handoff_digest)
            )
        })
    }));

    let mut submission = complete_submission_with(&engine, &resolved);
    let SubmissionArtifact::Curation { entries, .. } = &mut submission.artifact else {
        unreachable!("promotion-backed curation must produce entries");
    };
    assert!(
        entries
            .iter()
            .all(|entry| entry.promotion_handoff_digest.as_deref()
                == Some(handoff.handoff_digest.as_str()))
    );
    entries[0].promotion_handoff_digest = None;
    assert!(matches!(
        engine.validate_submission(&resolved, &prepared, &submission),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("promotion handoff digest")
    ));

    let ordinary_request = HarnessRequest {
        action: HarnessAction::VaultCuration,
        owner: DataOwner::Personal,
        targets: vec!["vault/personal/facts/ordinary-curation.md".to_owned()],
        objective: "일반 사용자가 직접 확인한 사실을 저장한다".to_owned(),
        curation_kind: Some(CurationKind::Fact),
        explicit_user_confirmation_reported: true,
        curation_sources: vec!["vault/personal/index.md".to_owned()],
        delete_targets: Vec::new(),
    };
    let ordinary_resolved = engine.resolve(&ordinary_request).unwrap();
    let ordinary_prepared = prepared_run_with(&engine, &ordinary_resolved);
    let ordinary_submission = complete_submission_with(&engine, &ordinary_resolved);
    let SubmissionArtifact::Curation { entries, .. } = &ordinary_submission.artifact else {
        unreachable!("ordinary curation must produce entries");
    };
    assert!(
        entries
            .iter()
            .all(|entry| entry.promotion_handoff_digest.is_none())
    );
    engine
        .validate_submission(&ordinary_resolved, &ordinary_prepared, &ordinary_submission)
        .expect("ordinary confirmed curation must remain valid");
}

#[test]
fn every_role_evidence_reference_must_stay_inside_the_prepared_scope() {
    let engine = engine();
    let resolved = engine
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let mut step = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("execution must begin");
    let writer = step.ready_role_invocations[0].clone();
    step = engine
        .advance_execution(
            &resolved,
            &prepared,
            &[],
            &step.record,
            completed_role_event(&resolved, &prepared, &writer, Vec::new(), "scoped-evidence"),
        )
        .expect("Writer must complete");
    let reviewer = step
        .ready_role_invocations
        .iter()
        .find(|invocation| invocation.role == HarnessRole::Reviewer)
        .cloned()
        .expect("Reviewer must be ready");
    let mut event = completed_role_event(
        &resolved,
        &prepared,
        &reviewer,
        Vec::new(),
        "scoped-evidence",
    );
    let HarnessExecutionEvent::RoleResult { result } = &mut event else {
        panic!("Reviewer event must contain a role result");
    };
    let RoleExecutionOutcome::Completed {
        result:
            CompletedRoleResult::Reviewer {
                requirement_results,
                ..
            },
    } = &mut result.outcome
    else {
        panic!("Reviewer result must be completed");
    };
    requirement_results[0]
        .evidence
        .push(ResultEvidenceReference::BoundDocument {
            relative_path: "vault/work/unbound/private.md".to_owned(),
            content_digest: "0".repeat(64),
            locator: "forged extra reference".to_owned(),
        });
    refresh_role_event_digest(&mut event);

    assert!(matches!(
        engine.advance_execution(&resolved, &prepared, &[], &step.record, event),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("outside the prepared role scope")
    ));
}

#[test]
fn tool_requirements_reject_empty_or_unstructured_evidence() {
    let engine = engine();
    let resolved = engine
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let mut step = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("execution must begin");
    let writer = step.ready_role_invocations[0].clone();
    step = engine
        .advance_execution(
            &resolved,
            &prepared,
            &[],
            &step.record,
            completed_role_event(&resolved, &prepared, &writer, Vec::new(), "tool-evidence"),
        )
        .expect("Writer must complete");
    let invocation = step
        .ready_tool_invocation
        .clone()
        .expect("tool verification must be ready");
    let mut evidence = passed_tool_evidence_set(&invocation);
    evidence.results[0].evidence.clear();
    evidence.evidence_set_digest = serialized_digest(&(
        &evidence.invocation_digest,
        &evidence.results,
        &evidence.executions,
    ))
    .expect("tampered evidence set must serialize");

    assert!(matches!(
        engine.advance_execution(
            &resolved,
            &prepared,
            &[],
            &step.record,
            HarnessExecutionEvent::ToolEvidence { evidence },
        ),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("every matching structured tool result")
    ));
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "promotion orchestration and digest ordering remain together to preserve one scenario"
)]
fn promotion_proposals_preserve_the_granted_source_owner() {
    let (_workspace, engine) = external_engine();
    let evidence_owner = DataOwner::Company {
        company: "cluml".to_owned(),
    };
    let evidence_path = "vault/work/cluml/index.md".to_owned();
    let request = HarnessRequest {
        action: HarnessAction::Investigation,
        owner: DataOwner::Personal,
        targets: vec!["README.md".to_owned()],
        objective: "analyze career evidence without changing its owner".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let grant = ContextGrant {
        owner: evidence_owner.clone(),
        purpose: ContextGrantPurpose::CareerWritingEvidence,
        access: ContextAccess::ReadOnly,
    };
    let manifest = selected_career_manifest(
        request.targets.clone(),
        vec![(evidence_owner.clone(), vec![evidence_path.clone()])],
    );
    let resolved = engine
        .resolve_with_career_composition(
            &request,
            std::slice::from_ref(&grant),
            Some(CareerOutputSurface::Resume),
            std::slice::from_ref(&evidence_path),
            Some(&manifest),
        )
        .expect("career investigation must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let source = prepared
        .role_bundles
        .iter()
        .find(|bundle| bundle.role == HarnessRole::Specialist)
        .and_then(|bundle| {
            bundle
                .bound_documents()
                .find(|document| document.relative_path == evidence_path)
        })
        .cloned()
        .expect("granted evidence must be in the Specialist bundle");
    let mut step = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("investigation must begin");
    let specialist = step.ready_role_invocations[0].clone();
    let proposal = PromotionProposal {
        identifier: "proposal-0000000000000002".to_owned(),
        owner: DataOwner::Personal,
        curation_kind: CurationKind::Fact,
        title: "Granted company fact".to_owned(),
        content: "This fact remains owned by its company evidence source.".to_owned(),
        origin: PromotionProposalOrigin::SpecialistMemory {
            claim_kind: PromotionClaimKind::ObservedFact,
            evidence: vec![PromotionEvidenceReference {
                source_kind: PromotionSourceKind::GrantedEvidence,
                source_owner: evidence_owner.clone(),
                relative_path: source.relative_path.clone(),
                content_digest: source.content_digest.clone(),
                locator: "bound company evidence".to_owned(),
            }],
        },
    };
    let mut wrong_owner_event = completed_role_event(
        &resolved,
        &prepared,
        &specialist,
        Vec::new(),
        "promotion-owner",
    );
    let HarnessExecutionEvent::RoleResult { result } = &mut wrong_owner_event else {
        panic!("Specialist event must contain a role result");
    };
    let RoleExecutionOutcome::Completed {
        result:
            CompletedRoleResult::Specialist {
                promotion_proposals,
                ..
            },
    } = &mut result.outcome
    else {
        panic!("Specialist result must be completed");
    };
    promotion_proposals.push(proposal.clone());
    refresh_role_event_digest(&mut wrong_owner_event);
    assert!(matches!(
        engine.advance_execution(
            &resolved,
            &prepared,
            &[],
            &step.record,
            wrong_owner_event,
        ),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("evidence owner must match")
    ));

    let mut correct_event = completed_role_event(
        &resolved,
        &prepared,
        &specialist,
        Vec::new(),
        "promotion-owner",
    );
    let HarnessExecutionEvent::RoleResult { result } = &mut correct_event else {
        panic!("Specialist event must contain a role result");
    };
    let RoleExecutionOutcome::Completed {
        result:
            CompletedRoleResult::Specialist {
                promotion_proposals,
                ..
            },
    } = &mut result.outcome
    else {
        panic!("Specialist result must be completed");
    };
    promotion_proposals.push(PromotionProposal {
        owner: evidence_owner.clone(),
        ..proposal
    });
    refresh_role_event_digest(&mut correct_event);
    step = engine
        .advance_execution(&resolved, &prepared, &[], &step.record, correct_event)
        .expect("source-owned proposal must advance");
    let reviewer = step.ready_role_invocations[0].clone();
    step = engine
        .advance_execution(
            &resolved,
            &prepared,
            &[],
            &step.record,
            completed_role_event(
                &resolved,
                &prepared,
                &reviewer,
                Vec::new(),
                "promotion-owner",
            ),
        )
        .expect("Reviewer must complete");
    let evaluation = engine
        .evaluate_execution(&resolved, &prepared, &[], &step.record)
        .expect("investigation must evaluate");
    assert!(matches!(
        evaluation.promotion_state,
        PromotionState::RequiresSeparateCuration { ref proposals }
            if proposals.len() == 1 && proposals[0].owner == evidence_owner
    ));
}

#[test]
fn failed_non_write_requirements_are_rejected_instead_of_completed() {
    let mut request = profile_code_request();
    request.action = HarnessAction::Investigation;
    request.objective = "analyze the implementation".to_owned();
    let engine = engine();
    let resolved = engine
        .resolve(&request)
        .expect("investigation must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let mut step = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("investigation must begin");
    let specialist = step.ready_role_invocations[0].clone();
    step = engine
        .advance_execution(
            &resolved,
            &prepared,
            &[],
            &step.record,
            completed_role_event(
                &resolved,
                &prepared,
                &specialist,
                Vec::new(),
                "analysis-failure",
            ),
        )
        .expect("Specialist must complete");
    let reviewer = step.ready_role_invocations[0].clone();
    let mut event = completed_role_event(
        &resolved,
        &prepared,
        &reviewer,
        Vec::new(),
        "analysis-failure",
    );
    let HarnessExecutionEvent::RoleResult { result } = &mut event else {
        panic!("Reviewer event must contain a role result");
    };
    let RoleExecutionOutcome::Completed {
        result:
            CompletedRoleResult::Reviewer {
                requirement_results,
                ..
            },
    } = &mut result.outcome
    else {
        panic!("Reviewer result must be completed");
    };
    requirement_results[0].passed = false;
    requirement_results[0].detail = "analysis completion contract failed".to_owned();
    refresh_role_event_digest(&mut event);
    step = engine
        .advance_execution(&resolved, &prepared, &[], &step.record, event)
        .expect("failed requirement remains a terminal evaluation");
    let evaluation = engine
        .evaluate_execution(&resolved, &prepared, &[], &step.record)
        .expect("failed analysis must evaluate");
    assert_eq!(evaluation.subject_status, SubjectStatus::Rejected);
    assert_eq!(
        evaluation.completion_state,
        HarnessCompletionState::Rejected
    );
}

#[test]
fn noncompleted_role_outcome_is_not_reported_as_subject_acceptance() {
    let engine = engine();
    let resolved = engine
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let step = engine
        .begin_execution(&resolved, &prepared, &[])
        .expect("execution must begin");
    let invocation = &step.ready_role_invocations[0];
    let context_id = "writer-timeout".to_owned();
    let lifecycle = ReportedRoleLifecycle {
        role: HarnessRole::Writer,
        context_id: context_id.clone(),
        started_at_millis: 1,
        context_ready_at_millis: 2,
        first_output_at_millis: None,
        interrupt_requested_at_millis: Some(3),
        grace_deadline_at_millis: Some(5),
        terminal_at_millis: 5,
        closed_at_millis: 6,
        terminal_state: RoleTerminalState::TimedOut,
    };
    let outcome = RoleExecutionOutcome::TimedOut;
    let result_digest = serialized_digest(&(
        HarnessRole::Writer,
        &invocation.invocation_digest,
        &context_id,
        &lifecycle,
        &outcome,
    ))
    .expect("timeout result must serialize");
    let terminal = engine
        .advance_execution(
            &resolved,
            &prepared,
            &[],
            &step.record,
            HarnessExecutionEvent::RoleResult {
                result: Box::new(RoleExecutionResult {
                    role: HarnessRole::Writer,
                    invocation_digest: invocation.invocation_digest.clone(),
                    context_id,
                    lifecycle,
                    outcome,
                    result_digest,
                }),
            },
        )
        .expect("timeout must produce a terminal execution record");
    let evaluation = engine
        .evaluate_execution(&resolved, &prepared, &[], &terminal.record)
        .expect("terminal timeout record must evaluate");
    assert_eq!(evaluation.execution_status, ExecutionStatus::TimedOut);
    assert_eq!(evaluation.subject_status, SubjectStatus::NotApplicable);
    assert_eq!(
        evaluation.completion_state,
        HarnessCompletionState::ExecutionHalted
    );
    assert_eq!(evaluation.assurance, ExecutionAssurance::Advisory);
    assert!(evaluation.artifact.is_none());
}

#[test]
fn user_statement_text_stays_out_of_plan_and_role_metadata() {
    let sentinel = "sensitive user statement sentinel";
    let envelope = user_profile_code_envelope(sentinel);
    let resolved = engine()
        .resolve_envelope(&envelope, None)
        .expect("user-language request envelope must resolve");
    let prepared = prepared_run(&resolved);
    let output = serde_json::to_string(&(resolved, prepared))
        .expect("resolved and prepared records must serialize");

    assert!(!output.contains(sentinel));
}

#[test]
fn unused_user_input_does_not_change_durable_plan_provenance() {
    let baseline = user_profile_code_envelope("change the request contract");
    let mut expanded = baseline.clone();
    let unused_statement_identifier = "statement-0000000000000002".to_owned();
    let RequestSource::UserLanguage { statements } = &mut expanded.source else {
        panic!("fixture must use user language");
    };
    statements.push(UserStatement {
        identifier: unused_statement_identifier.clone(),
        text: "unused statement".to_owned(),
    });
    expanded
        .decision_trace
        .records
        .push(DecisionRecord::UserStatement {
            identifier: "decision-0000000000000007".to_owned(),
            value_digest: serialized_digest(&true).expect("value must serialize"),
            statement_identifiers: vec![unused_statement_identifier],
        });

    let baseline = engine()
        .resolve_envelope(&baseline, None)
        .expect("baseline envelope must resolve");
    let expanded = engine()
        .resolve_envelope(&expanded, None)
        .expect("expanded envelope must resolve");

    assert_eq!(expanded.request_provenance, baseline.request_provenance);
    assert_eq!(expanded.resolved_plan_digest, baseline.resolved_plan_digest);
}

#[test]
fn noncurrent_plan_schema_is_rejected_everywhere() {
    let unsupported_version = HARNESS_SCHEMA_VERSION - 1;
    let mut resolved = engine()
        .resolve(&profile_code_request())
        .expect("current request must resolve");
    let prepared = prepared_run(&resolved);
    let submission = complete_submission(&resolved);
    resolved.plan.version = unsupported_version;
    resolved.resolved_plan_digest =
        serialized_digest(&resolved.plan).expect("fixture must serialize");

    assert!(matches!(
        engine().prepare(&resolved, &complete_runtime_capabilities(), None),
        Err(HarnessError::UnsupportedPlanVersion { found, current })
            if found == unsupported_version && current == HARNESS_SCHEMA_VERSION
    ));
    assert!(matches!(
        HarnessEngine::submission_template(&resolved, &prepared),
        Err(HarnessError::UnsupportedPlanVersion { found, current })
            if found == unsupported_version && current == HARNESS_SCHEMA_VERSION
    ));
    assert!(matches!(
        engine().evaluate_submission_with_history(&resolved, &prepared, &submission, &[]),
        Err(HarnessError::UnsupportedPlanVersion { found, current })
            if found == unsupported_version && current == HARNESS_SCHEMA_VERSION
    ));
    assert!(matches!(
        engine().apply_validated_submission(&resolved, &prepared, &submission, &[]),
        Err(HarnessError::UnsupportedPlanVersion { found, current })
            if found == unsupported_version && current == HARNESS_SCHEMA_VERSION
    ));
}

#[test]
fn plan_binds_workspace_agents_rules_and_prepare_keeps_role_boundaries() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("request must resolve");
    assert!(resolved.plan.workspace_policies.iter().any(|policy| {
        policy.workspace_relative_path == "AGENTS.md" && policy.content_digest.len() == 64
    }));

    let prepared = engine()
        .prepare(&resolved, &complete_runtime_capabilities(), None)
        .expect("unchanged workspace policies must prepare");
    assert_eq!(prepared.resolved_plan_digest, resolved.resolved_plan_digest);
    assert_eq!(prepared.role_metadata.len(), resolved.plan.role_count());
    assert!(prepared.role_metadata.iter().all(|metadata| {
        metadata.resolved_plan_digest == resolved.resolved_plan_digest
            && metadata.workspace_policies == resolved.plan.workspace_policies
    }));
    assert!(
        prepared
            .role_metadata
            .iter()
            .all(|metadata| metadata.independent_context_required)
    );

    let strict = strict_engine()
        .resolve(&profile_code_request())
        .expect("strict request must resolve");
    let strict_prepared = strict_engine()
        .prepare(&strict, &complete_runtime_capabilities(), None)
        .expect("strict request must prepare");
    assert!(
        strict_prepared
            .role_metadata
            .iter()
            .all(|metadata| metadata.independent_context_required)
    );
}

#[test]
fn prepare_rejects_a_runtime_without_separate_contexts() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let mut capabilities = complete_runtime_capabilities();
    capabilities.separate_contexts = false;
    assert!(matches!(
        engine().prepare(&resolved, &capabilities, None),
        Err(HarnessError::UnsupportedRuntime(_))
    ));
}

#[test]
fn prepare_rejects_an_unbounded_role_invocation_limit() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let mut capabilities = complete_runtime_capabilities();
    capabilities.max_role_invocation_bytes = MAX_ROLE_INVOCATION_BYTES + 1;
    assert!(matches!(
        engine().prepare(&resolved, &capabilities, None),
        Err(HarnessError::UnsupportedRuntime(message))
            if message.contains("role invocation")
    ));
}

#[test]
fn submission_cannot_bypass_the_prepared_runtime_receipt() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let prepared = prepared_run(&resolved);
    let mut submission = complete_submission(&resolved);
    submission.prepared_role_run_digest = "0".repeat(64);
    assert!(matches!(
        engine().validate_submission(&resolved, &prepared, &submission),
        Err(HarnessError::InvalidSubmission(_))
    ));
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "ontology retrieval orchestration and digest ordering remain together to preserve one scenario"
)]
fn ontology_read_scans_only_the_ontology_projection() {
    let request = HarnessRequest {
        action: HarnessAction::VaultRead,
        owner: DataOwner::Personal,
        targets: Vec::new(),
        objective: "read ontology context".to_owned(),
        curation_kind: Some(CurationKind::Ontology),
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = resolve_vault_read(
        &engine(),
        &request,
        HarnessExecutionProfile::Standard,
        "ontology",
        128 * 1024,
    );
    assert_eq!(
        resolved.plan.retrieval_roots,
        vec![ContextRoot {
            repository_relative_path: "vault/personal/ontology".to_owned(),
        }]
    );
    let bundle = engine()
        .retrieve_context(&resolved, "ontology", 128 * 1024)
        .expect("bounded ontology read must succeed");
    assert!(bundle.documents.iter().all(|document| {
        document
            .repository_relative_path
            .starts_with("vault/personal/ontology/")
    }));
    let mut forged = bundle.clone();
    forged.documents[0].repository_relative_path = "vault/work/cluml/index.md".to_owned();
    forged.bundle_digest = serialized_digest(&(
        &forged.resolved_plan_digest,
        &forged.query,
        forged.max_bytes,
        &forged.documents,
        forged.total_content_bytes,
    ))
    .expect("forged bundle must be serializable");
    assert!(matches!(
        engine().prepare(&resolved, &complete_runtime_capabilities(), Some(&forged)),
        Err(HarnessError::InvalidRequest(_))
    ));
    let prepared = engine()
        .prepare(&resolved, &complete_runtime_capabilities(), Some(&bundle))
        .expect("bounded read bundle must prepare");
    assert_eq!(prepared.context_bundle, Some(bundle.clone()));
    assert!(prepared.role_metadata.iter().all(|metadata| {
        metadata.scope.retrieval_roots == resolved.plan.retrieval_roots
            && metadata.scope.context_bundle_digest.as_deref()
                == Some(bundle.bundle_digest.as_str())
    }));
    let mut submission = HarnessEngine::submission_template(&resolved, &prepared)
        .expect("read template must bind the prepared bundle");
    fill_reported_role_contexts(&mut submission, &resolved.plan, "1");
    assert!(submission.reported_reviewer_context_id.is_empty());
    assert!(
        prepared
            .role_metadata
            .iter()
            .all(|metadata| !metadata.independent_context_required)
    );
    for check in &mut submission.verification_checks {
        check.passed = true;
        check.detail = "checked".to_owned();
    }
    let SubmissionArtifact::Analysis {
        context_bundle_digest,
        output,
        ..
    } = &mut submission.artifact
    else {
        panic!("Vault read must produce an analysis artifact");
    };
    *output = "bounded result".to_owned();
    assert_eq!(
        context_bundle_digest.as_deref(),
        Some(bundle.bundle_digest.as_str())
    );
    engine()
        .validate_submission(&resolved, &prepared, &submission)
        .expect("bundle-bound submission must validate");

    let mut unexpected_reviewer = submission.clone();
    unexpected_reviewer.reported_reviewer_context_id = "reviewer-1".to_owned();
    assert!(matches!(
        engine().validate_submission(&resolved, &prepared, &unexpected_reviewer),
        Err(HarnessError::InvalidSubmission(message))
            if message == "plans without a reviewer role require an empty reviewer context ID"
    ));

    let SubmissionArtifact::Analysis {
        context_bundle_digest,
        ..
    } = &mut submission.artifact
    else {
        panic!("Vault read must produce an analysis artifact");
    };
    *context_bundle_digest = None;
    assert!(matches!(
        engine().validate_submission(&resolved, &prepared, &submission),
        Err(HarnessError::InvalidSubmission(_))
    ));
}

#[test]
fn strict_vault_read_requires_distinct_context_for_every_planned_role() {
    let request = HarnessRequest {
        action: HarnessAction::VaultRead,
        owner: DataOwner::Personal,
        targets: Vec::new(),
        objective: "read personal knowledge with strict role separation".to_owned(),
        curation_kind: Some(CurationKind::Knowledge),
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let engine = strict_engine();
    let resolved = resolve_vault_read(
        &engine,
        &request,
        HarnessExecutionProfile::Strict,
        "knowledge",
        128 * 1024,
    );
    assert_eq!(
        planned_roles(&resolved.plan),
        vec![HarnessRole::Specialist, HarnessRole::Reviewer]
    );
    let bundle = engine
        .retrieve_context(&resolved, "knowledge", 128 * 1024)
        .expect("strict read retrieval must succeed");
    let prepared = engine
        .prepare(&resolved, &complete_runtime_capabilities(), Some(&bundle))
        .expect("strict read must prepare with a bundle");
    let mut submission = HarnessEngine::submission_template(&resolved, &prepared)
        .expect("strict read template must bind the prepared bundle");
    fill_reported_role_contexts(&mut submission, &resolved.plan, "strict");
    for check in &mut submission.verification_checks {
        check.passed = true;
        check.detail = "checked".to_owned();
    }
    let SubmissionArtifact::Analysis { output, .. } = &mut submission.artifact else {
        panic!("strict Vault read must produce an analysis artifact");
    };
    *output = "strict bounded result".to_owned();
    engine
        .validate_submission(&resolved, &prepared, &submission)
        .expect("strict read with distinct role contexts must validate");

    let mut duplicate_role_context = submission.clone();
    duplicate_role_context.reported_role_contexts[0].context_id = duplicate_role_context
        .reported_role_contexts[1]
        .context_id
        .clone();
    assert!(matches!(
        engine.validate_submission(&resolved, &prepared, &duplicate_role_context),
        Err(HarnessError::InvalidSubmission(message))
            if message == "reported role context IDs must be unique"
    ));

    let mut missing_role_context = submission;
    missing_role_context.reported_role_contexts.pop();
    assert!(matches!(
        engine.validate_submission(&resolved, &prepared, &missing_role_context),
        Err(HarnessError::InvalidSubmission(message))
            if message == "reported role context IDs must include every planned role"
    ));
}

#[test]
fn project_fact_read_uses_canonical_project_sources() {
    let vault = TemporaryWorkspace::create();
    copy_directory(
        &repository_root().join("vault/profile"),
        &vault.0.join("vault/profile"),
    );
    copy_ai_collaboration_values_fixture(&vault.0);
    let project = vault.0.join("vault/personal/projects/demo");
    let facts = project.join("facts");
    fs::create_dir_all(&facts).expect("synthetic typed fact root must be created");
    fs::write(
        vault.0.join("vault/personal/index.md"),
        "# Personal fixture\n",
    )
    .expect("synthetic personal index must be written");
    fs::write(
        vault.0.join("vault/personal/projects/demo.md"),
        "# Demo legacy entry\n\nLegacy database description.\n",
    )
    .expect("synthetic project entrypoint must be written");
    fs::write(
        project.join("summary.md"),
        "# Demo summary\n\nGeneral database description.\n",
    )
    .expect("synthetic untyped project context must be written");
    fs::write(
        facts.join("database.md"),
        "# Database facts\n\nSynthetic canonical database fact.\n",
    )
    .expect("synthetic typed fact must be written");
    let engine =
        HarnessEngine::open(&vault.0, &vault.0).expect("synthetic Vault fixture must open");
    let request = HarnessRequest {
        action: HarnessAction::VaultRead,
        owner: DataOwner::PersonalProject {
            project: "demo".to_owned(),
        },
        targets: Vec::new(),
        objective: "read project facts".to_owned(),
        curation_kind: Some(CurationKind::Fact),
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = resolve_vault_read(
        &engine,
        &request,
        HarnessExecutionProfile::Standard,
        "database",
        64 * 1024,
    );
    assert_eq!(
        resolved.plan.retrieval_roots,
        vec![ContextRoot {
            repository_relative_path: "vault/personal/projects/demo/facts".to_owned(),
        }]
    );
    let bundle = engine
        .retrieve_context(&resolved, "database", 64 * 1024)
        .expect("typed project facts must be retrievable");
    assert_eq!(bundle.documents.len(), 1);
    assert_eq!(
        bundle.documents[0].repository_relative_path,
        "vault/personal/projects/demo/facts/database.md"
    );
}

#[test]
fn typed_retrieval_traversal_excludes_nested_raw_conversations() {
    let temp = TemporaryWorkspace::create();
    let project = temp.0.join("vault/personal/projects/demo");
    let raw = project.join("conversations/raw");
    fs::create_dir_all(&raw).expect("raw conversation directory should be created");
    fs::write(project.join("summary.md"), "# Summary\nSafe context")
        .expect("summary fixture should be written");
    fs::write(raw.join("private.md"), "# Private\nSensitive raw context")
        .expect("raw fixture should be written");
    let repository = VaultRepository::open(&temp.0).expect("temporary Vault should open");
    let mut paths = Vec::new();

    repository
        .collect_markdown_paths(Path::new("vault/personal/projects/demo"), &mut paths, 0)
        .expect("retrieval traversal should succeed");

    assert_eq!(
        paths,
        vec![PathBuf::from("vault/personal/projects/demo/summary.md")]
    );
}

#[test]
fn retrieval_selects_whole_ranked_sections_with_exact_metadata() {
    let temp = TemporaryWorkspace::create();
    let knowledge = temp.0.join("vault/personal/knowledge");
    fs::create_dir_all(&knowledge).expect("knowledge fixture root must be created");
    let content = concat!(
        "---\n",
        "title: Synthetic\n",
        "aliases:\n",
        "  - metadata-token\n",
        "# preserved YAML comment\n",
        "---\n",
        "# Alpha\n",
        "needle alpha\n",
        "    ## indented code\n",
        "````text\n",
        "```\n",
        "## not a heading\n",
        "```` trailing text\n",
        "## still inside the fence\n",
        "````\n",
        "## Beta\n",
        "needle beta\n",
    );
    fs::write(knowledge.join("sections.md"), content).expect("section fixture must be written");
    let repository = VaultRepository::open(&temp.0).expect("temporary Vault should open");
    let documents = repository
        .retrieve_context_documents(
            &[ContextRoot {
                repository_relative_path: "vault/personal/knowledge".to_owned(),
            }],
            "needle beta",
            64 * 1024,
        )
        .expect("ranked sections must be retrievable");

    assert_eq!(documents.len(), 2);
    assert_eq!(documents[0].start_line, 16);
    assert_eq!(documents[0].end_line, 17);
    assert_eq!(documents[0].content, "## Beta\nneedle beta\n");
    assert_eq!(documents[0].matched_terms, vec!["beta", "needle"]);
    assert_eq!(documents[1].start_line, 1);
    assert_eq!(documents[1].end_line, 15);
    assert_eq!(documents[1].matched_terms, vec!["needle"]);
    assert!(
        documents
            .windows(2)
            .all(
                |pair| pair[0].repository_relative_path != pair[1].repository_relative_path
                    || pair[0].end_line < pair[1].start_line
                    || pair[1].end_line < pair[0].start_line
            )
    );

    let frontmatter_match = repository
        .retrieve_context_documents(
            &[ContextRoot {
                repository_relative_path: "vault/personal/knowledge".to_owned(),
            }],
            "metadata-token",
            64 * 1024,
        )
        .expect("frontmatter metadata must remain searchable");
    assert_eq!(frontmatter_match.len(), 1);
    assert_eq!(frontmatter_match[0].start_line, 1);
    assert_eq!(frontmatter_match[0].end_line, 15);
    assert!(
        frontmatter_match[0]
            .content
            .contains("# preserved YAML comment")
    );
    assert!(
        frontmatter_match[0]
            .content
            .contains("# Alpha\nneedle alpha\n")
    );
}

#[test]
fn retrieval_uses_canonical_yaml_validation_and_an_explicit_atx_section_contract() {
    let temp = TemporaryWorkspace::create();
    let knowledge = temp.0.join("vault/personal/knowledge");
    fs::create_dir_all(&knowledge).expect("knowledge fixture root must be created");
    fs::write(
        knowledge.join("invalid-frontmatter.md"),
        "---\ntitle: [unterminated\n---\n# Alpha\nneedle\n",
    )
    .expect("invalid frontmatter fixture must be written");
    let repository = VaultRepository::open(&temp.0).expect("temporary Vault should open");
    let roots = [ContextRoot {
        repository_relative_path: "vault/personal/knowledge".to_owned(),
    }];
    assert!(matches!(
        repository.retrieve_context_documents(&roots, "needle", 64 * 1024),
        Err(HarnessError::InvalidRepository(message))
            if message.contains("retrieval Markdown frontmatter is invalid")
    ));

    fs::remove_file(knowledge.join("invalid-frontmatter.md"))
        .expect("invalid frontmatter fixture must be removed");
    fs::write(
        knowledge.join("atx-contract.md"),
        "Alpha\n=====\nneedle alpha\nBeta\n-----\nneedle beta\n",
    )
    .expect("Setext body fixture must be written");
    let documents = repository
        .retrieve_context_documents(&roots, "needle beta", 64 * 1024)
        .expect("Setext syntax must remain ordinary body text under the ATX contract");
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].start_line, 1);
    assert_eq!(documents[0].end_line, 6);
    assert_eq!(
        documents[0].content,
        "Alpha\n=====\nneedle alpha\nBeta\n-----\nneedle beta\n"
    );
}

#[test]
fn retrieval_distinguishes_no_match_from_a_match_that_cannot_fit() {
    let temp = TemporaryWorkspace::create();
    let knowledge = temp.0.join("vault/personal/knowledge");
    fs::create_dir_all(&knowledge).expect("knowledge fixture root must be created");
    let content = "# Alpha\nunrelated\n## Beta\nneedle beta\n";
    fs::write(knowledge.join("sections.md"), content).expect("section fixture must be written");
    let repository = VaultRepository::open(&temp.0).expect("temporary Vault should open");
    let roots = [ContextRoot {
        repository_relative_path: "vault/personal/knowledge".to_owned(),
    }];

    assert!(matches!(
        repository.retrieve_context_documents(&roots, "missing-token", 64 * 1024),
        Err(HarnessError::InvalidRequest(message))
            if message == "retrieval query matched no Markdown sections"
    ));
    let matched_section = "## Beta\nneedle beta\n";
    assert!(matches!(
        repository.retrieve_context_documents(&roots, "beta", matched_section.len() - 1),
        Err(HarnessError::InvalidRequest(message))
            if message
                == "retrieval query matched Markdown sections, but none fit the byte budget"
    ));
}

#[test]
fn retrieval_rejects_a_section_count_explosion_before_ranking() {
    use std::fmt::Write as _;

    let temp = TemporaryWorkspace::create();
    let knowledge = temp.0.join("vault/personal/knowledge");
    fs::create_dir_all(&knowledge).expect("knowledge fixture root must be created");
    let content = (0..=MAX_RETRIEVAL_SECTIONS).fold(String::new(), |mut content, index| {
        writeln!(content, "# Section {index}\nneedle")
            .expect("section fixture must append to a String");
        content
    });
    fs::write(knowledge.join("many-sections.md"), content)
        .expect("section-count fixture must be written");
    let repository = VaultRepository::open(&temp.0).expect("temporary Vault should open");

    assert!(matches!(
        repository.retrieve_context_documents(
            &[ContextRoot {
                repository_relative_path: "vault/personal/knowledge".to_owned(),
            }],
            "needle",
            64 * 1024,
        ),
        Err(HarnessError::InvalidRepository(message))
            if message
                == format!(
                    "retrieval scan contains more than {MAX_RETRIEVAL_SECTIONS} Markdown sections"
                )
    ));
}

#[cfg(unix)]
#[test]
fn retrieval_rejects_a_symbolic_link_root() {
    use std::os::unix::fs::symlink;

    let temp = TemporaryWorkspace::create();
    let outside = TemporaryWorkspace::create();
    fs::create_dir_all(temp.0.join("vault/personal"))
        .expect("personal fixture root must be created");
    fs::create_dir_all(outside.0.join("knowledge")).expect("outside fixture root must be created");
    fs::write(
        outside.0.join("knowledge/private.md"),
        "# Private\nneedle\n",
    )
    .expect("outside fixture must be written");
    symlink(
        outside.0.join("knowledge"),
        temp.0.join("vault/personal/knowledge"),
    )
    .expect("retrieval root symlink must be created");
    let repository = VaultRepository::open(&temp.0).expect("temporary Vault should open");

    assert!(matches!(
        repository.retrieve_context_documents(
            &[ContextRoot {
                repository_relative_path: "vault/personal/knowledge".to_owned(),
            }],
            "needle",
            64 * 1024,
        ),
        Err(HarnessError::PathEscapesRoot(_))
    ));
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "role-bundle budget test keeps the exact mandatory-byte boundary, ordered controls, and optional-section remainder in one scenario"
)]
fn prepare_uses_actual_remaining_budget_and_orders_control_segments() {
    let vault = TemporaryWorkspace::create();
    copy_directory(
        &repository_root().join("vault/profile"),
        &vault.0.join("vault/profile"),
    );
    copy_ai_collaboration_values_fixture(&vault.0);
    for relative in [
        "vault/personal/decisions",
        "vault/personal/facts",
        "vault/personal/learning",
        "vault/personal/ontology",
        "vault/personal/projects/ideas",
        "vault/personal/writing",
    ] {
        fs::create_dir_all(vault.0.join(relative))
            .expect("allowed personal fixture root must be created");
    }
    let knowledge = vault.0.join("vault/personal/knowledge");
    fs::create_dir_all(&knowledge).expect("knowledge fixture root must be created");
    fs::write(
        vault.0.join("vault/personal/index.md"),
        "# Personal fixture\n",
    )
    .expect("personal fixture index must be written");
    fs::write(
        vault.0.join("vault/personal/profile.md"),
        "# Personal profile fixture\n",
    )
    .expect("personal profile fixture must be written");
    fs::write(
        knowledge.join("sections.md"),
        format!(
            "# First\nneedle {}\n## Second\nneedle {}\n",
            "a".repeat(4_096),
            "b".repeat(4_096)
        ),
    )
    .expect("section fixture must be written");
    let engine = HarnessEngine::open(&vault.0, &vault.0).expect("temporary Vault must open");
    let request = HarnessRequest {
        action: HarnessAction::VaultRead,
        owner: DataOwner::Personal,
        targets: Vec::new(),
        objective: "read the exact bounded knowledge sections".to_owned(),
        curation_kind: Some(CurationKind::Knowledge),
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = resolve_vault_read(
        &engine,
        &request,
        HarnessExecutionProfile::Standard,
        "needle",
        64 * 1024,
    );
    let context = engine
        .retrieve_context(&resolved, "needle", 64 * 1024)
        .expect("both matched sections must be retrieved");
    assert_eq!(context.documents.len(), 2);
    let full = engine
        .prepare(&resolved, &complete_runtime_capabilities(), Some(&context))
        .expect("full bounded context must prepare");
    let full_bundle = &full.role_bundles[0];
    assert_eq!(
        full_bundle
            .bound_documents()
            .filter(|document| document.source == HarnessBoundDocumentSource::Retrieval)
            .count(),
        2
    );
    let second_section_bytes = context.documents[1].content.len();
    let mut constrained = complete_runtime_capabilities();
    constrained.max_role_bundle_bytes = full_bundle.serialized_bytes - second_section_bytes / 2;
    let selected = engine
        .prepare(&resolved, &constrained, Some(&context))
        .expect("actual remaining budget must retain the highest-ranked whole section");
    let selected_bundle = &selected.role_bundles[0];
    assert_eq!(
        selected_bundle
            .bound_documents()
            .filter(|document| document.source == HarnessBoundDocumentSource::Retrieval)
            .count(),
        1
    );
    let Some(HarnessRoleSegment::ControlHead {
        content,
        content_digest,
    }) = selected_bundle.segments.first()
    else {
        panic!("role bundle must begin with a control head");
    };
    assert!(content.contains(&request.objective));
    assert_eq!(content_digest, &byte_digest(content.as_bytes()));
    let Some(HarnessRoleSegment::ControlTail {
        content,
        content_digest,
    }) = selected_bundle.segments.last()
    else {
        panic!("role bundle must end with a control tail");
    };
    assert!(content.contains(&request.objective));
    assert_eq!(content_digest, &byte_digest(content.as_bytes()));

    let mut forged = context.clone();
    forged.documents[0].start_line += 1;
    forged.bundle_digest = serialized_digest(&(
        &forged.resolved_plan_digest,
        &forged.query,
        forged.max_bytes,
        &forged.documents,
        forged.total_content_bytes,
    ))
    .expect("forged bundle must remain serializable");
    assert!(matches!(
        engine.prepare(&resolved, &complete_runtime_capabilities(), Some(&forged)),
        Err(HarnessError::InvalidRequest(_))
    ));
    let mut forged_terms = context;
    forged_terms.documents[0].matched_terms = vec!["needle".to_owned(), "needle".to_owned()];
    forged_terms.bundle_digest = serialized_digest(&(
        &forged_terms.resolved_plan_digest,
        &forged_terms.query,
        forged_terms.max_bytes,
        &forged_terms.documents,
        forged_terms.total_content_bytes,
    ))
    .expect("forged bundle must remain serializable");
    assert!(matches!(
        engine.prepare(
            &resolved,
            &complete_runtime_capabilities(),
            Some(&forged_terms)
        ),
        Err(HarnessError::InvalidRequest(_))
    ));
}

#[test]
fn journal_read_requires_confirmation_and_excludes_other_personal_roots() {
    let mut request = HarnessRequest {
        action: HarnessAction::VaultRead,
        owner: DataOwner::Personal,
        targets: Vec::new(),
        objective: "read journal context".to_owned(),
        curation_kind: Some(CurationKind::Journal),
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    assert!(matches!(
        engine().resolve(&request),
        Err(HarnessError::InvalidRequest(_))
    ));
    request.explicit_user_confirmation_reported = true;
    let resolved = resolve_vault_read(
        &engine(),
        &request,
        HarnessExecutionProfile::Standard,
        "journal",
        64 * 1024,
    );
    assert_eq!(
        resolved.plan.retrieval_roots,
        vec![ContextRoot {
            repository_relative_path: "vault/personal/journal".to_owned(),
        }]
    );
    assert!(
        !resolved
            .plan
            .denied_context_roots
            .iter()
            .any(|root| root.repository_relative_path == "vault/personal/journal")
    );
}

#[test]
fn validated_single_file_apply_supports_create_update_and_delete() {
    let workspace = TemporaryWorkspace::create();
    let engine = HarnessEngine::open(repository_root(), &workspace.0)
        .expect("temporary workspace must open");
    let mut request = HarnessRequest {
        action: HarnessAction::DocumentWrite,
        owner: DataOwner::Personal,
        targets: vec!["nested/facts/note.md".to_owned()],
        objective: "write a note".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };

    let resolved = engine.resolve(&request).expect("create must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let mut submission = complete_submission_with(&engine, &resolved);
    let SubmissionArtifact::Changes { changes } = &mut submission.artifact else {
        panic!("document write must contain changes");
    };
    changes[0] = FileChange::Create {
        path: "nested/facts/note.md".to_owned(),
        content: "first".to_owned(),
    };
    let applied = engine
        .apply_validated_submission(&resolved, &prepared, &submission, &[])
        .expect("validated create must apply");
    assert_eq!(applied.operation, TargetOperation::Create);
    assert_eq!(
        applied.created_parent_directories,
        vec!["nested".to_owned(), "nested/facts".to_owned()]
    );
    assert_eq!(
        fs::read_to_string(workspace.0.join("nested/facts/note.md"))
            .expect("created file must be readable"),
        "first"
    );

    let resolved = engine.resolve(&request).expect("update must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let mut submission = complete_submission_with(&engine, &resolved);
    let SubmissionArtifact::Changes { changes } = &mut submission.artifact else {
        panic!("document write must contain changes");
    };
    let TargetState::Existing { content_digest } = &resolved.plan.targets[0].state else {
        panic!("update target must exist");
    };
    changes[0] = FileChange::Update {
        path: "nested/facts/note.md".to_owned(),
        expected_content_digest: content_digest.clone(),
        content: "second".to_owned(),
    };
    engine
        .apply_validated_submission(&resolved, &prepared, &submission, &[])
        .expect("validated update must apply");
    assert_eq!(
        fs::read_to_string(workspace.0.join("nested/facts/note.md"))
            .expect("updated file must be readable"),
        "second"
    );

    request.delete_targets = vec!["nested/facts/note.md".to_owned()];
    let resolved = engine.resolve(&request).expect("delete must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let submission = complete_submission_with(&engine, &resolved);
    let applied = engine
        .apply_validated_submission(&resolved, &prepared, &submission, &[])
        .expect("validated delete must apply");
    assert_eq!(applied.operation, TargetOperation::Delete);
    assert!(!workspace.0.join("nested/facts/note.md").exists());
}

#[test]
fn vault_markdown_apply_reports_file_apply_boundary() {
    assert!(requires_vault_markdown_validation(
        true,
        "vault/personal/knowledge/new.md"
    ));
    assert!(!requires_vault_markdown_validation(true, "README.md"));
    assert!(!requires_vault_markdown_validation(
        false,
        "vault/personal/knowledge/new.md"
    ));
}

#[test]
fn vault_markdown_apply_rejects_invalid_yaml_before_writing() {
    let workspace = TemporaryWorkspace::create();
    let target = "vault/personal/knowledge/invalid-yaml.md";
    fs::create_dir_all(workspace.0.join("vault/personal/knowledge"))
        .expect("Vault fixture directory should be created");
    let change = FileChange::Create {
        path: target.to_owned(),
        content: "---\ntitle: [unterminated\n---\nBody".to_owned(),
    };

    let error = apply_single_change_reported(&workspace.0, &change, &"0".repeat(64), &[], true)
        .expect_err("invalid Vault YAML must not be applied");

    assert!(
        error.to_string().contains("Vault Markdown is invalid"),
        "{error}"
    );
    assert!(!workspace.0.join(target).exists());
}

#[test]
fn vault_markdown_apply_rejects_invalid_ontology_before_writing() {
    let workspace = TemporaryWorkspace::create();
    let target = "vault/personal/knowledge/invalid-ontology.md";
    fs::create_dir_all(workspace.0.join("vault/personal/knowledge"))
        .expect("Vault fixture directory should be created");
    let change = FileChange::Create {
        path: target.to_owned(),
        content: "---\ntitle: Invalid Ontology\nscope: personal\nontology: true\ntype: unsupported\n---\nBody"
            .to_owned(),
    };

    let error = apply_single_change_reported(&workspace.0, &change, &"0".repeat(64), &[], true)
        .expect_err("invalid ontology metadata must not be applied");

    assert!(error.to_string().contains("supported vocabulary"));
    assert!(!workspace.0.join(target).exists());
}

#[test]
fn validated_multi_file_execution_applies_as_one_recoverable_batch() {
    let (workspace, engine) = external_engine();
    let request = HarnessRequest {
        action: HarnessAction::CodeWrite,
        owner: DataOwner::PersonalProject {
            project: "coupler".to_owned(),
        },
        targets: vec![
            "generated/new.rs".to_owned(),
            "src/lib.rs".to_owned(),
            "src/security.rs".to_owned(),
        ],
        objective: "create, update, and delete source files".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: vec!["src/security.rs".to_owned()],
    };
    let resolved = engine.resolve(&request).expect("request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let record = complete_execution_record(&engine, &resolved, &prepared, &[], "batch");
    let applied = engine
        .apply_validated_execution(&resolved, &prepared, &[], &record)
        .expect("validated changes must apply as one batch");

    assert_eq!(applied.targets.len(), 3);
    assert_eq!(applied.completion_state, HarnessCompletionState::Applied);
    assert!(!applied.lifecycle_receipt.journal_retained);
    assert_eq!(
        fs::read_to_string(workspace.0.join("src/lib.rs"))
            .expect("first applied target must be readable"),
        "candidate"
    );
    assert_eq!(
        fs::read_to_string(workspace.0.join("generated/new.rs"))
            .expect("created target must be readable"),
        "candidate"
    );
    assert!(!workspace.0.join("src/security.rs").exists());
    assert_batch_completion_receipt(
        &workspace.0,
        applied
            .lifecycle_receipt
            .completion_receipt_relative_path
            .as_ref(),
    );
}

#[test]
fn failed_batch_apply_rolls_every_target_back_before_cleanup() {
    let workspace = TemporaryWorkspace::create();
    fs::write(workspace.0.join("first.txt"), "first-original")
        .expect("first fixture must be written");
    fs::write(workspace.0.join("second.txt"), "second-original")
        .expect("second fixture must be written");
    let changes = [
        FileChange::Create {
            path: "nested/new.txt".to_owned(),
            content: "created".to_owned(),
        },
        FileChange::Update {
            path: "first.txt".to_owned(),
            expected_content_digest: byte_digest(b"first-original"),
            content: "first-new".to_owned(),
        },
        FileChange::Delete {
            path: "second.txt".to_owned(),
            expected_content_digest: byte_digest(b"second-original"),
        },
    ];
    let created_parents = vec!["nested".to_owned()];
    let inputs = vec![
        repository::BatchApplyInput {
            change: &changes[0],
            planned_parent_directories: &created_parents,
            validate_vault_markdown: false,
        },
        repository::BatchApplyInput {
            change: &changes[1],
            planned_parent_directories: &[],
            validate_vault_markdown: false,
        },
        repository::BatchApplyInput {
            change: &changes[2],
            planned_parent_directories: &[],
            validate_vault_markdown: false,
        },
    ];
    let root = workspace
        .0
        .canonicalize()
        .expect("batch workspace must canonicalize");
    let report = repository::apply_change_batch_reported_with_test_hook(
        &root,
        &inputs,
        &"1".repeat(64),
        &"2".repeat(64),
        |index| {
            if index == 2 {
                Err(HarnessError::InvalidRepository(
                    "injected third-target failure".to_owned(),
                ))
            } else {
                Ok(())
            }
        },
    )
    .expect("batch failure must produce a recovery report");
    let report = repository::complete_batch_cleanup_after_lock_release(&root, report);

    assert_eq!(report.outcome, repository::BatchApplyOutcome::RolledBack);
    assert!(!report.journal_retained);
    assert_eq!(
        fs::read_to_string(workspace.0.join("first.txt")).expect("first target must be restored"),
        "first-original"
    );
    assert_eq!(
        fs::read_to_string(workspace.0.join("second.txt"))
            .expect("second target must remain original"),
        "second-original"
    );
    assert!(!workspace.0.join("nested").exists());
    assert_batch_completion_receipt(
        &workspace.0,
        report.completion_receipt_relative_path.as_ref(),
    );
}

#[test]
fn batch_stages_every_candidate_and_backup_before_the_first_source_mutation() {
    let workspace = TemporaryWorkspace::create();
    fs::write(workspace.0.join("first.txt"), "first-original")
        .expect("first fixture must be written");
    fs::write(workspace.0.join("second.txt"), "second-original")
        .expect("second fixture must be written");
    let changes = [
        FileChange::Update {
            path: "first.txt".to_owned(),
            expected_content_digest: byte_digest(b"first-original"),
            content: "first-new".to_owned(),
        },
        FileChange::Update {
            path: "second.txt".to_owned(),
            expected_content_digest: byte_digest(b"second-original"),
            content: "second-new".to_owned(),
        },
    ];
    let inputs = changes
        .iter()
        .map(|change| repository::BatchApplyInput {
            change,
            planned_parent_directories: &[],
            validate_vault_markdown: false,
        })
        .collect::<Vec<_>>();
    let root = workspace
        .0
        .canonicalize()
        .expect("batch workspace must canonicalize");
    let report = repository::apply_change_batch_reported_with_test_hook(
        &root,
        &inputs,
        &"7".repeat(64),
        &"8".repeat(64),
        |index| {
            if index != 0 {
                return Ok(());
            }
            let batches = root.join(".llm-context-vault-harness/batches");
            let batch = fs::read_dir(&batches)
                .expect("batch directory must be readable")
                .next()
                .expect("one batch directory must exist")
                .expect("batch directory entry must be readable")
                .path();
            let staged = fs::read_dir(batch.join("staged"))
                .expect("staged directory must be readable")
                .count();
            let backups = fs::read_dir(batch.join("backups"))
                .expect("backup directory must be readable")
                .count();
            assert_eq!(staged, 2);
            assert_eq!(backups, 2);
            assert_eq!(
                fs::read_to_string(root.join("first.txt"))
                    .expect("first source must remain unchanged before commit"),
                "first-original"
            );
            assert_eq!(
                fs::read_to_string(root.join("second.txt"))
                    .expect("second source must remain unchanged before commit"),
                "second-original"
            );
            Err(HarnessError::InvalidRepository(
                "stop after staging inspection".to_owned(),
            ))
        },
    )
    .expect("staging inspection failure must produce a report");
    let report = repository::complete_batch_cleanup_after_lock_release(&root, report);

    assert_eq!(report.outcome, repository::BatchApplyOutcome::RolledBack);
    assert!(!report.journal_retained);
    assert_batch_completion_receipt(
        &workspace.0,
        report.completion_receipt_relative_path.as_ref(),
    );
}

#[test]
fn batch_reuses_only_its_own_confirmed_shared_parent_directory() {
    let workspace = TemporaryWorkspace::create();
    let changes = [
        FileChange::Create {
            path: "generated/first.txt".to_owned(),
            content: "first".to_owned(),
        },
        FileChange::Create {
            path: "generated/second.txt".to_owned(),
            content: "second".to_owned(),
        },
    ];
    let created_parents = vec!["generated".to_owned()];
    let inputs = changes
        .iter()
        .map(|change| repository::BatchApplyInput {
            change,
            planned_parent_directories: &created_parents,
            validate_vault_markdown: false,
        })
        .collect::<Vec<_>>();
    let root = workspace
        .0
        .canonicalize()
        .expect("batch workspace must canonicalize");
    let report =
        repository::apply_change_batch_reported(&root, &inputs, &"9".repeat(64), &"a".repeat(64))
            .expect("shared-parent batch must return a report");
    assert_eq!(report.outcome, repository::BatchApplyOutcome::Applied);
    assert!(report.journal_retained);
    assert!(workspace.0.join(&report.journal_relative_path).exists());
    let report = repository::complete_batch_cleanup_after_lock_release(&root, report);

    assert_eq!(report.outcome, repository::BatchApplyOutcome::Applied);
    assert!(!report.journal_retained);
    let completion_receipt_relative_path = report
        .completion_receipt_relative_path
        .clone()
        .expect("completed batch must expose a durable receipt");
    let active_journal = workspace.0.join(&report.journal_relative_path);
    fs::create_dir_all(
        active_journal
            .parent()
            .expect("active journal must have a parent"),
    )
    .expect("duplicate handoff batch directory must be created");
    fs::copy(
        workspace.0.join(&completion_receipt_relative_path),
        &active_journal,
    )
    .expect("duplicate durable handoff fixture must be copied");
    let engine = HarnessEngine::open(repository_root(), &workspace.0)
        .expect("temporary workspace must open");
    let duplicate_recovered = engine
        .recover_apply_batch(&report.journal_relative_path)
        .expect("identical active and completion receipts must recover idempotently");
    assert_eq!(
        duplicate_recovered.outcome,
        BatchApplyOutcomeReceipt::Applied
    );
    assert!(!active_journal.exists());
    assert!(workspace.0.join(&completion_receipt_relative_path).exists());
    assert_eq!(
        fs::read_to_string(workspace.0.join("generated/first.txt"))
            .expect("first shared-parent target must be readable"),
        "first"
    );
    assert_eq!(
        fs::read_to_string(workspace.0.join("generated/second.txt"))
            .expect("second shared-parent target must be readable"),
        "second"
    );
    assert_batch_completion_receipt(
        &workspace.0,
        report.completion_receipt_relative_path.as_ref(),
    );
}

#[test]
fn batch_rejects_a_replaced_batch_created_parent_before_target_commit() {
    let workspace = TemporaryWorkspace::create();
    let change = FileChange::Create {
        path: "nested/new.txt".to_owned(),
        content: "created".to_owned(),
    };
    let created_parents = vec!["nested".to_owned()];
    let inputs = [repository::BatchApplyInput {
        change: &change,
        planned_parent_directories: &created_parents,
        validate_vault_markdown: false,
    }];
    let root = workspace
        .0
        .canonicalize()
        .expect("batch workspace must canonicalize");
    let report = repository::apply_change_batch_reported_with_test_hook(
        &root,
        &inputs,
        &"b".repeat(64),
        &"c".repeat(64),
        |_| {
            fs::rename(root.join("nested"), root.join("nested-old"))
                .expect("batch-created parent must be moved");
            fs::create_dir(root.join("nested")).expect("replacement parent must be created");
            Ok(())
        },
    )
    .expect("parent substitution must return a recovery report");

    assert_eq!(
        report.outcome,
        repository::BatchApplyOutcome::RecoveryRequired
    );
    assert!(report.journal_retained);
    assert!(!workspace.0.join("nested/new.txt").exists());
    assert!(workspace.0.join("nested-old").is_dir());
    assert!(workspace.0.join("nested").is_dir());

    fs::remove_dir(workspace.0.join("nested")).expect("replacement parent must be removed");
    fs::rename(workspace.0.join("nested-old"), workspace.0.join("nested"))
        .expect("recorded parent identity must be restored");
    let engine = HarnessEngine::open(repository_root(), &workspace.0)
        .expect("temporary workspace must open");
    let recovered = engine
        .recover_apply_batch(&report.journal_relative_path)
        .expect("restored parent identity must allow rollback completion");
    assert_eq!(recovered.outcome, BatchApplyOutcomeReceipt::RolledBack);
    assert!(!workspace.0.join("nested").exists());
    assert_batch_completion_receipt(
        &workspace.0,
        recovered.completion_receipt_relative_path.as_ref(),
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "single recovery scenario cohesion keeps external-change and rollback ordering explicit"
)]
fn recovery_journal_preserves_external_changes_until_the_user_resolves_them() {
    let workspace = TemporaryWorkspace::create();
    fs::write(workspace.0.join("first.txt"), "first-original")
        .expect("first fixture must be written");
    fs::write(workspace.0.join("second.txt"), "second-original")
        .expect("second fixture must be written");
    let changes = [
        FileChange::Update {
            path: "first.txt".to_owned(),
            expected_content_digest: byte_digest(b"first-original"),
            content: "first-new".to_owned(),
        },
        FileChange::Update {
            path: "second.txt".to_owned(),
            expected_content_digest: byte_digest(b"second-original"),
            content: "second-new".to_owned(),
        },
    ];
    let inputs = changes
        .iter()
        .map(|change| repository::BatchApplyInput {
            change,
            planned_parent_directories: &[],
            validate_vault_markdown: false,
        })
        .collect::<Vec<_>>();
    let root = workspace
        .0
        .canonicalize()
        .expect("batch workspace must canonicalize");
    let report = repository::apply_change_batch_reported_with_test_hook(
        &root,
        &inputs,
        &"3".repeat(64),
        &"4".repeat(64),
        |index| {
            if index == 1 {
                fs::write(workspace.0.join("second.txt"), "external-change")
                    .expect("external change fixture must be written");
                Err(HarnessError::InvalidRepository(
                    "injected conflicting change".to_owned(),
                ))
            } else {
                Ok(())
            }
        },
    )
    .expect("conflicting failure must produce a recovery report");

    assert_eq!(
        report.outcome,
        repository::BatchApplyOutcome::RecoveryRequired
    );
    assert!(report.journal_retained);
    assert_eq!(
        fs::read_to_string(workspace.0.join("first.txt"))
            .expect("first target must be rolled back"),
        "first-original"
    );
    assert_eq!(
        fs::read_to_string(workspace.0.join("second.txt"))
            .expect("external target must be preserved"),
        "external-change"
    );
    let pending_error = repository::ensure_no_pending_batch_recovery(&root)
        .expect_err("retained recovery journal must block another batch");
    assert!(matches!(
        &pending_error,
        HarnessError::PendingBatchRecovery { artifacts }
            if artifacts == std::slice::from_ref(&report.journal_relative_path)
    ));

    let engine = HarnessEngine::open(repository_root(), &workspace.0)
        .expect("temporary workspace must open");
    let request = HarnessRequest {
        action: HarnessAction::DocumentWrite,
        owner: DataOwner::Personal,
        targets: vec!["third.txt".to_owned()],
        objective: "attempt another write while recovery is pending".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = engine.resolve(&request).expect("request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let record = complete_execution_record(&engine, &resolved, &prepared, &[], "third");
    let blocked_apply = engine
        .apply_validated_execution(&resolved, &prepared, &[], &record)
        .expect_err("pending recovery must block another apply");
    assert!(matches!(
        blocked_apply,
        HarnessError::BatchApply { receipt, .. }
            if receipt.outcome == BatchApplyOutcomeReceipt::RecoveryRequired
                && receipt.journal_retained
                && receipt.journal_relative_path == report.journal_relative_path
                && receipt.orchestration_failures.iter().any(|failure| {
                    failure.stage == "pending-batch-recovery"
                })
    ));
    assert!(!workspace.0.join("third.txt").exists());

    let still_blocked =
        repository::recover_change_batch_reported(&root, &report.journal_relative_path)
            .expect("recovery inspection must return a report");
    assert_eq!(
        still_blocked.outcome,
        repository::BatchApplyOutcome::RecoveryRequired
    );
    assert_eq!(
        fs::read_to_string(workspace.0.join("second.txt"))
            .expect("blocked recovery must preserve the external target"),
        "external-change"
    );

    fs::write(workspace.0.join("second.txt"), "second-original")
        .expect("user-resolved target must be restored");
    let recovered = engine
        .recover_apply_batch(&report.journal_relative_path)
        .expect("resolved public recovery must return a success receipt");
    assert_eq!(recovered.outcome, BatchApplyOutcomeReceipt::RolledBack);
    assert!(!recovered.journal_retained);
    assert_eq!(recovered.resolved_plan_digest, "3".repeat(64));
    assert_eq!(recovered.candidate_digest, "4".repeat(64));
    assert!(recovered.orchestration_failures.is_empty());
    assert!(!recovered.failure_history.is_empty());
    assert_batch_completion_receipt(
        &workspace.0,
        recovered.completion_receipt_relative_path.as_ref(),
    );
    let repeated = engine
        .recover_apply_batch(&report.journal_relative_path)
        .expect("completed recovery receipt must make retries idempotent");
    assert_eq!(repeated.outcome, BatchApplyOutcomeReceipt::RolledBack);
    assert_eq!(
        repeated.completion_receipt_relative_path,
        recovered.completion_receipt_relative_path
    );
    repository::ensure_no_pending_batch_recovery(&root)
        .expect("completed recovery must allow a later batch");
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "single recovery scenario cohesion keeps applied-state and cleanup ordering explicit"
)]
fn recovery_finalizes_a_fully_applied_batch_after_an_interrupted_cleanup() {
    let workspace = TemporaryWorkspace::create();
    fs::write(workspace.0.join("first.txt"), "first-original")
        .expect("first fixture must be written");
    fs::write(workspace.0.join("second.txt"), "second-original")
        .expect("second fixture must be written");
    let changes = [
        FileChange::Update {
            path: "first.txt".to_owned(),
            expected_content_digest: byte_digest(b"first-original"),
            content: "first-new".to_owned(),
        },
        FileChange::Update {
            path: "second.txt".to_owned(),
            expected_content_digest: byte_digest(b"second-original"),
            content: "second-new".to_owned(),
        },
    ];
    let inputs = changes
        .iter()
        .map(|change| repository::BatchApplyInput {
            change,
            planned_parent_directories: &[],
            validate_vault_markdown: false,
        })
        .collect::<Vec<_>>();
    let root = workspace
        .0
        .canonicalize()
        .expect("batch workspace must canonicalize");
    let interrupted = repository::apply_change_batch_reported_before_cleanup_for_test(
        &root,
        &inputs,
        &"5".repeat(64),
        &"6".repeat(64),
    )
    .expect("interrupted cleanup fixture must produce a report");
    assert_eq!(
        interrupted.outcome,
        repository::BatchApplyOutcome::RecoveryRequired
    );
    assert!(interrupted.journal_retained);
    assert_eq!(
        fs::read_to_string(workspace.0.join("first.txt"))
            .expect("first applied target must be readable"),
        "first-new"
    );
    assert_eq!(
        fs::read_to_string(workspace.0.join("second.txt"))
            .expect("second applied target must be readable"),
        "second-new"
    );

    fs::write(workspace.0.join("first.txt"), "external-after-apply")
        .expect("external applied-target change must be written");
    let engine = HarnessEngine::open(repository_root(), &workspace.0)
        .expect("temporary workspace must open");
    let drift_error = engine
        .recover_apply_batch(&interrupted.journal_relative_path)
        .expect_err("applied-target drift must keep the recovery journal");
    assert!(matches!(
        drift_error,
        HarnessError::BatchApply { receipt, .. }
            if receipt.batch_id == interrupted.batch_id
                && receipt.journal_relative_path == interrupted.journal_relative_path
                && receipt.journal_retained
                && receipt.targets.len() == 2
                && receipt.orchestration_failures.iter().any(|failure| {
                    failure.message.contains("first.txt")
                })
    ));
    assert_eq!(
        fs::read_to_string(workspace.0.join("first.txt"))
            .expect("recovery must preserve the external applied-target change"),
        "external-after-apply"
    );
    fs::write(workspace.0.join("first.txt"), "first-new")
        .expect("applied target must be restored before cleanup");

    let batch_directory = workspace
        .0
        .join(&interrupted.journal_relative_path)
        .parent()
        .expect("journal must have a batch directory")
        .to_path_buf();
    fs::write(batch_directory.join("unexpected.txt"), "external-artifact")
        .expect("unexpected cleanup artifact must be written");
    let cleanup_blocked =
        repository::recover_change_batch_reported(&root, &interrupted.journal_relative_path)
            .expect("blocked cleanup recovery must return a report");
    assert_eq!(
        cleanup_blocked.outcome,
        repository::BatchApplyOutcome::RecoveryRequired
    );
    assert!(cleanup_blocked.journal_retained);
    assert!(
        workspace
            .0
            .join(&interrupted.journal_relative_path)
            .exists()
    );
    assert_eq!(
        fs::read_to_string(workspace.0.join("first.txt"))
            .expect("blocked cleanup must preserve the applied first target"),
        "first-new"
    );
    assert_eq!(
        fs::read_to_string(workspace.0.join("second.txt"))
            .expect("blocked cleanup must preserve the applied second target"),
        "second-new"
    );

    fs::remove_file(batch_directory.join("unexpected.txt"))
        .expect("unexpected cleanup artifact must be resolved");
    let recovered = engine
        .recover_apply_batch(&interrupted.journal_relative_path)
        .expect("resolved public cleanup recovery must return a success receipt");
    assert_eq!(recovered.outcome, BatchApplyOutcomeReceipt::Applied);
    assert!(!recovered.journal_retained);
    assert_eq!(recovered.resolved_plan_digest, "5".repeat(64));
    assert_eq!(recovered.candidate_digest, "6".repeat(64));
    assert!(recovered.orchestration_failures.is_empty());
    assert!(!recovered.failure_history.is_empty());
    assert_batch_completion_receipt(
        &workspace.0,
        recovered.completion_receipt_relative_path.as_ref(),
    );
}

#[test]
fn apply_requires_the_workspace_mutation_lock_and_preserves_the_target() {
    let workspace = TemporaryWorkspace::create();
    let engine = HarnessEngine::open(repository_root(), &workspace.0)
        .expect("temporary workspace must open");
    let request = HarnessRequest {
        action: HarnessAction::DocumentWrite,
        owner: DataOwner::Personal,
        targets: vec!["note.md".to_owned()],
        objective: "write a note".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = engine.resolve(&request).expect("request must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    let record = complete_execution_record(&engine, &resolved, &prepared, &[], "locked");
    fs::write(workspace.0.join(".llm-context-vault-harness.lock"), "busy")
        .expect("test lock must be created");
    assert!(matches!(
        engine.apply_validated_execution(&resolved, &prepared, &[], &record),
        Err(HarnessError::FileWrite { .. } | HarnessError::BatchApply { .. })
    ));
    assert!(!workspace.0.join("note.md").exists());
    fs::remove_file(workspace.0.join(".llm-context-vault-harness.lock"))
        .expect("test lock must be removed");
}

#[test]
fn final_workspace_and_context_evidence_share_the_workspace_mutation_lock() {
    let (workspace, engine) = external_engine();
    let acquisition = repository::WorkspaceMutationLock::acquire_reported(&workspace.0)
        .expect("test must acquire the workspace mutation lock");
    let mutation_lock = acquisition
        .lock
        .expect("reported acquisition must yield the test lock");
    assert!(
        engine.with_workspace_finalization_lock(|| Ok(())).is_err(),
        "finalization evidence must not run beside another run's source mutation"
    );
    let release = mutation_lock.release();
    assert!(release.explicit_unlink_committed);
    assert_eq!(
        release.verified_final_absence,
        repository::LockLifecycleConfirmation::Confirmed
    );
    engine
        .with_workspace_finalization_lock(|| Ok(()))
        .expect("finalization evidence may proceed after the shared lock is released");
}

#[test]
fn vault_curation_rejects_cross_owner_target() {
    let request = HarnessRequest {
        action: HarnessAction::VaultCuration,
        owner: DataOwner::Company {
            company: "cluml".to_owned(),
        },
        targets: vec!["vault/personal/knowledge/example.md".to_owned()],
        objective: "curate company knowledge".to_owned(),
        curation_kind: Some(CurationKind::Knowledge),
        explicit_user_confirmation_reported: true,
        curation_sources: vec!["vault/work/cluml/index.md".to_owned()],
        delete_targets: Vec::new(),
    };
    assert!(matches!(
        engine().resolve(&request),
        Err(HarnessError::InvalidRequest(_))
    ));
}

#[test]
fn non_file_ideation_does_not_require_a_target() {
    let request = HarnessRequest {
        action: HarnessAction::Ideation,
        owner: DataOwner::Personal,
        targets: Vec::new(),
        objective: "compare new project ideas".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = engine().resolve(&request).expect("ideation must resolve");
    assert!(resolved.plan.targets.is_empty());
    assert!(matches!(
        HarnessEngine::submission_template(&resolved, &prepared_run(&resolved))
            .expect("template must resolve")
            .artifact,
        SubmissionArtifact::Analysis { .. }
    ));
    {
        let engine = engine();
        let plan = HarnessPlan::from_resolved(
            resolved,
            &engine.workspace_root,
            harness_lifecycle_limits(),
        )
        .expect("targetless ideation must have a plan-level workspace identity in current");
        assert!(plan.frozen_targets.targets.is_empty());
        assert!(!plan.workspace_root_identity.is_empty());
    }
}

#[test]
fn solo_mvp_ideation_binds_exact_registry_policy_and_verification_contract() {
    for current_engine in [engine(), strict_engine()] {
        let request = HarnessRequest {
            action: HarnessAction::Ideation,
            owner: DataOwner::Personal,
            targets: vec![SOLO_MVP_IDEA_REGISTRY_PATH.to_owned()],
            objective: "compare one-person MVP candidates".to_owned(),
            curation_kind: None,
            explicit_user_confirmation_reported: false,
            curation_sources: Vec::new(),
            delete_targets: Vec::new(),
        };
        let resolved = current_engine
            .resolve_with_intent(&request, HarnessIntent::SoloMvpIdeation)
            .expect("explicit solo MVP ideation must resolve");

        assert_eq!(resolved.plan.intent, HarnessIntent::SoloMvpIdeation);
        assert!(
            resolved
                .plan
                .required_policies
                .iter()
                .any(|policy| policy.id == "solo-mvp-idea-discovery")
        );
        for binding in &resolved.plan.role_policy_bindings {
            assert!(
                binding
                    .policies
                    .iter()
                    .any(|policy| policy.id == "solo-mvp-idea-discovery"),
                "{:?} must bind the canonical solo MVP policy",
                binding.role
            );
        }
        for unit in [
            VerificationUnit::IdeaNotPromotedToDecision,
            VerificationUnit::EvidenceAndUncertainty,
            VerificationUnit::SoloMvpIdeationContract,
        ] {
            assert!(
                resolved
                    .plan
                    .verification_requirements
                    .iter()
                    .any(|requirement| requirement.unit == unit),
                "solo MVP ideation must require {}",
                unit.as_str()
            );
        }

        let prepared = prepared_run_with(&current_engine, &resolved);
        for bundle in &prepared.role_bundles {
            assert!(bundle.bound_documents().any(|document| {
                document.source == HarnessBoundDocumentSource::Target
                    && document.relative_path == SOLO_MVP_IDEA_REGISTRY_PATH
            }));
            assert!(bundle.bound_documents().any(|document| {
                document.source == HarnessBoundDocumentSource::Policy
                    && document.relative_path
                        == "vault/personal/decisions/solo-mvp-idea-discovery.md"
            }));
        }
    }
}

#[test]
fn solo_mvp_candidate_analysis_delivers_registry_and_explicit_candidate_to_every_role() {
    let candidate_path =
        "vault/personal/projects/ideas/solo-founder-validation-platform.md".to_owned();
    let request = HarnessRequest {
        action: HarnessAction::Ideation,
        owner: DataOwner::Personal,
        targets: vec![
            SOLO_MVP_IDEA_REGISTRY_PATH.to_owned(),
            candidate_path.clone(),
        ],
        objective: "inspect the explicitly selected solo founder validation candidate".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let current_engine = strict_engine();
    let resolved = current_engine
        .resolve_with_intent(&request, HarnessIntent::SoloMvpIdeation)
        .expect("explicit candidate analysis must resolve");
    let prepared = prepared_run_with(&current_engine, &resolved);

    for bundle in &prepared.role_bundles {
        for path in [SOLO_MVP_IDEA_REGISTRY_PATH, candidate_path.as_str()] {
            assert!(bundle.bound_documents().any(|document| {
                document.source == HarnessBoundDocumentSource::Target
                    && document.relative_path == path
            }));
        }
    }
}

#[test]
fn solo_mvp_ideation_intent_rejects_wrong_action_owner_scope_and_registry() {
    let valid = HarnessRequest {
        action: HarnessAction::Ideation,
        owner: DataOwner::Personal,
        targets: vec![SOLO_MVP_IDEA_REGISTRY_PATH.to_owned()],
        objective: "compare one-person MVP candidates".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };

    for request in [
        HarnessRequest {
            action: HarnessAction::Design,
            ..valid.clone()
        },
        HarnessRequest {
            owner: DataOwner::PersonalBusiness,
            ..valid.clone()
        },
        HarnessRequest {
            owner: DataOwner::PersonalProject {
                project: "ideas".to_owned(),
            },
            ..valid.clone()
        },
        HarnessRequest {
            owner: DataOwner::Company {
                company: "cluml".to_owned(),
            },
            ..valid.clone()
        },
        HarnessRequest {
            targets: Vec::new(),
            ..valid.clone()
        },
    ] {
        assert!(matches!(
            validate_intent(&request, HarnessIntent::SoloMvpIdeation, &[]),
            Err(HarnessError::InvalidRequest(_))
        ));
    }
}

#[test]
fn objective_keywords_do_not_infer_the_solo_mvp_contract() {
    let request = HarnessRequest {
        action: HarnessAction::Ideation,
        owner: DataOwner::Personal,
        targets: Vec::new(),
        objective: "compare MVP cost distribution and TikTok acquisition".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = engine()
        .resolve(&request)
        .expect("general ideation with domain keywords must resolve");

    assert_eq!(resolved.plan.intent, HarnessIntent::General);
    assert!(
        resolved
            .plan
            .required_policies
            .iter()
            .all(|policy| policy.id != "solo-mvp-idea-discovery")
    );
    assert!(
        resolved
            .plan
            .verification_requirements
            .iter()
            .all(|requirement| requirement.unit != VerificationUnit::SoloMvpIdeationContract)
    );
}

#[test]
fn personal_idea_curation_binds_registry_policy_and_existing_curation_checks() {
    for current_engine in [engine(), strict_engine()] {
        let request = HarnessRequest {
            action: HarnessAction::VaultCuration,
            owner: DataOwner::Personal,
            targets: vec![
                "vault/personal/projects/ideas/solo-founder-validation-platform.md".to_owned(),
            ],
            objective: "update a solo MVP candidate".to_owned(),
            curation_kind: Some(CurationKind::Idea),
            explicit_user_confirmation_reported: true,
            curation_sources: vec![SOLO_MVP_IDEA_REGISTRY_PATH.to_owned()],
            delete_targets: Vec::new(),
        };
        let resolved = current_engine
            .resolve(&request)
            .expect("personal Idea curation with the registry source must resolve");

        assert_eq!(resolved.plan.intent, HarnessIntent::General);
        assert_eq!(
            planned_roles(&resolved.plan),
            vec![
                HarnessRole::Writer,
                HarnessRole::Verifier,
                HarnessRole::Reviewer,
            ]
        );
        for binding in &resolved.plan.role_policy_bindings {
            assert!(
                binding
                    .policies
                    .iter()
                    .any(|policy| policy.id == "solo-mvp-idea-discovery"),
                "{:?} must bind the canonical solo MVP policy",
                binding.role
            );
        }
        for unit in [
            VerificationUnit::CurationProvenance,
            VerificationUnit::SourceAndOntologyBoundary,
            VerificationUnit::IdeaNotPromotedToDecision,
            VerificationUnit::EvidenceAndUncertainty,
            VerificationUnit::SoloMvpIdeationContract,
        ] {
            assert!(
                resolved
                    .plan
                    .verification_requirements
                    .iter()
                    .any(|requirement| requirement.unit == unit),
                "personal Idea curation must require {}",
                unit.as_str()
            );
        }
        assert!(
            resolved
                .plan
                .verification_requirements
                .iter()
                .any(|requirement| {
                    requirement.unit == VerificationUnit::EvidenceAndUncertainty
                        && requirement.owner
                            == VerificationOwner::Role {
                                role: HarnessRole::Verifier,
                            }
                })
        );
        assert!(
            resolved
                .plan
                .verification_requirements
                .iter()
                .any(|requirement| {
                    requirement.unit == VerificationUnit::IdeaNotPromotedToDecision
                        && requirement.owner
                            == VerificationOwner::Role {
                                role: HarnessRole::Reviewer,
                            }
                })
        );
        assert_eq!(
            resolved.plan.curation_sources[0].repository_relative_path,
            SOLO_MVP_IDEA_REGISTRY_PATH
        );
        let prepared = prepared_run_with(&current_engine, &resolved);
        for bundle in &prepared.role_bundles {
            assert!(bundle.bound_documents().any(|document| {
                document.source == HarnessBoundDocumentSource::Curation
                    && document.relative_path == SOLO_MVP_IDEA_REGISTRY_PATH
            }));
            assert!(bundle.bound_documents().any(|document| {
                document.source == HarnessBoundDocumentSource::Policy
                    && document.relative_path
                        == "vault/personal/decisions/solo-mvp-idea-discovery.md"
            }));
        }
    }
}

fn prepare_current_harness_run(
    engine: &HarnessEngine,
    workspace_root: &Path,
    resolved: &ResolvedHarnessRequest,
) -> (PreparedHarnessRun, Vec<u8>) {
    let plan =
        HarnessPlan::from_resolved(resolved.clone(), workspace_root, harness_lifecycle_limits())
            .expect("personal Idea curation plan must freeze");
    let capabilities = harness_capabilities(&plan);
    let tool_plan = harness_tool_plan(&plan);
    let raw_plan = serde_json::to_vec(&plan).expect("Harness plan must serialize");
    let raw_tool_plan = tool_plan
        .as_ref()
        .map(|value| serde_json::to_vec(value).expect("tool plan must serialize"));
    let prepared = PreparedHarnessRun::prepare(
        engine,
        &raw_plan,
        plan,
        capabilities,
        raw_tool_plan.as_deref(),
        tool_plan,
    )
    .expect("personal Idea curation run must prepare");
    let raw_prepared = serde_json::to_vec(&prepared).expect("prepared run must serialize");
    (prepared, raw_prepared)
}

fn execute_current_harness_to_validation(
    engine: &HarnessEngine,
    resolved: &ResolvedHarnessRequest,
    prepared: &PreparedHarnessRun,
    raw_prepared: &[u8],
    run_identifier: &str,
) -> HarnessExecutionRecord {
    let mut execution =
        HarnessExecutionRecord::begin(engine, raw_prepared, prepared, run_identifier.to_owned())
            .expect("personal Idea curation execution must begin");
    loop {
        if let Some(invocation) = execution.ready_role_invocations.first().cloned() {
            execution = execution
                .advance(
                    engine,
                    raw_prepared,
                    prepared,
                    completed_role_event(
                        resolved,
                        &prepared.role_run,
                        &invocation,
                        Vec::new(),
                        run_identifier,
                    ),
                )
                .expect("issued role result must advance");
            continue;
        }
        if let Some(invocation) = execution.ready_tool_invocation.clone() {
            let tool_plan = prepared
                .accepted_tool_plan
                .as_ref()
                .expect("issued tool invocation must have an exact tool plan");
            execution = execution
                .advance(
                    engine,
                    raw_prepared,
                    prepared,
                    passed_exact_tool_event(&invocation, tool_plan),
                )
                .expect("issued tool evidence must advance");
            continue;
        }
        break;
    }
    let exact_tool_evidence = prepared
        .accepted_tool_plan
        .as_ref()
        .map(harness_tool_evidence);
    execution = execution
        .evaluate(engine, raw_prepared, prepared, exact_tool_evidence.as_ref())
        .expect("personal Idea curation must evaluate");
    execution
        .validate_evaluation(engine, raw_prepared, prepared)
        .expect("accepted personal Idea curation must validate")
}

fn assert_personal_idea_curation_contract(
    resolved: &ResolvedHarnessRequest,
    prepared: &PreparedHarnessRun,
) {
    assert_eq!(
        planned_roles(&resolved.plan),
        vec![
            HarnessRole::Writer,
            HarnessRole::Verifier,
            HarnessRole::Reviewer,
        ]
    );
    for unit in [
        VerificationUnit::CurationProvenance,
        VerificationUnit::SourceAndOntologyBoundary,
        VerificationUnit::IdeaNotPromotedToDecision,
        VerificationUnit::EvidenceAndUncertainty,
        VerificationUnit::SoloMvpIdeationContract,
    ] {
        assert!(
            resolved
                .plan
                .verification_requirements
                .iter()
                .any(|requirement| requirement.unit == unit),
            "personal Idea curation must preserve {}",
            unit.as_str()
        );
    }
    assert!(
        resolved
            .plan
            .verification_requirements
            .iter()
            .all(|requirement| requirement.owner != VerificationOwner::Tool)
    );
    assert!(prepared.accepted_tool_plan.is_none());
    for bundle in &prepared.role_run.role_bundles {
        for (source, path) in [
            (
                HarnessBoundDocumentSource::Policy,
                AI_COLLABORATION_VALUES_PATH,
            ),
            (
                HarnessBoundDocumentSource::Policy,
                "vault/personal/decisions/solo-mvp-idea-discovery.md",
            ),
            (
                HarnessBoundDocumentSource::Curation,
                SOLO_MVP_IDEA_REGISTRY_PATH,
            ),
        ] {
            assert!(
                bundle.bound_documents().any(|document| {
                    document.source == source && document.relative_path == path
                })
            );
        }
    }
}

fn assert_personal_idea_curation_full_lifecycle(profile: HarnessExecutionProfile) {
    let repository = TemporaryWorkspace::create();
    copy_directory(
        &repository_root().join("vault"),
        &repository.0.join("vault"),
    );
    fs::create_dir(repository.0.join("src")).expect("tool cwd fixture must exist");
    let router = HarnessRouter::with_execution_profile(2, profile)
        .expect("execution profile must build a router");
    let engine = HarnessEngine::with_router(&repository.0, &repository.0, router)
        .expect("isolated personal Idea curation engine must open");
    let target = "vault/personal/projects/ideas/solo-founder-validation-platform.md";
    let request = HarnessRequest {
        action: HarnessAction::VaultCuration,
        owner: DataOwner::Personal,
        targets: vec![target.to_owned()],
        objective: "update a solo MVP candidate through the current lifecycle".to_owned(),
        curation_kind: Some(CurationKind::Idea),
        explicit_user_confirmation_reported: true,
        curation_sources: vec![SOLO_MVP_IDEA_REGISTRY_PATH.to_owned()],
        delete_targets: Vec::new(),
    };
    let resolved = engine
        .resolve(&request)
        .expect("personal Idea curation must resolve");
    let (prepared, raw_prepared) = prepare_current_harness_run(&engine, &repository.0, &resolved);
    assert_personal_idea_curation_contract(&resolved, &prepared);
    let profile_name = match profile {
        HarnessExecutionProfile::Standard => "standard",
        HarnessExecutionProfile::Strict => "strict",
    };
    let execution = execute_current_harness_to_validation(
        &engine,
        &resolved,
        &prepared,
        &raw_prepared,
        &format!("run-personal-idea-curation-{profile_name}"),
    );
    let evaluation = execution
        .evaluation
        .as_ref()
        .expect("validated execution must preserve its evaluation");
    assert!(execution.ready_tool_invocation.is_none());
    assert!(execution.role_execution.tool_evidence.is_none());
    assert!(execution.exact_tool_evidence.is_none());
    assert_eq!(evaluation.subject_status, SubjectStatus::Accepted);
    assert_eq!(
        evaluation.completion_state,
        HarnessCompletionState::ValidatedPendingApply
    );
    let (finalized, receipt) = execution
        .finalize(&engine, &raw_prepared, &prepared, None)
        .expect("validated personal Idea curation must finalize");
    assert_eq!(finalized.state, HarnessExecutionState::Finalized);
    assert_eq!(
        receipt.applied_batch.completion_state,
        HarnessCompletionState::Applied
    );
    assert!(receipt.context_commit_receipt.is_none());
    assert_eq!(
        receipt.final_workspace.content_states.get(target),
        Some(&Some(byte_digest(b"candidate")))
    );
    receipt
        .revalidate_from_disk_and_core(&engine, &raw_prepared, &prepared, &finalized)
        .expect("finalized personal Idea curation evidence must revalidate");
    assert_eq!(
        fs::read_to_string(repository.0.join(target)).expect("applied target must be readable"),
        "candidate"
    );
    let next = crate::read_context(
        repository.0.join("vault"),
        crate::Scope::Personal,
        &[PathBuf::from(
            "projects/ideas/solo-founder-validation-platform.md",
        )],
    )
    .expect("next exact Core read must return the saved candidate");
    assert!(next.contains("candidate"));
}

#[test]
fn personal_idea_curation_completes_the_current_standard_and_strict_lifecycle() {
    for profile in [
        HarnessExecutionProfile::Standard,
        HarnessExecutionProfile::Strict,
    ] {
        assert_personal_idea_curation_full_lifecycle(profile);
    }
}

#[test]
fn personal_idea_curation_requires_registry_as_target_or_source() {
    let mut request = HarnessRequest {
        action: HarnessAction::VaultCuration,
        owner: DataOwner::Personal,
        targets: vec![
            "vault/personal/projects/ideas/solo-founder-validation-platform.md".to_owned(),
        ],
        objective: "update a solo MVP candidate".to_owned(),
        curation_kind: Some(CurationKind::Idea),
        explicit_user_confirmation_reported: true,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    assert!(matches!(
        engine().resolve(&request),
        Err(HarnessError::InvalidRequest(message))
            if message.contains(SOLO_MVP_IDEA_REGISTRY_PATH)
    ));

    request.targets = vec![SOLO_MVP_IDEA_REGISTRY_PATH.to_owned()];
    engine()
        .resolve(&request)
        .expect("the exact registry target must satisfy its own fact binding");
}

fn assert_non_idea_curation_contract(
    resolved: &ResolvedHarnessRequest,
    expected_roles: &[HarnessRole],
) {
    assert_eq!(planned_roles(&resolved.plan), expected_roles);
    assert_eq!(resolved.plan.intent, HarnessIntent::General);
    assert!(
        resolved
            .plan
            .required_policies
            .iter()
            .all(|policy| policy.id != "solo-mvp-idea-discovery")
    );
    for unit in [
        VerificationUnit::SoloMvpIdeationContract,
        VerificationUnit::IdeaNotPromotedToDecision,
        VerificationUnit::EvidenceAndUncertainty,
    ] {
        assert!(
            resolved
                .plan
                .verification_requirements
                .iter()
                .all(|requirement| requirement.unit != unit),
            "non-Idea curation must not require {}",
            unit.as_str()
        );
    }
}

#[test]
fn non_idea_curation_keeps_the_general_contract() {
    let request = HarnessRequest {
        action: HarnessAction::VaultCuration,
        owner: DataOwner::Personal,
        targets: vec!["vault/personal/knowledge/example.md".to_owned()],
        objective: "curate personal knowledge".to_owned(),
        curation_kind: Some(CurationKind::Knowledge),
        explicit_user_confirmation_reported: true,
        curation_sources: vec!["vault/personal/knowledge/index.md".to_owned()],
        delete_targets: Vec::new(),
    };
    let resolved = engine()
        .resolve(&request)
        .expect("non-Idea curation must retain the general contract");
    let strict_resolved = strict_engine()
        .resolve(&request)
        .expect("strict non-Idea curation must retain the general contract");

    assert_non_idea_curation_contract(&resolved, &[HarnessRole::Writer, HarnessRole::Reviewer]);
    assert_non_idea_curation_contract(
        &strict_resolved,
        &[
            HarnessRole::Writer,
            HarnessRole::Verifier,
            HarnessRole::Reviewer,
        ],
    );
    assert_eq!(
        resolved.plan.curation_sources,
        vec![SourceBinding {
            repository_relative_path: "vault/personal/knowledge/index.md".to_owned(),
            content_digest: resolved.plan.curation_sources[0].content_digest.clone(),
        }],
        "general curation plan must bind the supplied source"
    );
    let prepared = prepared_run(&resolved);
    assert!(
        prepared.role_bundles.iter().all(|bundle| {
            bundle.bound_documents().any(|document| {
                document.source == HarnessBoundDocumentSource::Curation
                    && document.relative_path == "vault/personal/knowledge/index.md"
                    && document.matched_terms.is_empty()
                    && document.start_line == 1
            })
        }),
        "every general curation role must receive the full source: {:?}",
        prepared
            .role_bundles
            .iter()
            .map(|bundle| (
                bundle.role,
                bundle
                    .bound_documents()
                    .map(|document| (document.source, document.relative_path.clone()))
                    .collect::<Vec<_>>()
            ))
            .collect::<Vec<_>>()
    );

    let first_bundle = &prepared.role_bundles[0];
    let curation = first_bundle
        .bound_documents()
        .find(|document| document.source == HarnessBoundDocumentSource::Curation)
        .expect("general curation source must be materialized");
    let bytes_before_curation = first_bundle
        .bound_documents()
        .take_while(|document| document.source != HarnessBoundDocumentSource::Curation)
        .map(|document| document.content.len())
        .sum::<usize>();
    let mut capabilities = complete_runtime_capabilities();
    capabilities.max_role_bundle_bytes = bytes_before_curation + curation.content.len() - 1;
    assert!(matches!(
        engine().prepare(&resolved, &capabilities, None),
        Err(HarnessError::UnsupportedRuntime(message))
            if message.contains("vault/personal/knowledge/index.md")
                && message.contains("remaining role bundle content budget")
    ));
}

#[test]
fn solo_mvp_ideation_intent_has_a_current_serialized_form() {
    assert_eq!(
        serde_json::to_value(HarnessIntent::SoloMvpIdeation)
            .expect("solo MVP intent must serialize"),
        serde_json::json!({"kind": "solo-mvp-ideation"})
    );
}

#[test]
fn journal_curation_is_direct_read_only_and_requires_confirmation() {
    let mut request = HarnessRequest {
        action: HarnessAction::VaultCuration,
        owner: DataOwner::Personal,
        targets: vec!["vault/personal/journal/2026-07-11.md".to_owned()],
        objective: "save a journal entry".to_owned(),
        curation_kind: Some(CurationKind::Journal),
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    assert!(matches!(
        engine().resolve(&request),
        Err(HarnessError::InvalidRequest(_))
    ));

    request.explicit_user_confirmation_reported = true;
    let resolved = engine()
        .resolve(&request)
        .expect("confirmed journal curation must resolve");
    assert!(
        resolved
            .plan
            .allowed_context_roots
            .iter()
            .any(|root| root.repository_relative_path == "vault/personal/journal")
    );
    assert!(
        !resolved
            .plan
            .denied_context_roots
            .iter()
            .any(|root| root.repository_relative_path == "vault/personal/journal")
    );
}

#[test]
fn target_digest_changes_when_target_content_changes() {
    let first = engine()
        .resolve(&profile_code_request())
        .expect("first plan must resolve");
    let mut request = profile_code_request();
    request.targets = vec!["crates/context-core/src/document.rs".to_owned()];
    let second = engine()
        .resolve(&request)
        .expect("second plan must resolve");
    assert_ne!(first.plan.targets, second.plan.targets);
    assert_ne!(first.resolved_plan_digest, second.resolved_plan_digest);
}

#[test]
fn paths_cannot_escape_workspace() {
    let mut request = profile_code_request();
    request.targets = vec!["../outside.rs".to_owned()];
    assert!(matches!(
        engine().resolve(&request),
        Err(HarnessError::InvalidRequest(_))
    ));
}

#[test]
fn normalized_request_produces_a_deterministic_plan() {
    let first = engine()
        .resolve(&profile_code_request())
        .expect("first resolution must succeed");
    let second = engine()
        .resolve(&profile_code_request())
        .expect("second resolution must succeed");
    assert_eq!(first.resolved_plan_digest, second.resolved_plan_digest);
    assert_eq!(first.plan, second.plan);
}

#[test]
fn target_order_does_not_change_the_plan() {
    let mut first_request = profile_code_request();
    first_request.action = HarnessAction::CodeReview;
    first_request
        .targets
        .push("crates/context-core/src/document.rs".to_owned());
    let mut second_request = first_request.clone();
    second_request.targets.reverse();

    let first = engine()
        .resolve(&first_request)
        .expect("first multi-target review request must resolve");
    let second = engine()
        .resolve(&second_request)
        .expect("second multi-target review request must resolve");
    assert_eq!(first.resolved_plan_digest, second.resolved_plan_digest);
    assert_eq!(first.request.targets, second.request.targets);
}

#[test]
fn file_targets_cannot_be_ancestors_of_other_file_targets() {
    let (_workspace, engine) = external_engine();
    let request = HarnessRequest {
        action: HarnessAction::CodeWrite,
        owner: DataOwner::PersonalProject {
            project: "coupler".to_owned(),
        },
        targets: vec!["generated".to_owned(), "generated/child.rs".to_owned()],
        objective: "create an impossible overlapping file set".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };

    assert!(matches!(
        engine.resolve(&request),
        Err(HarnessError::InvalidRequest(message))
            if message.contains("cannot be an ancestor")
    ));
}

#[test]
fn review_requires_every_target_to_exist() {
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Personal,
        targets: vec!["does-not-exist.md".to_owned()],
        objective: "review".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    assert!(matches!(
        engine().resolve(&request),
        Err(HarnessError::InvalidRequest(_))
    ));
}

#[test]
fn advisory_submission_requires_independent_contexts_and_all_checks() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let submission = complete_submission(&resolved);
    let validated = engine()
        .validate_submission(&resolved, &prepared_run(&resolved), &submission)
        .expect("complete advisory submission must validate");
    assert_eq!(validated.assurance, ExecutionAssurance::Advisory);

    let mut incomplete = submission;
    incomplete.artifact = SubmissionArtifact::Changes {
        changes: Vec::new(),
    };
    assert!(matches!(
        engine().validate_submission(&resolved, &prepared_run(&resolved), &incomplete),
        Err(HarnessError::InvalidSubmission(_))
    ));
}

#[test]
fn submission_rejects_extra_verification_checks() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let mut submission = complete_submission(&resolved);
    submission.verification_checks.push(VerificationCheck {
        id: "unexpected-check".to_owned(),
        passed: true,
        detail: "unexpected".to_owned(),
    });

    assert!(matches!(
        engine().validate_submission(&resolved, &prepared_run(&resolved), &submission),
        Err(HarnessError::InvalidSubmission(message))
            if message == "verification checks must exactly match the planned check list"
    ));
}

#[test]
fn submission_rejects_missing_verification_checks() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let mut submission = complete_submission(&resolved);
    submission.verification_checks.pop();

    assert!(matches!(
        engine().validate_submission(&resolved, &prepared_run(&resolved), &submission),
        Err(HarnessError::InvalidSubmission(message))
            if message == "verification checks must exactly match the planned check list"
    ));
}

#[test]
fn submission_rejects_reordered_verification_checks() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let mut submission = complete_submission(&resolved);
    submission.verification_checks.swap(0, 1);

    assert!(matches!(
        engine().validate_submission(&resolved, &prepared_run(&resolved), &submission),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("does not match planned check")
    ));
}

#[test]
fn submission_cannot_claim_enforced_assurance() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let mut submission = complete_submission(&resolved);
    submission.assurance = ExecutionAssurance::Enforced;
    assert!(matches!(
        engine().validate_submission(&resolved, &prepared_run(&resolved), &submission),
        Err(HarnessError::InvalidSubmission(_))
    ));
}

#[test]
fn submission_revision_cannot_exceed_the_plan_limit() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let mut submission = complete_submission(&resolved);
    submission.revision = resolved.plan.max_revisions + 1;
    assert!(matches!(
        engine().validate_submission(&resolved, &prepared_run(&resolved), &submission),
        Err(HarnessError::InvalidSubmission(_))
    ));
}

#[test]
fn non_write_plans_reject_revision_history_and_revised_candidates() {
    let (_workspace, engine) = external_engine();
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::Personal,
        targets: vec!["README.md".to_owned()],
        objective: "review a resume".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = engine
        .resolve_with_career_surface(&request, CareerOutputSurface::Resume)
        .expect("resume review must resolve");
    let prepared = prepared_run_with(&engine, &resolved);
    assert_eq!(resolved.plan.max_revisions, 0);
    let mut invalid_plan = resolved.plan.clone();
    invalid_plan.max_revisions = 1;
    assert!(matches!(
        invalid_plan.validate(),
        Err(HarnessError::InvalidPlan(message))
            if message == "non-write plans must not permit candidate revisions"
    ));

    let mut investigation_request = request.clone();
    investigation_request.action = HarnessAction::Investigation;
    investigation_request.objective = "investigate the target without editing it".to_owned();
    let investigation = engine
        .resolve(&investigation_request)
        .expect("investigation must resolve");
    assert_eq!(investigation.plan.max_revisions, 0);
    let mut invalid_investigation = investigation.plan;
    invalid_investigation.max_revisions = 1;
    assert!(matches!(
        invalid_investigation.validate(),
        Err(HarnessError::InvalidPlan(message))
            if message == "non-write plans must not permit candidate revisions"
    ));

    let mut first_submission = complete_submission_with(&engine, &resolved);
    first_submission.findings = vec![Finding {
        severity: "P1".to_owned(),
        message: "report this review finding without revising the target".to_owned(),
    }];
    let first_result = engine
        .evaluate_submission_with_history(&resolved, &prepared, &first_submission, &[])
        .expect("an initial review result may contain findings");
    assert!(!first_result.accepted);

    let mut revised_submission = complete_submission_with(&engine, &resolved);
    revised_submission.revision = 1;
    revised_submission.previous_candidate_digest = Some(first_result.candidate_digest.clone());
    revised_submission.revision_feedback = first_result.findings.clone();
    assert!(matches!(
        engine.evaluate_submission_with_history(&resolved, &prepared, &revised_submission, &[]),
        Err(HarnessError::InvalidSubmission(message))
            if message == "non-write actions do not accept revision history or revised candidates"
    ));

    let initial_submission = complete_submission_with(&engine, &resolved);
    assert!(matches!(
        engine.evaluate_submission_with_history(
            &resolved,
            &prepared,
            &initial_submission,
            std::slice::from_ref(&first_result)
        ),
        Err(HarnessError::InvalidSubmission(message))
            if message == "non-write actions do not accept revision history or revised candidates"
    ));

    assert!(matches!(
        engine.evaluate_submission_with_history(
            &resolved,
            &prepared,
            &revised_submission,
            &[first_result]
        ),
        Err(HarnessError::InvalidSubmission(message))
            if message == "non-write actions do not accept revision history or revised candidates"
    ));
}

#[test]
fn file_operation_must_match_the_starting_target_state() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let mut submission = complete_submission(&resolved);
    submission.artifact = SubmissionArtifact::Changes {
        changes: vec![FileChange::Create {
            path: "crates/context-core/src/lib.rs".to_owned(),
            content: "replacement".to_owned(),
        }],
    };
    assert!(matches!(
        engine().validate_submission(&resolved, &prepared_run(&resolved), &submission),
        Err(HarnessError::InvalidSubmission(_))
    ));
}

#[test]
fn delete_operation_requires_an_explicit_delete_target() {
    let mut request = profile_code_request();
    request.delete_targets = request.targets.clone();
    let resolved = engine()
        .resolve(&request)
        .expect("delete request must resolve");
    assert_eq!(resolved.plan.targets[0].operation, TargetOperation::Delete);
    assert!(matches!(
        HarnessEngine::submission_template(&resolved, &prepared_run(&resolved))
            .expect("delete template must resolve")
            .artifact,
        SubmissionArtifact::Changes {
            changes
        } if matches!(changes.as_slice(), [FileChange::Delete { .. }])
    ));
}

#[test]
fn knowledge_curation_requires_sources_and_user_confirmation() {
    let request = HarnessRequest {
        action: HarnessAction::VaultCuration,
        owner: DataOwner::Personal,
        targets: vec!["vault/personal/knowledge/new-fact.md".to_owned()],
        objective: "save verified knowledge".to_owned(),
        curation_kind: Some(CurationKind::Knowledge),
        explicit_user_confirmation_reported: true,
        curation_sources: vec!["vault/personal/index.md".to_owned()],
        delete_targets: Vec::new(),
    };
    let resolved = engine().resolve(&request).expect("request must resolve");
    let mut submission = complete_submission(&resolved);
    let SubmissionArtifact::Curation { entries, .. } = &mut submission.artifact else {
        panic!("curation request must produce a curation artifact");
    };
    entries[0].source_references.clear();
    assert!(matches!(
        engine().validate_submission(&resolved, &prepared_run(&resolved), &submission),
        Err(HarnessError::InvalidSubmission(_))
    ));
}

#[test]
fn curation_kind_cannot_write_to_another_semantic_category() {
    let request = HarnessRequest {
        action: HarnessAction::VaultCuration,
        owner: DataOwner::Personal,
        targets: vec!["vault/personal/decisions/not-knowledge.md".to_owned()],
        objective: "save verified knowledge".to_owned(),
        curation_kind: Some(CurationKind::Knowledge),
        explicit_user_confirmation_reported: true,
        curation_sources: vec!["vault/personal/index.md".to_owned()],
        delete_targets: Vec::new(),
    };
    assert!(matches!(
        engine().resolve(&request),
        Err(HarnessError::InvalidRequest(_))
    ));
}

#[test]
fn curation_source_must_stay_in_the_owner_context() {
    let request = HarnessRequest {
        action: HarnessAction::VaultCuration,
        owner: DataOwner::Personal,
        targets: vec!["vault/personal/knowledge/new.md".to_owned()],
        objective: "save verified knowledge".to_owned(),
        curation_kind: Some(CurationKind::Knowledge),
        explicit_user_confirmation_reported: true,
        curation_sources: vec!["vault/work/cluml/index.md".to_owned()],
        delete_targets: Vec::new(),
    };
    assert!(matches!(
        engine().resolve(&request),
        Err(HarnessError::InvalidRequest(_))
    ));
}

#[test]
fn submitted_curation_source_digest_must_match_the_plan() {
    let request = HarnessRequest {
        action: HarnessAction::VaultCuration,
        owner: DataOwner::Personal,
        targets: vec!["vault/personal/knowledge/new.md".to_owned()],
        objective: "save verified knowledge".to_owned(),
        curation_kind: Some(CurationKind::Knowledge),
        explicit_user_confirmation_reported: true,
        curation_sources: vec!["vault/personal/index.md".to_owned()],
        delete_targets: Vec::new(),
    };
    let resolved = engine().resolve(&request).expect("request must resolve");
    let mut submission = complete_submission(&resolved);
    let SubmissionArtifact::Curation { entries, .. } = &mut submission.artifact else {
        panic!("curation request must produce a curation artifact");
    };
    entries[0].source_references[0].content_digest = "0".repeat(64);
    assert!(matches!(
        engine().validate_submission(&resolved, &prepared_run(&resolved), &submission),
        Err(HarnessError::InvalidSubmission(_))
    ));
}

#[test]
fn ontology_source_cannot_be_inside_the_ontology_projection() {
    let request = HarnessRequest {
        action: HarnessAction::VaultCuration,
        owner: DataOwner::Personal,
        targets: vec!["vault/personal/ontology/new.md".to_owned()],
        objective: "project canonical facts".to_owned(),
        curation_kind: Some(CurationKind::Ontology),
        explicit_user_confirmation_reported: true,
        curation_sources: vec!["vault/personal/ontology/schema.md".to_owned()],
        delete_targets: Vec::new(),
    };
    assert!(matches!(
        engine().resolve(&request),
        Err(HarnessError::InvalidRequest(_))
    ));
}

#[test]
fn ontology_curation_binds_exact_canonical_project_sources() {
    let request = HarnessRequest {
        action: HarnessAction::VaultCuration,
        owner: DataOwner::Personal,
        targets: vec!["vault/personal/ontology/new.md".to_owned()],
        objective: "project canonical project facts".to_owned(),
        curation_kind: Some(CurationKind::Ontology),
        explicit_user_confirmation_reported: true,
        curation_sources: vec!["vault/personal/projects/coupler.md".to_owned()],
        delete_targets: Vec::new(),
    };

    let resolved = engine().resolve(&request).expect("request must resolve");

    assert!(
        resolved
            .plan
            .allowed_context_roots
            .iter()
            .any(|root| { root.repository_relative_path == "vault/personal/projects/coupler.md" })
    );
    assert_eq!(
        resolved.plan.curation_sources[0].repository_relative_path,
        "vault/personal/projects/coupler.md"
    );
}

#[test]
fn later_revision_requires_a_bound_previous_candidate_and_feedback() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let prepared = prepared_run(&resolved);
    let first_result = rejected_evaluation(
        &engine(),
        &resolved,
        &prepared,
        "fix the rejected candidate",
    );
    assert!(!first_result.accepted);
    let mut submission = complete_submission(&resolved);
    submission.revision = 1;
    assert!(matches!(
        engine().validate_submission(&resolved, &prepared, &submission),
        Err(HarnessError::InvalidSubmission(_))
    ));

    submission.previous_candidate_digest = Some(first_result.candidate_digest.clone());
    submission.revision_feedback = first_result.findings.clone();
    assert!(matches!(
        engine().validate_submission_with_history(
            &resolved,
            &prepared,
            &submission,
            std::slice::from_ref(&first_result),
        ),
        Err(HarnessError::InvalidSubmission(message))
            if message == "revised submissions must use fresh role context IDs"
    ));
    fill_reported_role_contexts(&mut submission, &resolved.plan, "2");
    let validated = engine()
        .validate_submission_with_history(
            &resolved,
            &prepared,
            &submission,
            std::slice::from_ref(&first_result),
        )
        .expect("bound later revision must validate");
    assert_eq!(validated.revision, 1);
}

#[test]
fn revision_history_rejects_context_receipt_tampering() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let prepared = prepared_run(&resolved);
    let first_result = rejected_evaluation(
        &engine(),
        &resolved,
        &prepared,
        "fix the rejected candidate",
    );
    let mut missing_context_history = first_result.clone();
    missing_context_history.reported_role_contexts.pop();
    refresh_evaluation_receipt(&mut missing_context_history);
    let mut next = complete_submission(&resolved);
    next.revision = 1;
    next.previous_candidate_digest = Some(missing_context_history.candidate_digest.clone());
    next.revision_feedback = missing_context_history.findings.clone();
    assert!(matches!(
        engine().validate_submission_with_history(
            &resolved,
            &prepared,
            &next,
            &[missing_context_history]
        ),
        Err(HarnessError::InvalidSubmission(message))
            if message == "reported role context IDs must include every planned role"
    ));

    let mut duplicate_context_history = first_result.clone();
    duplicate_context_history.reported_role_contexts[1].context_id = duplicate_context_history
        .reported_role_contexts[0]
        .context_id
        .clone();
    refresh_evaluation_receipt(&mut duplicate_context_history);
    let mut next = complete_submission(&resolved);
    next.revision = 1;
    next.previous_candidate_digest = Some(duplicate_context_history.candidate_digest.clone());
    next.revision_feedback = duplicate_context_history.findings.clone();
    assert!(matches!(
        engine().validate_submission_with_history(
            &resolved,
            &prepared,
            &next,
            &[duplicate_context_history]
        ),
        Err(HarnessError::InvalidSubmission(message))
            if message == "reported role context IDs must be unique"
    ));

    let mut tampered_receipt_history = first_result.clone();
    tampered_receipt_history.reported_reviewer_context_id = "reviewer-tampered".to_owned();
    for role_context in &mut tampered_receipt_history.reported_role_contexts {
        if role_context.role == HarnessRole::Reviewer {
            role_context.context_id = "reviewer-tampered".to_owned();
        }
    }
    for lifecycle in &mut tampered_receipt_history.reported_role_lifecycles {
        if lifecycle.role == HarnessRole::Reviewer {
            lifecycle.context_id = "reviewer-tampered".to_owned();
        }
    }
    let mut next = complete_submission(&resolved);
    next.revision = 1;
    next.previous_candidate_digest = Some(tampered_receipt_history.candidate_digest.clone());
    next.revision_feedback = tampered_receipt_history.findings.clone();
    assert!(matches!(
        engine().validate_submission_with_history(
            &resolved,
            &prepared,
            &next,
            &[tampered_receipt_history]
        ),
        Err(HarnessError::InvalidSubmission(message))
            if message == "revision history validation receipt digest is invalid"
    ));
}

#[test]
fn revision_history_receipts_bind_roles_and_candidate_digest() {
    let strict_engine = strict_engine();
    let strict_resolved = strict_engine
        .resolve(&profile_code_request())
        .expect("strict request must resolve");
    let strict_prepared = prepared_run_with(&strict_engine, &strict_resolved);
    let strict_first_result = rejected_evaluation(
        &strict_engine,
        &strict_resolved,
        &strict_prepared,
        "fix the rejected strict candidate",
    );
    let mut tampered_role_only_history = strict_first_result.clone();
    let mut tampered_verifier_context = false;
    for role_context in &mut tampered_role_only_history.reported_role_contexts {
        if role_context.role == HarnessRole::Verifier {
            role_context.context_id = "verifier-tampered".to_owned();
            tampered_verifier_context = true;
        }
    }
    for lifecycle in &mut tampered_role_only_history.reported_role_lifecycles {
        if lifecycle.role == HarnessRole::Verifier {
            lifecycle.context_id = "verifier-tampered".to_owned();
        }
    }
    assert!(tampered_verifier_context);
    let mut next = complete_submission_with(&strict_engine, &strict_resolved);
    next.revision = 1;
    next.previous_candidate_digest = Some(tampered_role_only_history.candidate_digest.clone());
    next.revision_feedback = tampered_role_only_history.findings.clone();
    assert!(matches!(
        strict_engine.validate_submission_with_history(
            &strict_resolved,
            &strict_prepared,
            &next,
            &[tampered_role_only_history]
        ),
        Err(HarnessError::InvalidSubmission(message))
            if message == "revision history validation receipt digest is invalid"
    ));

    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let prepared = prepared_run(&resolved);
    let first_result = rejected_evaluation(
        &engine(),
        &resolved,
        &prepared,
        "fix the rejected candidate",
    );
    let mut tampered_history = first_result;
    tampered_history.candidate_digest = "0".repeat(64);
    let mut next = complete_submission(&resolved);
    next.revision = 1;
    next.previous_candidate_digest = Some(tampered_history.candidate_digest.clone());
    next.revision_feedback = tampered_history.findings.clone();
    assert!(matches!(
        engine().validate_submission_with_history(&resolved, &prepared, &next, &[tampered_history]),
        Err(HarnessError::InvalidSubmission(_))
    ));
}

#[test]
fn failed_verification_produces_a_revision_receipt() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let mut first_submission = complete_submission(&resolved);
    first_submission.verification_checks[0].passed = false;
    first_submission.verification_checks[0].detail = "test failed".to_owned();
    let first_result = engine()
        .evaluate_submission_with_history(
            &resolved,
            &prepared_run(&resolved),
            &first_submission,
            &[],
        )
        .expect("failed verification must still produce an evaluation receipt");
    assert!(!first_result.accepted);

    let mut revision = complete_submission(&resolved);
    revision.revision = 1;
    revision.previous_candidate_digest = Some(first_result.candidate_digest.clone());
    revision.revision_feedback = vec![Finding {
        severity: "verification".to_owned(),
        message: format!(
            "{}: {}",
            first_result.verification_checks[0].id, first_result.verification_checks[0].detail
        ),
    }];
    fill_reported_role_contexts(&mut revision, &resolved.plan, "2");
    assert!(
        engine()
            .validate_submission_with_history(
                &resolved,
                &prepared_run(&resolved),
                &revision,
                &[first_result],
            )
            .is_ok()
    );
}

#[test]
fn validation_receipt_binds_reviewer_and_checks() {
    let resolved = engine()
        .resolve(&profile_code_request())
        .expect("request must resolve");
    let first = complete_submission(&resolved);
    let mut second = first.clone();
    second.reported_reviewer_context_id = "reviewer-2".to_owned();
    for role_context in &mut second.reported_role_contexts {
        if role_context.role == HarnessRole::Reviewer {
            role_context.context_id = "reviewer-2".to_owned();
        }
    }
    for lifecycle in &mut second.reported_role_lifecycles {
        if lifecycle.role == HarnessRole::Reviewer {
            lifecycle.context_id = "reviewer-2".to_owned();
        }
    }
    let first = engine()
        .validate_submission(&resolved, &prepared_run(&resolved), &first)
        .expect("first submission must validate");
    let second = engine()
        .validate_submission(&resolved, &prepared_run(&resolved), &second)
        .expect("second submission must validate");
    assert_eq!(first.candidate_digest, second.candidate_digest);
    assert_ne!(
        first.validation_receipt_digest,
        second.validation_receipt_digest
    );
}

#[test]
fn harness_core_has_no_llm_product_routing() {
    let source = [
        include_str!("../harness.rs"),
        include_str!("repository.rs"),
        include_str!("execution.rs"),
    ]
    .join("\n")
    .to_ascii_lowercase();
    let forbidden = [
        ["cod", "ex"].concat(),
        ["clau", "de"].concat(),
        ["open", "ai"].concat(),
        ["g", "pt"].concat(),
        ["gem", "ini"].concat(),
    ];
    for product in forbidden {
        assert!(
            !source.contains(&product),
            "Harness Harness engine must not route on an LLM product name"
        );
    }
}

fn harness_lifecycle_limits() -> RoleLifecycleLimits {
    RoleLifecycleLimits {
        max_role_execution_millis: default_max_role_execution_millis(),
        max_role_grace_millis: default_max_role_grace_millis(),
        max_role_close_millis: 10_000,
        max_total_role_millis: default_max_role_execution_millis()
            + default_max_role_grace_millis()
            + 10_000,
    }
}

fn harness_capabilities(plan: &HarnessPlan) -> HarnessRuntimeCapabilities {
    let available_roles = plan.required_roles().into_iter().collect::<Vec<_>>();
    HarnessRuntimeCapabilities {
        version: HARNESS_SCHEMA_VERSION,
        max_concurrent_roles: plan.resolved_request.plan.required_concurrent_roles,
        available_roles,
        separate_contexts: true,
        file_reading: true,
        tool_execution: plan.requires_tool_capability(),
        deterministic_validation: true,
        max_role_bundle_bytes: default_max_role_bundle_bytes(),
        max_role_invocation_bytes: default_max_role_invocation_bytes(),
        lifecycle: plan.lifecycle.clone(),
    }
}

fn role_runtime_capabilities(capabilities: &HarnessRuntimeCapabilities) -> RoleRuntimeCapabilities {
    RoleRuntimeCapabilities {
        available_roles: capabilities.available_roles.clone(),
        max_concurrent_roles: capabilities.max_concurrent_roles,
        separate_contexts: capabilities.separate_contexts,
        file_reading: capabilities.file_reading,
        tool_execution: capabilities.tool_execution,
        max_role_bundle_bytes: capabilities.max_role_bundle_bytes,
        max_role_invocation_bytes: capabilities.max_role_invocation_bytes,
        max_role_execution_millis: capabilities.lifecycle.max_role_execution_millis,
        max_role_grace_millis: capabilities.lifecycle.max_role_grace_millis,
    }
}

fn harness_tool_plan(plan: &HarnessPlan) -> Option<ToolExecutionPlan> {
    if !plan.requires_tool_execution() {
        return None;
    }
    let executable = "/usr/bin/true".to_owned();
    let executable_digest = byte_digest(&fs::read(&executable).unwrap());
    let checks = plan
        .requirements
        .iter()
        .filter(|requirement| requirement.owner == VerificationOwner::Tool)
        .enumerate()
        .flat_map(|(index, requirement)| {
            let executable = executable.clone();
            let executable_digest = executable_digest.clone();
            [Vec::new(), vec!["--version".to_owned()]]
                .into_iter()
                .enumerate()
                .map(move |(recipe_index, extra_arguments)| {
                    let mut argv = vec![executable.clone()];
                    argv.extend(extra_arguments);
                    ToolCheck {
                        check_identifier: format!("check-{:02}-{}", index + 1, recipe_index + 1),
                        verification_requirement: *requirement,
                        executable: executable.clone(),
                        executable_digest: executable_digest.clone(),
                        argv,
                        cwd_workspace_relative: "src".to_owned(),
                        environment: std::collections::BTreeMap::new(),
                        inherit_environment: false,
                        stdin: ToolStdin {
                            bytes: Vec::new(),
                            bytes_digest: byte_digest(&[]),
                        },
                        timeout_millis: 1_000,
                    }
                })
        })
        .collect();
    ToolExecutionPlan::build(
        plan.resolved_plan_digest.clone(),
        plan.frozen_targets.targets[0].locator.root_identity.clone(),
        checks,
    )
    .ok()
}

fn harness_tool_evidence(plan: &ToolExecutionPlan) -> ToolExecutionEvidence {
    ToolExecutionEvidence {
        version: HARNESS_SCHEMA_VERSION,
        tool_plan_digest: plan.tool_plan_digest.clone(),
        checks: plan
            .checks
            .iter()
            .cloned()
            .map(|check| ToolCheckEvidence {
                check,
                started_millis: 1,
                finished_millis: 2,
                exit_code: 0,
                stdout_digest: byte_digest(b""),
                stderr_digest: byte_digest(b""),
                semantic_result: "passed".to_owned(),
            })
            .collect(),
    }
}

fn passed_exact_tool_event(
    invocation: &ToolInvocationContract,
    plan: &ToolExecutionPlan,
) -> HarnessExecutionEvent {
    let exact = harness_tool_evidence(plan);
    exact_tool_event(invocation, &exact)
}

fn exact_tool_event(
    invocation: &ToolInvocationContract,
    exact: &ToolExecutionEvidence,
) -> HarnessExecutionEvent {
    let executions = exact
        .checks
        .iter()
        .map(|evidence| {
            let unit = evidence.check.verification_requirement.unit;
            assert!(
                invocation
                    .requirements
                    .iter()
                    .any(|requirement| requirement.unit == unit)
            );
            let termination = ToolTermination::Exited {
                code: evidence.exit_code,
            };
            let evidence_digest = serialized_digest(&(
                &invocation.invocation_digest,
                unit,
                &evidence.check.argv,
                &termination,
                &evidence.stdout_digest,
                &evidence.stderr_digest,
            ))
            .unwrap();
            ToolCommandEvidence {
                unit,
                command: evidence.check.argv.clone(),
                termination,
                stdout_digest: evidence.stdout_digest.clone(),
                stderr_digest: evidence.stderr_digest.clone(),
                evidence_digest,
            }
        })
        .collect::<Vec<_>>();
    let results = invocation
        .requirements
        .iter()
        .map(|requirement| {
            let matching = executions
                .iter()
                .filter(|execution| execution.unit == requirement.unit)
                .collect::<Vec<_>>();
            let passed = matching.iter().all(|execution| {
                matches!(execution.termination, ToolTermination::Exited { code: 0 })
            });
            RequirementResult {
                unit: requirement.unit,
                passed,
                detail: if passed {
                    "all tool checks passed".to_owned()
                } else {
                    "one or more tool checks failed".to_owned()
                },
                evidence: matching
                    .into_iter()
                    .map(|execution| ResultEvidenceReference::ToolResult {
                        unit: execution.unit,
                        result_digest: execution.evidence_digest.clone(),
                    })
                    .collect(),
            }
        })
        .collect::<Vec<_>>();
    let evidence_set_digest =
        serialized_digest(&(&invocation.invocation_digest, &results, &executions)).unwrap();
    HarnessExecutionEvent::ToolEvidence {
        evidence: ToolEvidenceSet {
            invocation_digest: invocation.invocation_digest.clone(),
            results,
            executions,
            evidence_set_digest,
        },
    }
}

fn refresh_record_digest(record: &mut HarnessExecutionRecord) {
    record.record_digest = serialized_digest(&(
        (
            record.version,
            &record.run_identifier,
            &record.prepared_run_digest,
            &record.prepared_run_raw_digest,
            record.sequence,
            &record.predecessor_record_digest,
            record.state,
            &record.revision_history,
            &record.role_execution,
        ),
        (
            &record.ready_role_invocations,
            &record.ready_tool_invocation,
            &record.evaluation,
            &record.exact_tool_evidence,
            &record.tool_evidence_digest,
            &record.candidate_digest,
            &record.run_evaluation_receipt_digest,
            &record.validation_receipt_digest,
            &record.finalization_digest,
            &record.batch_receipt_digest,
        ),
    ))
    .unwrap();
}

fn plan_with_duplicate_verification_requirement(
    plan: &HarnessPlan,
    duplicate: VerificationRequirement,
) -> HarnessPlan {
    let mut forged = plan.clone();
    forged.requirements.push(duplicate);
    forged
        .resolved_request
        .plan
        .verification_requirements
        .push(duplicate);
    forged.authority_digest = serialized_digest(&forged.resolved_request)
        .expect("forged resolved request must serialize");
    forged
}

fn refresh_prepared_harness_run_digest(prepared: &mut PreparedHarnessRun) {
    prepared.prepared_run_digest = serialized_digest(&(
        prepared.version,
        &prepared.plan_identity,
        &prepared.tool_plan_identity,
        &prepared.plan,
        &prepared.runtime_capabilities,
        &prepared.accepted_tool_plan,
        &prepared.role_run,
    ))
    .expect("forged prepared run must serialize");
}

fn durable_code_fixture() -> (
    TemporaryWorkspace,
    HarnessEngine,
    ResolvedHarnessRequest,
    PreparedHarnessRun,
    Vec<u8>,
    Option<ToolExecutionPlan>,
) {
    let (workspace, engine) = external_engine();
    let resolved = engine
        .resolve(&personal_project_code_request())
        .expect("durable code request must resolve");
    let plan =
        HarnessPlan::from_resolved(resolved.clone(), &workspace.0, harness_lifecycle_limits())
            .expect("durable code plan must freeze");
    let capabilities = harness_capabilities(&plan);
    let tool_plan = harness_tool_plan(&plan);
    let raw_plan = serde_json::to_vec(&plan).expect("durable code plan must serialize");
    let raw_tool_plan = tool_plan
        .as_ref()
        .map(|value| serde_json::to_vec(value).expect("durable tool plan must serialize"));
    let prepared = PreparedHarnessRun::prepare(
        &engine,
        &raw_plan,
        plan,
        capabilities,
        raw_tool_plan.as_deref(),
        tool_plan.clone(),
    )
    .expect("durable code run must prepare");
    let raw_prepared = serde_json::to_vec(&prepared).expect("durable prepared run must serialize");
    (
        workspace,
        engine,
        resolved,
        prepared,
        raw_prepared,
        tool_plan,
    )
}

#[cfg(unix)]
fn validated_durable_code_run(
    run_identifier: &str,
) -> (
    TemporaryWorkspace,
    HarnessEngine,
    PreparedHarnessRun,
    Vec<u8>,
    HarnessExecutionRecord,
    HarnessExecutionRecord,
) {
    let (workspace, engine, resolved, prepared, raw_prepared, tool_plan) = durable_code_fixture();
    let begun =
        HarnessExecutionRecord::begin_durable(&engine, &raw_prepared, &prepared, run_identifier)
            .expect("durable execution must begin");
    let mut head = begun.clone();
    loop {
        if let Some(invocation) = head.ready_role_invocations.first().cloned() {
            head = HarnessExecutionRecord::advance_durable(
                &engine,
                run_identifier,
                completed_role_event(
                    &resolved,
                    &prepared.role_run,
                    &invocation,
                    Vec::new(),
                    "durable",
                ),
            )
            .expect("durable role result must advance");
            continue;
        }
        if let Some(invocation) = head.ready_tool_invocation.clone() {
            head = HarnessExecutionRecord::advance_durable(
                &engine,
                run_identifier,
                passed_exact_tool_event(
                    &invocation,
                    tool_plan.as_ref().expect("code run requires a tool plan"),
                ),
            )
            .expect("durable tool result must advance");
            continue;
        }
        break;
    }
    let evidence = tool_plan.as_ref().map(harness_tool_evidence);
    let evaluated =
        HarnessExecutionRecord::evaluate_durable(&engine, run_identifier, evidence.as_ref())
            .expect("durable execution must evaluate");
    assert_eq!(evaluated.state, HarnessExecutionState::Evaluated);
    head = HarnessExecutionRecord::validate_durable(&engine, run_identifier)
        .expect("durable execution must validate");
    (workspace, engine, prepared, raw_prepared, begun, head)
}

fn assert_duplicate_verification_unit_error<T>(result: HarnessResult<T>, boundary: &str) {
    assert!(
        matches!(
            result,
            Err(HarnessError::InvalidPlan(message))
                if message == "verification requirement units must be unique"
        ),
        "{boundary} must reject the repeated verification unit with the canonical error"
    );
}

#[test]
fn public_and_internal_plans_accept_the_same_runtime_support_capabilities() {
    let (workspace, engine) = external_engine();
    let mut request = personal_project_code_request();
    request.action = HarnessAction::CodeReview;
    let resolved = engine
        .resolve(&request)
        .expect("code review request must resolve");
    let plan = HarnessPlan::from_resolved(resolved, &workspace.0, harness_lifecycle_limits())
        .expect("code review plan must freeze");
    let mut capabilities = harness_capabilities(&plan);
    capabilities.available_roles = vec![
        HarnessRole::Writer,
        HarnessRole::Verifier,
        HarnessRole::Reviewer,
        HarnessRole::Specialist,
    ];
    capabilities.tool_execution = true;

    assert!(plan.required_roles().len() > 1);
    assert_eq!(plan.resolved_request.plan.required_concurrent_roles, 1);
    assert_eq!(capabilities.max_concurrent_roles, 1);
    plan.validate_capabilities(&capabilities)
        .expect("public plan must accept supported role and tool supersets");
    engine
        .prepare(
            &plan.resolved_request,
            &role_runtime_capabilities(&capabilities),
            None,
        )
        .expect("internal plan must accept the same supported role and tool supersets");

    let tool_plan = harness_tool_plan(&plan).expect("code review requires a tool plan");
    let raw_plan = serde_json::to_vec(&plan).expect("code review plan must serialize");
    let raw_tool_plan =
        serde_json::to_vec(&tool_plan).expect("code review tool plan must serialize");
    PreparedHarnessRun::prepare(
        &engine,
        &raw_plan,
        plan.clone(),
        capabilities.clone(),
        Some(&raw_tool_plan),
        Some(tool_plan),
    )
    .expect("public preparation must preserve sequential role capacity");

    let mut missing_role = capabilities.clone();
    missing_role
        .available_roles
        .retain(|role| *role != HarnessRole::Reviewer);
    assert!(matches!(
        plan.validate_capabilities(&missing_role),
        Err(HarnessError::UnsupportedRuntime(message))
            if message == "runtime does not provide required roles: Reviewer"
    ));

    let mut shared_context = capabilities.clone();
    shared_context.separate_contexts = false;
    assert!(matches!(
        plan.validate_capabilities(&shared_context),
        Err(HarnessError::UnsupportedRuntime(message))
            if message == "runtime cannot provide separate role contexts"
    ));

    let mut missing_tool = capabilities;
    missing_tool.tool_execution = false;
    assert!(matches!(
        plan.validate_capabilities(&missing_tool),
        Err(HarnessError::UnsupportedRuntime(message))
            if message == "runtime cannot execute required verification tools"
    ));
}

#[test]
fn prepared_run_rejects_raw_inputs_that_differ_from_bound_plans() {
    let (workspace, engine) = external_engine();
    let mut request = personal_project_code_request();
    request.action = HarnessAction::CodeReview;
    let resolved = engine
        .resolve(&request)
        .expect("review request must resolve");
    let plan = HarnessPlan::from_resolved(resolved, &workspace.0, harness_lifecycle_limits())
        .expect("review plan must freeze");
    let capabilities = harness_capabilities(&plan);
    let tool_plan = harness_tool_plan(&plan).expect("review requires a tool plan");
    let raw_plan = serde_json::to_vec(&plan).expect("plan must serialize");
    let raw_tool_plan = serde_json::to_vec(&tool_plan).expect("tool plan must serialize");

    let mut different_plan = plan.clone();
    different_plan.version = 0;
    let mismatched_plan = serde_json::to_vec(&different_plan).expect("plan must serialize");
    assert!(matches!(
        PreparedHarnessRun::prepare(
            &engine,
            &mismatched_plan,
            plan.clone(),
            capabilities.clone(),
            Some(&raw_tool_plan),
            Some(tool_plan.clone()),
        ),
        Err(HarnessError::InvalidPlan(message))
            if message == "decoded current Harness plan does not match the bound artifact"
    ));

    let mut different_tool_plan = tool_plan.clone();
    different_tool_plan.version = 0;
    let mismatched_tool_plan =
        serde_json::to_vec(&different_tool_plan).expect("tool plan must serialize");
    assert!(matches!(
        PreparedHarnessRun::prepare(
            &engine,
            &raw_plan,
            plan,
            capabilities,
            Some(&mismatched_tool_plan),
            Some(tool_plan),
        ),
        Err(HarnessError::InvalidPlan(message))
            if message == "decoded current Harness tool plan does not match the bound artifact"
    ));
}

#[test]
fn public_and_internal_non_tool_plans_accept_extra_tool_support() {
    let (workspace, engine) = external_engine();
    let request = HarnessRequest {
        action: HarnessAction::DocumentWrite,
        owner: DataOwner::PersonalProject {
            project: "coupler".to_owned(),
        },
        targets: vec!["README.md".to_owned()],
        objective: "update an external project document".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = engine
        .resolve(&request)
        .expect("document write request must resolve");
    let plan = HarnessPlan::from_resolved(resolved, &workspace.0, harness_lifecycle_limits())
        .expect("document write plan must freeze");
    let mut capabilities = harness_capabilities(&plan);
    capabilities.available_roles = vec![
        HarnessRole::Writer,
        HarnessRole::Verifier,
        HarnessRole::Reviewer,
        HarnessRole::Specialist,
    ];
    capabilities.tool_execution = true;

    assert!(!plan.requires_tool_capability());
    plan.validate_capabilities(&capabilities)
        .expect("public plan must accept extra runtime tool support");
    engine
        .prepare(
            &plan.resolved_request,
            &role_runtime_capabilities(&capabilities),
            None,
        )
        .expect("internal plan must accept extra runtime tool support");

    let raw_plan = serde_json::to_vec(&plan).expect("document write plan must serialize");
    PreparedHarnessRun::prepare(&engine, &raw_plan, plan, capabilities, None, None)
        .expect("public preparation must preserve extra runtime tool support");
}

#[test]
fn current_plan_and_replay_reject_repeated_units_with_changed_bindings() {
    let (workspace, engine) = external_engine();
    let request = HarnessRequest {
        action: HarnessAction::DocumentReview,
        owner: DataOwner::PersonalProject {
            project: "coupler".to_owned(),
        },
        targets: vec!["README.md".to_owned()],
        objective: "review the external document".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = engine
        .resolve(&request)
        .expect("review request must resolve");
    let plan = HarnessPlan::from_resolved(resolved, &workspace.0, harness_lifecycle_limits())
        .expect("review plan must freeze");
    let capabilities = harness_capabilities(&plan);
    let raw_plan = serde_json::to_vec(&plan).expect("review plan must serialize");
    let prepared =
        PreparedHarnessRun::prepare(&engine, &raw_plan, plan.clone(), capabilities, None, None)
            .expect("review plan must prepare");
    let original = plan
        .requirements
        .iter()
        .find(|requirement| requirement.unit == VerificationUnit::CompletionContract)
        .copied()
        .expect("review plan must contain the completion contract");
    let changed_owner = match original.owner {
        VerificationOwner::Role {
            role: HarnessRole::Verifier,
        } => VerificationOwner::Role {
            role: HarnessRole::Reviewer,
        },
        _ => VerificationOwner::Role {
            role: HarnessRole::Verifier,
        },
    };
    let changed_subject = match original.subject {
        EvaluationSubjectKind::FrozenTargets => EvaluationSubjectKind::ProducedArtifact,
        _ => EvaluationSubjectKind::FrozenTargets,
    };
    let duplicates = [
        VerificationRequirement {
            owner: changed_owner,
            ..original
        },
        VerificationRequirement {
            subject: changed_subject,
            ..original
        },
    ];

    for (index, duplicate) in duplicates.into_iter().enumerate() {
        let forged_plan = plan_with_duplicate_verification_requirement(&plan, duplicate);
        assert_duplicate_verification_unit_error(
            forged_plan.validate(),
            &format!("plan variant {index}"),
        );

        let forged_plan_bytes =
            serde_json::to_vec(&forged_plan).expect("forged plan must serialize");
        let mut forged_prepared = prepared.clone();
        forged_prepared.plan_identity = RawNormalizedInputIdentity::from_current_json(
            &forged_plan_bytes,
            "forged Harness plan",
            &forged_plan,
        )
        .expect("forged plan identity must bind its exact bytes");
        forged_prepared.plan = forged_plan;
        refresh_prepared_harness_run_digest(&mut forged_prepared);
        let raw_forged_prepared =
            serde_json::to_vec(&forged_prepared).expect("forged prepared run must serialize");
        assert_duplicate_verification_unit_error(
            forged_prepared.validate_with_engine(&raw_forged_prepared, &engine),
            &format!("replay variant {index}"),
        );
    }
}

#[test]
fn replay_rejects_missing_tool_plan_bindings_and_role_requirement_mutation() {
    let (workspace, engine) = external_engine();
    let mut request = personal_project_code_request();
    request.action = HarnessAction::CodeReview;
    let resolved = engine
        .resolve(&request)
        .expect("code review request must resolve");
    let plan = HarnessPlan::from_resolved(resolved, &workspace.0, harness_lifecycle_limits())
        .expect("code review plan must freeze");
    let capabilities = harness_capabilities(&plan);
    let tool_plan = harness_tool_plan(&plan).expect("code review requires a tool plan");
    let raw_plan = serde_json::to_vec(&plan).expect("code review plan must serialize");
    let raw_tool_plan =
        serde_json::to_vec(&tool_plan).expect("code review tool plan must serialize");
    let prepared = PreparedHarnessRun::prepare(
        &engine,
        &raw_plan,
        plan,
        capabilities,
        Some(&raw_tool_plan),
        Some(tool_plan),
    )
    .expect("code review must prepare");

    for (identity_present, plan_present) in [(false, false), (false, true), (true, false)] {
        let mut forged = prepared.clone();
        if !identity_present {
            forged.tool_plan_identity = None;
        }
        if !plan_present {
            forged.accepted_tool_plan = None;
        }
        refresh_prepared_harness_run_digest(&mut forged);
        let raw_forged = serde_json::to_vec(&forged).expect("forged run must serialize");
        assert!(
            matches!(
                forged.validate_with_engine(&raw_forged, &engine),
                Err(HarnessError::InvalidPlan(message))
                    if message.contains("Tool-owned requirements require both")
            ),
            "replay must reject tool-plan identity={identity_present}, plan={plan_present}"
        );
    }

    let mut forged = prepared;
    let reviewer = forged
        .role_run
        .role_metadata
        .iter_mut()
        .find(|metadata| metadata.role == HarnessRole::Reviewer)
        .expect("reviewer metadata must exist");
    let RoleTaskContract::Reviewer {
        verification_requirements,
        ..
    } = &mut reviewer.task
    else {
        panic!("Reviewer metadata must contain a Reviewer task");
    };
    verification_requirements.push(verification_requirements[0]);
    forged.role_run.prepared_role_run_digest = serialized_digest(&(
        &forged.role_run.resolved_plan_digest,
        &forged.role_run.runtime_capabilities_digest,
        &forged.role_run.source_versions,
        &forged.role_run.context_bundle,
        &forged.role_run.role_bundles,
        &forged.role_run.role_metadata,
    ))
    .expect("forged role run must serialize");
    refresh_prepared_harness_run_digest(&mut forged);
    let raw_forged = serde_json::to_vec(&forged).expect("forged run must serialize");
    assert!(matches!(
        forged.validate_with_engine(&raw_forged, &engine),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("exact current Harness engine preparation")
    ));
}

#[test]
fn replay_derives_role_capabilities_from_bound_runtime_capabilities() {
    let (workspace, engine) = external_engine();
    let mut request = personal_project_code_request();
    request.action = HarnessAction::CodeReview;
    let resolved = engine
        .resolve(&request)
        .expect("code review request must resolve");
    let plan = HarnessPlan::from_resolved(resolved, &workspace.0, harness_lifecycle_limits())
        .expect("code review plan must freeze");
    let capabilities = harness_capabilities(&plan);
    let tool_plan = harness_tool_plan(&plan).expect("code review requires a tool plan");
    let raw_plan = serde_json::to_vec(&plan).expect("code review plan must serialize");
    let raw_tool_plan =
        serde_json::to_vec(&tool_plan).expect("code review tool plan must serialize");
    let prepared = PreparedHarnessRun::prepare(
        &engine,
        &raw_plan,
        plan,
        capabilities,
        Some(&raw_tool_plan),
        Some(tool_plan),
    )
    .expect("code review must prepare");

    let mut alternative_capabilities = prepared.role_run.runtime_capabilities.clone();
    alternative_capabilities.max_role_execution_millis = alternative_capabilities
        .max_role_execution_millis
        .checked_sub(1)
        .expect("test execution limit must remain positive");
    let alternative_role_run = engine
        .prepare(
            &prepared.plan.resolved_request,
            &alternative_capabilities,
            None,
        )
        .expect("independently valid role capabilities must prepare");
    let mut inconsistent = prepared;
    inconsistent.role_run = alternative_role_run;
    refresh_prepared_harness_run_digest(&mut inconsistent);
    let raw_inconsistent =
        serde_json::to_vec(&inconsistent).expect("inconsistent run must serialize");

    assert!(matches!(
        inconsistent.validate_with_engine(&raw_inconsistent, &engine),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("exact current Harness engine preparation")
    ));
}

#[test]
fn replay_derives_vault_context_from_bound_contract() {
    let engine = engine();
    let request = HarnessRequest {
        action: HarnessAction::VaultRead,
        owner: DataOwner::Personal,
        targets: Vec::new(),
        objective: "read bounded Vault context".to_owned(),
        curation_kind: Some(CurationKind::Knowledge),
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = resolve_vault_read(
        &engine,
        &request,
        HarnessExecutionProfile::Standard,
        "knowledge",
        4096,
    );
    let plan = HarnessPlan::from_resolved(resolved, &repository_root(), harness_lifecycle_limits())
        .expect("Vault read plan must freeze");
    let capabilities = harness_capabilities(&plan);
    let raw_plan = serde_json::to_vec(&plan).expect("Vault read plan must serialize");
    let prepared = PreparedHarnessRun::prepare(&engine, &raw_plan, plan, capabilities, None, None)
        .expect("Vault read must prepare");

    for (query, maximum_context_bytes) in [("project", 4096), ("knowledge", 2048)] {
        let alternative_bundle = engine
            .retrieve_context(
                &prepared.plan.resolved_request,
                query,
                maximum_context_bytes,
            )
            .expect("alternative bounded retrieval must succeed");
        let alternative_role_run = engine
            .prepare(
                &prepared.plan.resolved_request,
                &prepared.role_run.runtime_capabilities,
                Some(&alternative_bundle),
            )
            .expect("coherent alternative context must prepare");
        let mut inconsistent = prepared.clone();
        inconsistent.role_run = alternative_role_run;
        refresh_prepared_harness_run_digest(&mut inconsistent);
        let raw_inconsistent =
            serde_json::to_vec(&inconsistent).expect("inconsistent run must serialize");

        assert!(matches!(
            inconsistent.validate_with_engine(&raw_inconsistent, &engine),
            Err(HarnessError::InvalidSubmission(message))
                if message.contains("exact current Harness engine preparation")
        ));
    }
}

#[test]
fn every_target_bearing_read_only_action_freezes_an_inspection() {
    let (workspace, engine) = external_engine();
    for (action, target) in [
        (HarnessAction::CodeReview, "src/lib.rs"),
        (HarnessAction::DocumentReview, "README.md"),
        (HarnessAction::Investigation, "README.md"),
        (HarnessAction::Design, "README.md"),
        (HarnessAction::Ideation, "README.md"),
    ] {
        let request = HarnessRequest {
            action,
            owner: DataOwner::PersonalProject {
                project: "coupler".to_owned(),
            },
            targets: vec![target.to_owned()],
            objective: format!("exercise the {} read-only path", action.as_str()),
            curation_kind: None,
            explicit_user_confirmation_reported: false,
            curation_sources: Vec::new(),
            delete_targets: Vec::new(),
        };
        let resolved = engine
            .resolve(&request)
            .expect("target-bearing read-only action must resolve");
        let plan = HarnessPlan::from_resolved(resolved, &workspace.0, harness_lifecycle_limits())
            .expect("target-bearing read-only action must freeze");

        assert!(!plan.source_write_allowed);
        assert_eq!(plan.frozen_targets.targets.len(), 1);
        assert_eq!(
            plan.frozen_targets.targets[0].operation,
            TargetOperation::Inspect
        );
        assert!(plan.frozen_targets.targets[0].content_digest.is_some());
        plan.validate()
            .expect("frozen read-only plan must remain internally valid");
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "read-only lifecycle coverage keeps one complete state transition visible"
)]
fn assert_read_only_target_lifecycle(action: HarnessAction) {
    let (workspace, engine) = external_engine();
    let request = HarnessRequest {
        action,
        owner: DataOwner::PersonalProject {
            project: "coupler".to_owned(),
        },
        targets: vec!["README.md".to_owned()],
        objective: format!("exercise the {} lifecycle", action.as_str()),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let resolved = engine
        .resolve(&request)
        .expect("read-only lifecycle request must resolve");
    let plan =
        HarnessPlan::from_resolved(resolved.clone(), &workspace.0, harness_lifecycle_limits())
            .expect("read-only lifecycle request must freeze");
    assert_eq!(
        plan.frozen_targets.targets[0].operation,
        TargetOperation::Inspect
    );
    assert!(!plan.requires_tool_execution());
    let capabilities = harness_capabilities(&plan);
    let raw_plan = serde_json::to_vec(&plan).expect("read-only plan must serialize");
    let prepared = PreparedHarnessRun::prepare(&engine, &raw_plan, plan, capabilities, None, None)
        .expect("read-only plan must prepare without a tool plan");
    let raw_prepared =
        serde_json::to_vec(&prepared).expect("prepared read-only run must serialize");
    let mut execution = HarnessExecutionRecord::begin(
        &engine,
        &raw_prepared,
        &prepared,
        format!("run-{}-inspection", action.as_str()),
    )
    .expect("read-only execution must begin");

    while let Some(invocation) = execution.ready_role_invocations.first().cloned() {
        execution = execution
            .advance(
                &engine,
                &raw_prepared,
                &prepared,
                completed_role_event(
                    &resolved,
                    &prepared.role_run,
                    &invocation,
                    Vec::new(),
                    action.as_str(),
                ),
            )
            .expect("read-only role result must advance");
    }
    assert!(execution.ready_tool_invocation.is_none());
    execution = execution
        .evaluate(&engine, &raw_prepared, &prepared, None)
        .expect("read-only execution must evaluate");
    execution = execution
        .validate_evaluation(&engine, &raw_prepared, &prepared)
        .expect("read-only evaluation must validate");
    assert_eq!(execution.state, HarnessExecutionState::Validated);

    let finalization_error = execution
        .preflight_finalize(&engine, &raw_prepared, &prepared, None)
        .expect_err("read-only execution must never enter finalization");
    assert!(matches!(
        finalization_error,
        HarnessError::InvalidSubmission(message)
            if message == "current finalization requires a validated source-writing execution"
    ));
    let apply_error = execution
        .finalize(&engine, &raw_prepared, &prepared, None)
        .expect_err("read-only execution must never apply");
    assert!(matches!(
        apply_error,
        HarnessError::InvalidSubmission(message)
            if message == "current finalization requires a validated source-writing execution"
    ));
    assert!(!workspace.0.join(".llm-context-vault-harness").exists());

    assert!(matches!(
        FinalizationTarget::from_frozen_targets(
            &prepared.plan.frozen_targets,
            &BTreeMap::new(),
        ),
        Err(HarnessError::InvalidSubmission(message))
            if message == "read-only frozen targets cannot enter finalization"
    ));

    fs::write(
        workspace.0.join("README.md"),
        "# Drifted external project\n",
    )
    .expect("read-only drift fixture must be written");
    assert!(matches!(
        prepared.validate_with_engine(&raw_prepared, &engine),
        Err(HarnessError::PlanDrift { .. })
    ));
}

#[test]
fn read_only_review_and_analysis_complete_without_apply_authority() {
    for action in [HarnessAction::DocumentReview, HarnessAction::Investigation] {
        assert_read_only_target_lifecycle(action);
    }
}

fn assert_current_harness_plan_digest_json(plan: &HarnessPlan) {
    let mut plan_json = serde_json::to_value(plan).expect("Harness plan must serialize to JSON");
    let plan_object = plan_json
        .as_object()
        .expect("Harness plan must be a JSON object");
    assert!(plan_object.contains_key("resolved_plan_digest"));
    assert!(!plan_object.contains_key("plan_identifier"));
    let resolved_plan_digest = plan_json
        .as_object_mut()
        .expect("Harness plan must remain mutable")
        .remove("resolved_plan_digest")
        .expect("current Harness plan must bind its resolved plan digest");
    plan_json
        .as_object_mut()
        .expect("Harness plan must remain mutable")
        .insert("plan_identifier".to_owned(), resolved_plan_digest);
    assert!(
        decode_current_json::<HarnessPlan>(
            &serde_json::to_vec(&plan_json).expect("legacy plan shape must serialize"),
            "Harness plan",
        )
        .is_err(),
        "plan_identifier must not remain a compatibility alias"
    );
}

fn assert_current_tool_plan_digest_json(tool_plan: &ToolExecutionPlan) {
    let mut tool_plan_json =
        serde_json::to_value(tool_plan).expect("tool plan must serialize to JSON");
    let tool_plan_object = tool_plan_json
        .as_object()
        .expect("tool plan must be a JSON object");
    assert!(tool_plan_object.contains_key("resolved_plan_digest"));
    assert!(tool_plan_object.contains_key("tool_plan_digest"));
    assert!(!tool_plan_object.contains_key("plan_identifier"));
    assert!(!tool_plan_object.contains_key("plan_digest"));
    let tool_plan_object = tool_plan_json
        .as_object_mut()
        .expect("tool plan must remain mutable");
    let resolved_plan_digest = tool_plan_object
        .remove("resolved_plan_digest")
        .expect("tool plan must bind its resolved plan");
    let tool_plan_digest = tool_plan_object
        .remove("tool_plan_digest")
        .expect("tool plan must bind its own digest");
    tool_plan_object.insert("plan_identifier".to_owned(), resolved_plan_digest);
    tool_plan_object.insert("plan_digest".to_owned(), tool_plan_digest);
    assert!(
        decode_current_json::<ToolExecutionPlan>(
            &serde_json::to_vec(&tool_plan_json).expect("legacy tool plan shape must serialize"),
            "Harness tool plan",
        )
        .is_err(),
        "the previous ambiguous tool-plan fields must not remain aliases"
    );
}

fn assert_current_apply_attempt_digest_json(attempt: &HarnessApplyAttemptReceipt) {
    let attempt_json = serde_json::to_value(attempt).expect("apply attempt must serialize to JSON");
    let attempt_object = attempt_json
        .as_object()
        .expect("apply attempt must be a JSON object");
    assert!(attempt_object.contains_key("harness_plan_digest"));
    assert!(attempt_object.contains_key("resolved_plan_digest"));
    assert!(!attempt_object.contains_key("plan_digest"));
    assert!(!attempt_object.contains_key("batch_plan_digest"));

    for (current_name, legacy_name) in [
        ("harness_plan_digest", "plan_digest"),
        ("resolved_plan_digest", "batch_plan_digest"),
    ] {
        let mut legacy_attempt_json = attempt_json.clone();
        let legacy_attempt = legacy_attempt_json
            .as_object_mut()
            .expect("apply attempt must remain mutable");
        let digest = legacy_attempt
            .remove(current_name)
            .expect("current apply attempt digest must be present");
        legacy_attempt.insert(legacy_name.to_owned(), digest);
        assert!(
            decode_current_json::<HarnessApplyAttemptReceipt>(
                &serde_json::to_vec(&legacy_attempt_json)
                    .expect("legacy apply attempt must serialize"),
                "apply attempt",
            )
            .is_err(),
            "legacy apply-attempt digest name {legacy_name} must not remain an alias"
        );
    }
}

fn assert_current_apply_receipt_digest_json(receipt: &HarnessApplyReceipt) {
    let receipt_json = serde_json::to_value(receipt).expect("apply receipt must serialize to JSON");
    let receipt_object = receipt_json
        .as_object()
        .expect("apply receipt must be a JSON object");
    assert!(receipt_object.contains_key("harness_plan_digest"));
    assert!(!receipt_object.contains_key("resolved_plan_digest"));
    assert!(!receipt_object.contains_key("plan_digest"));
    let applied_batch = receipt_object
        .get("applied_batch")
        .and_then(serde_json::Value::as_object)
        .expect("apply receipt must contain an applied batch");
    assert!(applied_batch.contains_key("resolved_plan_digest"));
    assert!(!applied_batch.contains_key("plan_digest"));
    let lifecycle_receipt = applied_batch
        .get("lifecycle_receipt")
        .and_then(serde_json::Value::as_object)
        .expect("applied batch must contain a lifecycle receipt");
    assert!(lifecycle_receipt.contains_key("resolved_plan_digest"));
    assert!(!lifecycle_receipt.contains_key("plan_digest"));

    let mut legacy_outer_json = receipt_json.clone();
    let legacy_outer = legacy_outer_json
        .as_object_mut()
        .expect("apply receipt must remain mutable");
    let harness_plan_digest = legacy_outer
        .remove("harness_plan_digest")
        .expect("current apply receipt must bind its Harness plan");
    legacy_outer.insert("plan_digest".to_owned(), harness_plan_digest);
    assert!(
        decode_current_json::<HarnessApplyReceipt>(
            &serde_json::to_vec(&legacy_outer_json).expect("legacy apply receipt must serialize"),
            "apply receipt",
        )
        .is_err(),
        "the ambiguous outer apply-receipt plan_digest must not remain an alias"
    );

    let mut legacy_inner_json = receipt_json.clone();
    let legacy_inner = legacy_inner_json
        .get_mut("applied_batch")
        .and_then(serde_json::Value::as_object_mut)
        .expect("applied batch must remain mutable");
    let resolved_plan_digest = legacy_inner
        .remove("resolved_plan_digest")
        .expect("current applied batch must bind its resolved plan");
    legacy_inner.insert("plan_digest".to_owned(), resolved_plan_digest);
    assert!(
        decode_current_json::<HarnessApplyReceipt>(
            &serde_json::to_vec(&legacy_inner_json).expect("legacy applied batch must serialize"),
            "apply receipt",
        )
        .is_err(),
        "the ambiguous applied-batch plan_digest must not remain an alias"
    );

    let mut legacy_lifecycle_json = receipt_json;
    let legacy_lifecycle = legacy_lifecycle_json
        .get_mut("applied_batch")
        .and_then(|batch| batch.get_mut("lifecycle_receipt"))
        .and_then(serde_json::Value::as_object_mut)
        .expect("lifecycle receipt must remain mutable");
    let resolved_plan_digest = legacy_lifecycle
        .remove("resolved_plan_digest")
        .expect("current lifecycle receipt must bind its resolved plan");
    legacy_lifecycle.insert("plan_digest".to_owned(), resolved_plan_digest);
    assert!(
        decode_current_json::<HarnessApplyReceipt>(
            &serde_json::to_vec(&legacy_lifecycle_json)
                .expect("legacy lifecycle receipt must serialize"),
            "apply receipt",
        )
        .is_err(),
        "the ambiguous lifecycle-receipt plan_digest must not remain an alias"
    );
}

fn assert_current_prepared_role_json(prepared: &PreparedHarnessRun) {
    let prepared_json =
        serde_json::to_value(prepared).expect("prepared Harness run must serialize to JSON");
    let prepared_object = prepared_json
        .as_object()
        .expect("prepared Harness run must be a JSON object");
    assert!(prepared_object.contains_key("prepared_run_digest"));
    let role_run = prepared_object
        .get("role_run")
        .and_then(serde_json::Value::as_object)
        .expect("prepared Harness run must contain role preparation");
    assert!(role_run.contains_key("prepared_role_run_digest"));
    assert!(!role_run.contains_key("prepared_run_digest"));
    assert!(role_run.contains_key("resolved_plan_digest"));
    assert!(!role_run.contains_key("plan_digest"));
    assert!(role_run.contains_key("role_metadata"));
    assert!(!role_run.contains_key("role_inputs"));
    let role_metadata = role_run
        .get("role_metadata")
        .and_then(serde_json::Value::as_array)
        .expect("prepared role run must contain role metadata");
    assert!(role_metadata.iter().all(|metadata| {
        metadata.get("resolved_plan_digest").is_some()
            && metadata.get("plan_digest").is_none()
            && metadata.get("role_bundle_digest").is_some()
            && metadata.get("role_input_digest").is_none()
    }));

    let mut legacy_prepared_json = prepared_json;
    let legacy_role_run = legacy_prepared_json
        .get_mut("role_run")
        .and_then(serde_json::Value::as_object_mut)
        .expect("prepared Harness run must contain mutable role preparation");
    let prepared_role_run_digest = legacy_role_run
        .remove("prepared_role_run_digest")
        .expect("current role preparation must bind its digest");
    let role_metadata = legacy_role_run
        .remove("role_metadata")
        .expect("current role preparation must contain role metadata");
    legacy_role_run.insert("prepared_run_digest".to_owned(), prepared_role_run_digest);
    legacy_role_run.insert("role_inputs".to_owned(), role_metadata);
    assert!(
        decode_current_json::<PreparedHarnessRun>(
            &serde_json::to_vec(&legacy_prepared_json)
                .expect("legacy prepared-run shape must serialize"),
            "prepared Harness run",
        )
        .is_err(),
        "the previous ambiguous preparation fields must not remain aliases"
    );
}

#[test]
fn durable_json_uses_distinct_plan_and_preparation_digest_names() {
    let (_workspace, engine, _, prepared, _, tool_plan) = durable_code_fixture();
    assert_current_harness_plan_digest_json(&prepared.plan);
    assert_current_tool_plan_digest_json(
        tool_plan
            .as_ref()
            .expect("code work must have an exact tool plan"),
    );
    assert_current_prepared_role_json(&prepared);

    let raw_prepared =
        serde_json::to_vec(&prepared).expect("prepared Harness run must serialize exactly");
    let begun = HarnessExecutionRecord::begin(
        &engine,
        &raw_prepared,
        &prepared,
        "run-current-terminology".to_owned(),
    )
    .expect("current execution must begin");
    let invocation = serde_json::to_value(&begun.ready_role_invocations[0])
        .expect("final role invocation must serialize");
    assert!(invocation.get("prepared_role_run_digest").is_some());
    assert!(invocation.get("resolved_plan_digest").is_some());
    assert!(invocation.get("plan_digest").is_none());
    assert!(invocation.get("role_input_digest").is_some());
    assert!(invocation.get("prepared_run_digest").is_none());
    assert!(invocation.get("role_bundle_digest").is_none());
}

#[test]
fn task_and_run_evaluation_json_use_distinct_receipt_names() {
    let (_workspace, engine, resolved, prepared, raw_prepared, _) = durable_code_fixture();
    let role_execution =
        complete_execution_record(&engine, &resolved, &prepared.role_run, &[], "terminology");
    let task_evaluation = engine
        .evaluate_execution(&resolved, &prepared.role_run, &[], &role_execution)
        .expect("terminal role execution must evaluate");
    let mut task_json =
        serde_json::to_value(&task_evaluation).expect("task evaluation must serialize");
    let task_object = task_json
        .as_object()
        .expect("task evaluation must be a JSON object");
    assert!(task_object.contains_key("task_evaluation_receipt_digest"));
    assert!(task_object.contains_key("resolved_plan_digest"));
    assert!(!task_object.contains_key("plan_digest"));
    assert!(!task_object.contains_key("evaluation_receipt_digest"));
    assert!(!task_object.contains_key("run_evaluation_receipt_digest"));
    let task_receipt = task_json
        .as_object_mut()
        .expect("task evaluation must remain mutable")
        .remove("task_evaluation_receipt_digest")
        .expect("task evaluation must contain its receipt");
    task_json
        .as_object_mut()
        .expect("task evaluation must remain mutable")
        .insert("evaluation_receipt_digest".to_owned(), task_receipt);
    assert!(
        decode_current_json::<HarnessTaskEvaluation>(
            &serde_json::to_vec(&task_json).expect("legacy task evaluation must serialize"),
            "task evaluation",
        )
        .is_err(),
        "the ambiguous task evaluation receipt name must not remain an alias"
    );

    let run = HarnessExecutionRecord::begin(
        &engine,
        &raw_prepared,
        &prepared,
        "run-evaluation-terminology".to_owned(),
    )
    .expect("current execution must begin");
    let mut run_json = serde_json::to_value(&run).expect("execution record must serialize");
    let run_object = run_json
        .as_object()
        .expect("execution record must be a JSON object");
    assert!(run_object.contains_key("run_evaluation_receipt_digest"));
    assert!(!run_object.contains_key("evaluation_receipt_digest"));
    assert!(!run_object.contains_key("task_evaluation_receipt_digest"));
    let run_receipt = run_json
        .as_object_mut()
        .expect("execution record must remain mutable")
        .remove("run_evaluation_receipt_digest")
        .expect("execution record must contain its optional run receipt");
    run_json
        .as_object_mut()
        .expect("execution record must remain mutable")
        .insert("evaluation_receipt_digest".to_owned(), run_receipt);
    assert!(
        decode_current_json::<HarnessExecutionRecord>(
            &serde_json::to_vec(&run_json).expect("legacy execution record must serialize"),
            "execution record",
        )
        .is_err(),
        "the ambiguous run evaluation receipt name must not remain an alias"
    );
}

#[test]
fn schema_v5_v6_and_v7_request_prepared_run_and_execution_record_fail_closed() {
    assert_eq!(HARNESS_SCHEMA_VERSION, 8);

    for legacy_version in [5, 6, 7] {
        let mut envelope = user_profile_code_envelope("change the request contract");
        envelope.version = legacy_version;
        assert!(matches!(
            engine().resolve_envelope(&envelope, None),
            Err(HarnessError::InvalidRequest(message))
                if message.contains("request envelope version")
        ));

        let (workspace, engine) = external_engine();
        let resolved = engine
            .resolve(&personal_project_code_request())
            .expect("external code request must resolve");
        let plan = HarnessPlan::from_resolved(resolved, &workspace.0, harness_lifecycle_limits())
            .expect("current plan must resolve");
        let capabilities = harness_capabilities(&plan);
        let tool_plan = harness_tool_plan(&plan);
        let raw_plan = serde_json::to_vec(&plan).unwrap();
        let raw_tool_plan = tool_plan
            .as_ref()
            .map(|value| serde_json::to_vec(value).unwrap());
        let prepared = PreparedHarnessRun::prepare(
            &engine,
            &raw_plan,
            plan,
            capabilities,
            raw_tool_plan.as_deref(),
            tool_plan,
        )
        .expect("current preparation must succeed");
        let raw_prepared = serde_json::to_vec(&prepared).unwrap();

        let mut legacy_prepared = prepared.clone();
        legacy_prepared.version = legacy_version;
        let raw_legacy_prepared = serde_json::to_vec(&legacy_prepared).unwrap();
        assert!(matches!(
            legacy_prepared.validate_serialized_integrity(&raw_legacy_prepared),
            Err(HarnessError::InvalidPlan(message))
                if message.contains("prepared run version")
        ));

        let current_record = HarnessExecutionRecord::begin(
            &engine,
            &raw_prepared,
            &prepared,
            "run-previous-schema-rejection".to_owned(),
        )
        .expect("current execution must begin");
        let mut legacy_record = current_record.clone();
        legacy_record.version = legacy_version;
        refresh_record_digest(&mut legacy_record);
        assert!(matches!(
            legacy_record.validate(&prepared, &raw_prepared),
            Err(HarnessError::InvalidPlan(message))
                if message.contains("current execution record version")
        ));
    }
}

#[cfg(unix)]
#[test]
fn durable_run_initialization_is_atomic_owner_only_and_single_create() {
    use std::os::unix::fs::PermissionsExt as _;

    let (workspace, engine, _, prepared, raw_prepared, _) = durable_code_fixture();
    let runs = workspace.0.join(".llm-context-vault-harness").join("runs");
    fs::create_dir_all(&runs).expect("test run parent must be created");
    fs::set_permissions(
        runs.parent().expect("runs must have a parent"),
        fs::Permissions::from_mode(0o700),
    )
    .expect("control directory permissions must be set");
    fs::set_permissions(&runs, fs::Permissions::from_mode(0o700))
        .expect("run directory permissions must be set");
    let stale = runs.join(".run-durable-init.init-orphan");
    fs::create_dir(&stale).expect("stale initialization fixture must be created");
    fs::set_permissions(&stale, fs::Permissions::from_mode(0o700))
        .expect("stale initialization permissions must be set");

    let head = HarnessExecutionRecord::begin_durable(
        &engine,
        &raw_prepared,
        &prepared,
        "run-durable-init",
    )
    .expect("canonical durable run must initialize beside a stale sibling");
    assert!(stale.is_dir(), "ambiguous stale initialization must remain");
    let run = runs.join("run-durable-init");
    assert!(run.is_dir());
    assert_eq!(
        fs::read(run.join("prepared.json")).expect("prepared bytes must persist"),
        raw_prepared
    );
    let persisted: HarnessExecutionRecord = decode_current_json(
        &fs::read(run.join("head.json")).expect("head must persist"),
        "persisted durable head",
    )
    .expect("persisted durable head must decode strictly");
    assert_eq!(persisted, head);
    for path in [
        runs.parent()
            .expect("runs must have a parent")
            .to_path_buf(),
        runs.clone(),
        run.clone(),
    ] {
        assert_eq!(
            fs::metadata(path)
                .expect("durable directory metadata must exist")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    for path in [run.join("prepared.json"), run.join("head.json")] {
        assert_eq!(
            fs::metadata(path)
                .expect("durable file metadata must exist")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    assert!(
        HarnessExecutionRecord::begin_durable(
            &engine,
            &raw_prepared,
            &prepared,
            "run-durable-init",
        )
        .is_err(),
        "an existing run ID must never be initialized again"
    );
    let (reloaded_raw, reloaded_prepared) =
        HarnessExecutionRecord::load_durable_prepared(&workspace.0, "run-durable-init")
            .expect("durable prepared bytes must reload");
    assert_eq!(reloaded_raw, raw_prepared);
    assert_eq!(reloaded_prepared, prepared);
}

#[cfg(unix)]
#[test]
fn interrupted_durable_initialization_retains_only_a_noncanonical_half_state() {
    let (workspace, engine, _, prepared, raw_prepared, _) = durable_code_fixture();
    assert!(
        HarnessExecutionRecord::begin_durable_with_initialization_failure_for_test(
            &engine,
            &raw_prepared,
            &prepared,
            "run-durable-init-interrupted",
        )
        .is_err(),
        "the injected interruption must fail initialization"
    );
    let runs = workspace.0.join(".llm-context-vault-harness/runs");
    assert!(!runs.join("run-durable-init-interrupted").exists());
    let retained = fs::read_dir(&runs)
        .expect("run parent must remain inspectable")
        .map(|entry| entry.expect("run sibling must be readable").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".run-durable-init-interrupted.init-"))
        })
        .collect::<Vec<_>>();
    assert_eq!(retained.len(), 1);
    assert!(retained[0].join("prepared.json").is_file());
    assert!(!retained[0].join("head.json").exists());

    let head = HarnessExecutionRecord::begin_durable(
        &engine,
        &raw_prepared,
        &prepared,
        "run-durable-init-interrupted",
    )
    .expect("a half-state sibling must not claim the canonical run ID");
    assert_eq!(head.state, HarnessExecutionState::Begun);
    assert!(
        retained[0].is_dir(),
        "ambiguous half-state evidence must remain"
    );
    assert!(runs.join("run-durable-init-interrupted").is_dir());
}

#[cfg(unix)]
#[test]
fn durable_transitions_persist_one_exact_current_head() {
    let (workspace, _, prepared, raw_prepared, _, validated) =
        validated_durable_code_run("run-durable-head");
    assert_eq!(validated.state, HarnessExecutionState::Validated);
    assert!(validated.sequence > 0);
    let persisted: HarnessExecutionRecord = decode_current_json(
        &fs::read(
            workspace
                .0
                .join(".llm-context-vault-harness/runs/run-durable-head/head.json"),
        )
        .expect("durable head must be readable"),
        "durable execution head",
    )
    .expect("durable head must decode strictly");
    assert_eq!(persisted, validated);
    persisted
        .validate(&prepared, &raw_prepared)
        .expect("persisted current head must revalidate");
}

#[cfg(unix)]
#[test]
fn pending_apply_can_abort_only_before_mutation_then_apply_once() {
    let (workspace, engine, _, _, _, validated) = validated_durable_code_run("run-durable-abort");
    let pending =
        HarnessExecutionRecord::persist_pending_apply_for_test(&engine, "run-durable-abort")
            .expect("pending apply must persist before source mutation");
    assert_current_apply_attempt_digest_json(&pending);
    assert!(matches!(
        pending.attempt_state,
        HarnessApplyAttemptState::PendingApply
    ));
    assert!(
        !workspace
            .0
            .join(&pending.expected_journal_relative_path)
            .exists()
    );
    assert!(
        !workspace
            .0
            .join(&pending.expected_completion_relative_path)
            .exists()
    );
    assert_eq!(
        fs::read_to_string(workspace.0.join("src/lib.rs")).expect("source must remain readable"),
        "pub fn fixture() {}\n"
    );

    let (unchanged, aborted) =
        HarnessExecutionRecord::recover_durable(&engine, "run-durable-abort", None)
            .expect("recovery must prove the pre-mutation abort");
    assert_eq!(unchanged, validated);
    assert!(matches!(
        aborted.attempt_state,
        HarnessApplyAttemptState::AbortedBeforeMutation { .. }
    ));

    let (finalized, terminal) =
        HarnessExecutionRecord::apply_durable(&engine, "run-durable-abort", None)
            .expect("an explicitly proven abort must permit one fresh apply");
    assert_eq!(finalized.state, HarnessExecutionState::Finalized);
    assert!(matches!(
        terminal.attempt_state,
        HarnessApplyAttemptState::AppliedFinalized { .. }
    ));
    assert_eq!(
        fs::read_to_string(workspace.0.join("src/lib.rs")).expect("source must remain readable"),
        "candidate"
    );
    let (reentered, same_terminal) =
        HarnessExecutionRecord::apply_durable(&engine, "run-durable-abort", None)
            .expect("terminal apply re-entry must be idempotent");
    assert_eq!(reentered, finalized);
    assert_eq!(same_terminal, terminal);
}

#[cfg(unix)]
#[test]
fn pre_mutation_batch_failure_can_be_proven_aborted_and_retried() {
    let (workspace, engine, _, _, _, validated) =
        validated_durable_code_run("run-durable-lock-failure");
    let unrelated_pending_batch = workspace
        .0
        .join(".llm-context-vault-harness/batches/unrelated-pending");
    fs::create_dir_all(&unrelated_pending_batch)
        .expect("an unrelated pending batch must be installed");
    let apply_error =
        HarnessExecutionRecord::apply_durable(&engine, "run-durable-lock-failure", None)
            .expect_err("batch preflight must fail before source mutation");
    assert!(
        matches!(apply_error, HarnessError::BatchApply { .. }),
        "unexpected pre-mutation apply error: {apply_error:?}"
    );
    let failed: HarnessApplyAttemptReceipt =
        decode_current_json(
            &fs::read(workspace.0.join(
                ".llm-context-vault-harness/runs/run-durable-lock-failure/apply-attempt.json",
            ))
            .expect("batch failure receipt must persist"),
            "pre-mutation batch failure",
        )
        .expect("batch failure receipt must decode");
    assert!(matches!(
        failed.attempt_state,
        HarnessApplyAttemptState::Failure {
            evidence: HarnessApplyFailureEvidence::BeforeBatch { .. }
        }
    ));
    assert_eq!(
        fs::read_to_string(workspace.0.join("src/lib.rs")).expect("source must remain readable"),
        "pub fn fixture() {}\n"
    );

    fs::remove_dir(&unrelated_pending_batch).expect("unrelated pending batch must be cleared");
    let (unchanged, aborted) =
        HarnessExecutionRecord::recover_durable(&engine, "run-durable-lock-failure", None)
            .expect("recovery must prove that the failed batch never mutated source");
    assert_eq!(unchanged, validated);
    assert!(matches!(
        aborted.attempt_state,
        HarnessApplyAttemptState::AbortedBeforeMutation { .. }
    ));
    let (finalized, terminal) =
        HarnessExecutionRecord::apply_durable(&engine, "run-durable-lock-failure", None)
            .expect("a proven pre-mutation batch failure must permit one fresh apply");
    assert_eq!(finalized.state, HarnessExecutionState::Finalized);
    assert!(matches!(
        terminal.attempt_state,
        HarnessApplyAttemptState::AppliedFinalized { .. }
    ));
}

#[cfg(unix)]
#[test]
fn terminal_apply_receipt_completes_only_the_missing_head_cas_and_rejects_divergence() {
    let (workspace, engine, _, _, begun, validated) =
        validated_durable_code_run("run-durable-terminal");
    let (finalized, terminal) =
        HarnessExecutionRecord::apply_durable(&engine, "run-durable-terminal", None)
            .expect("durable apply must finalize");
    let run = workspace
        .0
        .join(".llm-context-vault-harness/runs/run-durable-terminal");
    fs::write(
        run.join("head.json"),
        serde_json::to_vec_pretty(&validated).expect("validated head must serialize"),
    )
    .expect("validated predecessor must simulate the pre-CAS crash");

    let (completed, reloaded_terminal) =
        HarnessExecutionRecord::apply_durable(&engine, "run-durable-terminal", None)
            .expect("terminal receipt must complete only the missing head CAS");
    assert_eq!(completed, finalized);
    assert_eq!(reloaded_terminal, terminal);
    assert_eq!(
        fs::read_to_string(workspace.0.join("src/lib.rs")).expect("source must remain readable"),
        "candidate",
        "terminal re-entry must not mutate source again"
    );

    let mut divergent = validated.clone();
    divergent.sequence += 1;
    refresh_record_digest(&mut divergent);
    fs::write(
        run.join("head.json"),
        serde_json::to_vec_pretty(&divergent).expect("divergent accepted head must serialize"),
    )
    .expect("divergent accepted head must be installed");
    let mut session = NativeFaultSession::new();
    assert!(matches!(
        HarnessExecutionRecord::apply_durable(&engine, "run-durable-terminal", Some(&mut session)),
        Err(HarnessError::PlanDrift { expected, actual })
            if expected == validated.record_digest && actual == divergent.record_digest
    ));
    assert!(matches!(
        HarnessExecutionRecord::open_durable_recovery(
            &workspace.0, "run-durable-terminal", Some(&mut session)
        ),
        Err(HarnessError::PlanDrift { expected, actual })
            if expected == validated.record_digest && actual == divergent.record_digest
    ));
    assert!(session.calls.is_empty());

    fs::write(
        run.join("head.json"),
        serde_json::to_vec_pretty(&begun).expect("divergent head must serialize"),
    )
    .expect("divergent valid head must be installed");
    assert!(matches!(
        HarnessExecutionRecord::apply_durable(&engine, "run-durable-terminal", None),
        Err(HarnessError::PlanDrift { expected, actual })
            if expected == validated.record_digest && actual == begun.record_digest
    ));
}

#[cfg(unix)]
#[test]
fn recovered_terminal_receipt_completes_only_the_missing_head_cas() {
    let (workspace, engine, prepared, raw_prepared, _, validated) =
        validated_durable_code_run("run-durable-recovered");
    let pending =
        HarnessExecutionRecord::persist_pending_apply_for_test(&engine, "run-durable-recovered")
            .expect("pending apply must persist");
    let (_, ordinary_receipt) = validated
        .finalize_for_attempt(
            &engine,
            &raw_prepared,
            &prepared,
            None,
            &pending.attempt_identifier,
        )
        .expect("deterministic apply must complete before the simulated crash");
    let (recovered_record, recovered_receipt) = validated
        .recover_finalize(
            &engine,
            &raw_prepared,
            &prepared,
            &ordinary_receipt.batch_journal_relative_path,
            None,
        )
        .expect("durable completion journal must reproduce a recovered receipt");
    let terminal = pending
        .replace_state(HarnessApplyAttemptState::RecoveredFinalized {
            resulting_record: Box::new(recovered_record.clone()),
            apply_receipt: Box::new(recovered_receipt),
        })
        .expect("recovered terminal attempt must bind");
    terminal
        .validate(&prepared)
        .expect("recovered terminal receipt must validate");
    let run = workspace
        .0
        .join(".llm-context-vault-harness/runs/run-durable-recovered");
    fs::write(
        run.join("apply-attempt.json"),
        serde_json::to_vec_pretty(&terminal).expect("terminal attempt must serialize"),
    )
    .expect("terminal receipt must simulate persistence before head CAS");

    let (completed, reloaded_terminal) =
        HarnessExecutionRecord::recover_durable(&engine, "run-durable-recovered", None)
            .expect("recovery re-entry must complete only the missing head CAS");
    assert_eq!(completed, recovered_record);
    assert_eq!(reloaded_terminal, terminal);
    assert_eq!(
        fs::read_to_string(workspace.0.join("src/lib.rs")).expect("source must remain readable"),
        "candidate"
    );
}

#[cfg(unix)]
#[test]
fn recovery_finalizes_a_source_mutation_interrupted_before_finalization() {
    let (workspace, engine, _, _, _, validated) =
        validated_durable_code_run("run-durable-mutation-crash");
    let (pending, applied) = HarnessExecutionRecord::apply_source_without_finalization_for_test(
        &engine,
        "run-durable-mutation-crash",
    )
    .expect("the injected interruption boundary must stop after the core source apply");
    assert_eq!(
        fs::read_to_string(workspace.0.join("src/lib.rs")).expect("source must remain readable"),
        "candidate"
    );
    assert!(
        workspace
            .0
            .join(&pending.expected_completion_relative_path)
            .is_file(),
        "the durable core completion must survive the interruption"
    );
    assert_eq!(
        applied
            .lifecycle_receipt
            .completion_receipt_relative_path
            .as_deref(),
        Some(pending.expected_completion_relative_path.as_str())
    );
    let persisted_head: HarnessExecutionRecord = decode_current_json(
        &fs::read(
            workspace
                .0
                .join(".llm-context-vault-harness/runs/run-durable-mutation-crash/head.json"),
        )
        .expect("validated predecessor must remain current"),
        "interrupted mutation head",
    )
    .expect("interrupted mutation head must decode");
    assert_eq!(persisted_head, validated);

    let (finalized, terminal) =
        HarnessExecutionRecord::recover_durable(&engine, "run-durable-mutation-crash", None)
            .expect("recovery must replay the core completion and finalize exactly once");
    assert_eq!(finalized.state, HarnessExecutionState::Finalized);
    assert!(matches!(
        terminal.attempt_state,
        HarnessApplyAttemptState::RecoveredFinalized { .. }
    ));
}

#[cfg(unix)]
#[test]
fn failed_terminal_receipt_replacement_preserves_recoverable_predecessor() {
    let (workspace, engine, _, _, _, validated) =
        validated_durable_code_run("run-durable-receipt-replace");
    let (_, pending) = HarnessExecutionRecord::fail_terminal_apply_receipt_replacement_for_test(
        &engine,
        "run-durable-receipt-replace",
    )
    .expect("the injected terminal receipt replacement must fail safely");
    let run = workspace
        .0
        .join(".llm-context-vault-harness/runs/run-durable-receipt-replace");
    let persisted_attempt: HarnessApplyAttemptReceipt = decode_current_json(
        &fs::read(run.join("apply-attempt.json"))
            .expect("the predecessor apply attempt must remain readable"),
        "replacement-failure apply attempt",
    )
    .expect("the predecessor apply attempt must remain valid");
    assert_eq!(persisted_attempt, pending);
    let persisted_head: HarnessExecutionRecord = decode_current_json(
        &fs::read(run.join("head.json")).expect("the predecessor head must remain readable"),
        "replacement-failure head",
    )
    .expect("the predecessor head must remain valid");
    assert_eq!(persisted_head, validated);
    assert_eq!(
        fs::read_to_string(workspace.0.join("src/lib.rs")).expect("source must remain readable"),
        "candidate",
        "source success must not be reported as a durable terminal success"
    );
    assert!(
        workspace
            .0
            .join(&pending.expected_completion_relative_path)
            .is_file(),
        "core completion evidence must remain available for recovery"
    );

    let (finalized, terminal) =
        HarnessExecutionRecord::recover_durable(&engine, "run-durable-receipt-replace", None)
            .expect("the retained predecessor must recover from core completion evidence");
    assert_eq!(finalized.state, HarnessExecutionState::Finalized);
    assert!(matches!(
        terminal.attempt_state,
        HarnessApplyAttemptState::RecoveredFinalized { .. }
    ));
}

#[cfg(unix)]
#[test]
fn any_attempt_batch_artifact_forbids_a_pre_mutation_abort() {
    let (workspace, engine, _, _, _, _) = validated_durable_code_run("run-durable-artifact");
    let pending =
        HarnessExecutionRecord::persist_pending_apply_for_test(&engine, "run-durable-artifact")
            .expect("pending apply must persist");
    fs::create_dir_all(
        workspace
            .0
            .join(".llm-context-vault-harness/batches")
            .join(&pending.expected_batch_identifier),
    )
    .expect("ambiguous batch artifact must be created");
    assert!(
        HarnessExecutionRecord::recover_durable(&engine, "run-durable-artifact", None).is_err(),
        "any exact-attempt batch artifact must force recovery instead of declaring no mutation"
    );
    let persisted: HarnessApplyAttemptReceipt = decode_current_json(
        &fs::read(
            workspace
                .0
                .join(".llm-context-vault-harness/runs/run-durable-artifact/apply-attempt.json"),
        )
        .expect("failed recovery evidence must persist"),
        "failed durable apply attempt",
    )
    .expect("failed apply attempt must decode");
    assert!(matches!(
        persisted.attempt_state,
        HarnessApplyAttemptState::Failure { .. }
    ));
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "current integration test preserves one auditable Harness engine-to-finalization sequence"
)]
fn harness_executes_state_machine_and_applies_the_actual_workspace() {
    let (workspace, engine) = external_engine();
    let resolved = engine
        .resolve(&personal_project_code_request())
        .expect("external code request must resolve");
    let plan =
        HarnessPlan::from_resolved(resolved.clone(), &workspace.0, harness_lifecycle_limits())
            .expect("Harness engine resolution must produce a strict current plan");
    let capabilities = harness_capabilities(&plan);
    let tool_plan = harness_tool_plan(&plan);
    let raw_plan = serde_json::to_vec(&plan).unwrap();
    let raw_tool_plan = tool_plan
        .as_ref()
        .map(|value| serde_json::to_vec(value).unwrap());
    let prepared = PreparedHarnessRun::prepare(
        &engine,
        &raw_plan,
        plan,
        capabilities,
        raw_tool_plan.as_deref(),
        tool_plan.clone(),
    )
    .expect("strict current preparation must succeed");
    let raw_prepared = serde_json::to_vec(&prepared).unwrap();
    let mut execution = HarnessExecutionRecord::begin(
        &engine,
        &raw_prepared,
        &prepared,
        "run-current-integration".to_owned(),
    )
    .expect("current execution must begin through Harness engine");

    let first_invocation = execution.ready_role_invocations[0].clone();
    let mut late_close = completed_role_event(
        &resolved,
        &prepared.role_run,
        &first_invocation,
        Vec::new(),
        "current-late-close",
    );
    let HarnessExecutionEvent::RoleResult { result } = &mut late_close else {
        unreachable!();
    };
    result.lifecycle.closed_at_millis = result
        .lifecycle
        .terminal_at_millis
        .checked_add(prepared.plan.lifecycle.max_role_close_millis)
        .unwrap()
        .checked_add(1)
        .unwrap();
    refresh_role_event_digest(&mut late_close);
    assert!(
        execution
            .advance(&engine, &raw_prepared, &prepared, late_close)
            .is_err(),
        "current must enforce the close and total lifecycle bounds on actual role results"
    );

    loop {
        if let Some(invocation) = execution.ready_role_invocations.first().cloned() {
            execution = execution
                .advance(
                    &engine,
                    &raw_prepared,
                    &prepared,
                    completed_role_event(
                        &resolved,
                        &prepared.role_run,
                        &invocation,
                        Vec::new(),
                        "current",
                    ),
                )
                .expect("current role event must advance Harness engine");
            continue;
        }
        if let Some(invocation) = execution.ready_tool_invocation.clone() {
            execution = execution
                .advance(
                    &engine,
                    &raw_prepared,
                    &prepared,
                    passed_exact_tool_event(&invocation, tool_plan.as_ref().unwrap()),
                )
                .expect("current tool event must advance Harness engine");
            continue;
        }
        break;
    }

    let exact_tool_evidence = tool_plan.as_ref().map(harness_tool_evidence);
    let mut mismatched_tool_evidence = exact_tool_evidence.clone().unwrap();
    mismatched_tool_evidence.checks[0].stdout_digest = "f".repeat(64);
    assert!(
        execution
            .evaluate(
                &engine,
                &raw_prepared,
                &prepared,
                Some(&mismatched_tool_evidence),
            )
            .is_err(),
        "exact current tool evidence must be the same command/output evidence used by Harness engine"
    );
    execution = execution
        .evaluate(
            &engine,
            &raw_prepared,
            &prepared,
            exact_tool_evidence.as_ref(),
        )
        .expect("current execution must evaluate through Harness engine");
    execution = execution
        .validate_evaluation(&engine, &raw_prepared, &prepared)
        .expect("current evaluation must reproduce through Harness engine");
    let core_evaluation_receipt_digest = execution
        .evaluation
        .as_ref()
        .expect("evaluated execution must embed the core evaluation")
        .task_evaluation_receipt_digest
        .clone();
    let orchestration_evaluation_receipt_digest = execution
        .run_evaluation_receipt_digest
        .clone()
        .expect("evaluated execution must bind its orchestration receipt");
    let validation_receipt_digest = execution
        .validation_receipt_digest
        .clone()
        .expect("validated execution must bind its validation receipt");
    assert_ne!(
        orchestration_evaluation_receipt_digest, validation_receipt_digest,
        "evaluation and validation are distinct receipt contracts"
    );

    let mut tampered_candidate = execution.clone();
    tampered_candidate.candidate_digest = Some("0".repeat(64));
    refresh_record_digest(&mut tampered_candidate);
    assert!(
        tampered_candidate
            .validate(&prepared, &raw_prepared)
            .is_err(),
        "a rehashed wrapper candidate must still match the deterministic Harness engine evaluation"
    );

    let mut alternate_raw_prepared = raw_prepared.clone();
    alternate_raw_prepared.push(b' ');
    assert!(
        execution
            .validate(&prepared, &alternate_raw_prepared)
            .is_err(),
        "a normalized-equivalent but unregistered prepared byte stream must be rejected"
    );

    let (finalized, receipt) = execution
        .finalize(&engine, &raw_prepared, &prepared, None)
        .expect("validated current execution must apply through the Harness engine transaction");
    assert_eq!(finalized.state, HarnessExecutionState::Finalized);
    receipt.validate().unwrap();
    assert_eq!(
        receipt.applied_batch.task_evaluation_receipt_digest,
        core_evaluation_receipt_digest
    );
    assert_eq!(receipt.validation_receipt_digest, validation_receipt_digest);
    receipt
        .revalidate_from_disk_and_core(&engine, &raw_prepared, &prepared, &finalized)
        .expect("a genuine finalized current receipt must revalidate after source apply");
    let decoded_receipt: HarnessApplyReceipt = decode_current_json(
        &serde_json::to_vec(&receipt).unwrap(),
        "current apply receipt",
    )
    .expect("the exact current receipt must survive durable reload");
    assert_eq!(decoded_receipt, receipt);
    assert_current_apply_receipt_digest_json(&receipt);

    let mut forged_evaluation_receipt = receipt.clone();
    forged_evaluation_receipt
        .applied_batch
        .task_evaluation_receipt_digest = "a".repeat(64);
    refresh_apply_receipt_digests(&mut forged_evaluation_receipt);
    forged_evaluation_receipt.validate().unwrap();
    assert!(
        forged_evaluation_receipt
            .revalidate_from_disk_and_core(&engine, &raw_prepared, &prepared, &finalized)
            .is_err(),
        "a self-consistent inner batch cannot replace the evaluated receipt reproduced by the durable journal"
    );

    let mut forged_validation_receipt = receipt.clone();
    forged_validation_receipt.validation_receipt_digest = "b".repeat(64);
    refresh_apply_receipt_digests(&mut forged_validation_receipt);
    forged_validation_receipt.validate().unwrap();
    assert!(
        forged_validation_receipt
            .revalidate_from_disk_and_core(&engine, &raw_prepared, &prepared, &finalized)
            .is_err(),
        "a self-consistent outer receipt cannot replace the validation receipt bound by the finalized record"
    );

    let mut legacy_receipt = receipt.clone();
    legacy_receipt.version = HARNESS_SCHEMA_VERSION - 1;
    refresh_apply_receipt_digests(&mut legacy_receipt);
    assert!(matches!(
        legacy_receipt.validate(),
        Err(HarnessError::InvalidPlan(message))
            if message.contains("current apply receipt contract version")
    ));
    let mut legacy_receipt_json = serde_json::to_value(&receipt).unwrap();
    legacy_receipt_json["version"] = serde_json::Value::from(HARNESS_SCHEMA_VERSION - 1);
    let legacy_applied_batch = legacy_receipt_json["applied_batch"]
        .as_object_mut()
        .expect("applied batch must be an object");
    let legacy_inner_receipt = legacy_applied_batch
        .remove("task_evaluation_receipt_digest")
        .expect("current inner receipt must be present");
    legacy_applied_batch.insert("validation_receipt_digest".to_owned(), legacy_inner_receipt);
    assert!(
        decode_current_json::<HarnessApplyReceipt>(
            &serde_json::to_vec(&legacy_receipt_json).unwrap(),
            "legacy apply receipt",
        )
        .is_err(),
        "the old durable apply-envelope shape must not migrate implicitly"
    );

    assert_eq!(
        fs::read_to_string(workspace.0.join("src/lib.rs")).unwrap(),
        "candidate"
    );
    let mut forged_receipt = receipt.clone();
    let forged_journal = ".llm-context-vault-harness/batches/forged/journal.json".to_owned();
    forged_receipt
        .batch_journal_relative_path
        .clone_from(&forged_journal);
    forged_receipt
        .applied_batch
        .lifecycle_receipt
        .journal_relative_path
        .clone_from(&forged_journal);
    refresh_apply_receipt_digests(&mut forged_receipt);
    forged_receipt.validate().unwrap();
    let mut forged_record = execution.clone();
    forged_record.sequence += 1;
    forged_record.predecessor_record_digest = Some(execution.record_digest.clone());
    forged_record.state = HarnessExecutionState::Finalized;
    forged_record.finalization_digest = Some(forged_receipt.finalization_digest.clone());
    forged_record.batch_receipt_digest = Some(forged_receipt.batch_receipt_digest.clone());
    refresh_record_digest(&mut forged_record);
    forged_record.validate(&prepared, &raw_prepared).unwrap();
    assert!(
        forged_receipt
            .revalidate_from_disk_and_core(&engine, &raw_prepared, &prepared, &forged_record)
            .is_err(),
        "self-consistent finalized bytes cannot replace a nonexistent durable Harness engine journal"
    );
    let (recovered, recovered_receipt) = execution
        .recover_finalize(
            &engine,
            &raw_prepared,
            &prepared,
            &receipt.batch_journal_relative_path,
            None,
        )
        .expect("a completed Harness engine journal must resume post-apply current finalization without prestate replay");
    assert_eq!(recovered.state, HarnessExecutionState::Finalized);
    recovered_receipt.validate().unwrap();
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "current index integration test preserves the complete apply-to-scoped-search evidence path"
)]
fn vault_apply_finalizes_common_workspace_evidence() {
    let repository = TemporaryWorkspace::create();
    copy_directory(
        &repository_root().join("vault"),
        &repository.0.join("vault"),
    );
    fs::create_dir(repository.0.join("src")).unwrap();
    fs::write(
        repository.0.join("README.md"),
        "# Repository-only current-index-root-sentinel\n",
    )
    .unwrap();
    let engine = HarnessEngine::open(&repository.0, &repository.0).unwrap();
    let request = HarnessRequest {
        action: HarnessAction::VaultCuration,
        owner: DataOwner::Personal,
        targets: vec!["vault/personal/knowledge/current-index-scope.md".to_owned()],
        objective: "curate a scoped index fixture".to_owned(),
        curation_kind: Some(CurationKind::Knowledge),
        explicit_user_confirmation_reported: true,
        curation_sources: vec!["vault/personal/index.md".to_owned()],
        delete_targets: Vec::new(),
    };
    let resolved = engine.resolve(&request).unwrap();
    let plan =
        HarnessPlan::from_resolved(resolved.clone(), &repository.0, harness_lifecycle_limits())
            .unwrap();
    let capabilities = harness_capabilities(&plan);
    let tool_plan = harness_tool_plan(&plan);
    let raw_plan = serde_json::to_vec(&plan).unwrap();
    let raw_tool_plan = tool_plan
        .as_ref()
        .map(|value| serde_json::to_vec(value).unwrap());
    let prepared = PreparedHarnessRun::prepare(
        &engine,
        &raw_plan,
        plan,
        capabilities,
        raw_tool_plan.as_deref(),
        tool_plan.clone(),
    )
    .unwrap();
    let raw_prepared = serde_json::to_vec(&prepared).unwrap();
    let mut execution = HarnessExecutionRecord::begin(
        &engine,
        &raw_prepared,
        &prepared,
        "run-current-index-scope".to_owned(),
    )
    .unwrap();
    loop {
        if let Some(invocation) = execution.ready_role_invocations.first().cloned() {
            execution = execution
                .advance(
                    &engine,
                    &raw_prepared,
                    &prepared,
                    completed_role_event(
                        &resolved,
                        &prepared.role_run,
                        &invocation,
                        Vec::new(),
                        "current-index",
                    ),
                )
                .unwrap();
            continue;
        }
        if let Some(invocation) = execution.ready_tool_invocation.clone() {
            execution = execution
                .advance(
                    &engine,
                    &raw_prepared,
                    &prepared,
                    passed_exact_tool_event(&invocation, tool_plan.as_ref().unwrap()),
                )
                .unwrap();
            continue;
        }
        break;
    }
    let exact_tool_evidence = tool_plan.as_ref().map(harness_tool_evidence);
    execution = execution
        .evaluate(
            &engine,
            &raw_prepared,
            &prepared,
            exact_tool_evidence.as_ref(),
        )
        .unwrap();
    execution = execution
        .validate_evaluation(&engine, &raw_prepared, &prepared)
        .unwrap();
    execution
        .preflight_finalize(&engine, &raw_prepared, &prepared, None)
        .unwrap();
    let (finalized, receipt) = execution
        .finalize(&engine, &raw_prepared, &prepared, None)
        .unwrap();
    assert_eq!(finalized.state, HarnessExecutionState::Finalized);
    assert!(receipt.context_commit_receipt.is_none());
    receipt
        .revalidate_from_disk_and_core(&engine, &raw_prepared, &prepared, &finalized)
        .unwrap();
    assert_eq!(receipt.final_workspace.content_states.len(), 1);
    assert_eq!(
        fs::read_to_string(
            repository
                .0
                .join("vault/personal/knowledge/current-index-scope.md")
        )
        .unwrap(),
        "candidate"
    );
    assert!(
        !receipt
            .final_workspace
            .content_states
            .contains_key("README.md")
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "promotion handoff integration preserves candidate, approval, apply, index, and next-read evidence in one scenario"
)]
fn approved_promotion_is_saved_and_found_by_the_next_core_read() {
    let repository = TemporaryWorkspace::create();
    copy_directory(
        &repository_root().join("vault"),
        &repository.0.join("vault"),
    );
    fs::create_dir_all(repository.0.join("vault/index")).unwrap();
    let engine = HarnessEngine::open(&repository.0, &repository.0).unwrap();
    let (source_resolved, source_prepared, source_evaluation) =
        promotion_source_evaluation(&engine, false);
    let handoff = PromotionHandoff::from_evaluation(
        &source_resolved.plan,
        &source_prepared,
        &[],
        &source_evaluation,
        "proposal-0000000000000003",
    )
    .expect("accepted source evaluation must create a handoff");
    let envelope = promotion_curation_envelope(
        handoff.clone(),
        DataOwner::Personal,
        CurationKind::Fact,
        "vault/personal/facts/promotion-flow.md",
        true,
    );
    let resolved = engine
        .resolve_envelope(&envelope, None)
        .expect("exactly confirmed handoff must resolve as separate curation");
    let plan =
        HarnessPlan::from_resolved(resolved.clone(), &repository.0, harness_lifecycle_limits())
            .unwrap();
    let capabilities = harness_capabilities(&plan);
    let tool_plan = harness_tool_plan(&plan);
    let raw_plan = serde_json::to_vec(&plan).unwrap();
    let raw_tool_plan = tool_plan
        .as_ref()
        .map(|value| serde_json::to_vec(value).unwrap());
    let prepared = PreparedHarnessRun::prepare(
        &engine,
        &raw_plan,
        plan,
        capabilities,
        raw_tool_plan.as_deref(),
        tool_plan.clone(),
    )
    .unwrap();
    let raw_prepared = serde_json::to_vec(&prepared).unwrap();
    let mut execution = HarnessExecutionRecord::begin(
        &engine,
        &raw_prepared,
        &prepared,
        "run-promotion-handoff".to_owned(),
    )
    .unwrap();
    loop {
        if let Some(invocation) = execution.ready_role_invocations.first().cloned() {
            let mut event = completed_role_event(
                &resolved,
                &prepared.role_run,
                &invocation,
                Vec::new(),
                "promotion-handoff",
            );
            if invocation.role == HarnessRole::Writer {
                let HarnessExecutionEvent::RoleResult { result } = &mut event else {
                    unreachable!("Writer event must contain a role result");
                };
                let RoleExecutionOutcome::Completed {
                    result:
                        CompletedRoleResult::Writer {
                            artifact: WriterArtifact::Curation { entries, .. },
                        },
                } = &mut result.outcome
                else {
                    unreachable!("promotion Writer must produce curation entries");
                };
                let FileChange::Create { content, .. } = &mut entries[0].change else {
                    unreachable!("promotion fixture must create a new memory file");
                };
                content.clone_from(&handoff.proposal.content);
                refresh_role_event_digest(&mut event);
            }
            execution = execution
                .advance(&engine, &raw_prepared, &prepared, event)
                .unwrap();
            continue;
        }
        if let Some(invocation) = execution.ready_tool_invocation.clone() {
            execution = execution
                .advance(
                    &engine,
                    &raw_prepared,
                    &prepared,
                    passed_exact_tool_event(&invocation, tool_plan.as_ref().unwrap()),
                )
                .unwrap();
            continue;
        }
        break;
    }
    let exact_tool_evidence = tool_plan.as_ref().map(harness_tool_evidence);
    execution = execution
        .evaluate(
            &engine,
            &raw_prepared,
            &prepared,
            exact_tool_evidence.as_ref(),
        )
        .unwrap();
    execution = execution
        .validate_evaluation(&engine, &raw_prepared, &prepared)
        .unwrap();
    execution
        .finalize(&engine, &raw_prepared, &prepared, None)
        .expect("approved memory must apply and finalize its common workspace evidence");

    let saved_path = repository.0.join("vault/personal/facts/promotion-flow.md");
    assert_eq!(
        fs::read_to_string(&saved_path).unwrap(),
        handoff.proposal.content
    );
    let read_request = HarnessRequest {
        action: HarnessAction::VaultRead,
        owner: DataOwner::Personal,
        targets: Vec::new(),
        objective: "승격된 장기기억을 다시 조회한다".to_owned(),
        curation_kind: Some(CurationKind::Fact),
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let read = resolve_vault_read(
        &engine,
        &read_request,
        HarnessExecutionProfile::Standard,
        "promotionloopsearchtoken",
        64 * 1024,
    );
    let bundle = engine
        .retrieve_context(&read, "promotionloopsearchtoken", 64 * 1024)
        .expect("the next Vault read must find the saved memory");
    assert!(bundle.documents.iter().any(|document| {
        document.repository_relative_path == "vault/personal/facts/promotion-flow.md"
            && document.content.contains("promotionloopsearchtoken")
    }));
}

#[test]
fn tool_plan_rejects_a_different_frozen_workspace() {
    let (first_workspace, first_engine) = external_engine();
    let (second_workspace, second_engine) = external_engine();
    let first = first_engine
        .resolve(&personal_project_code_request())
        .unwrap();
    let second = second_engine
        .resolve(&personal_project_code_request())
        .unwrap();
    let first_plan =
        HarnessPlan::from_resolved(first, &first_workspace.0, harness_lifecycle_limits()).unwrap();
    let second_plan =
        HarnessPlan::from_resolved(second, &second_workspace.0, harness_lifecycle_limits())
            .unwrap();
    let tool_plan = harness_tool_plan(&first_plan).expect("code writes require tool checks");
    assert_eq!(tool_plan.checks.len(), 2);
    assert_eq!(
        tool_plan.checks[0].verification_requirement,
        tool_plan.checks[1].verification_requirement
    );
    assert_ne!(tool_plan.checks[0].argv, tool_plan.checks[1].argv);
    tool_plan
        .validate_for(&first_plan)
        .expect("one Tool-owned unit must accept multiple distinct exact checks");
    assert!(tool_plan.validate_for(&second_plan).is_err());
    let mut duplicated = tool_plan.clone();
    let mut duplicate = duplicated.checks[0].clone();
    duplicate.check_identifier = "duplicate-check".to_owned();
    duplicated.checks.push(duplicate);
    duplicated
        .checks
        .sort_by(|left, right| left.check_identifier.cmp(&right.check_identifier));
    duplicated.tool_plan_digest = serialized_digest(&(
        duplicated.version,
        &duplicated.resolved_plan_digest,
        &duplicated.workspace_root_identity,
        &duplicated.checks,
    ))
    .unwrap();
    assert!(
        duplicated.validate_for(&first_plan).is_err(),
        "a duplicate exact recipe must not collapse through set comparison"
    );
}

#[test]
fn one_failed_check_fails_the_single_aggregated_tool_requirement() {
    let (_workspace, engine, resolved, prepared, raw_prepared, tool_plan) = durable_code_fixture();
    let tool_plan = tool_plan.expect("code writes require an exact tool plan");
    let mut execution = HarnessExecutionRecord::begin(
        &engine,
        &raw_prepared,
        &prepared,
        "run-multi-check-failure".to_owned(),
    )
    .unwrap();
    while let Some(invocation) = execution.ready_role_invocations.first().cloned() {
        execution = execution
            .advance(
                &engine,
                &raw_prepared,
                &prepared,
                completed_role_event(
                    &resolved,
                    &prepared.role_run,
                    &invocation,
                    Vec::new(),
                    "multi-check-failure",
                ),
            )
            .unwrap();
    }
    let invocation = execution
        .ready_tool_invocation
        .clone()
        .expect("tool invocation must be ready");
    let mut exact = harness_tool_evidence(&tool_plan);
    assert_eq!(exact.checks.len(), 2);
    exact.checks[1].exit_code = 1;
    exact.checks[1].semantic_result = "failed".to_owned();
    execution = execution
        .advance(
            &engine,
            &raw_prepared,
            &prepared,
            exact_tool_event(&invocation, &exact),
        )
        .expect("aggregate failed tool evidence must advance");
    execution = execution
        .evaluate(&engine, &raw_prepared, &prepared, Some(&exact))
        .expect("aggregate failed tool evidence must evaluate");
    let evaluation = execution.evaluation.expect("evaluation must be present");
    let tool_requirement = evaluation
        .requirements
        .iter()
        .find(|result| result.requirement.unit == VerificationUnit::TestsAndStaticAnalysis)
        .expect("aggregated tool requirement must be evaluated");
    assert!(!tool_requirement.passed);
    assert_eq!(tool_requirement.evidence.len(), 2);
    assert_eq!(
        evaluation.completion_state,
        HarnessCompletionState::RevisionRequired
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "current revision test preserves one auditable immutable history sequence"
)]
fn revision_is_a_deterministic_history_chain_with_fresh_contexts() {
    let (workspace, engine) = external_engine();
    let resolved = engine.resolve(&personal_project_code_request()).unwrap();
    let plan =
        HarnessPlan::from_resolved(resolved.clone(), &workspace.0, harness_lifecycle_limits())
            .unwrap();
    let capabilities = harness_capabilities(&plan);
    let tool_plan = harness_tool_plan(&plan);
    let raw_plan = serde_json::to_vec(&plan).unwrap();
    let raw_tool_plan = tool_plan
        .as_ref()
        .map(|value| serde_json::to_vec(value).unwrap());
    let prepared = PreparedHarnessRun::prepare(
        &engine,
        &raw_plan,
        plan,
        capabilities,
        raw_tool_plan.as_deref(),
        tool_plan.clone(),
    )
    .unwrap();
    let raw_prepared = serde_json::to_vec(&prepared).unwrap();
    let mut execution = HarnessExecutionRecord::begin(
        &engine,
        &raw_prepared,
        &prepared,
        "run-current-revision".to_owned(),
    )
    .unwrap();

    loop {
        if let Some(invocation) = execution.ready_role_invocations.first().cloned() {
            let findings = (invocation.role == HarnessRole::Reviewer)
                .then(|| BlockingFinding {
                    message: "candidate requires correction".to_owned(),
                    evidence: vec![subject_evidence(&resolved.plan, &invocation.subject)],
                })
                .into_iter()
                .collect();
            execution = execution
                .advance(
                    &engine,
                    &raw_prepared,
                    &prepared,
                    completed_role_event(
                        &resolved,
                        &prepared.role_run,
                        &invocation,
                        findings,
                        "current-first",
                    ),
                )
                .unwrap();
            continue;
        }
        if let Some(invocation) = execution.ready_tool_invocation.clone() {
            execution = execution
                .advance(
                    &engine,
                    &raw_prepared,
                    &prepared,
                    passed_exact_tool_event(&invocation, tool_plan.as_ref().unwrap()),
                )
                .unwrap();
            continue;
        }
        break;
    }
    let tool_evidence = tool_plan.as_ref().map(harness_tool_evidence);
    execution = execution
        .evaluate(&engine, &raw_prepared, &prepared, tool_evidence.as_ref())
        .unwrap();
    assert_eq!(
        execution.evaluation.as_ref().unwrap().completion_state,
        HarnessCompletionState::RevisionRequired
    );

    execution = execution
        .revise(&engine, &raw_prepared, &prepared)
        .expect("rejected current write must begin a Harness engine-derived revision");
    assert_eq!(execution.revision_history.len(), 1);
    assert_eq!(execution.role_execution.revision_contract.revision, 1);

    loop {
        if let Some(invocation) = execution.ready_role_invocations.first().cloned() {
            execution = execution
                .advance(
                    &engine,
                    &raw_prepared,
                    &prepared,
                    completed_role_event(
                        &resolved,
                        &prepared.role_run,
                        &invocation,
                        Vec::new(),
                        "current-second",
                    ),
                )
                .unwrap();
            continue;
        }
        if let Some(invocation) = execution.ready_tool_invocation.clone() {
            execution = execution
                .advance(
                    &engine,
                    &raw_prepared,
                    &prepared,
                    passed_exact_tool_event(&invocation, tool_plan.as_ref().unwrap()),
                )
                .unwrap();
            continue;
        }
        break;
    }
    execution = execution
        .evaluate(&engine, &raw_prepared, &prepared, tool_evidence.as_ref())
        .unwrap();
    execution
        .validate_evaluation(&engine, &raw_prepared, &prepared)
        .expect("fresh-context revision must validate through Harness engine");
}

/// Metadata is frozen once from an owned test fixture; provider calls never enumerate/read bodies.
struct NativeBoundarySource {
    filesystem: FilesystemContextSource,
    versions: BTreeMap<String, SourceVersion>,
}
impl NativeBoundarySource {
    fn fixture(root: &Path) -> Self {
        fn collect(root: &Path, directory: &Path, versions: &mut BTreeMap<String, SourceVersion>) {
            for entry in fs::read_dir(directory).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if entry.file_type().unwrap().is_dir() {
                    collect(root, &path, versions);
                } else if entry.file_type().unwrap().is_file() {
                    let logical_path = path
                        .strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_owned();
                    let state = StoredSourceState::Live {
                        material_id: format!("material-{}", byte_digest(logical_path.as_bytes())),
                        revision: 1,
                        content_digest: byte_digest(&fs::read(&path).unwrap()),
                    };
                    versions.insert(
                        logical_path.clone(),
                        SourceVersion {
                            logical_path,
                            state,
                        },
                    );
                }
            }
        }
        let mut versions = BTreeMap::new();
        collect(root, root, &mut versions);
        Self {
            filesystem: FilesystemContextSource::open(root).unwrap(),
            versions,
        }
    }
}
impl ContextSource for NativeBoundarySource {
    fn view_root(&self) -> &Path {
        self.filesystem.view_root()
    }
    fn store_identity(&self) -> HarnessResult<Option<SourceStoreIdentity>> {
        Ok(Some(SourceStoreIdentity {
            store_id: "boundary-test-store".into(),
        }))
    }
    fn metadata(&self, path: &Path) -> HarnessResult<SourceMetadata> {
        let mut metadata = self.filesystem.metadata(path)?;
        let logical_path = path.to_str().unwrap().to_owned();
        metadata.stored_version = match metadata.kind {
            SourcePathKind::RegularFile => Some(
                self.versions
                    .get(&logical_path)
                    .expect("only original fixture files are stored sources")
                    .clone(),
            ),
            SourcePathKind::Missing => Some(SourceVersion {
                logical_path,
                state: StoredSourceState::Missing,
            }),
            _ => None,
        };
        Ok(metadata)
    }
    fn children(&self, path: &Path, max: usize) -> HarnessResult<Vec<OsString>> {
        self.filesystem.children(path, max)
    }
    fn open_file(&self, path: &Path, max: u64) -> HarnessResult<File> {
        self.filesystem.open_file(path, max)
    }
    fn validate_regular_file(&self, path: &Path) -> HarnessResult<SourceMetadata> {
        self.filesystem.validate_regular_file(path)?;
        self.metadata(path)
    }
}

#[derive(Default)]
struct NativeFaultSession {
    identity: Option<SourceStoreIdentity>,
    calls: Vec<&'static str>,
    allow_begin: bool,
    recovery: Option<ContextRecoveryStatus>,
    fail_abort: bool,
    last_contract: Option<ContextApplyContract>,
    reject_invalid_receipts: bool,
    abort_attempt_path: Option<PathBuf>,
}
impl NativeFaultSession {
    fn new() -> Self {
        Self {
            identity: Some(SourceStoreIdentity {
                store_id: "boundary-test-store".into(),
            }),
            ..Self::default()
        }
    }
}
impl ContextCommitSession for NativeFaultSession {
    fn store_identity(&self) -> &SourceStoreIdentity {
        self.identity.as_ref().unwrap()
    }
    fn begin(&mut self, contract: &ContextApplyContract) -> HarnessResult<()> {
        self.calls.push("begin");
        self.last_contract = Some(contract.clone());
        if self.allow_begin {
            Ok(())
        } else {
            Err(HarnessError::InvalidRepository(
                "injected begin failure".into(),
            ))
        }
    }
    fn recover(
        &mut self,
        contract: &ContextRecoveryContract,
    ) -> HarnessResult<ContextRecoveryStatus> {
        self.calls.push("recover");
        self.last_contract = Some(contract.apply().clone());
        Ok(self
            .recovery
            .clone()
            .unwrap_or(if contract.apply().requires_commit() {
                ContextRecoveryStatus::Pending
            } else {
                ContextRecoveryStatus::NotRequired
            }))
    }
    fn commit(
        &mut self,
        verified: &VerifiedContextCommit<'_>,
    ) -> HarnessResult<ContextCommitReceipt> {
        self.calls.push("commit");
        assert!(verified.contract().requires_commit());
        for target in verified.contract().targets() {
            assert_eq!(
                verified
                    .target_content(&target.previous().logical_path)
                    .map(byte_digest)
                    .as_deref(),
                target.resulting_content_digest()
            );
        }
        if self.reject_invalid_receipts {
            let contract = verified.contract();
            let target = &contract.targets()[0];
            let previous = vec![target.previous().clone()];
            assert!(
                ContextCommitReceipt::from_persisted(verified, vec![], vec![], "0".repeat(64))
                    .is_err()
            );
            for (revision, digest) in [
                (2, target.resulting_content_digest().unwrap().to_owned()),
                (1, "0".repeat(64)),
            ] {
                let result = SourceVersion {
                    logical_path: target.previous().logical_path.clone(),
                    state: StoredSourceState::Live {
                        material_id: "new-material".into(),
                        revision,
                        content_digest: digest,
                    },
                };
                assert!(
                    ContextCommitReceipt::from_persisted(
                        verified,
                        previous.clone(),
                        vec![result],
                        "0".repeat(64)
                    )
                    .is_err()
                );
            }
            let wrong = SourceVersion {
                logical_path: "vault/personal/knowledge/extra.md".into(),
                state: StoredSourceState::Live {
                    material_id: "new-material".into(),
                    revision: 1,
                    content_digest: target.resulting_content_digest().unwrap().into(),
                },
            };
            assert!(
                ContextCommitReceipt::from_persisted(
                    verified,
                    previous,
                    vec![wrong],
                    "0".repeat(64)
                )
                .is_err()
            );
        }
        // No test manufactures a successful database commit or positive persistence receipt.
        Err(HarnessError::InvalidRepository(
            "injected uncertain commit response".into(),
        ))
    }
    fn finalize(&mut self, _proof: &ContextTerminalProof) -> HarnessResult<()> {
        self.calls.push("finalize");
        Err(HarnessError::InvalidRepository(
            "unexpected terminal cleanup".into(),
        ))
    }
    fn abort(&mut self, proof: &ContextAbortProof) -> HarnessResult<()> {
        self.calls.push("abort");
        if let Some(path) = &self.abort_attempt_path {
            let persisted: HarnessApplyAttemptReceipt = decode_current_json(
                &fs::read(path).expect("abort proof must already be persisted"),
                "persisted abort proof",
            )
            .expect("persisted abort proof must decode");
            assert_eq!(&persisted, proof.persisted_attempt());
        }
        assert!(matches!(
            proof.persisted_attempt().attempt_state,
            HarnessApplyAttemptState::AbortedBeforeMutation { .. }
                | HarnessApplyAttemptState::AbortedAfterRollback { .. }
        ));
        if self.fail_abort {
            Err(HarnessError::InvalidRepository(
                "injected abort cleanup failure".into(),
            ))
        } else {
            Ok(())
        }
    }
}

#[cfg(unix)]
fn native_boundary_run(
    run: &str,
) -> (
    TemporaryWorkspace,
    HarnessEngine,
    PreparedHarnessRun,
    Vec<u8>,
    HarnessExecutionRecord,
) {
    native_boundary_run_with_targets(
        run,
        vec!["vault/personal/knowledge/native-effect-test.md".into()],
        |_| {},
    )
}

#[cfg(unix)]
fn native_boundary_run_with_targets(
    run: &str,
    targets: Vec<String>,
    setup: impl FnOnce(&Path),
) -> (
    TemporaryWorkspace,
    HarnessEngine,
    PreparedHarnessRun,
    Vec<u8>,
    HarnessExecutionRecord,
) {
    let repository = TemporaryWorkspace::create();
    copy_directory(
        &repository_root().join("vault"),
        &repository.0.join("vault"),
    );
    fs::create_dir(repository.0.join("src")).unwrap();
    setup(&repository.0);
    let source = std::sync::Arc::new(NativeBoundarySource::fixture(&repository.0));
    let engine = HarnessEngine::with_source(&repository.0, &repository.0, source).unwrap();
    let is_curation = targets.len() == 1;
    let request = HarnessRequest {
        action: if is_curation {
            HarnessAction::VaultCuration
        } else {
            HarnessAction::DocumentWrite
        },
        owner: DataOwner::Personal,
        targets,
        objective: "curate a native effect boundary fixture".into(),
        curation_kind: is_curation.then_some(CurationKind::Knowledge),
        explicit_user_confirmation_reported: is_curation,
        curation_sources: if is_curation {
            vec!["vault/personal/index.md".into()]
        } else {
            vec![]
        },
        delete_targets: vec![],
    };
    let resolved = engine.resolve(&request).unwrap();
    let (prepared, raw) = prepare_current_harness_run(&engine, &repository.0, &resolved);
    let mut head = HarnessExecutionRecord::begin_durable(&engine, &raw, &prepared, run).unwrap();
    loop {
        if let Some(invocation) = head.ready_role_invocations.first().cloned() {
            head = HarnessExecutionRecord::advance_durable(
                &engine,
                run,
                completed_role_event(&resolved, &prepared.role_run, &invocation, vec![], run),
            )
            .unwrap();
            continue;
        }
        if let Some(invocation) = head.ready_tool_invocation.clone() {
            head = HarnessExecutionRecord::advance_durable(
                &engine,
                run,
                passed_exact_tool_event(&invocation, prepared.accepted_tool_plan.as_ref().unwrap()),
            )
            .unwrap();
            continue;
        }
        break;
    }
    let evidence = prepared
        .accepted_tool_plan
        .as_ref()
        .map(harness_tool_evidence);
    HarnessExecutionRecord::evaluate_durable(&engine, run, evidence.as_ref()).unwrap();
    let head = HarnessExecutionRecord::validate_durable(&engine, run).unwrap();
    (repository, engine, prepared, raw, head)
}

#[cfg(unix)]
#[test]
fn native_missing_provider_and_public_bypasses_fail_before_mutation() {
    let (repository, engine, prepared, raw, head) = native_boundary_run("native-missing-provider");
    assert!(
        HarnessExecutionRecord::apply_durable(&engine, "native-missing-provider", None).is_err()
    );
    assert!(
        !repository
            .0
            .join(".llm-context-vault-harness/runs/native-missing-provider/apply-attempt.json")
            .exists()
    );
    let mut session = NativeFaultSession::new();
    assert!(
        head.finalize(&engine, &raw, &prepared, Some(&mut session))
            .is_err()
    );
    assert!(
        engine
            .apply_validated_execution(
                &prepared.plan.resolved_request,
                &prepared.role_run,
                &head.revision_history,
                &head.role_execution
            )
            .is_err()
    );
    assert!(
        engine
            .apply_validated_execution_for_attempt(
                &prepared.plan.resolved_request,
                &prepared.role_run,
                &head.revision_history,
                &head.role_execution,
                "attempt-bypass"
            )
            .is_err()
    );
    assert!(
        engine
            .recover_apply_batch(".llm-context-vault-harness/batches/bypass/journal.json")
            .is_err()
    );
    assert!(session.calls.is_empty());
    assert!(
        !repository
            .0
            .join("vault/personal/knowledge/native-effect-test.md")
            .exists()
    );
}

#[cfg(unix)]
#[test]
fn native_source_root_and_missing_target_bindings_fail_closed() {
    let (repository, engine, prepared, raw, head) = native_boundary_run("native-binding-tamper");
    let mut session = NativeFaultSession::new();
    for root in [None, Some("foreign-root".to_owned())] {
        let mut changed = prepared.clone();
        changed.role_run.source_versions.source_root_identity = root;
        assert!(
            head.preflight_finalize(&engine, &raw, &changed, Some(&session))
                .is_err()
        );
    }
    let mut changed = prepared.clone();
    changed
        .plan
        .resolved_request
        .plan
        .source_versions
        .versions
        .retain(|version| version.logical_path != "vault/personal/knowledge/native-effect-test.md");
    assert!(persistence::native_target_paths(&changed).is_err());
    session.identity = Some(SourceStoreIdentity {
        store_id: "foreign-store".into(),
    });
    assert!(
        HarnessExecutionRecord::apply_durable(&engine, "native-binding-tamper", Some(&mut session))
            .is_err()
    );
    assert!(session.calls.is_empty());
    assert!(
        !repository
            .0
            .join("vault/personal/knowledge/native-effect-test.md")
            .exists()
    );
}

#[cfg(unix)]
#[test]
fn native_begin_failure_preserves_files_and_abort_cleanup_requires_durable_proof() {
    let run = "native-begin-failure";
    let (repository, engine, _, _, head) = native_boundary_run(run);
    let mut session = NativeFaultSession::new();
    assert!(HarnessExecutionRecord::apply_durable(&engine, run, Some(&mut session)).is_err());
    assert_eq!(session.calls, vec!["begin"]);
    assert!(
        !repository
            .0
            .join("vault/personal/knowledge/native-effect-test.md")
            .exists()
    );
    session.fail_abort = true;
    assert!(
        HarnessExecutionRecord::open_durable_recovery(&repository.0, run, Some(&mut session))
            .unwrap()
            .finish(&engine)
            .is_err()
    );
    let path = repository.0.join(format!(
        ".llm-context-vault-harness/runs/{run}/apply-attempt.json"
    ));
    let aborted: HarnessApplyAttemptReceipt =
        decode_current_json(&fs::read(&path).unwrap(), "abort proof").unwrap();
    assert!(matches!(
        aborted.attempt_state,
        HarnessApplyAttemptState::AbortedBeforeMutation { .. }
    ));
    session.fail_abort = false;
    let (unchanged, again) =
        HarnessExecutionRecord::open_durable_recovery(&repository.0, run, Some(&mut session))
            .unwrap()
            .finish(&engine)
            .unwrap();
    assert_eq!(unchanged, head);
    assert_eq!(again, aborted);
    assert_eq!(
        session.calls,
        vec!["begin", "recover", "abort", "recover", "abort"]
    );
}

#[cfg(unix)]
#[test]
fn native_forged_foreign_and_stale_durable_records_fail_before_provider_io() {
    let run = "native-forged-durable";
    let (repository, engine, _, _, _) = native_boundary_run(run);
    HarnessExecutionRecord::persist_pending_apply_for_test(&engine, run).unwrap();
    let mut session = NativeFaultSession::new();
    let foreign = TemporaryWorkspace::create();
    copy_directory(
        &repository.0.join(".llm-context-vault-harness"),
        &foreign.0.join(".llm-context-vault-harness"),
    );
    assert!(
        HarnessExecutionRecord::open_durable_recovery(&foreign.0, run, Some(&mut session)).is_err()
    );
    let path = repository.0.join(format!(
        ".llm-context-vault-harness/runs/{run}/apply-attempt.json"
    ));
    let original = fs::read(&path).unwrap();
    for field in [
        "validated_record_digest",
        "candidate_digest",
        "resolved_plan_digest",
        "receipt_digest",
    ] {
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        value[field] = serde_json::json!("f".repeat(64));
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(
            HarnessExecutionRecord::open_durable_recovery(&repository.0, run, Some(&mut session))
                .is_err()
        );
    }
    fs::write(&path, original).unwrap();
    assert!(session.calls.is_empty());
}

#[cfg(unix)]
#[test]
fn native_uncertain_commit_keeps_pending_and_wrong_committed_evidence_cannot_rollback() {
    let run = "native-commit-response-loss";
    let (repository, engine, _, _, _) = native_boundary_run(run);
    let mut session = NativeFaultSession::new();
    session.allow_begin = true;
    assert!(HarnessExecutionRecord::apply_durable(&engine, run, Some(&mut session)).is_err());
    assert_eq!(session.calls, vec!["begin", "commit"]);
    let path = repository
        .0
        .join("vault/personal/knowledge/native-effect-test.md");
    assert_eq!(fs::read_to_string(&path).unwrap(), "candidate");
    let attempt_path = repository.0.join(format!(
        ".llm-context-vault-harness/runs/{run}/apply-attempt.json"
    ));
    let attempt: HarnessApplyAttemptReceipt =
        decode_current_json(&fs::read(&attempt_path).unwrap(), "pending finalization").unwrap();
    assert!(matches!(
        attempt.attempt_state,
        HarnessApplyAttemptState::SourceAppliedFinalizationPending { .. }
    ));
    let invalid:ContextCommitReceipt=serde_json::from_value(serde_json::json!({
        "version":HARNESS_SCHEMA_VERSION,"store_identity":{"store_id":"foreign-store"},"contract_digest":"0".repeat(64),
        "attempt_identifier":"attempt-foreign","batch_identifier":"batch-foreign","journal_relative_path":"foreign/journal.json",
        "observed_batch_digest":"0".repeat(64),"previous":[],"resulting":[],"projection_metadata_digest":"0".repeat(64),"receipt_digest":"0".repeat(64)
    })).unwrap();
    session.recovery = Some(ContextRecoveryStatus::Committed(invalid));
    assert!(
        HarnessExecutionRecord::open_durable_recovery(&repository.0, run, Some(&mut session))
            .is_err()
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "candidate");
    let unchanged: HarnessApplyAttemptReceipt =
        decode_current_json(&fs::read(&attempt_path).unwrap(), "unchanged pending proof").unwrap();
    assert_eq!(unchanged, attempt);
    assert!(!session.calls.contains(&"finalize"));
    assert!(!session.calls.contains(&"abort"));
}

#[cfg(unix)]
#[test]
fn native_external_vault_path_collision_requires_no_context_commit() {
    let source_root = TemporaryWorkspace::create();
    copy_directory(
        &repository_root().join("vault"),
        &source_root.0.join("vault"),
    );
    fs::write(
        source_root.0.join("vault/profile/collision.rs"),
        "pub fn stored_target() {}\n",
    )
    .expect("stored collision target must exist before binding source versions");
    let workspace = TemporaryWorkspace::create();
    fs::create_dir(workspace.0.join("src")).unwrap();
    fs::create_dir_all(workspace.0.join("vault/profile")).unwrap();
    fs::write(
        workspace.0.join("vault/profile/collision.rs"),
        "pub fn external_target() {}\n",
    )
    .unwrap();
    let engine = HarnessEngine::with_source(
        &source_root.0,
        &workspace.0,
        std::sync::Arc::new(NativeBoundarySource::fixture(&source_root.0)),
    )
    .unwrap();
    let mut request = personal_project_code_request();
    request.targets = vec!["vault/profile/collision.rs".into()];
    let resolved = engine.resolve(&request).unwrap();
    let (prepared, raw) = prepare_current_harness_run(&engine, &workspace.0, &resolved);
    assert!(
        persistence::native_target_paths(&prepared)
            .unwrap()
            .is_empty()
    );
    let head = execute_current_harness_to_validation(
        &engine,
        &resolved,
        &prepared,
        &raw,
        "native-external-collision",
    );
    let (finalized, receipt) = head.finalize(&engine, &raw, &prepared, None).unwrap();
    assert_eq!(finalized.state, HarnessExecutionState::Finalized);
    assert!(receipt.context_commit_receipt.is_none());
    assert_eq!(
        fs::read_to_string(workspace.0.join("vault/profile/collision.rs")).unwrap(),
        "candidate"
    );
}

#[cfg(unix)]
#[test]
fn native_invalid_final_bytes_prevent_commit_and_preserve_pending_evidence() {
    let run = "native-final-byte-drift";
    let (repository, engine, _, _, _) = native_boundary_run(run);
    let mut session = NativeFaultSession::new();
    session.allow_begin = true;
    assert!(HarnessExecutionRecord::apply_durable(&engine, run, Some(&mut session)).is_err());
    let path = repository
        .0
        .join("vault/personal/knowledge/native-effect-test.md");
    fs::write(&path, "unexpected concurrent bytes").unwrap();
    assert!(
        HarnessExecutionRecord::open_durable_recovery(&repository.0, run, Some(&mut session))
            .unwrap()
            .finish(&engine)
            .is_err()
    );
    assert_eq!(
        session
            .calls
            .iter()
            .filter(|call| **call == "commit")
            .count(),
        1
    );
    assert!(!session.calls.contains(&"finalize"));
    assert!(!session.calls.contains(&"abort"));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "unexpected concurrent bytes"
    );
    let attempt: HarnessApplyAttemptReceipt = decode_current_json(
        &fs::read(repository.0.join(format!(
            ".llm-context-vault-harness/runs/{run}/apply-attempt.json"
        )))
        .unwrap(),
        "retained pending evidence",
    )
    .unwrap();
    assert!(matches!(
        attempt.attempt_state,
        HarnessApplyAttemptState::SourceAppliedFinalizationPending { .. }
    ));
}

#[cfg(unix)]
#[test]
fn native_receipts_reject_missing_extra_wrong_revision_and_wrong_bytes() {
    let run = "native-receipt-rejection";
    let (_repository, engine, _, _, _) = native_boundary_run(run);
    let mut session = NativeFaultSession::new();
    session.allow_begin = true;
    session.reject_invalid_receipts = true;
    assert!(HarnessExecutionRecord::apply_durable(&engine, run, Some(&mut session)).is_err());
    assert_eq!(session.calls, vec!["begin", "commit"]);
}

#[cfg(unix)]
#[test]
fn context_terminal_proof_rejects_pending_attempt_and_unfinalized_or_divergent_head() {
    let run = "terminal-proof-rejection";
    let (_workspace, engine, prepared, _raw, _, validated) = validated_durable_code_run(run);
    let (finalized, terminal) = HarnessExecutionRecord::apply_durable(&engine, run, None)
        .expect("filesystem terminal evidence must be produced by actual Core apply");
    let contract = ContextApplyContract::from_durable(&prepared, &finalized, &terminal)
        .expect("actual terminal evidence must bind its contract");
    let proof = ContextTerminalProof::new(contract.clone(), terminal.clone(), finalized.clone())
        .expect("actual filesystem terminal and read-back head must bind");
    assert_eq!(proof.persisted_attempt(), &terminal);
    assert_eq!(proof.finalized_head(), &finalized);
    assert!(!proof.contract().requires_commit());
    let pending = terminal
        .replace_state(HarnessApplyAttemptState::PendingApply)
        .expect("negative pending fixture must retain the actual attempt identity");
    assert!(matches!(
        ContextTerminalProof::new(contract.clone(), pending, finalized),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("persisted terminal attempt")
    ));
    assert!(matches!(
        ContextTerminalProof::new(contract.clone(), terminal.clone(), validated.clone()),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("read-back finalized head")
    ));
    let mut unfinalized = terminal;
    if let HarnessApplyAttemptState::AppliedFinalized {
        resulting_record, ..
    } = &mut unfinalized.attempt_state
    {
        **resulting_record = validated.clone();
    }
    assert!(matches!(
        ContextTerminalProof::new(contract, unfinalized, validated),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("read-back finalized head")
    ));
}

#[cfg(unix)]
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "actual rollback, durable cleanup failure and retry revalidation form one crash scenario"
)]
fn native_completed_rollback_persists_abort_proof_and_revalidates_cleanup_retry() {
    let run = "native-completed-rollback";
    let original = "vault/personal/knowledge/native-original.md";
    let created = "vault/personal/knowledge/native-rollback/first.md";
    let untouched = "vault/personal/knowledge/native-rollback/second.md";
    let (repository, engine, prepared, _, head) = native_boundary_run_with_targets(
        run,
        vec![original.into(), created.into(), untouched.into()],
        |root| {
            fs::create_dir_all(root.join("vault/personal/knowledge"))
                .expect("native fixture parent must exist before source binding");
            fs::write(root.join(original), "# Original material\n")
                .expect("original native material must exist before source binding");
        },
    );
    let pending = HarnessExecutionRecord::persist_pending_apply_for_test(&engine, run)
        .expect("actual pending attempt must persist before beginning its effect");
    let contract = ContextApplyContract::from_durable(&prepared, &head, &pending)
        .expect("actual native attempt must bind its effect");
    assert!(contract.requires_commit());
    assert!(matches!(
        ContextTerminalProof::new(contract.clone(), pending.clone(), head.clone()),
        Err(HarnessError::InvalidSubmission(message))
            if message.contains("persisted terminal attempt")
    ));
    let attempt_path = repository.0.join(format!(
        ".llm-context-vault-harness/runs/{run}/apply-attempt.json"
    ));
    let mut session = NativeFaultSession::new();
    session.allow_begin = true;
    session.fail_abort = true;
    session.abort_attempt_path = Some(attempt_path.clone());
    session
        .begin(&contract)
        .expect("fault session must allow pending begin");
    let mut files = evaluated_execution_changes(
        head.evaluation
            .as_ref()
            .expect("validated head has an accepted evaluation"),
    )
    .expect("native fixture must have accepted file changes");
    files.sort_by(|a, b| a.path().cmp(b.path()));
    let inputs = files
        .iter()
        .map(|change| {
            let target = prepared
                .plan
                .frozen_targets
                .targets
                .iter()
                .find(|target| target.workspace_relative_path == change.path())
                .expect("every accepted change must have a frozen target");
            repository::BatchApplyInput {
                change,
                planned_parent_directories: &target.parent_directories_to_create,
                validate_vault_markdown: true,
            }
        })
        .collect::<Vec<_>>();
    let report = repository::apply_change_batch_reported_for_attempt_with_test_hook(
        &engine.workspace_root,
        &inputs,
        &pending.resolved_plan_digest,
        &pending.candidate_digest,
        &pending.attempt_identifier,
        |index| {
            if index == 2 {
                assert_eq!(
                    fs::read_to_string(repository.0.join(original)).unwrap(),
                    "candidate"
                );
                assert_eq!(
                    fs::read_to_string(repository.0.join(created)).unwrap(),
                    "candidate"
                );
                Err(HarnessError::InvalidRepository(
                    "injected third-target failure".into(),
                ))
            } else {
                Ok(())
            }
        },
    )
    .expect("real partially applied batch must execute rollback");
    let report =
        repository::complete_batch_cleanup_after_lock_release(&engine.workspace_root, report);
    assert_eq!(report.outcome, repository::BatchApplyOutcome::RolledBack);
    assert_eq!(report.batch_id, pending.expected_batch_identifier);
    assert!(!report.journal_retained);
    assert_eq!(
        fs::read_to_string(repository.0.join(original)).unwrap(),
        "# Original material\n"
    );
    assert!(
        !repository
            .0
            .join("vault/personal/knowledge/native-rollback")
            .exists()
    );
    assert!(
        !repository
            .0
            .join(format!(
                ".llm-context-vault-harness/batches/{}",
                pending.expected_batch_identifier
            ))
            .exists()
    );
    assert!(
        HarnessExecutionRecord::open_durable_recovery(&repository.0, run, Some(&mut session))
            .expect("pending native recovery must bind before rollback proof cleanup")
            .finish(&engine)
            .is_err()
    );
    assert_eq!(session.calls, vec!["begin", "recover", "abort"]);
    let aborted: HarnessApplyAttemptReceipt = decode_current_json(
        &fs::read(&attempt_path).unwrap(),
        "durable rollback abort proof",
    )
    .expect("abort proof must survive cleanup failure");
    assert!(matches!(&aborted.attempt_state,
        HarnessApplyAttemptState::AbortedAfterRollback { rollback_receipt, .. }
            if rollback_receipt.outcome == BatchApplyOutcomeReceipt::RolledBack
    ));
    fs::write(repository.0.join(original), "external bytes")
        .expect("external drift must be installed before retry");
    assert!(
        HarnessExecutionRecord::open_durable_recovery(&repository.0, run, Some(&mut session))
            .expect("retry must obtain actual effect status")
            .finish(&engine)
            .is_err()
    );
    assert_eq!(session.calls, vec!["begin", "recover", "abort", "recover"]);
    fs::write(repository.0.join(original), "# Original material\n")
        .expect("original bytes must be restored before journal revalidation");
    let completion_path = repository
        .0
        .join(&pending.expected_completion_relative_path);
    let completion = fs::read(&completion_path).expect("real rollback completion must remain");
    fs::write(&completion_path, b"{}").expect("invalid completion must be installed");
    assert!(
        HarnessExecutionRecord::open_durable_recovery(&repository.0, run, Some(&mut session))
            .expect("retry must obtain actual effect status")
            .finish(&engine)
            .is_err()
    );
    assert_eq!(
        session.calls,
        vec!["begin", "recover", "abort", "recover", "recover"]
    );
    fs::write(&completion_path, completion).expect("real completion must be restored");
    session.fail_abort = false;
    let (unchanged, repeated) =
        HarnessExecutionRecord::open_durable_recovery(&repository.0, run, Some(&mut session))
            .expect("restored rollback must reopen")
            .finish(&engine)
            .expect("proved persisted rollback cleanup must retry successfully");
    assert_eq!(unchanged, head);
    assert_eq!(repeated, aborted);
    assert_eq!(
        session.calls,
        vec![
            "begin", "recover", "abort", "recover", "recover", "recover", "abort"
        ]
    );
    assert!(!session.calls.contains(&"commit"));
    assert!(!session.calls.contains(&"finalize"));
}

#[cfg(unix)]
#[test]
fn native_private_view_creation_modes_are_umask_independent() {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    const CHILD: &str = "CONTEXT_PRIVATE_MODE_TEST_CHILD";
    if let Ok(mask) = std::env::var(CHILD) {
        let mask = libc::mode_t::from_str_radix(&mask, 8).expect("test caller mask");
        // SAFETY: this branch runs only in a dedicated child test process.
        unsafe { libc::umask(mask) };
        let (repository, engine, _, _, _) = native_boundary_run("private-mode");
        let mut session = NativeFaultSession::new();
        session.allow_begin = true;
        assert!(
            HarnessExecutionRecord::apply_durable(&engine, "private-mode", Some(&mut session))
                .is_err()
        );
        assert!(
            session.calls.contains(&"commit"),
            "actual filesystem apply must reach commit"
        );
        let target = repository
            .0
            .join("vault/personal/knowledge/native-effect-test.md");
        assert_eq!(
            fs::metadata(target)
                .expect("published target")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        fn inspect(path: &Path) {
            use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
            let metadata = fs::symlink_metadata(path).expect("owned apply artifact");
            assert_eq!(
                metadata.permissions().mode() & 0o777,
                if metadata.is_dir() { 0o700 } else { 0o600 },
                "{}",
                path.display()
            );
            if metadata.is_dir() {
                for entry in fs::read_dir(path).expect("owned directory") {
                    inspect(&entry.expect("entry").path());
                }
            } else {
                assert_eq!(metadata.nlink(), 1);
            }
        }
        inspect(&repository.0.join(".llm-context-vault-harness"));
        let metadata = fs::metadata(
            repository
                .0
                .join("vault/personal/knowledge/native-effect-test.md"),
        )
        .expect("target");
        assert_eq!(metadata.nlink(), 1);
        return;
    }
    for mask in ["000", "022", "077"] {
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "harness::tests::native_private_view_creation_modes_are_umask_independent",
                "--nocapture",
            ])
            .env(CHILD, mask)
            .output()
            .expect("isolated caller umask process");
        assert!(
            output.status.success(),
            "caller mask {mask}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[cfg(unix)]
#[test]
fn native_private_create_crash_keeps_single_links_and_recovers() {
    use std::os::unix::fs::MetadataExt as _;
    let (repository, engine, _, _, _) = native_boundary_run("private-create-crash");
    let changes = [
        FileChange::Create {
            path: "first.txt".into(),
            content: "first native publication".into(),
        },
        FileChange::Create {
            path: "second.txt".into(),
            content: "second native publication".into(),
        },
    ];
    let inputs = changes
        .iter()
        .map(|change| repository::BatchApplyInput {
            change,
            planned_parent_directories: &[],
            validate_vault_markdown: false,
        })
        .collect::<Vec<_>>();
    let policy = engine
        .workspace_policy()
        .expect("validated native workspace policy");
    assert!(policy == repository::WorkspacePolicy::NativePrivate);
    let crash = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        repository::apply_change_batch_reported_with_test_hook_with_policy(
            policy,
            &engine.workspace_root,
            &inputs,
            &"a".repeat(64),
            &"b".repeat(64),
            |index| {
                if index == 1 {
                    panic!("crash immediately after the first published Create");
                }
                Ok(())
            },
        )
    }));
    assert!(
        crash.is_err(),
        "publication did not reach the crash hook: {crash:?}"
    );
    assert_eq!(
        fs::metadata(repository.0.join("first.txt"))
            .expect("published before crash")
            .nlink(),
        1
    );
    let pending = repository::ensure_no_pending_batch_recovery(&engine.workspace_root)
        .expect_err("retained crash journal");
    let HarnessError::PendingBatchRecovery { artifacts } = pending else {
        panic!("actual pending journal required")
    };
    let report = repository::recover_change_batch_reported_with_policy(
        policy,
        &engine.workspace_root,
        &artifacts[0],
    )
    .expect("actual repository crash recovery");
    assert!(report.outcome == repository::BatchApplyOutcome::RolledBack);
    let report = repository::complete_batch_cleanup_after_lock_release_with_policy(
        policy,
        &engine.workspace_root,
        report,
    );
    assert!(report.outcome == repository::BatchApplyOutcome::RolledBack);
    assert!(!repository.0.join("first.txt").exists());
    assert!(!repository.0.join("second.txt").exists());
    let repeated = repository::recover_change_batch_reported_with_policy(
        policy,
        &engine.workspace_root,
        &artifacts[0],
    )
    .expect("idempotent actual recovery");
    assert!(repeated.outcome == repository::BatchApplyOutcome::RolledBack);
}

#[cfg(unix)]
#[test]
fn native_private_policy_preserves_external_permissions() {
    use std::os::unix::fs::PermissionsExt as _;
    let source = TemporaryWorkspace::create();
    fs::create_dir(source.0.join("vault")).expect("source vault root");
    let workspace = TemporaryWorkspace::create();
    let engine = HarnessEngine::with_source(
        &source.0,
        &workspace.0,
        std::sync::Arc::new(NativeBoundarySource::fixture(&source.0)),
    )
    .expect("external workspace with stored source");
    let policy = engine
        .workspace_policy()
        .expect("validated distinct workspace");
    assert!(policy == repository::WorkspacePolicy::External);
    fs::write(workspace.0.join("probe"), "caller defaults").expect("mode probe");
    let expected_mode = fs::metadata(workspace.0.join("probe"))
        .expect("probe")
        .permissions()
        .mode()
        & 0o777;
    let created = FileChange::Create {
        path: "created.txt".into(),
        content: "external".into(),
    };
    let report = repository::apply_change_batch_reported_with_policy(
        policy,
        &engine.workspace_root,
        &[repository::BatchApplyInput {
            change: &created,
            planned_parent_directories: &[],
            validate_vault_markdown: false,
        }],
        &"a".repeat(64),
        &"b".repeat(64),
    )
    .expect("external create");
    let report = repository::complete_batch_cleanup_after_lock_release_with_policy(
        policy,
        &engine.workspace_root,
        report,
    );
    assert!(report.outcome == repository::BatchApplyOutcome::Applied);
    assert_eq!(
        fs::metadata(workspace.0.join("created.txt"))
            .expect("external created file")
            .permissions()
            .mode()
            & 0o777,
        expected_mode
    );
    fs::set_permissions(
        workspace.0.join("created.txt"),
        fs::Permissions::from_mode(0o640),
    )
    .expect("existing external mode");
    let changes = [
        FileChange::Update {
            path: "created.txt".into(),
            expected_content_digest: byte_digest(b"external"),
            content: "updated".into(),
        },
        FileChange::Create {
            path: "blocked.txt".into(),
            content: "blocked".into(),
        },
    ];
    let inputs = changes
        .iter()
        .map(|change| repository::BatchApplyInput {
            change,
            planned_parent_directories: &[],
            validate_vault_markdown: false,
        })
        .collect::<Vec<_>>();
    let report = repository::apply_change_batch_reported_with_test_hook_with_policy(
        policy,
        &engine.workspace_root,
        &inputs,
        &"c".repeat(64),
        &"d".repeat(64),
        |index| {
            if index == 1 {
                assert_eq!(
                    fs::metadata(workspace.0.join("created.txt"))
                        .expect("applied external update")
                        .permissions()
                        .mode()
                        & 0o777,
                    0o640
                );
                Err(HarnessError::InvalidRepository(
                    "injected next-target failure".into(),
                ))
            } else {
                Ok(())
            }
        },
    )
    .expect("rollback report");
    assert!(report.outcome == repository::BatchApplyOutcome::RolledBack);
    assert_eq!(
        fs::read(workspace.0.join("created.txt")).expect("restored bytes"),
        b"external"
    );
    assert_eq!(
        fs::metadata(workspace.0.join("created.txt"))
            .expect("rollback mode")
            .permissions()
            .mode()
            & 0o777,
        0o640
    );
}

#[cfg(unix)]
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one relocated Core scenario checks the complete owner, source and binding boundary"
)]
fn relocated_core_policy_maintenance_preserves_boundaries() {
    use std::os::unix::fs::symlink;
    let source_root = TemporaryWorkspace::create();
    copy_directory(
        &repository_root().join("vault"),
        &source_root.0.join("vault"),
    );
    let collision = source_root.0.join("crates/context-core/src/lib.rs");
    fs::create_dir_all(collision.parent().expect("collision has a parent"))
        .expect("create owned source collision parent");
    fs::write(collision, "pub fn source_view_collision() {}\n")
        .expect("write owned source collision");
    let workspace = TemporaryWorkspace::create();
    for (path, contents) in [
        ("crates/context-core/src/lib.rs", "pub fn original() {}\n"),
        ("crates/context-core/README.md", "# Core\n"),
        (
            "crates/context-core-other/src/lib.rs",
            "pub fn unrelated() {}\n",
        ),
        ("src/lib.rs", "pub fn application() {}\n"),
        ("AGENTS.md", "# Workspace rules\n"),
    ] {
        let path = workspace.0.join(path);
        fs::create_dir_all(path.parent().expect("fixture has a parent"))
            .expect("create fixture parents");
        fs::write(path, contents).expect("write owned target");
    }
    let native = std::sync::Arc::new(NativeBoundarySource::fixture(&source_root.0));
    let engine = HarnessEngine::with_source(&source_root.0, &workspace.0, native)
        .expect("distinct native source must open");
    let request = HarnessRequest {
        targets: vec!["crates/context-core/src/lib.rs".into()],
        ..profile_code_request()
    };
    let intent = HarnessIntent::PolicyMaintenance;
    for (action, target) in [
        (HarnessAction::CodeWrite, "crates/context-core/src/lib.rs"),
        (HarnessAction::CodeReview, "crates/context-core/src/lib.rs"),
        (
            HarnessAction::DocumentWrite,
            "crates/context-core/README.md",
        ),
        (
            HarnessAction::DocumentReview,
            "crates/context-core/README.md",
        ),
    ] {
        let request = HarnessRequest {
            action,
            targets: vec![target.into()],
            ..request.clone()
        };
        let resolved = engine
            .resolve_with_intent(&request, intent)
            .expect("relocated maintenance must resolve");
        assert_eq!(resolved.plan.intent, intent);
        assert!(resolved.plan.context_grants.is_empty());
        for id in [
            "common-code-quality",
            "context-vault-operating-model",
            "context-document-stability",
        ] {
            if id != "common-code-quality"
                || matches!(action, HarnessAction::CodeWrite | HarnessAction::CodeReview)
            {
                assert!(
                    resolved
                        .plan
                        .required_policies
                        .iter()
                        .any(|policy| policy.id == id)
                );
            }
        }
        assert!(resolved.plan.required_policies.iter().all(|policy| {
            !policy
                .repository_relative_path
                .starts_with("vault/personal/")
        }));
        assert!(
            resolved
                .plan
                .workspace_policies
                .iter()
                .any(|policy| policy.workspace_relative_path == "AGENTS.md")
        );
        let prepared = engine
            .prepare(&resolved, &complete_runtime_capabilities(), None)
            .expect("native policy and external target bindings must prepare");
        engine
            .validate_prepared_run(&resolved, &prepared)
            .expect("subsequent validation must re-resolve the same boundary");
        if target.ends_with("lib.rs") {
            assert!(
                prepared
                    .role_bundles
                    .iter()
                    .all(|bundle| bundle.bound_documents().any(|document| {
                        document.source == HarnessBoundDocumentSource::Target
                            && document.relative_path == target
                            && document.content == "pub fn original() {}\n"
                    })),
                "target bytes must come from the implementation workspace, not the same logical source-view path"
            );
        }
        assert!(
            prepared
                .source_versions
                .versions
                .iter()
                .all(|version| version.logical_path != target)
        );
    }
    let grant = ContextGrant {
        owner: DataOwner::Company {
            company: "cluml".into(),
        },
        purpose: ContextGrantPurpose::CareerWritingEvidence,
        access: ContextAccess::ReadOnly,
    };
    assert!(
        engine
            .resolve_with_intent_and_career_composition(&request, intent, &[grant], &[], None)
            .is_err()
    );
    assert!(
        engine.resolve(&request).is_err(),
        "generic external Profile work must stay denied"
    );
    for owner in [
        DataOwner::Personal,
        DataOwner::Company {
            company: "cluml".into(),
        },
        DataOwner::PersonalProject {
            project: "coupler".into(),
        },
        DataOwner::CompanyProject {
            company: "cluml".into(),
            project: "giganto".into(),
        },
    ] {
        let request = HarnessRequest {
            owner,
            ..request.clone()
        };
        assert!(engine.resolve_with_intent(&request, intent).is_err());
        assert!(engine.resolve(&request).is_err());
    }
    for targets in [
        vec!["src/lib.rs"],
        vec!["crates/context-core-other/src/lib.rs"],
        vec!["crates/context-core/../context-core/src/lib.rs"],
        vec!["crates/context-core/./src/lib.rs"],
        vec!["crates/context-core/src/lib.rs", "src/lib.rs"],
    ] {
        let request = HarnessRequest {
            targets: targets.into_iter().map(str::to_owned).collect(),
            ..request.clone()
        };
        assert!(engine.resolve_with_intent(&request, intent).is_err());
    }
    for (action, target) in [
        (
            HarnessAction::DocumentWrite,
            "crates/context-core/src/lib.rs",
        ),
        (HarnessAction::CodeReview, "crates/context-core/README.md"),
    ] {
        let request = HarnessRequest {
            action,
            targets: vec![target.into()],
            ..request.clone()
        };
        assert!(engine.resolve_with_intent(&request, intent).is_err());
    }
    let filesystem = HarnessEngine::open(&source_root.0, &workspace.0)
        .expect("filesystem source fixture must open");
    assert!(
        filesystem.resolve_with_intent(&request, intent).is_err(),
        "the exception requires the native policy source"
    );
    engine
        .resolve(&personal_project_code_request())
        .expect("ordinary native application code retains repository ownership");
    symlink(
        workspace.0.join("src/lib.rs"),
        workspace.0.join("crates/context-core/src/linked.rs"),
    )
    .expect("make owned symbolic alias");
    fs::hard_link(
        workspace.0.join("src/lib.rs"),
        workspace.0.join("crates/context-core/src/hard.rs"),
    )
    .expect("make owned hard alias");
    for target in [
        "crates/context-core/src/linked.rs",
        "crates/context-core/src/hard.rs",
    ] {
        let request = HarnessRequest {
            targets: vec![target.into()],
            ..request.clone()
        };
        assert!(engine.resolve_with_intent(&request, intent).is_err());
    }
    let resolved = engine
        .resolve_with_intent(&request, intent)
        .expect("fresh original target must resolve");
    let prepared = engine
        .prepare(&resolved, &complete_runtime_capabilities(), None)
        .expect("fresh binding must prepare");
    fs::write(
        workspace.0.join(&request.targets[0]),
        "pub fn changed() {}\n",
    )
    .expect("change owned target");
    assert!(engine.validate_prepared_run(&resolved, &prepared).is_err());
    let fresh = engine
        .resolve_with_intent(&request, intent)
        .expect("fresh resolve must bind changed target");
    assert_ne!(fresh.resolved_plan_digest, resolved.resolved_plan_digest);
    let prepared = engine
        .prepare(&fresh, &complete_runtime_capabilities(), None)
        .expect("fresh target must prepare");
    fs::write(
        source_root.0.join("vault/profile/rules/agent-harness.md"),
        "# Changed source policy\n",
    )
    .expect("change owned policy source");
    assert!(
        engine.validate_prepared_run(&fresh, &prepared).is_err(),
        "policy currentness is still mandatory"
    );
}
