//! Role execution, evaluation, promotion, and receipt validation.

use std::fs::OpenOptions;
#[cfg(unix)]
use std::os::{
    fd::AsRawFd,
    unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
};

use super::*;

const RUNS_RELATIVE_DIRECTORY: &str = ".llm-context-vault-harness/runs";
const RUN_PREPARED_FILE: &str = "prepared.json";
const RUN_HEAD_FILE: &str = "head.json";
const RUN_APPLY_ATTEMPT_FILE: &str = "apply-attempt.json";
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum EvaluationSubject {
    TaskContract { contract_digest: String },
    FrozenTargets { target_set_digest: String },
    ProducedArtifact { artifact_digest: String },
}

impl EvaluationSubject {
    const fn kind(&self) -> EvaluationSubjectKind {
        match self {
            Self::TaskContract { .. } => EvaluationSubjectKind::TaskContract,
            Self::FrozenTargets { .. } => EvaluationSubjectKind::FrozenTargets,
            Self::ProducedArtifact { .. } => EvaluationSubjectKind::ProducedArtifact,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "source")]
pub enum ResultEvidenceReference {
    Target {
        workspace_relative_path: String,
        content_digest: String,
        locator: String,
    },
    BoundDocument {
        relative_path: String,
        content_digest: String,
        locator: String,
    },
    ToolResult {
        unit: VerificationUnit,
        result_digest: String,
    },
    ProducedArtifact {
        artifact_digest: String,
        locator: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BlockingFinding {
    pub message: String,
    pub evidence: Vec<ResultEvidenceReference>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ImprovementOpportunity {
    pub message: String,
    pub evidence: Vec<ResultEvidenceReference>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LearningObservationOutcome {
    Passed,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LearningObservation {
    pub learning_id: String,
    pub repository_relative_path: String,
    pub content_digest: String,
    pub verification_unit: VerificationUnit,
    pub outcome: LearningObservationOutcome,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PromotionClaimKind {
    ObservedFact,
    ModelInterpretation,
    ModelProposal,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PromotionSourceKind {
    Target,
    GrantedEvidence,
    Retrieval,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PromotionEvidenceReference {
    pub source_kind: PromotionSourceKind,
    pub source_owner: DataOwner,
    pub relative_path: String,
    pub content_digest: String,
    pub locator: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum PromotionProposalOrigin {
    SpecialistMemory {
        claim_kind: PromotionClaimKind,
        evidence: Vec<PromotionEvidenceReference>,
    },
    ReviewerLearning {
        learning_id: String,
        source_action: HarnessAction,
        source_intent: HarnessIntent,
        failed_unit: VerificationUnit,
        requirement_result_digest: String,
        evidence: Vec<ResultEvidenceReference>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PromotionProposal {
    pub identifier: String,
    pub owner: DataOwner,
    pub curation_kind: CurationKind,
    pub title: String,
    pub content: String,
    pub origin: PromotionProposalOrigin,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReviewerLearningCandidate {
    pub title: String,
    pub guidance: String,
    pub failed_unit: VerificationUnit,
    pub requirement_result_digest: String,
    pub evidence: Vec<ResultEvidenceReference>,
}

/// Same-turn advisory lineage from an accepted task evaluation to a separately
/// user-authorized Vault curation request.
///
/// The digest makes transport tampering detectable. Revalidating the source run
/// after it has left the current adapter turn requires a future durable-run lookup.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PromotionHandoff {
    pub source_resolved_plan_digest: String,
    pub source_task_evaluation_receipt_digest: String,
    pub proposal: PromotionProposal,
    pub handoff_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub enum PromotionState {
    None,
    RequiresSeparateCuration { proposals: Vec<PromotionProposal> },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum RevisionCorrection {
    FailedRequirement {
        unit: VerificationUnit,
        detail: String,
    },
    BlockingFinding {
        finding_digest: String,
        finding: BlockingFinding,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RevisionContract {
    pub revision: u32,
    pub previous_candidate_digest: Option<String>,
    /// Digest of the preceding rejected task evaluation receipt, when revising.
    pub previous_task_evaluation_receipt_digest: Option<String>,
    pub corrections: Vec<RevisionCorrection>,
    pub revision_contract_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum RoleInvocationSegment {
    ControlHead {
        content: String,
        content_digest: String,
    },
    BoundDocument {
        document: HarnessBoundDocument,
    },
    DynamicContext {
        content: String,
        content_digest: String,
    },
    ControlTail {
        content: String,
        content_digest: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoleInvocationContract {
    pub version: u32,
    pub resolved_plan_digest: String,
    /// Digest of the static prepared role run from which this final invocation was issued.
    pub prepared_role_run_digest: String,
    pub revision_contract_digest: String,
    pub role: HarnessRole,
    pub role_input_digest: String,
    pub predecessor_results: Vec<RoleExecutionResult>,
    pub subject: EvaluationSubject,
    pub segments: Vec<RoleInvocationSegment>,
    pub total_context_bytes: usize,
    pub invocation_digest: String,
}

impl RoleInvocationContract {
    /// Binds an adapter's observed lifecycle and outcome to this invocation.
    /// Acceptance, including role, evidence, freshness and lifecycle validation,
    /// remains the responsibility of the Core execution transition.
    pub fn bind_result(
        &self,
        lifecycle: ReportedRoleLifecycle,
        outcome: RoleExecutionOutcome,
    ) -> HarnessResult<RoleExecutionResult> {
        let context_id = lifecycle.context_id.clone();
        let result_digest = serialized_digest(&(
            self.role,
            &self.invocation_digest,
            &context_id,
            &lifecycle,
            &outcome,
        ))?;
        Ok(RoleExecutionResult {
            role: self.role,
            invocation_digest: self.invocation_digest.clone(),
            context_id,
            lifecycle,
            outcome,
            result_digest,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolInvocationContract {
    pub version: u32,
    pub resolved_plan_digest: String,
    /// Digest of the static prepared role run that authorized this tool invocation.
    pub prepared_role_run_digest: String,
    pub revision_contract_digest: String,
    pub requirements: Vec<VerificationRequirement>,
    pub subject: EvaluationSubject,
    pub invocation_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequirementResult {
    pub unit: VerificationUnit,
    pub passed: bool,
    pub detail: String,
    pub evidence: Vec<ResultEvidenceReference>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolEvidenceSet {
    pub invocation_digest: String,
    pub results: Vec<RequirementResult>,
    pub executions: Vec<ToolCommandEvidence>,
    pub evidence_set_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub enum ToolTermination {
    Exited { code: i32 },
    Signaled { signal: i32 },
    FailedToStart { message: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolCommandEvidence {
    pub unit: VerificationUnit,
    pub command: Vec<String>,
    pub termination: ToolTermination,
    pub stdout_digest: String,
    pub stderr_digest: String,
    pub evidence_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum WriterArtifact {
    Changes {
        changes: Vec<FileChange>,
    },
    Curation {
        curation_kind: CurationKind,
        entries: Vec<CurationEntry>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SpecialistArtifact {
    pub source_targets: Vec<String>,
    pub context_bundle_digest: Option<String>,
    pub output: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum MissingContextCandidate {
    AdditionalWorkspaceTarget { workspace_relative_path: String },
    VaultEvidence { repository_relative_path: String },
}

impl MissingContextCandidate {
    pub(super) fn relative_path(&self) -> &str {
        match self {
            Self::AdditionalWorkspaceTarget {
                workspace_relative_path,
            } => workspace_relative_path,
            Self::VaultEvidence {
                repository_relative_path,
            } => repository_relative_path,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MissingContextRequest {
    pub reason: String,
    pub blocked_verification_units: Vec<VerificationUnit>,
    pub subject_evidence: Vec<ResultEvidenceReference>,
    pub candidate: MissingContextCandidate,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "role")]
pub enum CompletedRoleResult {
    Writer {
        artifact: WriterArtifact,
    },
    Specialist {
        artifact: SpecialistArtifact,
        requirement_results: Vec<RequirementResult>,
        promotion_proposals: Vec<PromotionProposal>,
    },
    Verifier {
        subject_evidence: Vec<ResultEvidenceReference>,
        requirement_results: Vec<RequirementResult>,
    },
    Reviewer {
        summary: String,
        subject_evidence: Vec<ResultEvidenceReference>,
        requirement_results: Vec<RequirementResult>,
        blocking_findings: Vec<BlockingFinding>,
        improvements: Vec<ImprovementOpportunity>,
        learning_candidates: Vec<ReviewerLearningCandidate>,
    },
}

impl CompletedRoleResult {
    const fn role(&self) -> HarnessRole {
        match self {
            Self::Writer { .. } => HarnessRole::Writer,
            Self::Specialist { .. } => HarnessRole::Specialist,
            Self::Verifier { .. } => HarnessRole::Verifier,
            Self::Reviewer { .. } => HarnessRole::Reviewer,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "status")]
pub enum RoleExecutionOutcome {
    Completed { result: CompletedRoleResult },
    MissingContext { request: MissingContextRequest },
    Failed { message: String },
    Cancelled,
    TimedOut,
    Unsupported { reason: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoleExecutionResult {
    pub role: HarnessRole,
    pub invocation_digest: String,
    pub context_id: String,
    pub lifecycle: ReportedRoleLifecycle,
    pub outcome: RoleExecutionOutcome,
    pub result_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoleExecutionRecord {
    pub version: u32,
    pub resolved_plan_digest: String,
    /// Digest of the static prepared role run executed by this role record.
    pub prepared_role_run_digest: String,
    pub revision_contract: RevisionContract,
    pub role_results: Vec<RoleExecutionResult>,
    pub accepted_role_order: Vec<HarnessRole>,
    pub tool_evidence_after_accepted_roles: Option<usize>,
    pub tool_evidence: Option<ToolEvidenceSet>,
    pub execution_record_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HarnessExecutionStep {
    pub record: RoleExecutionRecord,
    pub ready_role_invocations: Vec<RoleInvocationContract>,
    pub ready_tool_invocation: Option<ToolInvocationContract>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum HarnessExecutionEvent {
    RoleResult { result: Box<RoleExecutionResult> },
    ToolEvidence { evidence: ToolEvidenceSet },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionStatus {
    Completed,
    MissingContext,
    Failed,
    Cancelled,
    TimedOut,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SubjectStatus {
    Accepted,
    Rejected,
    NotApplicable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessCompletionState {
    ExecutionHalted,
    MissingContext,
    RevisionRequired,
    Rejected,
    ValidatedPendingApply,
    AnalysisComplete,
    ReviewComplete,
    ReadComplete,
    Applied,
    RecoveryRequired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvaluatedRequirement {
    pub requirement: VerificationRequirement,
    pub passed: bool,
    pub detail: String,
    pub evidence: Vec<ResultEvidenceReference>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoleExecutionBinding {
    pub role: HarnessRole,
    pub context_id: String,
    pub lifecycle: ReportedRoleLifecycle,
    pub result_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum EvaluatedArtifact {
    Writer { artifact: WriterArtifact },
    Specialist { artifact: SpecialistArtifact },
    Review { summary: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HarnessTaskEvaluation {
    pub version: u32,
    pub resolved_plan_digest: String,
    /// Digest of the static prepared role run whose result is being evaluated.
    pub prepared_role_run_digest: String,
    pub revision: u32,
    pub revision_contract_digest: String,
    pub execution_record: RoleExecutionRecord,
    pub execution_status: ExecutionStatus,
    pub subject_status: SubjectStatus,
    pub subject: Option<EvaluationSubject>,
    pub missing_context: Option<MissingContextRequest>,
    pub candidate_digest: Option<String>,
    pub artifact: Option<EvaluatedArtifact>,
    pub requirements: Vec<EvaluatedRequirement>,
    pub blocking_findings: Vec<BlockingFinding>,
    pub improvements: Vec<ImprovementOpportunity>,
    pub promotion_state: PromotionState,
    /// Correlation only: records the result of the exact verification unit when an approved
    /// learning document was present in the primary producer's prepared input.
    pub learning_observations: Vec<LearningObservation>,
    pub role_executions: Vec<RoleExecutionBinding>,
    pub completion_state: HarnessCompletionState,
    pub assurance: ExecutionAssurance,
    /// Digest of this task-level evaluation and its complete structured evidence.
    pub task_evaluation_receipt_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VerificationExecutionBinding {
    pub unit: VerificationUnit,
    pub owner: VerificationOwner,
    pub result_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CareerExecutionReviewReceipt {
    pub version: u32,
    pub resolved: ResolvedHarnessRequest,
    pub prepared: PreparedRoleRun,
    pub evaluation: HarnessTaskEvaluation,
    pub reviewer_result_digest: String,
    pub verification_bindings: Vec<VerificationExecutionBinding>,
    pub artifact_set_digest: String,
    pub evidence_bundle_digest: Option<String>,
    pub receipt_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CareerExecutionReviewBinding {
    pub evidence_owner: Option<DataOwner>,
    pub resolved_plan_digest: String,
    /// Digest of the prepared role run that produced the reviewed career artifact.
    pub prepared_role_run_digest: String,
    pub execution_record_digest: String,
    pub task_evaluation_receipt_digest: String,
    pub reviewer_result_digest: String,
    pub verification_bindings: Vec<VerificationExecutionBinding>,
    pub artifact_set_digest: String,
    pub evidence_bundle_digest: Option<String>,
    pub receipt_digest: String,
    pub role_executions: Vec<RoleExecutionBinding>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CareerExecutionCompositionReceipt {
    pub version: u32,
    pub manifest_digest: String,
    pub career_output_surface: CareerOutputSurface,
    pub coverage: CareerCoverageMode,
    pub declared_evidence_owners: Vec<DataOwner>,
    pub artifact_targets: Vec<TargetBinding>,
    pub artifact_set_digest: String,
    pub holistic_review: CareerExecutionReviewBinding,
    pub evidence_reviews: Vec<CareerExecutionReviewBinding>,
    pub coverage_is_caller_attested: bool,
    pub assurance: ExecutionAssurance,
    pub composition_digest: String,
}

pub(super) fn begin_execution_record(
    plan: &ResolvedHarnessPlan,
    prepared: &PreparedRoleRun,
    revision_history: &[HarnessTaskEvaluation],
) -> HarnessResult<HarnessExecutionStep> {
    let revision_contract = build_revision_contract(plan, prepared, revision_history)?;
    let mut record = RoleExecutionRecord {
        version: HARNESS_SCHEMA_VERSION,
        resolved_plan_digest: resolved_plan_digest_from_prepared(plan, prepared)?,
        prepared_role_run_digest: prepared.prepared_role_run_digest.clone(),
        revision_contract,
        role_results: Vec::new(),
        accepted_role_order: Vec::new(),
        tool_evidence_after_accepted_roles: None,
        tool_evidence: None,
        execution_record_digest: String::new(),
    };
    refresh_execution_record_digest(&mut record)?;
    execution_step(plan, prepared, revision_history, record)
}

pub(super) fn advance_execution_record(
    plan: &ResolvedHarnessPlan,
    prepared: &PreparedRoleRun,
    revision_history: &[HarnessTaskEvaluation],
    record: &RoleExecutionRecord,
    event: HarnessExecutionEvent,
) -> HarnessResult<HarnessExecutionStep> {
    let mut record = record.clone();
    validate_execution_record(plan, prepared, revision_history, &record)?;
    let current_step = execution_step(plan, prepared, revision_history, record.clone())?;
    match event {
        HarnessExecutionEvent::RoleResult { result } => {
            let result = *result;
            let invocation = current_step
                .ready_role_invocations
                .iter()
                .find(|invocation| invocation.role == result.role)
                .ok_or_else(|| {
                    HarnessError::InvalidSubmission(format!(
                        "role {:?} is not ready for this execution state",
                        result.role
                    ))
                })?;
            validate_role_result(plan, prepared, invocation, &result)?;
            record.accepted_role_order.push(result.role);
            record.role_results.push(result);
            record
                .role_results
                .sort_by_key(|result| plan.role_position(result.role).unwrap_or(usize::MAX));
        }
        HarnessExecutionEvent::ToolEvidence { evidence } => {
            let invocation = current_step.ready_tool_invocation.as_ref().ok_or_else(|| {
                HarnessError::InvalidSubmission(
                    "tool evidence is not ready for this execution state".to_owned(),
                )
            })?;
            validate_tool_evidence(invocation, &evidence)?;
            record.tool_evidence_after_accepted_roles = Some(record.accepted_role_order.len());
            record.tool_evidence = Some(evidence);
        }
    }
    refresh_execution_record_digest(&mut record)?;
    execution_step(plan, prepared, revision_history, record)
}

fn resolved_plan_digest_from_prepared(
    plan: &ResolvedHarnessPlan,
    prepared: &PreparedRoleRun,
) -> HarnessResult<String> {
    plan.validate()?;
    prepared
        .source_versions
        .require_superset(plan.bound_source_versions())?;
    let resolved_plan_digest = serialized_digest(plan)?;
    if prepared.resolved_plan_digest != resolved_plan_digest {
        return Err(HarnessError::InvalidSubmission(
            "prepared execution does not match the supplied plan".to_owned(),
        ));
    }
    Ok(resolved_plan_digest)
}

fn build_revision_contract(
    plan: &ResolvedHarnessPlan,
    prepared: &PreparedRoleRun,
    history: &[HarnessTaskEvaluation],
) -> HarnessResult<RevisionContract> {
    if history.len() > plan.max_revisions as usize {
        return Err(HarnessError::InvalidSubmission(
            "revision history exceeds the plan limit".to_owned(),
        ));
    }
    if !history.is_empty() && !plan.action.is_write() {
        return Err(HarnessError::InvalidSubmission(
            "non-write actions do not accept revision history".to_owned(),
        ));
    }
    for (index, evaluation) in history.iter().enumerate() {
        let expected_revision = u32::try_from(index).map_err(|_| {
            HarnessError::InvalidSubmission(
                "revision number exceeds the supported range".to_owned(),
            )
        })?;
        if evaluation.version != HARNESS_SCHEMA_VERSION
            || evaluation.resolved_plan_digest != prepared.resolved_plan_digest
            || evaluation.prepared_role_run_digest != prepared.prepared_role_run_digest
            || evaluation.revision != expected_revision
        {
            return Err(HarnessError::InvalidSubmission(
                "revision evaluation does not match the current execution".to_owned(),
            ));
        }
        let expected_contract = build_revision_contract_unchecked(&history[..index])?;
        if evaluation.revision_contract_digest != expected_contract.revision_contract_digest {
            return Err(HarnessError::InvalidSubmission(
                "revision evaluation is bound to the wrong correction contract".to_owned(),
            ));
        }
        validate_task_evaluation_receipt(evaluation)?;
        let current = evaluate_execution_record(
            plan,
            prepared,
            &history[..index],
            &evaluation.execution_record,
        )?;
        if &current != evaluation {
            return Err(HarnessError::InvalidSubmission(
                "revision evaluation does not match its complete execution record".to_owned(),
            ));
        }
        if evaluation.execution_status != ExecutionStatus::Completed
            || evaluation.subject_status != SubjectStatus::Rejected
            || evaluation.candidate_digest.is_none()
        {
            return Err(HarnessError::InvalidSubmission(
                "only a completed rejected write candidate may be revised".to_owned(),
            ));
        }
    }
    build_revision_contract_unchecked(history)
}

fn build_revision_contract_unchecked(
    history: &[HarnessTaskEvaluation],
) -> HarnessResult<RevisionContract> {
    let revision = u32::try_from(history.len()).map_err(|_| {
        HarnessError::InvalidSubmission("revision number exceeds the supported range".to_owned())
    })?;
    let (previous_candidate_digest, previous_task_evaluation_receipt_digest, corrections) =
        if let Some(previous) = history.last() {
            let mut corrections = previous
                .requirements
                .iter()
                .filter(|result| !result.passed)
                .map(|result| RevisionCorrection::FailedRequirement {
                    unit: result.requirement.unit,
                    detail: result.detail.clone(),
                })
                .collect::<Vec<_>>();
            for finding in &previous.blocking_findings {
                corrections.push(RevisionCorrection::BlockingFinding {
                    finding_digest: serialized_digest(finding)?,
                    finding: finding.clone(),
                });
            }
            (
                previous.candidate_digest.clone(),
                Some(previous.task_evaluation_receipt_digest.clone()),
                corrections,
            )
        } else {
            (None, None, Vec::new())
        };
    let revision_contract_digest = serialized_digest(&(
        revision,
        &previous_candidate_digest,
        &previous_task_evaluation_receipt_digest,
        &corrections,
    ))?;
    Ok(RevisionContract {
        revision,
        previous_candidate_digest,
        previous_task_evaluation_receipt_digest,
        corrections,
        revision_contract_digest,
    })
}

fn refresh_execution_record_digest(record: &mut RoleExecutionRecord) -> HarnessResult<()> {
    record.execution_record_digest = serialized_digest(&(
        record.version,
        &record.resolved_plan_digest,
        &record.prepared_role_run_digest,
        &record.revision_contract,
        &record.role_results,
        &record.accepted_role_order,
        record.tool_evidence_after_accepted_roles,
        &record.tool_evidence,
    ))?;
    Ok(())
}

fn execution_step(
    plan: &ResolvedHarnessPlan,
    prepared: &PreparedRoleRun,
    revision_history: &[HarnessTaskEvaluation],
    record: RoleExecutionRecord,
) -> HarnessResult<HarnessExecutionStep> {
    validate_execution_record(plan, prepared, revision_history, &record)?;
    if record
        .role_results
        .iter()
        .any(|result| !matches!(result.outcome, RoleExecutionOutcome::Completed { .. }))
    {
        return Ok(HarnessExecutionStep {
            record,
            ready_role_invocations: Vec::new(),
            ready_tool_invocation: None,
        });
    }
    let ready_roles = ready_roles(plan, &record);
    let producer_result = record
        .role_results
        .iter()
        .find(|result| result.role == plan.primary_producer_role);
    let ready_role_invocations = ready_roles
        .into_iter()
        .map(|role| role_invocation(plan, prepared, &record, role))
        .collect::<HarnessResult<Vec<_>>>()?;
    let ready_tool_invocation = if record.tool_evidence.is_none()
        && (plan.action.is_review() || producer_result.is_some())
    {
        let subject = current_evaluation_subject(plan, &record)?;
        tool_invocation(plan, prepared, &record, &subject)?
    } else {
        None
    };
    Ok(HarnessExecutionStep {
        record,
        ready_role_invocations,
        ready_tool_invocation,
    })
}

fn ready_roles(plan: &ResolvedHarnessPlan, record: &RoleExecutionRecord) -> Vec<HarnessRole> {
    let completed_roles = record
        .role_results
        .iter()
        .filter_map(|result| {
            matches!(result.outcome, RoleExecutionOutcome::Completed { .. }).then_some(result.role)
        })
        .collect::<BTreeSet<_>>();
    plan.workflow
        .iter()
        .filter(|node| {
            !completed_roles.contains(&node.role)
                && node
                    .dependencies
                    .iter()
                    .all(|dependency| completed_roles.contains(dependency))
        })
        .map(|node| node.role)
        .collect()
}

fn role_invocation(
    plan: &ResolvedHarnessPlan,
    prepared: &PreparedRoleRun,
    record: &RoleExecutionRecord,
    role: HarnessRole,
) -> HarnessResult<RoleInvocationContract> {
    let metadata = prepared
        .role_metadata
        .iter()
        .find(|metadata| metadata.role == role)
        .ok_or_else(|| {
            HarnessError::InvalidPlan(format!("prepared metadata is missing for role {role:?}"))
        })?;
    if metadata.task.role() != role {
        return Err(HarnessError::InvalidPlan(format!(
            "prepared metadata task does not match role {role:?}"
        )));
    }
    let node = plan
        .workflow_node(role)
        .ok_or_else(|| HarnessError::InvalidPlan(format!("workflow is missing role {role:?}")))?;
    let predecessor_results = required_predecessor_results(record, node)?;
    let invocation_subject = workflow_subject(plan, record, node)?;
    let role_bundle = prepared
        .role_bundles
        .iter()
        .find(|bundle| bundle.role == role)
        .ok_or_else(|| {
            HarnessError::InvalidPlan(format!("prepared bundle is missing for role {role:?}"))
        })?;
    let (segments, total_context_bytes) = role_invocation_segments(
        role_bundle,
        &predecessor_results,
        &invocation_subject,
        &record.revision_contract,
    )?;
    let role_input_digest = serialized_digest(&(metadata, &segments, total_context_bytes))?;
    let invocation_digest = serialized_digest(&(
        HARNESS_SCHEMA_VERSION,
        &record.resolved_plan_digest,
        &record.prepared_role_run_digest,
        &record.revision_contract.revision_contract_digest,
        role,
        &role_input_digest,
        &predecessor_results,
        &invocation_subject,
        &segments,
        total_context_bytes,
    ))?;
    let invocation = RoleInvocationContract {
        version: HARNESS_SCHEMA_VERSION,
        resolved_plan_digest: record.resolved_plan_digest.clone(),
        prepared_role_run_digest: record.prepared_role_run_digest.clone(),
        revision_contract_digest: record.revision_contract.revision_contract_digest.clone(),
        role,
        role_input_digest,
        predecessor_results,
        subject: invocation_subject,
        segments,
        total_context_bytes,
        invocation_digest,
    };
    validate_invocation_predecessor_results(plan, prepared, record, &invocation)?;
    validate_role_invocation_size(prepared, &invocation)?;
    Ok(invocation)
}

fn role_invocation_segments(
    bundle: &HarnessRoleBundle,
    predecessor_results: &[RoleExecutionResult],
    subject: &EvaluationSubject,
    revision_contract: &RevisionContract,
) -> HarnessResult<(Vec<RoleInvocationSegment>, usize)> {
    #[derive(Serialize)]
    struct DynamicContext<'a> {
        predecessor_results: &'a [RoleExecutionResult],
        subject: &'a EvaluationSubject,
        revision_contract: Option<&'a RevisionContract>,
    }

    let dynamic_content = serde_json::to_string(&DynamicContext {
        predecessor_results,
        subject,
        revision_contract: (bundle.role == HarnessRole::Writer).then_some(revision_contract),
    })
    .map_err(|error| {
        HarnessError::InvalidPlan(format!(
            "failed to encode role invocation dynamic context: {error}"
        ))
    })?;
    let dynamic_segment = RoleInvocationSegment::DynamicContext {
        content_digest: byte_digest(dynamic_content.as_bytes()),
        content: dynamic_content,
    };
    let mut segments = Vec::with_capacity(bundle.segments.len() + 1);
    for segment in &bundle.segments {
        match segment {
            HarnessRoleSegment::ControlHead {
                content,
                content_digest,
            } => segments.push(RoleInvocationSegment::ControlHead {
                content: content.clone(),
                content_digest: content_digest.clone(),
            }),
            HarnessRoleSegment::BoundDocument { document } => {
                segments.push(RoleInvocationSegment::BoundDocument {
                    document: document.clone(),
                });
            }
            HarnessRoleSegment::ControlTail {
                content,
                content_digest,
            } => {
                segments.push(dynamic_segment.clone());
                segments.push(RoleInvocationSegment::ControlTail {
                    content: content.clone(),
                    content_digest: content_digest.clone(),
                });
            }
        }
    }
    if !matches!(
        segments.as_slice(),
        [
            RoleInvocationSegment::ControlHead { .. },
            ..,
            RoleInvocationSegment::ControlTail { .. }
        ]
    ) || segments
        .iter()
        .filter(|segment| matches!(segment, RoleInvocationSegment::DynamicContext { .. }))
        .count()
        != 1
    {
        return Err(HarnessError::InvalidPlan(
            "role invocation segments must contain one dynamic context immediately before the control tail"
                .to_owned(),
        ));
    }
    let total_context_bytes = segments.iter().try_fold(0_usize, |total, segment| {
        let bytes = match segment {
            RoleInvocationSegment::ControlHead { content, .. }
            | RoleInvocationSegment::DynamicContext { content, .. }
            | RoleInvocationSegment::ControlTail { content, .. } => content.len(),
            RoleInvocationSegment::BoundDocument { document } => document.content.len(),
        };
        total.checked_add(bytes).ok_or_else(|| {
            HarnessError::UnsupportedRuntime("role invocation context size overflow".to_owned())
        })
    })?;
    Ok((segments, total_context_bytes))
}

fn required_predecessor_results(
    record: &RoleExecutionRecord,
    node: &WorkflowRoleNode,
) -> HarnessResult<Vec<RoleExecutionResult>> {
    node.dependencies
        .iter()
        .map(|dependency| {
            let result = record
                .role_results
                .iter()
                .find(|result| result.role == *dependency)
                .ok_or_else(|| {
                    HarnessError::InvalidSubmission(format!(
                        "role {:?} requires predecessor {dependency:?}",
                        node.role
                    ))
                })?;
            if !matches!(result.outcome, RoleExecutionOutcome::Completed { .. }) {
                return Err(HarnessError::InvalidSubmission(format!(
                    "role {:?} requires completed predecessors",
                    node.role
                )));
            }
            Ok(result.clone())
        })
        .collect()
}

fn workflow_subject(
    plan: &ResolvedHarnessPlan,
    record: &RoleExecutionRecord,
    node: &WorkflowRoleNode,
) -> HarnessResult<EvaluationSubject> {
    match node.subject_source {
        WorkflowSubjectSource::TaskContract => Ok(EvaluationSubject::TaskContract {
            contract_digest: plan.contract_digest.clone(),
        }),
        WorkflowSubjectSource::FrozenTargets => frozen_target_subject(plan),
        WorkflowSubjectSource::PrimaryProducer => {
            let producer = record
                .role_results
                .iter()
                .find(|result| result.role == plan.primary_producer_role)
                .ok_or_else(|| {
                    HarnessError::InvalidSubmission(format!(
                        "role {:?} requires the primary producer result",
                        node.role
                    ))
                })?;
            produced_subject(producer)
        }
    }
}

pub(super) fn validate_invocation_predecessor_results(
    plan: &ResolvedHarnessPlan,
    prepared: &PreparedRoleRun,
    record: &RoleExecutionRecord,
    invocation: &RoleInvocationContract,
) -> HarnessResult<()> {
    let node = plan.workflow_node(invocation.role).ok_or_else(|| {
        HarnessError::InvalidSubmission("role invocation is outside the workflow".to_owned())
    })?;
    let expected = required_predecessor_results(record, node)?;
    if invocation.predecessor_results != expected {
        return Err(HarnessError::InvalidSubmission(
            "role invocation does not contain the exact ordered predecessor results".to_owned(),
        ));
    }
    for predecessor in &invocation.predecessor_results {
        validate_role_result_digest(prepared, predecessor)?;
    }
    if workflow_subject(plan, record, node)? != invocation.subject {
        return Err(HarnessError::InvalidSubmission(
            "role invocation subject does not match its workflow source".to_owned(),
        ));
    }
    let metadata = prepared
        .role_metadata
        .iter()
        .find(|metadata| metadata.role == invocation.role)
        .ok_or_else(|| {
            HarnessError::InvalidSubmission("role invocation has no prepared metadata".to_owned())
        })?;
    let bundle = prepared
        .role_bundles
        .iter()
        .find(|bundle| bundle.role == invocation.role)
        .ok_or_else(|| {
            HarnessError::InvalidSubmission("role invocation has no prepared bundle".to_owned())
        })?;
    let (segments, total_context_bytes) = role_invocation_segments(
        bundle,
        &expected,
        &invocation.subject,
        &record.revision_contract,
    )?;
    if invocation.segments != segments
        || invocation.total_context_bytes != total_context_bytes
        || invocation.role_input_digest
            != serialized_digest(&(metadata, &segments, total_context_bytes))?
    {
        return Err(HarnessError::InvalidSubmission(
            "role invocation does not contain the exact bounded segment sequence".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn validate_role_invocation_size(
    prepared: &PreparedRoleRun,
    invocation: &RoleInvocationContract,
) -> HarnessResult<()> {
    if invocation.total_context_bytes > prepared.runtime_capabilities.max_role_bundle_bytes {
        return Err(HarnessError::UnsupportedRuntime(format!(
            "role {:?} context is {} bytes, but the runtime role-input limit is {}",
            invocation.role,
            invocation.total_context_bytes,
            prepared.runtime_capabilities.max_role_bundle_bytes
        )));
    }
    let serialized_bytes = serde_json::to_vec(invocation)
        .map_err(|error| {
            HarnessError::InvalidSubmission(format!(
                "role invocation cannot be serialized canonically: {error}"
            ))
        })?
        .len();
    if serialized_bytes > prepared.runtime_capabilities.max_role_invocation_bytes {
        return Err(HarnessError::UnsupportedRuntime(format!(
            "role {:?} invocation is {serialized_bytes} serialized bytes, but the runtime limit is {}",
            invocation.role, prepared.runtime_capabilities.max_role_invocation_bytes
        )));
    }
    Ok(())
}

fn tool_invocation(
    plan: &ResolvedHarnessPlan,
    prepared: &PreparedRoleRun,
    record: &RoleExecutionRecord,
    subject: &EvaluationSubject,
) -> HarnessResult<Option<ToolInvocationContract>> {
    let requirements = plan
        .verification_requirements
        .iter()
        .copied()
        .filter(|requirement| requirement.owner == VerificationOwner::Tool)
        .collect::<Vec<_>>();
    if requirements.is_empty() {
        return Ok(None);
    }
    if requirements
        .iter()
        .any(|requirement| requirement.subject != subject.kind())
    {
        return Err(HarnessError::InvalidPlan(
            "tool verification requirements bind the wrong evaluation subject".to_owned(),
        ));
    }
    let invocation_digest = serialized_digest(&(
        HARNESS_SCHEMA_VERSION,
        &record.resolved_plan_digest,
        &prepared.prepared_role_run_digest,
        &record.revision_contract.revision_contract_digest,
        &requirements,
        subject,
    ))?;
    Ok(Some(ToolInvocationContract {
        version: HARNESS_SCHEMA_VERSION,
        resolved_plan_digest: record.resolved_plan_digest.clone(),
        prepared_role_run_digest: prepared.prepared_role_run_digest.clone(),
        revision_contract_digest: record.revision_contract.revision_contract_digest.clone(),
        requirements,
        subject: subject.clone(),
        invocation_digest,
    }))
}

fn frozen_target_subject(plan: &ResolvedHarnessPlan) -> HarnessResult<EvaluationSubject> {
    Ok(EvaluationSubject::FrozenTargets {
        target_set_digest: serialized_digest(&plan.targets)?,
    })
}

fn produced_subject(result: &RoleExecutionResult) -> HarnessResult<EvaluationSubject> {
    let RoleExecutionOutcome::Completed { result } = &result.outcome else {
        return Err(HarnessError::InvalidSubmission(
            "evaluation subject requires a completed producer result".to_owned(),
        ));
    };
    let artifact_digest = match result {
        CompletedRoleResult::Writer { artifact } => serialized_digest(artifact)?,
        CompletedRoleResult::Specialist { artifact, .. } => serialized_digest(artifact)?,
        CompletedRoleResult::Verifier { .. } | CompletedRoleResult::Reviewer { .. } => {
            return Err(HarnessError::InvalidSubmission(
                "evaluation subject requires a Writer or Specialist artifact".to_owned(),
            ));
        }
    };
    Ok(EvaluationSubject::ProducedArtifact { artifact_digest })
}

fn validate_execution_record(
    plan: &ResolvedHarnessPlan,
    prepared: &PreparedRoleRun,
    revision_history: &[HarnessTaskEvaluation],
    record: &RoleExecutionRecord,
) -> HarnessResult<()> {
    if record.version != HARNESS_SCHEMA_VERSION
        || record.resolved_plan_digest != prepared.resolved_plan_digest
        || record.prepared_role_run_digest != prepared.prepared_role_run_digest
    {
        return Err(HarnessError::InvalidSubmission(
            "execution record does not match the current plan and preparation".to_owned(),
        ));
    }
    let expected_revision = build_revision_contract(plan, prepared, revision_history)?;
    if record.revision_contract != expected_revision {
        return Err(HarnessError::InvalidSubmission(
            "execution record revision contract is invalid".to_owned(),
        ));
    }
    let expected_digest = serialized_digest(&(
        record.version,
        &record.resolved_plan_digest,
        &record.prepared_role_run_digest,
        &record.revision_contract,
        &record.role_results,
        &record.accepted_role_order,
        record.tool_evidence_after_accepted_roles,
        &record.tool_evidence,
    ))?;
    if record.execution_record_digest != expected_digest {
        return Err(HarnessError::InvalidSubmission(
            "execution record digest is invalid".to_owned(),
        ));
    }
    validate_serialized_size(
        "role execution record",
        record,
        MAX_ROLE_EXECUTION_RECORD_BYTES,
    )?;
    if record.role_results.len() > plan.role_count() {
        return Err(HarnessError::InvalidSubmission(
            "execution record contains more role results than the plan".to_owned(),
        ));
    }
    if record.accepted_role_order.len() != record.role_results.len() {
        return Err(HarnessError::InvalidSubmission(
            "execution record acceptance order must cover every role result exactly once"
                .to_owned(),
        ));
    }
    if record.tool_evidence.is_some() != record.tool_evidence_after_accepted_roles.is_some()
        || record
            .tool_evidence_after_accepted_roles
            .is_some_and(|accepted| accepted > record.accepted_role_order.len())
    {
        return Err(HarnessError::InvalidSubmission(
            "execution record tool evidence must bind its exact acceptance frontier".to_owned(),
        ));
    }
    let mut roles = BTreeSet::new();
    let mut context_ids = revision_history
        .iter()
        .flat_map(evaluation_context_ids)
        .collect::<BTreeSet<_>>();
    let mut last_role_position = None;
    for result in &record.role_results {
        if !plan.contains_role(result.role) || !roles.insert(result.role) {
            return Err(HarnessError::InvalidSubmission(
                "execution record roles must be planned and unique".to_owned(),
            ));
        }
        let role_position = plan
            .role_position(result.role)
            .expect("planned role position must exist");
        if last_role_position.is_some_and(|previous| previous >= role_position) {
            return Err(HarnessError::InvalidSubmission(
                "execution record role results must use the planned role order".to_owned(),
            ));
        }
        last_role_position = Some(role_position);
        if !context_ids.insert(result.context_id.clone()) {
            return Err(HarnessError::InvalidSubmission(
                "role context IDs must be unique across revision history".to_owned(),
            ));
        }
    }
    validate_execution_acceptance_order(plan, prepared, record)
}

fn validate_execution_acceptance_order(
    plan: &ResolvedHarnessPlan,
    prepared: &PreparedRoleRun,
    record: &RoleExecutionRecord,
) -> HarnessResult<()> {
    let result_roles = record
        .role_results
        .iter()
        .map(|result| result.role)
        .collect::<BTreeSet<_>>();
    let mut accepted_roles = BTreeSet::new();
    let mut prefix = record.clone();
    prefix.role_results.clear();
    prefix.accepted_role_order.clear();
    prefix.tool_evidence_after_accepted_roles = None;
    prefix.tool_evidence = None;
    for (accepted_count, role) in record.accepted_role_order.iter().enumerate() {
        if record.tool_evidence_after_accepted_roles == Some(accepted_count) {
            validate_tool_evidence_at_frontier(plan, prepared, record, &mut prefix)?;
        }
        if !accepted_roles.insert(*role) || !result_roles.contains(role) {
            return Err(HarnessError::InvalidSubmission(
                "execution record acceptance order must contain result roles exactly once"
                    .to_owned(),
            ));
        }
        if prefix
            .role_results
            .iter()
            .any(|result| !matches!(result.outcome, RoleExecutionOutcome::Completed { .. }))
        {
            return Err(HarnessError::InvalidSubmission(
                "no role result may be accepted after a terminal role result".to_owned(),
            ));
        }
        let result = record
            .role_results
            .iter()
            .find(|result| result.role == *role)
            .expect("acceptance role set was checked against result roles");
        let ready_roles = ready_roles(plan, &prefix);
        if !ready_roles.contains(role) {
            return Err(HarnessError::InvalidSubmission(format!(
                "role {role:?} result was recorded before its dependencies were ready"
            )));
        }
        let invocation = role_invocation(plan, prepared, &prefix, *role)?;
        validate_role_result(plan, prepared, &invocation, result)?;
        prefix.accepted_role_order.push(*role);
        prefix.role_results.push(result.clone());
        prefix
            .role_results
            .sort_by_key(|candidate| plan.role_position(candidate.role).unwrap_or(usize::MAX));
    }
    if record.tool_evidence_after_accepted_roles == Some(record.accepted_role_order.len()) {
        validate_tool_evidence_at_frontier(plan, prepared, record, &mut prefix)?;
    }
    Ok(())
}

fn validate_tool_evidence_at_frontier(
    plan: &ResolvedHarnessPlan,
    prepared: &PreparedRoleRun,
    record: &RoleExecutionRecord,
    prefix: &mut RoleExecutionRecord,
) -> HarnessResult<()> {
    if prefix
        .role_results
        .iter()
        .any(|result| !matches!(result.outcome, RoleExecutionOutcome::Completed { .. }))
    {
        return Err(HarnessError::InvalidSubmission(
            "tool evidence cannot be accepted after a terminal role result".to_owned(),
        ));
    }
    let evidence = record.tool_evidence.as_ref().ok_or_else(|| {
        HarnessError::InvalidSubmission(
            "execution record tool acceptance frontier has no evidence".to_owned(),
        )
    })?;
    let subject = current_evaluation_subject(plan, prefix)?;
    let invocation = tool_invocation(plan, prepared, prefix, &subject)?.ok_or_else(|| {
        HarnessError::InvalidSubmission(
            "execution record reports tool evidence without a ready tool requirement".to_owned(),
        )
    })?;
    validate_tool_evidence(&invocation, evidence)?;
    prefix.tool_evidence_after_accepted_roles = record.tool_evidence_after_accepted_roles;
    prefix.tool_evidence = Some(evidence.clone());
    Ok(())
}

fn current_evaluation_subject(
    plan: &ResolvedHarnessPlan,
    record: &RoleExecutionRecord,
) -> HarnessResult<EvaluationSubject> {
    if plan.action.is_review() {
        return frozen_target_subject(plan);
    }
    let producer = record
        .role_results
        .iter()
        .find(|result| result.role == plan.primary_producer_role)
        .ok_or_else(|| {
            HarnessError::InvalidSubmission(
                "evaluation subject is unavailable before the producer result".to_owned(),
            )
        })?;
    produced_subject(producer)
}

fn validate_role_result(
    plan: &ResolvedHarnessPlan,
    prepared: &PreparedRoleRun,
    invocation: &RoleInvocationContract,
    result: &RoleExecutionResult,
) -> HarnessResult<()> {
    if invocation.version != HARNESS_SCHEMA_VERSION
        || invocation.resolved_plan_digest != prepared.resolved_plan_digest
        || invocation.prepared_role_run_digest != prepared.prepared_role_run_digest
        || invocation.role != result.role
        || invocation.invocation_digest != result.invocation_digest
    {
        return Err(HarnessError::InvalidSubmission(
            "role result does not match its Harness-issued invocation".to_owned(),
        ));
    }
    let expected_metadata = prepared
        .role_metadata
        .iter()
        .find(|metadata| metadata.role == result.role)
        .ok_or_else(|| {
            HarnessError::InvalidSubmission("role result has no prepared metadata".to_owned())
        })?;
    let role_bundle = prepared
        .role_bundles
        .iter()
        .find(|bundle| bundle.role == result.role)
        .ok_or_else(|| {
            HarnessError::InvalidSubmission("role result has no prepared bundle".to_owned())
        })?;
    let expected_subject_kind = match plan
        .workflow_node(result.role)
        .ok_or_else(|| HarnessError::InvalidSubmission("role is outside the workflow".to_owned()))?
        .subject_source
    {
        WorkflowSubjectSource::TaskContract => EvaluationSubjectKind::TaskContract,
        WorkflowSubjectSource::FrozenTargets => EvaluationSubjectKind::FrozenTargets,
        WorkflowSubjectSource::PrimaryProducer => EvaluationSubjectKind::ProducedArtifact,
    };
    if invocation.subject.kind() != expected_subject_kind {
        return Err(HarnessError::InvalidSubmission(
            "role invocation binds the wrong evaluation subject".to_owned(),
        ));
    }
    validate_role_result_digest(prepared, result)?;
    validate_completed_role_result(
        plan,
        expected_metadata,
        role_bundle,
        &invocation.subject,
        &result.outcome,
    )?;
    validate_missing_context_result(
        plan,
        role_bundle,
        &invocation.subject,
        result.role,
        &result.outcome,
    )?;
    Ok(())
}

fn validate_role_result_digest(
    prepared: &PreparedRoleRun,
    result: &RoleExecutionResult,
) -> HarnessResult<()> {
    validate_runtime_text("role context ID", &result.context_id)?;
    if result.lifecycle.role != result.role || result.lifecycle.context_id != result.context_id {
        return Err(HarnessError::InvalidSubmission(
            "role lifecycle does not match the role result context".to_owned(),
        ));
    }
    validate_execution_lifecycle(
        &prepared.runtime_capabilities,
        &result.lifecycle,
        &result.outcome,
    )?;
    let expected_digest = serialized_digest(&(
        result.role,
        &result.invocation_digest,
        &result.context_id,
        &result.lifecycle,
        &result.outcome,
    ))?;
    if result.result_digest != expected_digest {
        return Err(HarnessError::InvalidSubmission(
            "role result digest is invalid".to_owned(),
        ));
    }
    Ok(())
}

fn validate_completed_role_result(
    plan: &ResolvedHarnessPlan,
    metadata: &PreparedRoleMetadata,
    role_bundle: &HarnessRoleBundle,
    invocation_subject: &EvaluationSubject,
    outcome: &RoleExecutionOutcome,
) -> HarnessResult<()> {
    let RoleExecutionOutcome::Completed { result } = outcome else {
        return Ok(());
    };
    if result.role() != metadata.role || metadata.task.role() != metadata.role {
        return Err(HarnessError::InvalidSubmission(
            "completed role result kind does not match the prepared role".to_owned(),
        ));
    }
    match (&metadata.task, result) {
        (
            RoleTaskContract::Writer {
                targets,
                curation_kind,
            },
            CompletedRoleResult::Writer { artifact },
        ) => validate_writer_artifact(plan, targets, *curation_kind, artifact),
        (
            RoleTaskContract::Specialist {
                source_targets,
                verification_requirements,
            },
            CompletedRoleResult::Specialist {
                artifact,
                requirement_results,
                promotion_proposals,
            },
        ) => {
            validate_specialist_artifact(metadata, source_targets, artifact)?;
            let produced_subject = EvaluationSubject::ProducedArtifact {
                artifact_digest: serialized_digest(artifact)?,
            };
            validate_requirement_results(
                plan,
                role_bundle,
                verification_requirements,
                requirement_results,
                &produced_subject,
            )?;
            validate_promotion_proposals(plan, role_bundle, &produced_subject, promotion_proposals)
        }
        (
            RoleTaskContract::Verifier {
                verification_requirements,
                ..
            },
            CompletedRoleResult::Verifier {
                subject_evidence,
                requirement_results,
            },
        ) => {
            validate_subject_evidence(plan, role_bundle, invocation_subject, subject_evidence)?;
            validate_requirement_results(
                plan,
                role_bundle,
                verification_requirements,
                requirement_results,
                invocation_subject,
            )
        }
        (
            RoleTaskContract::Reviewer {
                verification_requirements,
                ..
            },
            reviewer_result @ CompletedRoleResult::Reviewer { .. },
        ) => validate_completed_reviewer_result(
            plan,
            role_bundle,
            invocation_subject,
            verification_requirements,
            reviewer_result,
        ),
        _ => Err(HarnessError::InvalidSubmission(
            "role result does not match the prepared role task".to_owned(),
        )),
    }
}

fn validate_completed_reviewer_result(
    plan: &ResolvedHarnessPlan,
    role_bundle: &HarnessRoleBundle,
    invocation_subject: &EvaluationSubject,
    verification_requirements: &[VerificationRequirement],
    result: &CompletedRoleResult,
) -> HarnessResult<()> {
    let CompletedRoleResult::Reviewer {
        summary,
        subject_evidence,
        requirement_results,
        blocking_findings,
        improvements,
        learning_candidates,
    } = result
    else {
        return Err(HarnessError::InvalidSubmission(
            "Reviewer validation requires a Reviewer result".to_owned(),
        ));
    };
    validate_artifact_text("review summary", summary)?;
    if plan.owner == DataOwner::CommonWork && !learning_candidates.is_empty() {
        return Err(HarnessError::InvalidSubmission(
            "common work maintenance does not accept Reviewer learning candidates".to_owned(),
        ));
    }
    validate_subject_evidence(plan, role_bundle, invocation_subject, subject_evidence)?;
    validate_requirement_results(
        plan,
        role_bundle,
        verification_requirements,
        requirement_results,
        invocation_subject,
    )?;
    validate_review_observations(
        plan,
        role_bundle,
        invocation_subject,
        blocking_findings,
        improvements,
    )?;
    validate_professional_profile_improvement_result(
        verification_requirements,
        requirement_results,
        improvements,
    )?;
    validate_reviewer_learning_candidates(
        plan,
        role_bundle,
        verification_requirements,
        requirement_results,
        invocation_subject,
        learning_candidates,
    )
}

fn validate_missing_context_result(
    plan: &ResolvedHarnessPlan,
    role_bundle: &HarnessRoleBundle,
    invocation_subject: &EvaluationSubject,
    role: HarnessRole,
    outcome: &RoleExecutionOutcome,
) -> HarnessResult<()> {
    let RoleExecutionOutcome::MissingContext { request } = outcome else {
        return Ok(());
    };
    if role != HarnessRole::Reviewer {
        return Err(HarnessError::InvalidSubmission(
            "only a ready Reviewer may request missing context".to_owned(),
        ));
    }
    validate_artifact_text("missing context reason", &request.reason)?;
    if request.blocked_verification_units.is_empty()
        || request.blocked_verification_units.len() > MAX_SUBMISSION_LIST_ITEMS
    {
        return Err(HarnessError::InvalidSubmission(format!(
            "missing context must block between one and {MAX_SUBMISSION_LIST_ITEMS} Reviewer verification units"
        )));
    }
    let mut previous_position = None;
    for unit in &request.blocked_verification_units {
        let position = plan
            .verification_requirements
            .iter()
            .position(|requirement| {
                requirement.unit == *unit
                    && matches!(
                        requirement.owner,
                        VerificationOwner::Role {
                            role: HarnessRole::Reviewer
                        }
                    )
            })
            .ok_or_else(|| {
                HarnessError::InvalidSubmission(format!(
                    "missing context unit `{}` is not owned by the ready Reviewer",
                    unit.as_str()
                ))
            })?;
        if previous_position.is_some_and(|previous| position <= previous) {
            return Err(HarnessError::InvalidSubmission(
                "missing context units must be unique and follow plan order".to_owned(),
            ));
        }
        previous_position = Some(position);
    }
    validate_subject_evidence(
        plan,
        role_bundle,
        invocation_subject,
        &request.subject_evidence,
    )?;
    validate_missing_context_relative_path(request.candidate.relative_path())
}

fn validate_missing_context_relative_path(relative_path: &str) -> HarnessResult<()> {
    validate_single_line(
        "missing context candidate path",
        relative_path,
        MAX_TARGET_LENGTH,
    )
    .and_then(|()| {
        validate_relative_path(Path::new(relative_path), "missing context candidate path")
    })
    .map_err(|error| {
        HarnessError::InvalidSubmission(format!(
            "missing context candidate path is invalid: {error}"
        ))
    })
}

fn validate_writer_artifact(
    plan: &ResolvedHarnessPlan,
    targets: &[TargetBinding],
    curation_kind: Option<CurationKind>,
    artifact: &WriterArtifact,
) -> HarnessResult<()> {
    match (plan.action, artifact) {
        (
            HarnessAction::CodeWrite | HarnessAction::DocumentWrite,
            WriterArtifact::Changes { changes },
        ) => validate_file_changes(targets, changes),
        (
            HarnessAction::VaultCuration,
            WriterArtifact::Curation {
                curation_kind: submitted_kind,
                entries,
            },
        ) => {
            let planned_kind = curation_kind.ok_or_else(|| {
                HarnessError::InvalidPlan("curation writer task is missing its kind".to_owned())
            })?;
            validate_curation_entries(plan, *submitted_kind, entries, true)?;
            if *submitted_kind != planned_kind {
                return Err(HarnessError::InvalidSubmission(
                    "curation artifact kind does not match the Writer task".to_owned(),
                ));
            }
            Ok(())
        }
        _ => Err(HarnessError::InvalidSubmission(
            "Writer artifact does not match the planned action".to_owned(),
        )),
    }
}

fn validate_specialist_artifact(
    metadata: &PreparedRoleMetadata,
    source_targets: &[TargetBinding],
    artifact: &SpecialistArtifact,
) -> HarnessResult<()> {
    let planned_paths = source_targets
        .iter()
        .map(|target| target.workspace_relative_path.clone())
        .collect::<Vec<_>>();
    if artifact.source_targets != planned_paths
        || artifact.context_bundle_digest != metadata.scope.context_bundle_digest
    {
        return Err(HarnessError::InvalidSubmission(
            "Specialist artifact does not match its prepared targets or context bundle".to_owned(),
        ));
    }
    validate_artifact_text("Specialist output", &artifact.output)
}

fn validate_requirement_results(
    plan: &ResolvedHarnessPlan,
    role_bundle: &HarnessRoleBundle,
    requirements: &[VerificationRequirement],
    results: &[RequirementResult],
    subject: &EvaluationSubject,
) -> HarnessResult<()> {
    if requirements.len() != results.len() {
        return Err(HarnessError::InvalidSubmission(
            "role requirement results must exactly match the assigned requirements".to_owned(),
        ));
    }
    for (requirement, result) in requirements.iter().zip(results) {
        if requirement.unit != result.unit {
            return Err(HarnessError::InvalidSubmission(
                "role requirement result order does not match the assigned requirements".to_owned(),
            ));
        }
        validate_artifact_text("requirement result detail", &result.detail)?;
        if result.evidence.is_empty() {
            return Err(HarnessError::InvalidSubmission(
                "role requirement results require at least one evidence reference".to_owned(),
            ));
        }
        validate_result_evidence(&result.evidence)?;
        if result
            .evidence
            .iter()
            .any(|reference| !role_evidence_is_allowed(plan, role_bundle, subject, reference))
        {
            return Err(HarnessError::InvalidSubmission(
                "role requirement evidence contains a reference outside the prepared role scope"
                    .to_owned(),
            ));
        }
        if !result
            .evidence
            .iter()
            .any(|reference| evidence_binds_subject(plan, reference, subject))
        {
            return Err(HarnessError::InvalidSubmission(
                "role requirement evidence does not bind the evaluated subject".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_subject_evidence(
    plan: &ResolvedHarnessPlan,
    role_bundle: &HarnessRoleBundle,
    subject: &EvaluationSubject,
    evidence: &[ResultEvidenceReference],
) -> HarnessResult<()> {
    if evidence.is_empty() {
        return Err(HarnessError::InvalidSubmission(
            "Verifier and Reviewer results require subject evidence".to_owned(),
        ));
    }
    validate_result_evidence(evidence)?;
    if evidence
        .iter()
        .any(|reference| !role_evidence_is_allowed(plan, role_bundle, subject, reference))
    {
        return Err(HarnessError::InvalidSubmission(
            "role subject evidence contains a reference outside the prepared role scope".to_owned(),
        ));
    }
    if !evidence
        .iter()
        .any(|reference| evidence_binds_subject(plan, reference, subject))
    {
        return Err(HarnessError::InvalidSubmission(
            "role subject evidence does not bind the evaluated subject".to_owned(),
        ));
    }
    validate_frozen_target_evidence_coverage(plan, subject, evidence)
}

pub(super) fn validate_frozen_target_evidence_coverage(
    plan: &ResolvedHarnessPlan,
    subject: &EvaluationSubject,
    evidence: &[ResultEvidenceReference],
) -> HarnessResult<()> {
    let EvaluationSubject::FrozenTargets { target_set_digest } = subject else {
        return Ok(());
    };
    if target_set_digest != &serialized_digest(&plan.targets)? {
        return Err(HarnessError::InvalidSubmission(
            "frozen-target evidence is bound to the wrong target set".to_owned(),
        ));
    }
    let expected = plan
        .targets
        .iter()
        .map(|target| {
            let TargetState::Existing { content_digest } = &target.state else {
                return Err(HarnessError::InvalidPlan(
                    "frozen-target review requires every target to exist".to_owned(),
                ));
            };
            Ok((
                target.workspace_relative_path.clone(),
                content_digest.clone(),
            ))
        })
        .collect::<HarnessResult<BTreeSet<_>>>()?;
    let submitted = evidence
        .iter()
        .filter_map(|reference| {
            let ResultEvidenceReference::Target {
                workspace_relative_path,
                content_digest,
                ..
            } = reference
            else {
                return None;
            };
            Some((workspace_relative_path.clone(), content_digest.clone()))
        })
        .collect::<BTreeSet<_>>();
    if submitted != expected {
        return Err(HarnessError::InvalidSubmission(
            "each frozen-target role result must provide evidence coverage for every planned target"
                .to_owned(),
        ));
    }
    Ok(())
}

fn validate_review_observations(
    plan: &ResolvedHarnessPlan,
    role_bundle: &HarnessRoleBundle,
    subject: &EvaluationSubject,
    findings: &[BlockingFinding],
    improvements: &[ImprovementOpportunity],
) -> HarnessResult<()> {
    if findings.len() > MAX_SUBMISSION_LIST_ITEMS || improvements.len() > MAX_SUBMISSION_LIST_ITEMS
    {
        return Err(HarnessError::InvalidSubmission(format!(
            "review observations exceed the {MAX_SUBMISSION_LIST_ITEMS} item limit"
        )));
    }
    for finding in findings {
        validate_artifact_text("blocking finding", &finding.message)?;
        if finding.evidence.is_empty() {
            return Err(HarnessError::InvalidSubmission(
                "blocking findings require evidence".to_owned(),
            ));
        }
        validate_result_evidence(&finding.evidence)?;
        if finding
            .evidence
            .iter()
            .any(|reference| !role_evidence_is_allowed(plan, role_bundle, subject, reference))
        {
            return Err(HarnessError::InvalidSubmission(
                "blocking finding evidence contains a reference outside the prepared role scope"
                    .to_owned(),
            ));
        }
        if !finding
            .evidence
            .iter()
            .any(|reference| evidence_binds_subject(plan, reference, subject))
        {
            return Err(HarnessError::InvalidSubmission(
                "blocking finding evidence does not bind the evaluated subject".to_owned(),
            ));
        }
    }
    for improvement in improvements {
        validate_artifact_text("improvement opportunity", &improvement.message)?;
        if improvement.evidence.is_empty() {
            return Err(HarnessError::InvalidSubmission(
                "improvement opportunities require evidence".to_owned(),
            ));
        }
        validate_result_evidence(&improvement.evidence)?;
        if improvement
            .evidence
            .iter()
            .any(|reference| !role_evidence_is_allowed(plan, role_bundle, subject, reference))
        {
            return Err(HarnessError::InvalidSubmission(
                "improvement evidence contains a reference outside the prepared role scope"
                    .to_owned(),
            ));
        }
        if !improvement
            .evidence
            .iter()
            .any(|reference| evidence_binds_subject(plan, reference, subject))
        {
            return Err(HarnessError::InvalidSubmission(
                "improvement evidence does not bind the evaluated subject".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_professional_profile_improvement_result(
    requirements: &[VerificationRequirement],
    results: &[RequirementResult],
    improvements: &[ImprovementOpportunity],
) -> HarnessResult<()> {
    if improvements.is_empty()
        || !requirements
            .iter()
            .any(|requirement| requirement.unit == VerificationUnit::ProfessionalProfileArtifact)
    {
        return Ok(());
    }
    let result = results
        .iter()
        .find(|result| result.unit == VerificationUnit::ProfessionalProfileArtifact)
        .ok_or_else(|| {
            HarnessError::InvalidSubmission(
                "Reviewer result is missing the professional-profile requirement".to_owned(),
            )
        })?;
    if result.passed {
        return Err(HarnessError::InvalidSubmission(
            "professional-profile improvements require a failed ProfessionalProfileArtifact result"
                .to_owned(),
        ));
    }
    Ok(())
}

fn validate_reviewer_learning_candidates(
    plan: &ResolvedHarnessPlan,
    role_bundle: &HarnessRoleBundle,
    requirements: &[VerificationRequirement],
    results: &[RequirementResult],
    subject: &EvaluationSubject,
    candidates: &[ReviewerLearningCandidate],
) -> HarnessResult<()> {
    if candidates.len() > MAX_SUBMISSION_LIST_ITEMS {
        return Err(HarnessError::InvalidSubmission(format!(
            "Reviewer learning candidates exceed the {MAX_SUBMISSION_LIST_ITEMS} item limit"
        )));
    }
    let mut units = BTreeSet::new();
    for candidate in candidates {
        validate_single_line(
            "Reviewer learning candidate title",
            &candidate.title,
            MAX_OBJECTIVE_LENGTH,
        )?;
        validate_multiline(
            "Reviewer learning candidate guidance",
            &candidate.guidance,
            MAX_SUBMISSION_TEXT_BYTES,
        )?;
        validate_plan_digest(
            "Reviewer learning requirement result digest",
            &candidate.requirement_result_digest,
        )?;
        if candidate.evidence.is_empty() {
            return Err(HarnessError::InvalidSubmission(
                "Reviewer learning candidates require exact failed-result evidence".to_owned(),
            ));
        }
        validate_result_evidence(&candidate.evidence)?;
        if candidate
            .evidence
            .iter()
            .any(|reference| !role_evidence_is_allowed(plan, role_bundle, subject, reference))
        {
            return Err(HarnessError::InvalidSubmission(
                "Reviewer learning evidence is outside the prepared role scope".to_owned(),
            ));
        }
        if !candidate
            .evidence
            .iter()
            .any(|reference| evidence_binds_subject(plan, reference, subject))
        {
            return Err(HarnessError::InvalidSubmission(
                "Reviewer learning evidence does not bind the evaluated subject".to_owned(),
            ));
        }
        if !units.insert(candidate.failed_unit) {
            return Err(HarnessError::InvalidSubmission(
                "Reviewer learning candidates must identify unique failed verification units"
                    .to_owned(),
            ));
        }
        let requirement = requirements
            .iter()
            .find(|requirement| requirement.unit == candidate.failed_unit)
            .ok_or_else(|| {
                HarnessError::InvalidSubmission(
                    "Reviewer learning candidate unit is not assigned to Reviewer".to_owned(),
                )
            })?;
        let result = results
            .iter()
            .find(|result| result.unit == candidate.failed_unit)
            .ok_or_else(|| {
                HarnessError::InvalidSubmission(
                    "Reviewer learning candidate has no matching requirement result".to_owned(),
                )
            })?;
        if result.passed {
            return Err(HarnessError::InvalidSubmission(
                "Reviewer learning candidates require a failed requirement result".to_owned(),
            ));
        }
        if requirement.owner
            != (VerificationOwner::Role {
                role: HarnessRole::Reviewer,
            })
        {
            return Err(HarnessError::InvalidSubmission(
                "Reviewer learning candidate unit is not Reviewer-owned".to_owned(),
            ));
        }
        if candidate.requirement_result_digest != serialized_digest(result)?
            || candidate.evidence != result.evidence
        {
            return Err(HarnessError::InvalidSubmission(
                "Reviewer learning candidate does not match the exact failed requirement result"
                    .to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_promotion_proposals(
    plan: &ResolvedHarnessPlan,
    role_bundle: &HarnessRoleBundle,
    _subject: &EvaluationSubject,
    proposals: &[PromotionProposal],
) -> HarnessResult<()> {
    if proposals.len() > MAX_SUBMISSION_LIST_ITEMS {
        return Err(HarnessError::InvalidSubmission(format!(
            "promotion proposals exceed the {MAX_SUBMISSION_LIST_ITEMS} item limit"
        )));
    }
    let mut identifiers = BTreeSet::new();
    for proposal in proposals {
        validate_promotion_proposal_shape(proposal)?;
        if !identifiers.insert(proposal.identifier.as_str()) {
            return Err(HarnessError::InvalidSubmission(
                "promotion proposal identifiers must be unique".to_owned(),
            ));
        }
        let PromotionProposalOrigin::SpecialistMemory { evidence, .. } = &proposal.origin else {
            return Err(HarnessError::InvalidSubmission(
                "Specialist results cannot submit Reviewer learning promotions".to_owned(),
            ));
        };
        for reference in evidence {
            if !promotion_evidence_is_bound(plan, role_bundle, reference) {
                return Err(HarnessError::InvalidSubmission(
                    "promotion evidence is not an exact factual source in the prepared Specialist bundle"
                        .to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_promotion_proposal_shape(proposal: &PromotionProposal) -> HarnessResult<()> {
    validate_promotion_identifier(&proposal.identifier)?;
    proposal.owner.validate()?;
    if proposal.curation_kind == CurationKind::Journal {
        return Err(HarnessError::InvalidSubmission(
            "Journal content cannot be proposed by a model role".to_owned(),
        ));
    }
    validate_private_content_owner(&proposal.owner, proposal.curation_kind)?;
    validate_single_line("promotion title", &proposal.title, MAX_OBJECTIVE_LENGTH)?;
    validate_multiline(
        "promotion content",
        &proposal.content,
        MAX_SUBMISSION_TEXT_BYTES,
    )?;
    match &proposal.origin {
        PromotionProposalOrigin::SpecialistMemory {
            claim_kind,
            evidence,
        } => {
            let valid_claim = matches!(
                (proposal.curation_kind, claim_kind),
                (CurationKind::Fact, PromotionClaimKind::ObservedFact)
                    | (
                        CurationKind::Idea | CurationKind::Decision,
                        PromotionClaimKind::ModelProposal
                    )
                    | (
                        CurationKind::Knowledge | CurationKind::Ontology,
                        PromotionClaimKind::ObservedFact | PromotionClaimKind::ModelInterpretation
                    )
            );
            if !valid_claim {
                return Err(HarnessError::InvalidSubmission(
                    "promotion claim kind does not match the requested curation category"
                        .to_owned(),
                ));
            }
            if evidence.is_empty() {
                return Err(HarnessError::InvalidSubmission(
                    "promotion proposals require evidence".to_owned(),
                ));
            }
            if evidence.len() > MAX_SUBMISSION_LIST_ITEMS {
                return Err(HarnessError::InvalidSubmission(format!(
                    "promotion evidence exceeds the {MAX_SUBMISSION_LIST_ITEMS} item limit"
                )));
            }
            for reference in evidence {
                validate_relative_path(
                    Path::new(&reference.relative_path),
                    "promotion evidence source",
                )?;
                reference.source_owner.validate()?;
                validate_plan_digest("promotion evidence digest", &reference.content_digest)?;
                validate_artifact_text("promotion evidence locator", &reference.locator)?;
                if reference.source_owner != proposal.owner {
                    return Err(HarnessError::InvalidSubmission(
                        "promotion evidence owner must match the proposed content owner".to_owned(),
                    ));
                }
            }
        }
        PromotionProposalOrigin::ReviewerLearning {
            learning_id,
            requirement_result_digest,
            evidence,
            ..
        } => {
            if proposal.curation_kind != CurationKind::Knowledge {
                return Err(HarnessError::InvalidSubmission(
                    "Reviewer learning promotions must use Knowledge curation".to_owned(),
                ));
            }
            validate_learning_identifier(learning_id)?;
            validate_plan_digest(
                "Reviewer learning requirement result digest",
                requirement_result_digest,
            )?;
            if evidence.is_empty() {
                return Err(HarnessError::InvalidSubmission(
                    "Reviewer learning promotions require failed-result evidence".to_owned(),
                ));
            }
            validate_result_evidence(evidence)?;
        }
    }
    Ok(())
}

fn validate_learning_identifier(identifier: &str) -> HarnessResult<()> {
    let Some(suffix) = identifier.strip_prefix("learning-") else {
        return Err(HarnessError::InvalidSubmission(
            "learning identifier must use the `learning-` prefix".to_owned(),
        ));
    };
    if suffix.len() != 16
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(HarnessError::InvalidSubmission(
            "learning identifier must end with 16 lowercase hexadecimal characters".to_owned(),
        ));
    }
    Ok(())
}

fn validate_reviewer_learning_proposal(
    plan: &ResolvedHarnessPlan,
    evaluation: &HarnessTaskEvaluation,
    proposal: &PromotionProposal,
) -> HarnessResult<()> {
    let PromotionProposalOrigin::ReviewerLearning {
        source_action,
        source_intent,
        failed_unit,
        requirement_result_digest,
        evidence,
        ..
    } = &proposal.origin
    else {
        return Ok(());
    };
    if proposal.owner != plan.owner
        || *source_action != plan.action
        || *source_intent != plan.intent
    {
        return Err(HarnessError::InvalidSubmission(
            "Reviewer learning proposal applicability does not match its source plan".to_owned(),
        ));
    }
    let failed = evaluation
        .requirements
        .iter()
        .find(|result| result.requirement.unit == *failed_unit && !result.passed)
        .ok_or_else(|| {
            HarnessError::InvalidSubmission(
                "Reviewer learning proposal has no exact failed evaluated requirement".to_owned(),
            )
        })?;
    let reviewer_result = evaluation
        .execution_record
        .role_results
        .iter()
        .find(|result| result.role == HarnessRole::Reviewer)
        .and_then(|result| match &result.outcome {
            RoleExecutionOutcome::Completed {
                result:
                    CompletedRoleResult::Reviewer {
                        requirement_results,
                        ..
                    },
            } => requirement_results
                .iter()
                .find(|result| result.unit == *failed_unit),
            RoleExecutionOutcome::Completed { .. }
            | RoleExecutionOutcome::MissingContext { .. }
            | RoleExecutionOutcome::Failed { .. }
            | RoleExecutionOutcome::Cancelled
            | RoleExecutionOutcome::TimedOut
            | RoleExecutionOutcome::Unsupported { .. } => None,
        })
        .ok_or_else(|| {
            HarnessError::InvalidSubmission(
                "Reviewer learning proposal source requirement result is missing".to_owned(),
            )
        })?;
    if reviewer_result.passed
        || *requirement_result_digest != serialized_digest(reviewer_result)?
        || evidence != &reviewer_result.evidence
        || failed.detail != reviewer_result.detail
        || failed.evidence != reviewer_result.evidence
    {
        return Err(HarnessError::InvalidSubmission(
            "Reviewer learning proposal does not match its failed evaluation evidence".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn canonical_learning_markdown(handoff: &PromotionHandoff) -> HarnessResult<String> {
    let PromotionProposalOrigin::ReviewerLearning {
        learning_id,
        source_action,
        source_intent,
        failed_unit,
        ..
    } = &handoff.proposal.origin
    else {
        return Err(HarnessError::InvalidSubmission(
            "canonical learning Markdown requires a Reviewer learning handoff".to_owned(),
        ));
    };
    let frontmatter = LearningDocumentFrontmatter {
        title: handoff.proposal.title.clone(),
        scope: owner_scope(&handoff.proposal.owner).to_owned(),
        export: false,
        learning: LearningMetadata {
            learning_id: learning_id.clone(),
            owner: handoff.proposal.owner.clone(),
            action: *source_action,
            intent: *source_intent,
            verification_unit: *failed_unit,
            source_task_evaluation_receipt_digest: handoff
                .source_task_evaluation_receipt_digest
                .clone(),
            promotion_handoff_digest: handoff.handoff_digest.clone(),
        },
    };
    let yaml = serde_yaml_ng::to_string(&frontmatter).map_err(|error| {
        HarnessError::InvalidSubmission(format!(
            "canonical learning frontmatter could not serialize: {error}"
        ))
    })?;
    Ok(format!(
        "---\n{yaml}---\n# {}\n\n{}",
        handoff.proposal.title, handoff.proposal.content
    ))
}

const fn owner_scope(owner: &DataOwner) -> &'static str {
    match owner {
        DataOwner::Profile => "profile",
        DataOwner::PersonalBusiness | DataOwner::Personal | DataOwner::PersonalProject { .. } => {
            "personal"
        }
        DataOwner::CommonWork | DataOwner::Company { .. } | DataOwner::CompanyProject { .. } => {
            "work"
        }
    }
}

fn validate_promotion_identifier(identifier: &str) -> HarnessResult<()> {
    let Some(suffix) = identifier.strip_prefix("proposal-") else {
        return Err(HarnessError::InvalidSubmission(
            "promotion proposal identifier must use the `proposal-` prefix".to_owned(),
        ));
    };
    if suffix.len() != 16
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(HarnessError::InvalidSubmission(
            "promotion proposal identifier must end with 16 lowercase hexadecimal characters"
                .to_owned(),
        ));
    }
    Ok(())
}

fn promotion_evidence_is_bound(
    plan: &ResolvedHarnessPlan,
    role_bundle: &HarnessRoleBundle,
    reference: &PromotionEvidenceReference,
) -> bool {
    let is_in_bundle = role_bundle.bound_documents().any(|document| {
        document.relative_path == reference.relative_path
            && document.content_digest == reference.content_digest
    });
    if !is_in_bundle {
        return false;
    }
    match reference.source_kind {
        PromotionSourceKind::Target => {
            reference.source_owner == plan.owner
                && plan.targets.iter().any(|target| {
                    target.workspace_relative_path == reference.relative_path
                        && matches!(
                            &target.state,
                            TargetState::Existing { content_digest }
                                if content_digest == &reference.content_digest
                        )
                })
        }
        PromotionSourceKind::GrantedEvidence => {
            let [grant] = plan.context_grants.as_slice() else {
                return false;
            };
            reference.source_owner == grant.owner
                && plan.evidence_sources.iter().any(|source| {
                    source.repository_relative_path == reference.relative_path
                        && source.content_digest == reference.content_digest
                })
        }
        PromotionSourceKind::Retrieval => {
            reference.source_owner == plan.owner
                && role_bundle.bound_documents().any(|document| {
                    document.source == HarnessBoundDocumentSource::Retrieval
                        && document.relative_path == reference.relative_path
                        && document.content_digest == reference.content_digest
                })
        }
    }
}

fn role_evidence_is_allowed(
    plan: &ResolvedHarnessPlan,
    role_bundle: &HarnessRoleBundle,
    subject: &EvaluationSubject,
    reference: &ResultEvidenceReference,
) -> bool {
    match reference {
        ResultEvidenceReference::Target {
            workspace_relative_path,
            content_digest,
            ..
        } => plan.targets.iter().any(|target| {
            target.workspace_relative_path == *workspace_relative_path
                && matches!(
                    &target.state,
                    TargetState::Existing {
                        content_digest: expected
                    } if expected == content_digest
                )
        }),
        ResultEvidenceReference::BoundDocument {
            relative_path,
            content_digest,
            ..
        } => role_bundle.bound_documents().any(|document| {
            document.relative_path == *relative_path && document.content_digest == *content_digest
        }),
        ResultEvidenceReference::ProducedArtifact { .. } => {
            evidence_binds_subject(plan, reference, subject)
        }
        ResultEvidenceReference::ToolResult { .. } => false,
    }
}

fn evidence_binds_subject(
    plan: &ResolvedHarnessPlan,
    reference: &ResultEvidenceReference,
    subject: &EvaluationSubject,
) -> bool {
    match (reference, subject) {
        (
            ResultEvidenceReference::ProducedArtifact {
                artifact_digest, ..
            },
            EvaluationSubject::ProducedArtifact {
                artifact_digest: expected,
            },
        ) => artifact_digest == expected,
        (
            ResultEvidenceReference::Target {
                workspace_relative_path,
                content_digest,
                ..
            },
            EvaluationSubject::FrozenTargets { .. },
        ) => plan.targets.iter().any(|target| {
            target.workspace_relative_path == *workspace_relative_path
                && matches!(
                    &target.state,
                    TargetState::Existing {
                        content_digest: expected
                    } if expected == content_digest
                )
        }),
        _ => false,
    }
}

fn validate_result_evidence(evidence: &[ResultEvidenceReference]) -> HarnessResult<()> {
    if evidence.len() > MAX_SUBMISSION_LIST_ITEMS {
        return Err(HarnessError::InvalidSubmission(format!(
            "evidence references exceed the {MAX_SUBMISSION_LIST_ITEMS} item limit"
        )));
    }
    for reference in evidence {
        match reference {
            ResultEvidenceReference::Target {
                workspace_relative_path,
                content_digest,
                locator,
            } => {
                validate_relative_path(Path::new(workspace_relative_path), "evidence target")?;
                validate_plan_digest("evidence target digest", content_digest)?;
                validate_artifact_text("evidence locator", locator)?;
            }
            ResultEvidenceReference::BoundDocument {
                relative_path,
                content_digest,
                locator,
            } => {
                validate_relative_path(Path::new(relative_path), "bound evidence document")?;
                validate_plan_digest("bound evidence digest", content_digest)?;
                validate_artifact_text("evidence locator", locator)?;
            }
            ResultEvidenceReference::ToolResult { result_digest, .. } => {
                validate_plan_digest("tool result digest", result_digest)?;
            }
            ResultEvidenceReference::ProducedArtifact {
                artifact_digest,
                locator,
            } => {
                validate_plan_digest("produced artifact digest", artifact_digest)?;
                validate_artifact_text("evidence locator", locator)?;
            }
        }
    }
    Ok(())
}

fn validate_tool_evidence(
    invocation: &ToolInvocationContract,
    evidence: &ToolEvidenceSet,
) -> HarnessResult<()> {
    if evidence.invocation_digest != invocation.invocation_digest
        || evidence.results.len() != invocation.requirements.len()
        || evidence.executions.is_empty()
        || evidence.executions.len() > MAX_SUBMISSION_LIST_ITEMS
    {
        return Err(HarnessError::InvalidSubmission(
            "tool evidence does not match its Harness-issued invocation".to_owned(),
        ));
    }
    let mut execution_digests = BTreeSet::new();
    for execution in &evidence.executions {
        if !invocation
            .requirements
            .iter()
            .any(|requirement| requirement.unit == execution.unit)
        {
            return Err(HarnessError::InvalidSubmission(
                "tool execution belongs to an unplanned verification unit".to_owned(),
            ));
        }
        validate_tool_execution_evidence(&evidence.invocation_digest, execution)?;
        if !execution_digests.insert(execution.evidence_digest.as_str()) {
            return Err(HarnessError::InvalidSubmission(
                "tool evidence must not repeat an execution digest".to_owned(),
            ));
        }
    }
    for (requirement, result) in invocation.requirements.iter().zip(&evidence.results) {
        if requirement.owner != VerificationOwner::Tool || requirement.unit != result.unit {
            return Err(HarnessError::InvalidSubmission(
                "tool evidence requirement order is invalid".to_owned(),
            ));
        }
        validate_artifact_text("tool verification detail", &result.detail)?;
        validate_result_evidence(&result.evidence)?;
        let matching_executions = evidence
            .executions
            .iter()
            .filter(|execution| execution.unit == requirement.unit)
            .collect::<Vec<_>>();
        if matching_executions.is_empty()
            || result.evidence.len() != matching_executions.len()
            || result
                .evidence
                .iter()
                .zip(&matching_executions)
                .any(|(reference, execution)| {
                    !matches!(
                        reference,
                        ResultEvidenceReference::ToolResult {
                            unit,
                            result_digest
                        } if *unit == requirement.unit
                            && result_digest == &execution.evidence_digest
                    )
                })
        {
            return Err(HarnessError::InvalidSubmission(
                "tool requirement evidence must contain every matching structured tool result in execution order"
                    .to_owned(),
            ));
        }
        let all_executions_passed = matching_executions
            .iter()
            .all(|execution| matches!(execution.termination, ToolTermination::Exited { code: 0 }));
        if result.passed != all_executions_passed {
            return Err(HarnessError::InvalidSubmission(
                "tool verification result does not match the aggregate process terminations"
                    .to_owned(),
            ));
        }
    }
    let expected_digest = serialized_digest(&(
        &evidence.invocation_digest,
        &evidence.results,
        &evidence.executions,
    ))?;
    if evidence.evidence_set_digest != expected_digest {
        return Err(HarnessError::InvalidSubmission(
            "tool evidence set digest is invalid".to_owned(),
        ));
    }
    Ok(())
}

fn validate_tool_execution_evidence(
    invocation_digest: &str,
    execution: &ToolCommandEvidence,
) -> HarnessResult<()> {
    if execution.command.is_empty() || execution.command.len() > MAX_SUBMISSION_LIST_ITEMS {
        return Err(HarnessError::InvalidSubmission(format!(
            "tool command must contain between one and {MAX_SUBMISSION_LIST_ITEMS} arguments"
        )));
    }
    for argument in &execution.command {
        validate_single_line("tool command argument", argument, MAX_TARGET_LENGTH)?;
    }
    match &execution.termination {
        ToolTermination::Exited { code } if *code >= 0 => {}
        ToolTermination::Exited { .. } => {
            return Err(HarnessError::InvalidSubmission(
                "tool exit code must not be negative".to_owned(),
            ));
        }
        ToolTermination::Signaled { signal } if *signal > 0 => {}
        ToolTermination::Signaled { .. } => {
            return Err(HarnessError::InvalidSubmission(
                "tool signal must be positive".to_owned(),
            ));
        }
        ToolTermination::FailedToStart { message } => {
            validate_artifact_text("tool start failure", message)?;
        }
    }
    validate_plan_digest("tool standard output digest", &execution.stdout_digest)?;
    validate_plan_digest("tool standard error digest", &execution.stderr_digest)?;
    let expected_digest = serialized_digest(&(
        invocation_digest,
        execution.unit,
        &execution.command,
        &execution.termination,
        &execution.stdout_digest,
        &execution.stderr_digest,
    ))?;
    if execution.evidence_digest != expected_digest {
        return Err(HarnessError::InvalidSubmission(
            "tool execution evidence digest is invalid".to_owned(),
        ));
    }
    Ok(())
}

fn validate_execution_lifecycle(
    capabilities: &RoleRuntimeCapabilities,
    lifecycle: &ReportedRoleLifecycle,
    outcome: &RoleExecutionOutcome,
) -> HarnessResult<()> {
    if lifecycle.started_at_millis == 0
        || lifecycle.started_at_millis > lifecycle.context_ready_at_millis
        || lifecycle.context_ready_at_millis > lifecycle.terminal_at_millis
        || lifecycle.terminal_at_millis > lifecycle.closed_at_millis
        || lifecycle.first_output_at_millis.is_some_and(|first| {
            first < lifecycle.context_ready_at_millis || first > lifecycle.terminal_at_millis
        })
    {
        return Err(HarnessError::InvalidSubmission(format!(
            "role {:?} lifecycle timestamps are not monotonic",
            lifecycle.role
        )));
    }
    let expected_terminal = match outcome {
        RoleExecutionOutcome::Completed { .. } => {
            if lifecycle.first_output_at_millis.is_none() {
                return Err(HarnessError::UnsupportedRuntime(format!(
                    "role {:?} completed without usable output",
                    lifecycle.role
                )));
            }
            RoleTerminalState::Completed
        }
        RoleExecutionOutcome::MissingContext { .. } => {
            if lifecycle.first_output_at_millis.is_none() {
                return Err(HarnessError::InvalidSubmission(format!(
                    "role {:?} requested missing context without usable output",
                    lifecycle.role
                )));
            }
            RoleTerminalState::MissingContext
        }
        RoleExecutionOutcome::Failed { message } => {
            validate_artifact_text("role failure message", message)?;
            RoleTerminalState::Failed
        }
        RoleExecutionOutcome::Cancelled => RoleTerminalState::Cancelled,
        RoleExecutionOutcome::TimedOut => RoleTerminalState::TimedOut,
        RoleExecutionOutcome::Unsupported { reason } => {
            validate_artifact_text("unsupported role reason", reason)?;
            RoleTerminalState::Unsupported
        }
    };
    if lifecycle.terminal_state != expected_terminal {
        return Err(HarnessError::InvalidSubmission(
            "role lifecycle terminal state does not match its result outcome".to_owned(),
        ));
    }
    match (
        lifecycle.interrupt_requested_at_millis,
        lifecycle.grace_deadline_at_millis,
    ) {
        (None, None) => {
            let duration = lifecycle
                .terminal_at_millis
                .checked_sub(lifecycle.started_at_millis)
                .ok_or_else(|| {
                    HarnessError::InvalidSubmission(
                        "role execution timestamps are invalid".to_owned(),
                    )
                })?;
            if duration > capabilities.max_role_execution_millis {
                return Err(HarnessError::UnsupportedRuntime(format!(
                    "role {:?} exceeded the execution time limit",
                    lifecycle.role
                )));
            }
        }
        (Some(interrupt), Some(grace_deadline))
            if lifecycle.context_ready_at_millis <= interrupt
                && interrupt <= lifecycle.terminal_at_millis
                && lifecycle.terminal_at_millis <= grace_deadline =>
        {
            let duration = interrupt
                .checked_sub(lifecycle.started_at_millis)
                .ok_or_else(|| {
                    HarnessError::InvalidSubmission(
                        "role execution timestamps are invalid".to_owned(),
                    )
                })?;
            let grace = grace_deadline.checked_sub(interrupt).ok_or_else(|| {
                HarnessError::InvalidSubmission("role grace timestamps are invalid".to_owned())
            })?;
            if duration > capabilities.max_role_execution_millis
                || grace > capabilities.max_role_grace_millis
            {
                return Err(HarnessError::UnsupportedRuntime(format!(
                    "role {:?} exceeded the execution or grace time limit",
                    lifecycle.role
                )));
            }
        }
        _ => {
            return Err(HarnessError::InvalidSubmission(
                "role interrupt and grace timestamps are invalid".to_owned(),
            ));
        }
    }
    Ok(())
}

fn evaluation_context_ids(evaluation: &HarnessTaskEvaluation) -> impl Iterator<Item = String> + '_ {
    evaluation
        .role_executions
        .iter()
        .map(|binding| binding.context_id.clone())
}

fn validate_task_evaluation_receipt(evaluation: &HarnessTaskEvaluation) -> HarnessResult<()> {
    validate_serialized_size("task evaluation", evaluation, MAX_TASK_EVALUATION_BYTES)?;
    let expected = task_evaluation_receipt_digest(evaluation)?;
    if evaluation.task_evaluation_receipt_digest != expected {
        return Err(HarnessError::InvalidSubmission(
            "task evaluation receipt digest is invalid".to_owned(),
        ));
    }
    Ok(())
}

impl PromotionHandoff {
    /// Creates an advisory handoff from a currently held, fully recomputed Core evaluation.
    pub fn from_evaluation(
        plan: &ResolvedHarnessPlan,
        prepared: &PreparedRoleRun,
        revision_history: &[HarnessTaskEvaluation],
        evaluation: &HarnessTaskEvaluation,
        proposal_identifier: &str,
    ) -> HarnessResult<Self> {
        validate_task_evaluation_receipt(evaluation)?;
        let recomputed = evaluate_execution_record(
            plan,
            prepared,
            revision_history,
            &evaluation.execution_record,
        )?;
        if &recomputed != evaluation {
            return Err(HarnessError::InvalidSubmission(
                "promotion handoff source does not match the current Core evaluation".to_owned(),
            ));
        }
        if evaluation.execution_status != ExecutionStatus::Completed {
            return Err(HarnessError::InvalidSubmission(
                "promotion handoff requires a completed task evaluation".to_owned(),
            ));
        }
        let PromotionState::RequiresSeparateCuration { proposals } = &evaluation.promotion_state
        else {
            return Err(HarnessError::InvalidSubmission(
                "task evaluation has no promotion proposals".to_owned(),
            ));
        };
        let proposal = proposals
            .iter()
            .find(|proposal| proposal.identifier == proposal_identifier)
            .cloned()
            .ok_or_else(|| {
                HarnessError::InvalidSubmission(format!(
                    "promotion proposal `{proposal_identifier}` is not part of the current task evaluation"
                ))
            })?;
        match &proposal.origin {
            PromotionProposalOrigin::SpecialistMemory { .. } => {
                if evaluation.subject_status != SubjectStatus::Accepted {
                    return Err(HarnessError::InvalidSubmission(
                        "Specialist memory promotion requires an accepted task evaluation"
                            .to_owned(),
                    ));
                }
            }
            PromotionProposalOrigin::ReviewerLearning { .. } => {
                if evaluation.subject_status != SubjectStatus::Rejected {
                    return Err(HarnessError::InvalidSubmission(
                        "Reviewer learning promotion requires a rejected task evaluation"
                            .to_owned(),
                    ));
                }
                validate_reviewer_learning_proposal(plan, evaluation, &proposal)?;
            }
        }
        let mut handoff = Self {
            source_resolved_plan_digest: evaluation.resolved_plan_digest.clone(),
            source_task_evaluation_receipt_digest: evaluation
                .task_evaluation_receipt_digest
                .clone(),
            proposal,
            handoff_digest: String::new(),
        };
        handoff.handoff_digest = handoff.calculate_digest()?;
        handoff.validate()?;
        Ok(handoff)
    }

    pub(super) fn validate(&self) -> HarnessResult<()> {
        validate_plan_digest(
            "promotion handoff source resolved plan digest",
            &self.source_resolved_plan_digest,
        )?;
        validate_plan_digest(
            "promotion handoff source task evaluation receipt digest",
            &self.source_task_evaluation_receipt_digest,
        )?;
        validate_promotion_proposal_shape(&self.proposal)?;
        validate_plan_digest("promotion handoff digest", &self.handoff_digest)?;
        if self.handoff_digest != self.calculate_digest()? {
            return Err(HarnessError::InvalidSubmission(
                "promotion handoff digest is invalid".to_owned(),
            ));
        }
        Ok(())
    }

    fn calculate_digest(&self) -> HarnessResult<String> {
        serialized_digest(&(
            &self.source_resolved_plan_digest,
            &self.source_task_evaluation_receipt_digest,
            &self.proposal,
        ))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessExecutionState {
    Begun,
    Executing,
    Evaluated,
    Validated,
    Finalized,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessExecutionRecord {
    pub version: u32,
    pub run_identifier: String,
    pub prepared_run_digest: String,
    pub prepared_run_raw_digest: String,
    pub sequence: u64,
    pub predecessor_record_digest: Option<String>,
    pub state: HarnessExecutionState,
    pub revision_history: Vec<HarnessTaskEvaluation>,
    pub role_execution: RoleExecutionRecord,
    pub ready_role_invocations: Vec<RoleInvocationContract>,
    pub ready_tool_invocation: Option<ToolInvocationContract>,
    pub evaluation: Option<HarnessTaskEvaluation>,
    pub exact_tool_evidence: Option<ToolExecutionEvidence>,
    pub tool_evidence_digest: Option<String>,
    pub candidate_digest: Option<String>,
    /// Run-bound evaluation receipt digest that additionally binds the durable run context.
    pub run_evaluation_receipt_digest: Option<String>,
    pub validation_receipt_digest: Option<String>,
    pub finalization_digest: Option<String>,
    pub batch_receipt_digest: Option<String>,
    pub record_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum HarnessApplyFailureEvidence {
    BatchApply {
        message: String,
        receipt: Box<HarnessBatchApplyLifecycleReceipt>,
    },
    BeforeBatch {
        message: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub enum HarnessApplyAttemptState {
    PendingApply,
    Failure {
        evidence: HarnessApplyFailureEvidence,
    },
    SourceAppliedFinalizationPending {
        message: String,
        applied: Box<AppliedHarnessBatch>,
    },
    AbortedBeforeMutation {
        verified_at_millis: u64,
    },
    AbortedAfterRollback {
        verified_at_millis: u64,
        rollback_receipt: Box<HarnessBatchApplyLifecycleReceipt>,
    },
    AppliedFinalized {
        resulting_record: Box<HarnessExecutionRecord>,
        apply_receipt: Box<HarnessApplyReceipt>,
    },
    RecoveredFinalized {
        resulting_record: Box<HarnessExecutionRecord>,
        apply_receipt: Box<HarnessApplyReceipt>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessApplyAttemptReceipt {
    pub version: u32,
    pub run_identifier: String,
    pub attempt_identifier: String,
    pub validated_record_digest: String,
    pub validated_sequence: u64,
    pub prepared_run_digest: String,
    /// Digest of the complete serialized `HarnessPlan` artifact.
    pub harness_plan_digest: String,
    /// Digest of the embedded engine-resolved plan.
    pub resolved_plan_digest: String,
    pub candidate_digest: String,
    pub validation_receipt_digest: String,
    pub frozen_identity_set_digest: String,
    pub workspace_locator_map_digest: String,
    pub expected_batch_identifier: String,
    pub expected_journal_relative_path: String,
    pub expected_completion_relative_path: String,
    pub attempt_state: HarnessApplyAttemptState,
    pub receipt_digest: String,
}

impl HarnessApplyAttemptReceipt {
    pub(super) fn pending(
        prepared: &PreparedHarnessRun,
        record: &HarnessExecutionRecord,
        attempt_identifier: String,
    ) -> HarnessResult<Self> {
        if record.state != HarnessExecutionState::Validated {
            return Err(HarnessError::InvalidSubmission(
                "a new apply attempt requires the current validated head".to_owned(),
            ));
        }
        let candidate_digest = record.candidate_digest.clone().ok_or_else(|| {
            HarnessError::InvalidSubmission(
                "a new apply attempt requires a candidate digest".to_owned(),
            )
        })?;
        let validation_receipt_digest =
            record.validation_receipt_digest.clone().ok_or_else(|| {
                HarnessError::InvalidSubmission(
                    "a new apply attempt requires a validation receipt".to_owned(),
                )
            })?;
        let (
            expected_batch_identifier,
            expected_journal_relative_path,
            expected_completion_relative_path,
        ) = repository::batch_paths_for_attempt(
            &prepared.plan.resolved_request.resolved_plan_digest,
            &candidate_digest,
            &attempt_identifier,
        )?;
        let mut receipt = Self {
            version: HARNESS_SCHEMA_VERSION,
            run_identifier: record.run_identifier.clone(),
            attempt_identifier,
            validated_record_digest: record.record_digest.clone(),
            validated_sequence: record.sequence,
            prepared_run_digest: prepared.prepared_run_digest.clone(),
            harness_plan_digest: prepared.plan_identity.normalized_value_digest.clone(),
            resolved_plan_digest: prepared.plan.resolved_request.resolved_plan_digest.clone(),
            candidate_digest,
            validation_receipt_digest,
            frozen_identity_set_digest: prepared.plan.frozen_targets.identity_set_digest.clone(),
            workspace_locator_map_digest: prepared
                .plan
                .frozen_targets
                .workspace_locator_map_digest
                .clone(),
            expected_batch_identifier,
            expected_journal_relative_path,
            expected_completion_relative_path,
            attempt_state: HarnessApplyAttemptState::PendingApply,
            receipt_digest: String::new(),
        };
        receipt.refresh_digest()?;
        receipt.validate(prepared)?;
        Ok(receipt)
    }

    fn refresh_digest(&mut self) -> HarnessResult<()> {
        self.receipt_digest = serialized_digest(&(
            self.version,
            &self.run_identifier,
            &self.attempt_identifier,
            &self.validated_record_digest,
            self.validated_sequence,
            &self.prepared_run_digest,
            &self.harness_plan_digest,
            &self.resolved_plan_digest,
            &self.candidate_digest,
            &self.validation_receipt_digest,
            &self.frozen_identity_set_digest,
            &self.workspace_locator_map_digest,
            &self.expected_batch_identifier,
            &self.expected_journal_relative_path,
            &self.expected_completion_relative_path,
            &self.attempt_state,
        ))?;
        Ok(())
    }

    pub(super) fn replace_state(
        &self,
        attempt_state: HarnessApplyAttemptState,
    ) -> HarnessResult<Self> {
        let mut next = self.clone();
        next.attempt_state = attempt_state;
        next.refresh_digest()?;
        Ok(next)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "apply-attempt validation keeps every prepared, batch-path, failure, and terminal cross-binding in one fail-closed boundary"
    )]
    pub fn validate(&self, prepared: &PreparedHarnessRun) -> HarnessResult<()> {
        require_version(
            "current apply attempt receipt",
            self.version,
            HARNESS_SCHEMA_VERSION,
        )?;
        validate_run_identifier(&self.run_identifier)?;
        for (label, digest) in [
            (
                "apply attempt validated head",
                &self.validated_record_digest,
            ),
            ("apply attempt prepared run", &self.prepared_run_digest),
            ("apply attempt Harness plan", &self.harness_plan_digest),
            ("apply attempt resolved plan", &self.resolved_plan_digest),
            ("apply attempt candidate", &self.candidate_digest),
            (
                "apply attempt validation receipt",
                &self.validation_receipt_digest,
            ),
            (
                "apply attempt frozen identity set",
                &self.frozen_identity_set_digest,
            ),
            (
                "apply attempt workspace locator map",
                &self.workspace_locator_map_digest,
            ),
            ("apply attempt receipt", &self.receipt_digest),
        ] {
            validate_digest(label, digest)?;
        }
        let (batch, journal, completion) = repository::batch_paths_for_attempt(
            &self.resolved_plan_digest,
            &self.candidate_digest,
            &self.attempt_identifier,
        )?;
        if (
            &batch,
            &journal,
            &completion,
            &self.prepared_run_digest,
            &self.harness_plan_digest,
            &self.resolved_plan_digest,
            &self.frozen_identity_set_digest,
            &self.workspace_locator_map_digest,
        ) != (
            &self.expected_batch_identifier,
            &self.expected_journal_relative_path,
            &self.expected_completion_relative_path,
            &prepared.prepared_run_digest,
            &prepared.plan_identity.normalized_value_digest,
            &prepared.plan.resolved_request.resolved_plan_digest,
            &prepared.plan.frozen_targets.identity_set_digest,
            &prepared.plan.frozen_targets.workspace_locator_map_digest,
        ) {
            return Err(HarnessError::InvalidSubmission(
                "apply attempt is not bound to the exact prepared run and deterministic batch paths"
                    .to_owned(),
            ));
        }
        match &self.attempt_state {
            HarnessApplyAttemptState::PendingApply => {}
            HarnessApplyAttemptState::Failure { evidence } => {
                validate_apply_failure_evidence(self, evidence)?;
            }
            HarnessApplyAttemptState::SourceAppliedFinalizationPending { message, applied } => {
                validate_artifact_text("apply finalization failure", message)?;
                validate_attempt_applied_batch(self, applied)?;
            }
            HarnessApplyAttemptState::AbortedBeforeMutation { verified_at_millis } => {
                if *verified_at_millis == 0 {
                    return Err(HarnessError::InvalidSubmission(
                        "pre-mutation abort proof requires a verification timestamp".to_owned(),
                    ));
                }
            }
            HarnessApplyAttemptState::AbortedAfterRollback {
                verified_at_millis,
                rollback_receipt,
            } => {
                if *verified_at_millis == 0
                    || rollback_receipt.outcome != BatchApplyOutcomeReceipt::RolledBack
                    || rollback_receipt.journal_retained
                    || rollback_receipt.batch_id != self.expected_batch_identifier
                    || rollback_receipt.journal_relative_path != self.expected_journal_relative_path
                    || rollback_receipt.completion_receipt_relative_path.as_ref()
                        != Some(&self.expected_completion_relative_path)
                    || rollback_receipt.resolved_plan_digest != self.resolved_plan_digest
                    || rollback_receipt.candidate_digest != self.candidate_digest
                    || !rollback_receipt.lock.is_fully_confirmed()
                    || rollback_receipt
                        .targets
                        .iter()
                        .any(|target| target.state != BatchTargetApplyStateReceipt::RolledBack)
                {
                    return Err(HarnessError::InvalidSubmission(
                        "rollback abort requires the exact completed batch proof".to_owned(),
                    ));
                }
            }
            HarnessApplyAttemptState::AppliedFinalized {
                resulting_record,
                apply_receipt,
            } => {
                if apply_receipt.recovered {
                    return Err(HarnessError::InvalidSubmission(
                        "ordinary apply completion cannot carry a recovery receipt".to_owned(),
                    ));
                }
                validate_terminal_attempt_binding(self, resulting_record, apply_receipt)?;
            }
            HarnessApplyAttemptState::RecoveredFinalized {
                resulting_record,
                apply_receipt,
            } => {
                if !apply_receipt.recovered {
                    return Err(HarnessError::InvalidSubmission(
                        "recovered apply completion requires a recovery receipt".to_owned(),
                    ));
                }
                validate_terminal_attempt_binding(self, resulting_record, apply_receipt)?;
            }
        }
        if self.receipt_digest
            != serialized_digest(&(
                self.version,
                &self.run_identifier,
                &self.attempt_identifier,
                &self.validated_record_digest,
                self.validated_sequence,
                &self.prepared_run_digest,
                &self.harness_plan_digest,
                &self.resolved_plan_digest,
                &self.candidate_digest,
                &self.validation_receipt_digest,
                &self.frozen_identity_set_digest,
                &self.workspace_locator_map_digest,
                &self.expected_batch_identifier,
                &self.expected_journal_relative_path,
                &self.expected_completion_relative_path,
                &self.attempt_state,
            ))?
        {
            return Err(HarnessError::InvalidSubmission(
                "apply attempt receipt digest is invalid".to_owned(),
            ));
        }
        validate_serialized_size("apply attempt receipt", self, MAX_APPLY_ATTEMPT_BYTES)?;
        Ok(())
    }
}

fn validate_apply_failure_evidence(
    attempt: &HarnessApplyAttemptReceipt,
    evidence: &HarnessApplyFailureEvidence,
) -> HarnessResult<()> {
    match evidence {
        HarnessApplyFailureEvidence::BatchApply { message, receipt } => {
            validate_artifact_text("batch apply failure", message)?;
            if !batch_failure_receipt_belongs_to_attempt(attempt, receipt) {
                return Err(HarnessError::InvalidSubmission(
                    "batch failure evidence is outside its apply attempt".to_owned(),
                ));
            }
        }
        HarnessApplyFailureEvidence::BeforeBatch { message } => {
            validate_artifact_text("pre-batch apply failure", message)?;
        }
    }
    Ok(())
}

fn batch_failure_receipt_belongs_to_attempt(
    attempt: &HarnessApplyAttemptReceipt,
    receipt: &HarnessBatchApplyLifecycleReceipt,
) -> bool {
    (receipt.resolved_plan_digest.is_empty()
        || receipt.resolved_plan_digest == attempt.resolved_plan_digest)
        && (receipt.candidate_digest.is_empty()
            || receipt.candidate_digest == attempt.candidate_digest)
        && (receipt.batch_id.is_empty() || receipt.batch_id == attempt.expected_batch_identifier)
        && (receipt.journal_relative_path.is_empty()
            || receipt.journal_relative_path == attempt.expected_journal_relative_path)
        && receipt
            .completion_receipt_relative_path
            .as_deref()
            .is_none_or(|path| path == attempt.expected_completion_relative_path)
}

fn validate_attempt_applied_batch(
    attempt: &HarnessApplyAttemptReceipt,
    applied: &AppliedHarnessBatch,
) -> HarnessResult<()> {
    if applied.resolved_plan_digest != attempt.resolved_plan_digest
        || applied.candidate_digest != attempt.candidate_digest
        || applied.batch_id != attempt.expected_batch_identifier
        || applied.lifecycle_receipt.batch_id != attempt.expected_batch_identifier
        || applied.lifecycle_receipt.journal_relative_path != attempt.expected_journal_relative_path
        || applied
            .lifecycle_receipt
            .completion_receipt_relative_path
            .as_deref()
            != Some(attempt.expected_completion_relative_path.as_str())
    {
        return Err(HarnessError::InvalidSubmission(
            "applied batch evidence is outside its deterministic apply attempt".to_owned(),
        ));
    }
    Ok(())
}

fn validate_terminal_attempt_binding(
    attempt: &HarnessApplyAttemptReceipt,
    resulting_record: &HarnessExecutionRecord,
    apply_receipt: &HarnessApplyReceipt,
) -> HarnessResult<()> {
    apply_receipt.validate()?;
    resulting_record.validate_durable_head_identity()?;
    if resulting_record.state != HarnessExecutionState::Finalized
        || resulting_record.run_identifier != attempt.run_identifier
        || attempt.validated_sequence.checked_add(1) != Some(resulting_record.sequence)
        || resulting_record.predecessor_record_digest.as_deref()
            != Some(attempt.validated_record_digest.as_str())
        || resulting_record.candidate_digest.as_deref() != Some(attempt.candidate_digest.as_str())
        || resulting_record.validation_receipt_digest.as_deref()
            != Some(attempt.validation_receipt_digest.as_str())
        || apply_receipt.harness_plan_digest != attempt.harness_plan_digest
        || apply_receipt.prepared_run_digest != attempt.prepared_run_digest
        || apply_receipt.candidate_digest != attempt.candidate_digest
        || apply_receipt.validation_receipt_digest != attempt.validation_receipt_digest
        || apply_receipt.batch_journal_relative_path != attempt.expected_journal_relative_path
        || apply_receipt.applied_batch.batch_id != attempt.expected_batch_identifier
    {
        return Err(HarnessError::InvalidSubmission(
            "terminal apply attempt is not bound to its validated predecessor".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(unix)]
struct DurableRunLock {
    file: File,
}

#[cfg(unix)]
impl Drop for DurableRunLock {
    fn drop(&mut self) {
        // SAFETY: `file` owns a live descriptor for the lifetime of this guard.
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[cfg(unix)]
struct DurableRunStore {
    workspace_root: PathBuf,
    run_identifier: String,
    run_directory: PathBuf,
    _lock: DurableRunLock,
}

#[cfg(unix)]
impl DurableRunStore {
    fn initialize(
        workspace_root: &Path,
        run_identifier: &str,
        raw_prepared_run: &[u8],
        prepared: &PreparedHarnessRun,
        head: &HarnessExecutionRecord,
    ) -> HarnessResult<()> {
        Self::initialize_with_hook(
            workspace_root,
            run_identifier,
            raw_prepared_run,
            prepared,
            head,
            || Ok(()),
        )
    }

    fn initialize_with_hook(
        workspace_root: &Path,
        run_identifier: &str,
        raw_prepared_run: &[u8],
        prepared: &PreparedHarnessRun,
        head: &HarnessExecutionRecord,
        after_prepared_persisted: impl FnOnce() -> HarnessResult<()>,
    ) -> HarnessResult<()> {
        validate_run_identifier(run_identifier)?;
        let workspace_root =
            workspace_root
                .canonicalize()
                .map_err(|source| HarnessError::FileRead {
                    path: workspace_root.to_path_buf(),
                    message: source.to_string(),
                })?;
        let runs = ensure_run_parent_directories(&workspace_root)?;
        let lock = acquire_run_lock(&runs, run_identifier, true)?;
        let run_directory = runs.join(run_identifier);
        match fs::symlink_metadata(&run_directory) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(HarnessError::FileRead {
                    path: run_directory,
                    message: source.to_string(),
                });
            }
            Ok(_) => {
                return Err(HarnessError::InvalidRepository(format!(
                    "durable Harness run `{run_identifier}` already exists"
                )));
            }
        }
        let temporary = runs.join(format!(
            ".{run_identifier}.init-{}-{}",
            std::process::id(),
            durable_nonce()?
        ));
        create_owner_only_directory(&temporary, &runs)?;
        write_new_owner_only_file(
            &temporary.join(RUN_PREPARED_FILE),
            raw_prepared_run,
            &temporary,
        )?;
        after_prepared_persisted()?;
        write_new_owner_only_json(&temporary.join(RUN_HEAD_FILE), head, &temporary)?;
        File::open(&temporary)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| HarnessError::FileWrite {
                path: temporary.clone(),
                message: source.to_string(),
            })?;
        fs::rename(&temporary, &run_directory).map_err(|source| HarnessError::FileWrite {
            path: run_directory.clone(),
            message: format!(
                "durable run initialization rename failed; the sibling initialization directory is retained for inspection: {source}"
            ),
        })?;
        sync_run_directory(&runs)?;
        drop(lock);
        prepared.validate_serialized_integrity(raw_prepared_run)?;
        Ok(())
    }

    fn open_existing(workspace_root: &Path, run_identifier: &str) -> HarnessResult<Self> {
        validate_run_identifier(run_identifier)?;
        let workspace_root =
            workspace_root
                .canonicalize()
                .map_err(|source| HarnessError::FileRead {
                    path: workspace_root.to_path_buf(),
                    message: source.to_string(),
                })?;
        let runs = existing_run_parent_directories(&workspace_root)?;
        let lock = acquire_run_lock(&runs, run_identifier, false)?;
        let run_directory = runs.join(run_identifier);
        validate_owner_only_directory(&run_directory)?;
        Ok(Self {
            workspace_root,
            run_identifier: run_identifier.to_owned(),
            run_directory,
            _lock: lock,
        })
    }

    fn load(
        &self,
        engine: &HarnessEngine,
    ) -> HarnessResult<(Vec<u8>, PreparedHarnessRun, HarnessExecutionRecord)> {
        if self.workspace_root != engine.workspace_root {
            return Err(HarnessError::InvalidRepository(
                "durable run store workspace differs from the Harness engine workspace".to_owned(),
            ));
        }
        let raw_prepared_run = read_owner_only_file(&self.run_directory.join(RUN_PREPARED_FILE))?;
        let prepared = decode_current_json::<PreparedHarnessRun>(
            &raw_prepared_run,
            "durable prepared Harness run",
        )?;
        prepared.validate_recovery_with_engine(&raw_prepared_run, engine)?;
        let head = read_owner_only_json::<HarnessExecutionRecord>(
            &self.run_directory.join(RUN_HEAD_FILE),
            "durable Harness head",
        )?;
        if head.run_identifier != self.run_identifier {
            return Err(HarnessError::InvalidSubmission(
                "durable Harness head run identifier does not match its directory".to_owned(),
            ));
        }
        head.validate(&prepared, &raw_prepared_run)?;
        Ok((raw_prepared_run, prepared, head))
    }

    fn load_prepared(&self) -> HarnessResult<(Vec<u8>, PreparedHarnessRun)> {
        let raw_prepared_run = read_owner_only_file(&self.run_directory.join(RUN_PREPARED_FILE))?;
        let prepared = decode_current_json::<PreparedHarnessRun>(
            &raw_prepared_run,
            "durable prepared Harness run",
        )?;
        prepared.validate_serialized_integrity(&raw_prepared_run)?;
        Ok((raw_prepared_run, prepared))
    }

    fn compare_and_swap_head(
        &self,
        prepared: &PreparedHarnessRun,
        raw_prepared_run: &[u8],
        expected: &HarnessExecutionRecord,
        next: &HarnessExecutionRecord,
    ) -> HarnessResult<()> {
        let current = read_owner_only_json::<HarnessExecutionRecord>(
            &self.run_directory.join(RUN_HEAD_FILE),
            "durable Harness head",
        )?;
        current.validate(prepared, raw_prepared_run)?;
        if current.record_digest != expected.record_digest {
            return Err(HarnessError::PlanDrift {
                expected: expected.record_digest.clone(),
                actual: current.record_digest,
            });
        }
        if next.run_identifier != self.run_identifier
            || next.sequence
                != expected.sequence.checked_add(1).ok_or_else(|| {
                    HarnessError::InvalidSubmission(
                        "durable head sequence cannot advance beyond u64".to_owned(),
                    )
                })?
            || next.predecessor_record_digest.as_deref() != Some(expected.record_digest.as_str())
        {
            return Err(HarnessError::InvalidSubmission(
                "durable head successor does not bind the exact predecessor".to_owned(),
            ));
        }
        next.validate(prepared, raw_prepared_run)?;
        atomic_replace_owner_only_json(
            &self.run_directory.join(RUN_HEAD_FILE),
            next,
            &self.run_directory,
        )?;
        let persisted = read_owner_only_json::<HarnessExecutionRecord>(
            &self.run_directory.join(RUN_HEAD_FILE),
            "persisted durable Harness head",
        )?;
        if persisted != *next {
            return Err(HarnessError::InvalidRepository(
                "persisted durable Harness head differs from its successor".to_owned(),
            ));
        }
        Ok(())
    }

    fn read_apply_attempt(
        &self,
        prepared: &PreparedHarnessRun,
    ) -> HarnessResult<Option<HarnessApplyAttemptReceipt>> {
        let path = self.run_directory.join(RUN_APPLY_ATTEMPT_FILE);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(HarnessError::FileRead {
                path,
                message: source.to_string(),
            }),
            Ok(_) => {
                let receipt: HarnessApplyAttemptReceipt =
                    read_owner_only_json(&path, "durable Harness apply attempt receipt")?;
                receipt.validate(prepared)?;
                if receipt.run_identifier != self.run_identifier {
                    return Err(HarnessError::InvalidSubmission(
                        "apply attempt run identifier does not match its directory".to_owned(),
                    ));
                }
                Ok(Some(receipt))
            }
        }
    }

    fn persist_apply_attempt(
        &self,
        prepared: &PreparedHarnessRun,
        receipt: &HarnessApplyAttemptReceipt,
    ) -> HarnessResult<()> {
        receipt.validate(prepared)?;
        if receipt.run_identifier != self.run_identifier {
            return Err(HarnessError::InvalidSubmission(
                "apply attempt run identifier does not match its directory".to_owned(),
            ));
        }
        atomic_replace_owner_only_json(
            &self.run_directory.join(RUN_APPLY_ATTEMPT_FILE),
            receipt,
            &self.run_directory,
        )?;
        let persisted = self.read_apply_attempt(prepared)?.ok_or_else(|| {
            HarnessError::InvalidRepository(
                "persisted apply attempt receipt is unexpectedly absent".to_owned(),
            )
        })?;
        if persisted != *receipt {
            return Err(HarnessError::InvalidRepository(
                "persisted apply attempt receipt differs from the requested state".to_owned(),
            ));
        }
        Ok(())
    }
}

fn validate_run_identifier(run_identifier: &str) -> HarnessResult<()> {
    validate_identifier("current run identifier", run_identifier)?;
    let mut components = Path::new(run_identifier).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err(HarnessError::InvalidPlan(
            "current run identifier must be one normal path component".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn ensure_run_parent_directories(workspace_root: &Path) -> HarnessResult<PathBuf> {
    let control = workspace_root.join(".llm-context-vault-harness");
    create_or_validate_owner_only_directory(&control, workspace_root)?;
    let runs = workspace_root.join(RUNS_RELATIVE_DIRECTORY);
    create_or_validate_owner_only_directory(&runs, &control)?;
    Ok(runs)
}

#[cfg(unix)]
fn existing_run_parent_directories(workspace_root: &Path) -> HarnessResult<PathBuf> {
    let control = workspace_root.join(".llm-context-vault-harness");
    validate_owner_only_directory(&control)?;
    let runs = workspace_root.join(RUNS_RELATIVE_DIRECTORY);
    validate_owner_only_directory(&runs)?;
    Ok(runs)
}

#[cfg(unix)]
fn create_or_validate_owner_only_directory(path: &Path, parent: &Path) -> HarnessResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(HarnessError::InvalidRepository(format!(
                    "durable Harness directory `{}` must be a real directory",
                    path.display()
                )));
            }
            if metadata.permissions().mode() & 0o077 != 0 {
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|source| {
                    HarnessError::FileWrite {
                        path: path.to_path_buf(),
                        message: source.to_string(),
                    }
                })?;
                sync_run_directory(parent)?;
            }
            validate_owner_only_directory(path)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            create_owner_only_directory(path, parent)
        }
        Err(source) => Err(HarnessError::FileRead {
            path: path.to_path_buf(),
            message: source.to_string(),
        }),
    }
}

#[cfg(unix)]
fn create_owner_only_directory(path: &Path, parent: &Path) -> HarnessResult<()> {
    fs::DirBuilder::new()
        .mode(0o700)
        .create(path)
        .map_err(|source| HarnessError::FileWrite {
            path: path.to_path_buf(),
            message: source.to_string(),
        })?;
    sync_run_directory(parent)?;
    validate_owner_only_directory(path)
}

#[cfg(unix)]
fn validate_owner_only_directory(path: &Path) -> HarnessResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(|source| HarnessError::FileRead {
        path: path.to_path_buf(),
        message: source.to_string(),
    })?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(HarnessError::InvalidRepository(format!(
            "durable Harness directory `{}` must be a real owner-only directory",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn acquire_run_lock(
    runs: &Path,
    run_identifier: &str,
    create: bool,
) -> HarnessResult<DurableRunLock> {
    let path = runs.join(format!(".{run_identifier}.lock"));
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(create)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let file = options
        .open(&path)
        .map_err(|source| HarnessError::FileWrite {
            path: path.clone(),
            message: source.to_string(),
        })?;
    let metadata = file.metadata().map_err(|source| HarnessError::FileRead {
        path: path.clone(),
        message: source.to_string(),
    })?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(HarnessError::InvalidRepository(format!(
            "durable Harness run lock `{}` must be a real owner-only file",
            path.display()
        )));
    }
    // SAFETY: `file` owns a live descriptor and `flock` does not outlive this call.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(HarnessError::InvalidRepository(format!(
            "durable Harness run `{run_identifier}` is locked: {}",
            io::Error::last_os_error()
        )));
    }
    if create {
        file.sync_all()
            .and_then(|()| File::open(runs)?.sync_all())
            .map_err(|source| HarnessError::FileWrite {
                path,
                message: source.to_string(),
            })?;
    }
    Ok(DurableRunLock { file })
}

#[cfg(unix)]
fn write_new_owner_only_json<T: Serialize>(
    path: &Path,
    value: &T,
    parent: &Path,
) -> HarnessResult<()> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        HarnessError::InvalidSubmission(format!("durable Harness JSON cannot serialize: {error}"))
    })?;
    write_new_owner_only_file(path, &bytes, parent)
}

#[cfg(unix)]
fn write_new_owner_only_file(path: &Path, bytes: &[u8], parent: &Path) -> HarnessResult<()> {
    if bytes.len() as u64 > MAX_DURABLE_RUN_FILE_BYTES {
        return Err(HarnessError::InvalidSubmission(
            "durable Harness file exceeds the supported size".to_owned(),
        ));
    }
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|source| HarnessError::FileWrite {
            path: path.to_path_buf(),
            message: source.to_string(),
        })?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|source| HarnessError::FileWrite {
            path: path.to_path_buf(),
            message: source.to_string(),
        })?;
    drop(file);
    sync_run_directory(parent)
}

#[cfg(unix)]
fn atomic_replace_owner_only_json<T: Serialize>(
    path: &Path,
    value: &T,
    parent: &Path,
) -> HarnessResult<()> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        HarnessError::InvalidSubmission(format!("durable Harness JSON cannot serialize: {error}"))
    })?;
    if bytes.len() as u64 > MAX_DURABLE_RUN_FILE_BYTES {
        return Err(HarnessError::InvalidSubmission(
            "durable Harness file exceeds the supported size".to_owned(),
        ));
    }
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(HarnessError::PathEscapesRoot(path.to_path_buf()));
    }
    let temporary = parent.join(format!(
        ".{}-{}-{}.tmp",
        path.file_name().and_then(OsStr::to_str).unwrap_or("run"),
        std::process::id(),
        durable_nonce()?
    ));
    write_new_owner_only_file(&temporary, &bytes, parent)?;
    fs::rename(&temporary, path).map_err(|source| HarnessError::FileWrite {
        path: path.to_path_buf(),
        message: source.to_string(),
    })?;
    sync_run_directory(parent)?;
    validate_owner_only_file(path)
}

#[cfg(unix)]
fn read_owner_only_json<T: DeserializeOwned + Serialize>(
    path: &Path,
    label: &str,
) -> HarnessResult<T> {
    let bytes = read_owner_only_file(path)?;
    decode_current_json(&bytes, label)
}

#[cfg(unix)]
fn read_owner_only_file(path: &Path) -> HarnessResult<Vec<u8>> {
    validate_owner_only_file(path)?;
    let path_metadata = fs::symlink_metadata(path).map_err(|source| HarnessError::FileRead {
        path: path.to_path_buf(),
        message: source.to_string(),
    })?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|source| HarnessError::FileRead {
            path: path.to_path_buf(),
            message: source.to_string(),
        })?;
    let metadata = file.metadata().map_err(|source| HarnessError::FileRead {
        path: path.to_path_buf(),
        message: source.to_string(),
    })?;
    if path_metadata.dev() != metadata.dev() || path_metadata.ino() != metadata.ino() {
        return Err(HarnessError::InvalidRepository(format!(
            "durable Harness file `{}` changed while it was being opened",
            path.display()
        )));
    }
    if metadata.len() > MAX_DURABLE_RUN_FILE_BYTES {
        return Err(HarnessError::InvalidSubmission(
            "durable Harness file exceeds the supported size".to_owned(),
        ));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.take(MAX_DURABLE_RUN_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| HarnessError::FileRead {
            path: path.to_path_buf(),
            message: source.to_string(),
        })?;
    if bytes.len() as u64 > MAX_DURABLE_RUN_FILE_BYTES {
        return Err(HarnessError::InvalidSubmission(
            "durable Harness file exceeds the supported size".to_owned(),
        ));
    }
    Ok(bytes)
}

#[cfg(unix)]
fn validate_owner_only_file(path: &Path) -> HarnessResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(|source| HarnessError::FileRead {
        path: path.to_path_buf(),
        message: source.to_string(),
    })?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(HarnessError::InvalidRepository(format!(
            "durable Harness file `{}` must be a real owner-only file",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn sync_run_directory(path: &Path) -> HarnessResult<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| HarnessError::FileWrite {
            path: path.to_path_buf(),
            message: source.to_string(),
        })
}

fn durable_nonce() -> HarnessResult<u128> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| HarnessError::InvalidRepository(error.to_string()))?
        .as_nanos())
}

fn fresh_apply_attempt_identifier() -> HarnessResult<String> {
    Ok(format!(
        "attempt-{}-{}",
        std::process::id(),
        durable_nonce()?
    ))
}

fn durable_now_millis() -> HarnessResult<u64> {
    let value = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| HarnessError::InvalidRepository(error.to_string()))?
        .as_millis();
    u64::try_from(value)
        .map_err(|_| HarnessError::InvalidRepository("system time overflows u64".to_owned()))
}

fn durable_execution_transition(
    engine: &HarnessEngine,
    run_identifier: &str,
    transition: impl FnOnce(
        &HarnessExecutionRecord,
        &[u8],
        &PreparedHarnessRun,
    ) -> HarnessResult<HarnessExecutionRecord>,
) -> HarnessResult<HarnessExecutionRecord> {
    #[cfg(unix)]
    {
        let store = DurableRunStore::open_existing(&engine.workspace_root, run_identifier)?;
        let (raw_prepared_run, prepared, head) = store.load(engine)?;
        let next = transition(&head, &raw_prepared_run, &prepared)?;
        store.compare_and_swap_head(&prepared, &raw_prepared_run, &head, &next)?;
        Ok(next)
    }
    #[cfg(not(unix))]
    {
        let _ = (engine, run_identifier, transition);
        Err(HarnessError::UnsupportedRuntime(
            "durable Harness runs require Unix file locking and permissions".to_owned(),
        ))
    }
}

pub(super) fn validate_durable_apply_lineage(
    head: &HarnessExecutionRecord,
    attempt: &HarnessApplyAttemptReceipt,
) -> HarnessResult<()> {
    if let HarnessApplyAttemptState::AppliedFinalized {
        resulting_record, ..
    }
    | HarnessApplyAttemptState::RecoveredFinalized {
        resulting_record, ..
    } = &attempt.attempt_state
        && head.record_digest == resulting_record.record_digest
    {
        return if head == resulting_record.as_ref() {
            Ok(())
        } else {
            Err(HarnessError::InvalidSubmission(
                "durable head digest collision does not match the terminal apply record".to_owned(),
            ))
        };
    }
    if head.record_digest != attempt.validated_record_digest {
        return Err(HarnessError::PlanDrift {
            expected: attempt.validated_record_digest.clone(),
            actual: head.record_digest.clone(),
        });
    }
    if head.sequence != attempt.validated_sequence {
        return Err(HarnessError::InvalidSubmission(
            "durable head and apply attempt do not share a validated sequence".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(unix)]
#[allow(
    clippy::too_many_arguments,
    reason = "terminal recovery validates the same locked durable and storage evidence before cleanup"
)]
fn complete_terminal_apply_attempt(
    store: &DurableRunStore,
    engine: &HarnessEngine,
    raw_prepared_run: &[u8],
    prepared: &PreparedHarnessRun,
    head: &HarnessExecutionRecord,
    attempt: HarnessApplyAttemptReceipt,
    contract: &ContextRecoveryContract,
    status: &ContextRecoveryStatus,
    session: &mut Option<&mut dyn ContextCommitSession>,
) -> HarnessResult<(HarnessExecutionRecord, HarnessApplyAttemptReceipt)> {
    let (resulting_record, apply_receipt) = match &attempt.attempt_state {
        HarnessApplyAttemptState::AppliedFinalized {
            resulting_record,
            apply_receipt,
        }
        | HarnessApplyAttemptState::RecoveredFinalized {
            resulting_record,
            apply_receipt,
        } => (resulting_record.as_ref(), apply_receipt.as_ref()),
        _ => {
            return Err(HarnessError::InvalidSubmission(
                "terminal apply completion requires a terminal apply-attempt receipt".to_owned(),
            ));
        }
    };
    attempt.validate(prepared)?;
    apply_receipt.revalidate_from_disk_and_core(
        engine,
        raw_prepared_run,
        prepared,
        resulting_record,
    )?;
    contract.validate_status(status)?;
    if contract.apply().requires_commit() {
        let actual = status.committed_receipt().ok_or_else(|| {
            HarnessError::InvalidSubmission(
                "native terminal evidence has no actual database commit".to_owned(),
            )
        })?;
        if apply_receipt.context_commit_receipt.as_ref() != Some(actual) {
            return Err(HarnessError::InvalidSubmission(
                "terminal native receipt differs from database state".to_owned(),
            ));
        }
    } else if apply_receipt.context_commit_receipt.is_some() {
        return Err(HarnessError::InvalidSubmission(
            "filesystem terminal evidence cannot contain a context commit".to_owned(),
        ));
    }
    if head.record_digest == resulting_record.record_digest {
        if head != resulting_record {
            return Err(HarnessError::InvalidSubmission(
                "durable head digest collision does not match the terminal apply record".to_owned(),
            ));
        }
        finalize_context_cleanup(
            store,
            raw_prepared_run,
            prepared,
            contract.apply(),
            &attempt,
            session,
        )?;
        return Ok((head.clone(), attempt));
    }
    if head.record_digest != attempt.validated_record_digest {
        return Err(HarnessError::PlanDrift {
            expected: attempt.validated_record_digest,
            actual: head.record_digest.clone(),
        });
    }
    store.compare_and_swap_head(prepared, raw_prepared_run, head, resulting_record)?;
    finalize_context_cleanup(
        store,
        raw_prepared_run,
        prepared,
        contract.apply(),
        &attempt,
        session,
    )?;
    Ok((resulting_record.clone(), attempt))
}

#[cfg(unix)]
fn persist_apply_error(
    store: &DurableRunStore,
    prepared: &PreparedHarnessRun,
    attempt: &HarnessApplyAttemptReceipt,
    error: &HarnessError,
) -> HarnessResult<()> {
    let state = match error {
        HarnessError::BatchApply { message, receipt }
            if batch_failure_receipt_belongs_to_attempt(attempt, receipt) =>
        {
            HarnessApplyAttemptState::Failure {
                evidence: HarnessApplyFailureEvidence::BatchApply {
                    message: message.clone(),
                    receipt: receipt.clone(),
                },
            }
        }
        HarnessError::BatchApply { .. } => HarnessApplyAttemptState::Failure {
            evidence: HarnessApplyFailureEvidence::BeforeBatch {
                message: error.to_string(),
            },
        },
        HarnessError::Finalization { message, applied } => {
            HarnessApplyAttemptState::SourceAppliedFinalizationPending {
                message: message.clone(),
                applied: applied.clone(),
            }
        }
        _ => match &attempt.attempt_state {
            HarnessApplyAttemptState::SourceAppliedFinalizationPending { message, applied } => {
                HarnessApplyAttemptState::SourceAppliedFinalizationPending {
                    message: message.clone(),
                    applied: applied.clone(),
                }
            }
            _ => HarnessApplyAttemptState::Failure {
                evidence: HarnessApplyFailureEvidence::BeforeBatch {
                    message: error.to_string(),
                },
            },
        },
    };
    let failure = attempt.replace_state(state)?;
    store.persist_apply_attempt(prepared, &failure)
}

fn apply_attempt_can_abort_before_mutation(attempt: &HarnessApplyAttemptReceipt) -> bool {
    match &attempt.attempt_state {
        HarnessApplyAttemptState::PendingApply
        | HarnessApplyAttemptState::Failure {
            evidence: HarnessApplyFailureEvidence::BeforeBatch { .. },
        } => true,
        HarnessApplyAttemptState::Failure {
            evidence: HarnessApplyFailureEvidence::BatchApply { receipt, .. },
        } => receipt.targets.iter().all(|target| {
            matches!(
                target.state,
                BatchTargetApplyStateReceipt::Pending | BatchTargetApplyStateReceipt::Staged
            )
        }),
        HarnessApplyAttemptState::SourceAppliedFinalizationPending { .. }
        | HarnessApplyAttemptState::AbortedBeforeMutation { .. }
        | HarnessApplyAttemptState::AbortedAfterRollback { .. }
        | HarnessApplyAttemptState::AppliedFinalized { .. }
        | HarnessApplyAttemptState::RecoveredFinalized { .. } => false,
    }
}

#[cfg(unix)]
fn prove_no_apply_mutation(
    engine: &HarnessEngine,
    prepared: &PreparedHarnessRun,
    attempt: &HarnessApplyAttemptReceipt,
) -> HarnessResult<bool> {
    if prepared
        .plan
        .frozen_targets
        .revalidate_workspace(&engine.workspace_root)
        .is_err()
    {
        return Ok(false);
    }
    let artifact_paths = [
        engine
            .workspace_root
            .join(&attempt.expected_journal_relative_path),
        engine
            .workspace_root
            .join(&attempt.expected_completion_relative_path),
        engine
            .workspace_root
            .join(".llm-context-vault-harness/batches")
            .join(&attempt.expected_batch_identifier),
    ];
    for path in artifact_paths {
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Ok(_) => return Ok(false),
            Err(source) => {
                return Err(HarnessError::FileRead {
                    path,
                    message: source.to_string(),
                });
            }
        }
    }
    Ok(true)
}

#[cfg(unix)]
fn recover_context_status(
    contract: &ContextRecoveryContract,
    session: &mut Option<&mut dyn ContextCommitSession>,
) -> HarnessResult<ContextRecoveryStatus> {
    contract.apply().check_session(session.as_deref())?;
    let status = if let Some(effect) = session.as_deref_mut() {
        effect.recover(contract)?
    } else {
        if contract.apply().requires_commit() {
            return Err(HarnessError::InvalidSubmission(
                "native recovery has no effect session".to_owned(),
            ));
        }
        ContextRecoveryStatus::NotRequired
    };
    contract.apply().check_session(session.as_deref())?;
    contract.validate_status(&status)?;
    Ok(status)
}

#[cfg(unix)]
fn finalize_context_cleanup(
    store: &DurableRunStore,
    raw: &[u8],
    prepared: &PreparedHarnessRun,
    contract: &ContextApplyContract,
    terminal: &HarnessApplyAttemptReceipt,
    session: &mut Option<&mut dyn ContextCommitSession>,
) -> HarnessResult<()> {
    if !contract.requires_commit() {
        return Ok(());
    }
    contract.check_session(session.as_deref())?;
    let persisted = store.read_apply_attempt(prepared)?.ok_or_else(|| {
        HarnessError::InvalidSubmission("terminal context attempt disappeared".to_owned())
    })?;
    if &persisted != terminal {
        return Err(HarnessError::InvalidSubmission(
            "terminal context attempt differs from disk".to_owned(),
        ));
    }
    let head = read_owner_only_json::<HarnessExecutionRecord>(
        &store.run_directory.join(RUN_HEAD_FILE),
        "finalized durable head",
    )?;
    head.validate(prepared, raw)?;
    let proof = ContextTerminalProof::new(contract.clone(), persisted, head)?;
    session
        .as_deref_mut()
        .ok_or_else(|| {
            HarnessError::InvalidSubmission("native cleanup session is missing".to_owned())
        })?
        .finalize(&proof)?;
    contract.check_session(session.as_deref())
}

#[cfg(unix)]
fn cleanup_aborted_attempt(
    store: &DurableRunStore,
    engine: &HarnessEngine,
    prepared: &PreparedHarnessRun,
    attempt: &HarnessApplyAttemptReceipt,
    contract: &ContextRecoveryContract,
    status: &ContextRecoveryStatus,
    session: &mut Option<&mut dyn ContextCommitSession>,
) -> HarnessResult<()> {
    if status.committed_receipt().is_some() {
        return Err(HarnessError::InvalidSubmission(
            "committed context cannot abort".to_owned(),
        ));
    }
    match &attempt.attempt_state {
        HarnessApplyAttemptState::AbortedBeforeMutation { .. } => {
            if !prove_no_apply_mutation(engine, prepared, attempt)? {
                return Err(HarnessError::InvalidSubmission(
                    "no-mutation abort proof no longer holds".to_owned(),
                ));
            }
        }
        HarnessApplyAttemptState::AbortedAfterRollback {
            rollback_receipt, ..
        } => {
            let actual = engine.recover_apply_batch_internal(
                &attempt.expected_journal_relative_path,
                true,
                true,
            )?;
            if &actual != rollback_receipt.as_ref() {
                return Err(HarnessError::InvalidSubmission(
                    "rollback abort proof differs from the actual completed journal".to_owned(),
                ));
            }
            validate_rollback_candidate(
                prepared,
                &contract.actual_head().role_execution,
                &contract.actual_head().revision_history,
                &actual,
            )?;
        }
        _ => {
            return Err(HarnessError::InvalidSubmission(
                "context abort cleanup requires a durable aborted attempt".to_owned(),
            ));
        }
    }
    let persisted = store
        .read_apply_attempt(prepared)?
        .ok_or_else(|| HarnessError::InvalidSubmission("aborted attempt disappeared".to_owned()))?;
    if &persisted != attempt {
        return Err(HarnessError::InvalidSubmission(
            "abort cleanup differs from the persisted proof".to_owned(),
        ));
    }
    let current_head = read_owner_only_json::<HarnessExecutionRecord>(
        &store.run_directory.join(RUN_HEAD_FILE),
        "abort cleanup durable head",
    )?;
    if &current_head != contract.actual_head() {
        return Err(HarnessError::InvalidSubmission(
            "abort cleanup head differs from the proved durable lineage".to_owned(),
        ));
    }
    if contract.apply().requires_commit() {
        contract.apply().check_session(session.as_deref())?;
        let proof = ContextAbortProof::new(contract.apply().clone(), persisted, status)?;
        session
            .as_deref_mut()
            .ok_or_else(|| {
                HarnessError::InvalidSubmission("native abort session is missing".to_owned())
            })?
            .abort(&proof)?;
        contract.apply().check_session(session.as_deref())?;
    }
    Ok(())
}

fn validate_rollback_candidate(
    prepared: &PreparedHarnessRun,
    record: &RoleExecutionRecord,
    history: &[HarnessTaskEvaluation],
    receipt: &HarnessBatchApplyLifecycleReceipt,
) -> HarnessResult<()> {
    let evaluation = require_accepted_execution_evaluation(evaluate_execution_record(
        &prepared.plan.resolved_request.plan,
        &prepared.role_run,
        history,
        record,
    )?)?;
    let mut changes = evaluated_execution_changes(&evaluation)?;
    changes.sort_by(|a, b| a.path().cmp(b.path()));
    let mut targets = receipt.targets.iter().collect::<Vec<_>>();
    targets.sort_by(|a, b| a.workspace_relative_path.cmp(&b.workspace_relative_path));
    if targets.len() != changes.len() {
        return Err(HarnessError::InvalidSubmission(
            "rollback proof does not cover the accepted candidate".to_owned(),
        ));
    }
    for (change, target) in changes.iter().zip(targets) {
        let (operation, original, intended) = match change {
            FileChange::Create { content, .. } => (
                TargetOperation::Create,
                None,
                Some(byte_digest(content.as_bytes())),
            ),
            FileChange::Update {
                expected_content_digest,
                content,
                ..
            } => (
                TargetOperation::Update,
                Some(expected_content_digest.clone()),
                Some(byte_digest(content.as_bytes())),
            ),
            FileChange::Delete {
                expected_content_digest,
                ..
            } => (
                TargetOperation::Delete,
                Some(expected_content_digest.clone()),
                None,
            ),
        };
        if target.workspace_relative_path != change.path()
            || target.operation != operation
            || target.original_content_digest != original
            || target.intended_content_digest != intended
            || target.state != BatchTargetApplyStateReceipt::RolledBack
        {
            return Err(HarnessError::InvalidSubmission(
                "rollback proof differs from the exact accepted operations and bytes".to_owned(),
            ));
        }
    }
    Ok(())
}

/// Owns the durable run lock and borrows the command's context session until recovery completes.
#[cfg(unix)]
pub struct DurableRecovery<'a> {
    store: DurableRunStore,
    raw: Vec<u8>,
    prepared: PreparedHarnessRun,
    head: HarnessExecutionRecord,
    attempt: HarnessApplyAttemptReceipt,
    contract: ContextRecoveryContract,
    status: ContextRecoveryStatus,
    session: Option<&'a mut dyn ContextCommitSession>,
}
#[cfg(not(unix))]
pub struct DurableRecovery<'a> {
    _session: Option<&'a mut dyn ContextCommitSession>,
}

#[cfg(unix)]
impl DurableRecovery<'_> {
    pub fn prepared(&self) -> &PreparedHarnessRun {
        &self.prepared
    }
    pub fn contract(&self) -> &ContextRecoveryContract {
        &self.contract
    }
    pub fn workspace_root(&self) -> &Path {
        &self.store.workspace_root
    }
    pub fn status(&self) -> &ContextRecoveryStatus {
        &self.status
    }

    pub fn finish(
        mut self,
        engine: &HarnessEngine,
    ) -> HarnessResult<(HarnessExecutionRecord, HarnessApplyAttemptReceipt)> {
        if self.store.workspace_root != engine.workspace_root
            || self.contract.apply().store_identity() != engine.vault.store_identity.as_ref()
            || self
                .contract
                .apply()
                .source_root_identity()
                .is_some_and(|expected| {
                    requirements::workspace_root_identity(&engine.vault.root).as_deref()
                        != Ok(expected)
                })
        {
            return Err(HarnessError::InvalidSubmission(
                "recovery engine root/store differs from the locked durable contract".to_owned(),
            ));
        }
        self.prepared
            .validate_recovery_with_engine(&self.raw, engine)?;
        match &self.attempt.attempt_state {
            HarnessApplyAttemptState::AppliedFinalized { .. }
            | HarnessApplyAttemptState::RecoveredFinalized { .. } => {
                return complete_terminal_apply_attempt(
                    &self.store,
                    engine,
                    &self.raw,
                    &self.prepared,
                    &self.head,
                    self.attempt,
                    &self.contract,
                    &self.status,
                    &mut self.session,
                );
            }
            HarnessApplyAttemptState::AbortedBeforeMutation { .. }
            | HarnessApplyAttemptState::AbortedAfterRollback { .. } => {
                cleanup_aborted_attempt(
                    &self.store,
                    engine,
                    &self.prepared,
                    &self.attempt,
                    &self.contract,
                    &self.status,
                    &mut self.session,
                )?;
                return Ok((self.head, self.attempt));
            }
            _ => {}
        }
        if self.status.committed_receipt().is_none()
            && apply_attempt_can_abort_before_mutation(&self.attempt)
            && prove_no_apply_mutation(engine, &self.prepared, &self.attempt)?
        {
            let aborted =
                self.attempt
                    .replace_state(HarnessApplyAttemptState::AbortedBeforeMutation {
                        verified_at_millis: durable_now_millis()?,
                    })?;
            self.store.persist_apply_attempt(&self.prepared, &aborted)?;
            cleanup_aborted_attempt(
                &self.store,
                engine,
                &self.prepared,
                &aborted,
                &self.contract,
                &self.status,
                &mut self.session,
            )?;
            return Ok((self.head, aborted));
        }
        if self.status == ContextRecoveryStatus::Aborted {
            // A stored abort grants no mutation permission. Re-prove an already completed rollback.
            let actual = engine.recover_apply_batch_internal(
                &self.attempt.expected_journal_relative_path,
                true,
                true,
            )?;
            validate_rollback_candidate(
                &self.prepared,
                &self.head.role_execution,
                &self.head.revision_history,
                &actual,
            )?;
            let aborted =
                self.attempt
                    .replace_state(HarnessApplyAttemptState::AbortedAfterRollback {
                        verified_at_millis: durable_now_millis()?,
                        rollback_receipt: Box::new(actual),
                    })?;
            self.store.persist_apply_attempt(&self.prepared, &aborted)?;
            cleanup_aborted_attempt(
                &self.store,
                engine,
                &self.prepared,
                &aborted,
                &self.contract,
                &self.status,
                &mut self.session,
            )?;
            return Ok((self.head, aborted));
        }
        let lifecycle = match engine.recover_apply_batch_internal(
            &self.attempt.expected_journal_relative_path,
            self.status.committed_receipt().is_some(),
            false,
        ) {
            Ok(receipt) => receipt,
            Err(error) => {
                persist_apply_error(&self.store, &self.prepared, &self.attempt, &error)?;
                return Err(error);
            }
        };
        if lifecycle.outcome == BatchApplyOutcomeReceipt::RolledBack {
            if self.status.committed_receipt().is_some() {
                return Err(HarnessError::InvalidSubmission(
                    "committed context cannot be rolled back".to_owned(),
                ));
            }
            let actual = engine.recover_apply_batch_internal(
                &self.attempt.expected_journal_relative_path,
                true,
                true,
            )?;
            validate_rollback_candidate(
                &self.prepared,
                &self.head.role_execution,
                &self.head.revision_history,
                &actual,
            )?;
            let aborted =
                self.attempt
                    .replace_state(HarnessApplyAttemptState::AbortedAfterRollback {
                        verified_at_millis: durable_now_millis()?,
                        rollback_receipt: Box::new(actual),
                    })?;
            self.store.persist_apply_attempt(&self.prepared, &aborted)?;
            cleanup_aborted_attempt(
                &self.store,
                engine,
                &self.prepared,
                &aborted,
                &self.contract,
                &self.status,
                &mut self.session,
            )?;
            return Ok((self.head, aborted));
        }
        let evaluation = require_accepted_execution_evaluation(evaluate_execution_record(
            &self.prepared.plan.resolved_request.plan,
            &self.prepared.role_run,
            &self.head.revision_history,
            &self.head.role_execution,
        )?)?;
        let applied = engine.confirm_execution_applied_batch(
            &self.prepared.plan.resolved_request,
            evaluation,
            lifecycle,
            false,
        )?;
        let result = self
            .head
            .finalize_applied(
                engine,
                &self.raw,
                &self.prepared,
                Some(self.contract.apply()),
                &mut self.session,
                self.status.committed_receipt(),
                &applied,
                true,
            )
            .map_err(|error| HarnessError::Finalization {
                message: error.to_string(),
                applied: Box::new(applied),
            });
        match result {
            Ok((record, receipt)) => {
                let terminal =
                    self.attempt
                        .replace_state(HarnessApplyAttemptState::RecoveredFinalized {
                            resulting_record: Box::new(record.clone()),
                            apply_receipt: Box::new(receipt),
                        })?;
                self.store
                    .persist_apply_attempt(&self.prepared, &terminal)?;
                self.store
                    .compare_and_swap_head(&self.prepared, &self.raw, &self.head, &record)?;
                finalize_context_cleanup(
                    &self.store,
                    &self.raw,
                    &self.prepared,
                    self.contract.apply(),
                    &terminal,
                    &mut self.session,
                )?;
                Ok((record, terminal))
            }
            Err(error) => {
                persist_apply_error(&self.store, &self.prepared, &self.attempt, &error)?;
                Err(error)
            }
        }
    }
}
#[cfg(not(unix))]
impl DurableRecovery<'_> {
    pub fn finish(
        self,
        _engine: &HarnessEngine,
    ) -> HarnessResult<(HarnessExecutionRecord, HarnessApplyAttemptReceipt)> {
        Err(HarnessError::UnsupportedRuntime(
            "durable recovery requires Unix".to_owned(),
        ))
    }
}

impl HarnessExecutionRecord {
    /// Loads the authenticated current frontier without advancing it. The
    /// subsequent durable transition still compares against the latest head.
    pub fn load_durable_head(
        engine: &HarnessEngine,
        run_identifier: &str,
    ) -> HarnessResult<(PreparedHarnessRun, Self)> {
        #[cfg(unix)]
        {
            let store = DurableRunStore::open_existing(&engine.workspace_root, run_identifier)?;
            let (_, prepared, head) = store.load(engine)?;
            Ok((prepared, head))
        }
        #[cfg(not(unix))]
        {
            let _ = (engine, run_identifier);
            Err(HarnessError::UnsupportedRuntime(
                "durable Harness runs require Unix file locking and permissions".to_owned(),
            ))
        }
    }

    pub fn load_durable_prepared(
        workspace_root: &Path,
        run_identifier: &str,
    ) -> HarnessResult<(Vec<u8>, PreparedHarnessRun)> {
        #[cfg(unix)]
        {
            DurableRunStore::open_existing(workspace_root, run_identifier)?.load_prepared()
        }
        #[cfg(not(unix))]
        {
            let _ = (workspace_root, run_identifier);
            Err(HarnessError::UnsupportedRuntime(
                "durable Harness runs require Unix file locking and permissions".to_owned(),
            ))
        }
    }

    pub fn begin_durable(
        engine: &HarnessEngine,
        raw_prepared_run: &[u8],
        prepared: &PreparedHarnessRun,
        run_identifier: &str,
    ) -> HarnessResult<Self> {
        let head = Self::begin(
            engine,
            raw_prepared_run,
            prepared,
            run_identifier.to_owned(),
        )?;
        #[cfg(unix)]
        {
            DurableRunStore::initialize(
                &engine.workspace_root,
                run_identifier,
                raw_prepared_run,
                prepared,
                &head,
            )?;
            Ok(head)
        }
        #[cfg(not(unix))]
        {
            let _ = (engine, raw_prepared_run, prepared, run_identifier, head);
            Err(HarnessError::UnsupportedRuntime(
                "durable Harness runs require Unix file locking and permissions".to_owned(),
            ))
        }
    }

    #[cfg(all(test, unix))]
    pub(super) fn begin_durable_with_initialization_failure_for_test(
        engine: &HarnessEngine,
        raw_prepared_run: &[u8],
        prepared: &PreparedHarnessRun,
        run_identifier: &str,
    ) -> HarnessResult<()> {
        let head = Self::begin(
            engine,
            raw_prepared_run,
            prepared,
            run_identifier.to_owned(),
        )?;
        DurableRunStore::initialize_with_hook(
            &engine.workspace_root,
            run_identifier,
            raw_prepared_run,
            prepared,
            &head,
            || {
                Err(HarnessError::InvalidRepository(
                    "injected durable initialization interruption".to_owned(),
                ))
            },
        )
    }

    pub fn advance_durable(
        engine: &HarnessEngine,
        run_identifier: &str,
        event: HarnessExecutionEvent,
    ) -> HarnessResult<Self> {
        durable_execution_transition(engine, run_identifier, |head, raw, prepared| {
            head.advance(engine, raw, prepared, event)
        })
    }

    pub fn revise_durable(engine: &HarnessEngine, run_identifier: &str) -> HarnessResult<Self> {
        durable_execution_transition(engine, run_identifier, |head, raw, prepared| {
            head.revise(engine, raw, prepared)
        })
    }

    pub fn evaluate_durable(
        engine: &HarnessEngine,
        run_identifier: &str,
        tool_evidence: Option<&ToolExecutionEvidence>,
    ) -> HarnessResult<Self> {
        durable_execution_transition(engine, run_identifier, |head, raw, prepared| {
            head.evaluate(engine, raw, prepared, tool_evidence)
        })
    }

    pub fn validate_durable(engine: &HarnessEngine, run_identifier: &str) -> HarnessResult<Self> {
        durable_execution_transition(engine, run_identifier, |head, raw, prepared| {
            head.validate_evaluation(engine, raw, prepared)
        })
    }

    pub fn apply_durable(
        engine: &HarnessEngine,
        run_identifier: &str,
        mut session: Option<&mut dyn ContextCommitSession>,
    ) -> HarnessResult<(Self, HarnessApplyAttemptReceipt)> {
        #[cfg(unix)]
        {
            let store = DurableRunStore::open_existing(&engine.workspace_root, run_identifier)?;
            let (raw, prepared, head) = store.load(engine)?;
            if let Some(existing) = store.read_apply_attempt(&prepared)? {
                match existing.attempt_state {
                    HarnessApplyAttemptState::AppliedFinalized {..}|HarnessApplyAttemptState::RecoveredFinalized {..} => {
                        let recovery=ContextRecoveryContract::from_durable(&prepared,&head,&existing)?;
                        let status=recover_context_status(&recovery,&mut session)?;
                        return complete_terminal_apply_attempt(&store,engine,&raw,&prepared,&head,existing,&recovery,&status,&mut session);
                    }
                    HarnessApplyAttemptState::AbortedBeforeMutation {..}|HarnessApplyAttemptState::AbortedAfterRollback {..} => {
                        // Re-prove and finish any interrupted abort cleanup before issuing a new attempt.
                        let recovery=ContextRecoveryContract::from_durable(&prepared,&head,&existing)?;
                        let status=recover_context_status(&recovery,&mut session)?;
                        cleanup_aborted_attempt(&store,engine,&prepared,&existing,&recovery,&status,&mut session)?;
                    }
                    _=>return Err(HarnessError::InvalidSubmission("the current apply attempt is unresolved; run recover before applying again".to_owned())),
                }
            }
            head.preflight_finalize(engine, &raw, &prepared, session.as_deref())?;
            let attempt = HarnessApplyAttemptReceipt::pending(
                &prepared,
                &head,
                fresh_apply_attempt_identifier()?,
            )?;
            let contract = ContextApplyContract::from_durable(&prepared, &head, &attempt)?;
            contract.check_session(session.as_deref())?;
            store.persist_apply_attempt(&prepared, &attempt)?;
            if contract.requires_commit() {
                let effect = session.as_deref_mut().ok_or_else(|| {
                    HarnessError::InvalidSubmission(
                        "native context commit session is missing".to_owned(),
                    )
                })?;
                if let Err(error) = effect.begin(&contract) {
                    persist_apply_error(&store, &prepared, &attempt, &error)?;
                    return Err(error);
                }
                contract.check_session(session.as_deref())?;
            }
            let result = head.finalize_durable_attempt(
                engine,
                &raw,
                &prepared,
                &attempt,
                &contract,
                &mut session,
            );
            match result {
                Ok((resulting_record, apply_receipt)) => {
                    let terminal =
                        attempt.replace_state(HarnessApplyAttemptState::AppliedFinalized {
                            resulting_record: Box::new(resulting_record.clone()),
                            apply_receipt: Box::new(apply_receipt),
                        })?;
                    store.persist_apply_attempt(&prepared, &terminal)?;
                    store.compare_and_swap_head(&prepared, &raw, &head, &resulting_record)?;
                    finalize_context_cleanup(
                        &store,
                        &raw,
                        &prepared,
                        &contract,
                        &terminal,
                        &mut session,
                    )?;
                    Ok((resulting_record, terminal))
                }
                Err(error) => {
                    persist_apply_error(&store, &prepared, &attempt, &error)?;
                    Err(error)
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (engine, run_identifier, session);
            Err(HarnessError::UnsupportedRuntime(
                "durable Harness runs require Unix file locking and permissions".to_owned(),
            ))
        }
    }

    /// Source-free recovery acquires and retains the durable run lock before any provider callback.
    pub fn open_durable_recovery<'a>(
        workspace: &Path,
        run: &str,
        session: Option<&'a mut dyn ContextCommitSession>,
    ) -> HarnessResult<DurableRecovery<'a>> {
        #[cfg(unix)]
        {
            let store = DurableRunStore::open_existing(workspace, run)?;
            let (raw, prepared) = store.load_prepared()?;
            if prepared.plan.workspace_root_identity
                != requirements::workspace_root_identity(&store.workspace_root)?
            {
                return Err(HarnessError::InvalidSubmission(
                    "durable recovery belongs to another workspace root".to_owned(),
                ));
            }
            let head = read_owner_only_json::<HarnessExecutionRecord>(
                &store.run_directory.join(RUN_HEAD_FILE),
                "durable Harness head",
            )?;
            head.validate(&prepared, &raw)?;
            if head.run_identifier != run {
                return Err(HarnessError::InvalidSubmission(
                    "durable head run differs from its directory".to_owned(),
                ));
            }
            let attempt = store.read_apply_attempt(&prepared)?.ok_or_else(|| {
                HarnessError::InvalidSubmission(
                    "the durable Harness run has no apply attempt to recover".to_owned(),
                )
            })?;
            let contract = ContextRecoveryContract::from_durable(&prepared, &head, &attempt)?;
            let mut session = session;
            if contract.apply().store_identity().is_some() && session.is_none() {
                return Err(HarnessError::InvalidSubmission(
                    "stored-source recovery requires its locked persistence session".to_owned(),
                ));
            }
            let status = recover_context_status(&contract, &mut session)?;
            Ok(DurableRecovery {
                store,
                raw,
                prepared,
                head,
                attempt,
                contract,
                status,
                session,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = (workspace, run, session);
            Err(HarnessError::UnsupportedRuntime(
                "durable Harness recovery requires Unix".to_owned(),
            ))
        }
    }

    pub fn recover_durable(
        engine: &HarnessEngine,
        run_identifier: &str,
        session: Option<&mut dyn ContextCommitSession>,
    ) -> HarnessResult<(Self, HarnessApplyAttemptReceipt)> {
        if engine.vault.store_identity.is_some() || session.is_some() {
            return Err(HarnessError::InvalidSubmission("stored-source runs must open the source-free durable recovery handle before constructing the engine".to_owned()));
        }
        Self::open_durable_recovery(&engine.workspace_root, run_identifier, None)?.finish(engine)
    }

    #[cfg(all(test, unix))]
    pub(super) fn persist_pending_apply_for_test(
        engine: &HarnessEngine,
        run_identifier: &str,
    ) -> HarnessResult<HarnessApplyAttemptReceipt> {
        let store = DurableRunStore::open_existing(&engine.workspace_root, run_identifier)?;
        let (_, prepared, head) = store.load(engine)?;
        let attempt = HarnessApplyAttemptReceipt::pending(
            &prepared,
            &head,
            fresh_apply_attempt_identifier()?,
        )?;
        store.persist_apply_attempt(&prepared, &attempt)?;
        Ok(attempt)
    }

    #[cfg(all(test, unix))]
    pub(super) fn apply_source_without_finalization_for_test(
        engine: &HarnessEngine,
        run_identifier: &str,
    ) -> HarnessResult<(HarnessApplyAttemptReceipt, AppliedHarnessBatch)> {
        let store = DurableRunStore::open_existing(&engine.workspace_root, run_identifier)?;
        let (raw_prepared_run, prepared, head) = store.load(engine)?;
        let attempt = HarnessApplyAttemptReceipt::pending(
            &prepared,
            &head,
            fresh_apply_attempt_identifier()?,
        )?;
        store.persist_apply_attempt(&prepared, &attempt)?;
        head.preflight_finalize(engine, &raw_prepared_run, &prepared, None)?;
        let applied = engine.apply_validated_execution_for_attempt(
            &prepared.plan.resolved_request,
            &prepared.role_run,
            &head.revision_history,
            &head.role_execution,
            &attempt.attempt_identifier,
        )?;
        Ok((attempt, applied))
    }

    #[cfg(all(test, unix))]
    pub(super) fn fail_terminal_apply_receipt_replacement_for_test(
        engine: &HarnessEngine,
        run_identifier: &str,
    ) -> HarnessResult<(Self, HarnessApplyAttemptReceipt)> {
        let store = DurableRunStore::open_existing(&engine.workspace_root, run_identifier)?;
        let (raw_prepared_run, prepared, head) = store.load(engine)?;
        let attempt = HarnessApplyAttemptReceipt::pending(
            &prepared,
            &head,
            fresh_apply_attempt_identifier()?,
        )?;
        store.persist_apply_attempt(&prepared, &attempt)?;
        let (resulting_record, apply_receipt) = head.finalize_for_attempt(
            engine,
            &raw_prepared_run,
            &prepared,
            None,
            &attempt.attempt_identifier,
        )?;
        let terminal = attempt.replace_state(HarnessApplyAttemptState::AppliedFinalized {
            resulting_record: Box::new(resulting_record.clone()),
            apply_receipt: Box::new(apply_receipt),
        })?;
        let original_permissions = fs::metadata(&store.run_directory)
            .map_err(|source| HarnessError::FileRead {
                path: store.run_directory.clone(),
                message: source.to_string(),
            })?
            .permissions();
        fs::set_permissions(&store.run_directory, fs::Permissions::from_mode(0o500)).map_err(
            |source| HarnessError::FileWrite {
                path: store.run_directory.clone(),
                message: source.to_string(),
            },
        )?;
        let replacement = store.persist_apply_attempt(&prepared, &terminal);
        fs::set_permissions(&store.run_directory, original_permissions).map_err(|source| {
            HarnessError::FileWrite {
                path: store.run_directory.clone(),
                message: format!(
                    "failed to restore durable run permissions after the injected replacement failure: {source}"
                ),
            }
        })?;
        if replacement.is_ok() {
            return Err(HarnessError::InvalidRepository(
                "injected terminal receipt replacement unexpectedly succeeded".to_owned(),
            ));
        }
        Ok((resulting_record, attempt))
    }

    pub fn begin(
        engine: &HarnessEngine,
        raw_prepared_run: &[u8],
        prepared: &PreparedHarnessRun,
        run_identifier: String,
    ) -> HarnessResult<Self> {
        prepared.validate_with_engine(raw_prepared_run, engine)?;
        validate_identifier("current run identifier", &run_identifier)?;
        let step =
            engine.begin_execution(&prepared.plan.resolved_request, &prepared.role_run, &[])?;
        let mut record = Self {
            version: HARNESS_SCHEMA_VERSION,
            run_identifier,
            prepared_run_digest: prepared.prepared_run_digest.clone(),
            prepared_run_raw_digest: byte_digest(raw_prepared_run),
            sequence: 0,
            predecessor_record_digest: None,
            state: HarnessExecutionState::Begun,
            revision_history: Vec::new(),
            role_execution: step.record,
            ready_role_invocations: step.ready_role_invocations,
            ready_tool_invocation: step.ready_tool_invocation,
            evaluation: None,
            exact_tool_evidence: None,
            tool_evidence_digest: None,
            candidate_digest: None,
            run_evaluation_receipt_digest: None,
            validation_receipt_digest: None,
            finalization_digest: None,
            batch_receipt_digest: None,
            record_digest: String::new(),
        };
        record.record_digest = record.calculate_digest()?;
        record.validate(prepared, raw_prepared_run)?;
        Ok(record)
    }

    pub fn advance(
        &self,
        engine: &HarnessEngine,
        raw_prepared_run: &[u8],
        prepared: &PreparedHarnessRun,
        event: HarnessExecutionEvent,
    ) -> HarnessResult<Self> {
        prepared.validate_with_engine(raw_prepared_run, engine)?;
        self.validate(prepared, raw_prepared_run)?;
        if !matches!(
            self.state,
            HarnessExecutionState::Begun | HarnessExecutionState::Executing
        ) {
            return Err(HarnessError::InvalidSubmission(
                "current execution events are accepted only before evaluation".to_owned(),
            ));
        }
        let step = engine.advance_execution(
            &prepared.plan.resolved_request,
            &prepared.role_run,
            &self.revision_history,
            &self.role_execution,
            event,
        )?;
        let mut next = self.successor()?;
        next.role_execution = step.record;
        next.ready_role_invocations = step.ready_role_invocations;
        next.ready_tool_invocation = step.ready_tool_invocation;
        next.state = HarnessExecutionState::Executing;
        next.record_digest = next.calculate_digest()?;
        next.validate(prepared, raw_prepared_run)?;
        Ok(next)
    }

    pub fn evaluate(
        &self,
        engine: &HarnessEngine,
        raw_prepared_run: &[u8],
        prepared: &PreparedHarnessRun,
        tool_evidence: Option<&ToolExecutionEvidence>,
    ) -> HarnessResult<Self> {
        prepared.validate_with_engine(raw_prepared_run, engine)?;
        self.validate(prepared, raw_prepared_run)?;
        if !matches!(
            self.state,
            HarnessExecutionState::Begun | HarnessExecutionState::Executing
        ) {
            return Err(HarnessError::InvalidSubmission(
                "current evaluation requires an active Harness engine execution".to_owned(),
            ));
        }
        let tool_evidence_digest = match (&prepared.accepted_tool_plan, tool_evidence) {
            (Some(plan), Some(evidence)) => {
                evidence.validate_against(plan)?;
                Some(serialized_digest(evidence)?)
            }
            (None, None) => None,
            _ => {
                return Err(HarnessError::InvalidSubmission(
                    "tool evidence must be present exactly when the prepared run accepted a tool plan"
                        .to_owned(),
                ));
            }
        };
        let evaluation = engine.evaluate_execution(
            &prepared.plan.resolved_request,
            &prepared.role_run,
            &self.revision_history,
            &self.role_execution,
        )?;
        if let (Some(plan), Some(evidence)) = (&prepared.accepted_tool_plan, tool_evidence) {
            evidence.validate_tool_evidence(plan, self.role_execution.tool_evidence.as_ref())?;
        }
        let candidate_digest = evaluation.candidate_digest.clone();
        let candidate_expected = prepared.plan.source_write_allowed
            && evaluation.execution_status == ExecutionStatus::Completed;
        if candidate_expected != candidate_digest.is_some() {
            return Err(HarnessError::InvalidSubmission(
                "Harness engine evaluation candidate authority disagrees with the resolved current action"
                    .to_owned(),
            ));
        }
        let run_evaluation_receipt_digest = serialized_digest(&(
            HARNESS_SCHEMA_VERSION,
            &self.run_identifier,
            &prepared.prepared_run_digest,
            &tool_evidence_digest,
            &candidate_digest,
            &self.role_execution.execution_record_digest,
            &evaluation,
        ))?;
        let mut next = self.successor()?;
        next.state = HarnessExecutionState::Evaluated;
        next.exact_tool_evidence = tool_evidence.cloned();
        next.tool_evidence_digest = tool_evidence_digest;
        next.candidate_digest = candidate_digest;
        next.run_evaluation_receipt_digest = Some(run_evaluation_receipt_digest);
        next.evaluation = Some(evaluation);
        next.record_digest = next.calculate_digest()?;
        next.validate(prepared, raw_prepared_run)?;
        Ok(next)
    }

    pub fn revise(
        &self,
        engine: &HarnessEngine,
        raw_prepared_run: &[u8],
        prepared: &PreparedHarnessRun,
    ) -> HarnessResult<Self> {
        prepared.validate_with_engine(raw_prepared_run, engine)?;
        self.validate(prepared, raw_prepared_run)?;
        let evaluation = self.evaluation.as_ref().ok_or_else(|| {
            HarnessError::InvalidSubmission(
                "current revision requires a Harness engine evaluation".to_owned(),
            )
        })?;
        if self.state != HarnessExecutionState::Evaluated
            || evaluation.completion_state != HarnessCompletionState::RevisionRequired
        {
            return Err(HarnessError::InvalidSubmission(
                "current revision requires an evaluated write candidate in RevisionRequired"
                    .to_owned(),
            ));
        }
        let mut history = self.revision_history.clone();
        history.push(evaluation.clone());
        let step = engine.begin_execution(
            &prepared.plan.resolved_request,
            &prepared.role_run,
            &history,
        )?;
        let mut next = self.successor()?;
        next.state = HarnessExecutionState::Begun;
        next.revision_history = history;
        next.role_execution = step.record;
        next.ready_role_invocations = step.ready_role_invocations;
        next.ready_tool_invocation = step.ready_tool_invocation;
        next.evaluation = None;
        next.exact_tool_evidence = None;
        next.tool_evidence_digest = None;
        next.candidate_digest = None;
        next.run_evaluation_receipt_digest = None;
        next.validation_receipt_digest = None;
        next.finalization_digest = None;
        next.batch_receipt_digest = None;
        next.record_digest = next.calculate_digest()?;
        next.validate(prepared, raw_prepared_run)?;
        Ok(next)
    }

    pub fn validate_evaluation(
        &self,
        engine: &HarnessEngine,
        raw_prepared_run: &[u8],
        prepared: &PreparedHarnessRun,
    ) -> HarnessResult<Self> {
        prepared.validate_with_engine(raw_prepared_run, engine)?;
        self.validate(prepared, raw_prepared_run)?;
        if self.state != HarnessExecutionState::Evaluated {
            return Err(HarnessError::InvalidSubmission(
                "current validation requires an evaluated execution record".to_owned(),
            ));
        }
        let run_evaluation_receipt_digest =
            self.run_evaluation_receipt_digest.as_ref().ok_or_else(|| {
                HarnessError::InvalidSubmission("current evaluation receipt is missing".to_owned())
            })?;
        let validated = engine.validate_execution(
            &prepared.plan.resolved_request,
            &prepared.role_run,
            &self.revision_history,
            &self.role_execution,
        )?;
        if self.evaluation.as_ref() != Some(&validated) {
            return Err(HarnessError::InvalidSubmission(
                "current validation must reproduce the exact prior Harness engine evaluation"
                    .to_owned(),
            ));
        }
        let validation_receipt_digest = serialized_digest(&(
            HARNESS_SCHEMA_VERSION,
            &self.run_identifier,
            &prepared.prepared_run_digest,
            run_evaluation_receipt_digest,
            &self.candidate_digest,
            &validated,
        ))?;
        let mut next = self.successor()?;
        next.state = HarnessExecutionState::Validated;
        next.validation_receipt_digest = Some(validation_receipt_digest);
        next.record_digest = next.calculate_digest()?;
        next.validate(prepared, raw_prepared_run)?;
        Ok(next)
    }

    pub fn finalize(
        &self,
        engine: &HarnessEngine,
        raw: &[u8],
        prepared: &PreparedHarnessRun,
        session: Option<&mut dyn ContextCommitSession>,
    ) -> HarnessResult<(Self, HarnessApplyReceipt)> {
        self.preflight_finalize(engine, raw, prepared, session.as_deref())?;
        if !native_target_paths(prepared)?.is_empty() {
            return Err(HarnessError::InvalidSubmission(
                "native context finalization requires durable apply".to_owned(),
            ));
        }
        let applied = engine.apply_validated_execution(
            &prepared.plan.resolved_request,
            &prepared.role_run,
            &self.revision_history,
            &self.role_execution,
        )?;
        let mut session = session;
        self.finalize_applied(
            engine,
            raw,
            prepared,
            None,
            &mut session,
            None,
            &applied,
            false,
        )
        .map_err(|error| HarnessError::Finalization {
            message: error.to_string(),
            applied: Box::new(applied),
        })
    }

    #[cfg(test)]
    pub(super) fn finalize_for_attempt(
        &self,
        engine: &HarnessEngine,
        raw: &[u8],
        prepared: &PreparedHarnessRun,
        session: Option<&mut dyn ContextCommitSession>,
        attempt_identifier: &str,
    ) -> HarnessResult<(Self, HarnessApplyReceipt)> {
        self.preflight_finalize(engine, raw, prepared, session.as_deref())?;
        if !native_target_paths(prepared)?.is_empty() {
            return Err(HarnessError::InvalidSubmission(
                "native context finalization requires durable apply".to_owned(),
            ));
        }
        let applied = engine.apply_validated_execution_for_attempt(
            &prepared.plan.resolved_request,
            &prepared.role_run,
            &self.revision_history,
            &self.role_execution,
            attempt_identifier,
        )?;
        let mut session = session;
        self.finalize_applied(
            engine,
            raw,
            prepared,
            None,
            &mut session,
            None,
            &applied,
            false,
        )
        .map_err(|error| HarnessError::Finalization {
            message: error.to_string(),
            applied: Box::new(applied),
        })
    }

    fn finalize_durable_attempt(
        &self,
        engine: &HarnessEngine,
        raw: &[u8],
        prepared: &PreparedHarnessRun,
        attempt: &HarnessApplyAttemptReceipt,
        contract: &ContextApplyContract,
        session: &mut Option<&mut dyn ContextCommitSession>,
    ) -> HarnessResult<(Self, HarnessApplyReceipt)> {
        let applied = engine.apply_validated_execution_internal(
            &prepared.plan.resolved_request,
            &prepared.role_run,
            &self.revision_history,
            &self.role_execution,
            Some(&attempt.attempt_identifier),
            Some(contract),
        )?;
        self.finalize_applied(
            engine,
            raw,
            prepared,
            Some(contract),
            session,
            None,
            &applied,
            false,
        )
        .map_err(|error| HarnessError::Finalization {
            message: error.to_string(),
            applied: Box::new(applied),
        })
    }

    pub fn preflight_finalize(
        &self,
        engine: &HarnessEngine,
        raw: &[u8],
        prepared: &PreparedHarnessRun,
        session: Option<&dyn ContextCommitSession>,
    ) -> HarnessResult<()> {
        prepared.validate_with_engine(raw, engine)?;
        self.validate(prepared, raw)?;
        if self.state != HarnessExecutionState::Validated || !prepared.plan.source_write_allowed {
            return Err(HarnessError::InvalidSubmission(
                "current finalization requires a validated source-writing execution".to_owned(),
            ));
        }
        let native = native_target_paths(prepared)?;
        if engine.workspace_root == engine.vault.root
            && engine.vault.store_identity.is_some()
            && prepared
                .plan
                .frozen_targets
                .targets
                .iter()
                .any(|target| target.workspace_relative_path.starts_with("vault/"))
            && native.is_empty()
        {
            return Err(HarnessError::InvalidSubmission(
                "native target bindings were omitted from the prepared source contract".to_owned(),
            ));
        }
        if !native.is_empty()
            && (session.is_none()
                || session.map(ContextCommitSession::store_identity)
                    != engine.vault.store_identity.as_ref())
        {
            return Err(HarnessError::InvalidSubmission(
                "native context commit session is missing or belongs to another store".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn recover_finalize(
        &self,
        engine: &HarnessEngine,
        raw: &[u8],
        prepared: &PreparedHarnessRun,
        journal: &str,
        session: Option<&mut dyn ContextCommitSession>,
    ) -> HarnessResult<(Self, HarnessApplyReceipt)> {
        prepared.validate_recovery_with_engine(raw, engine)?;
        self.validate(prepared, raw)?;
        if self.state != HarnessExecutionState::Validated || !prepared.plan.source_write_allowed {
            return Err(HarnessError::InvalidSubmission(
                "current recovery finalization requires the exact validated source-writing record"
                    .to_owned(),
            ));
        }
        if prepared.role_run.source_versions.store_identity.is_some() || session.is_some() {
            return Err(HarnessError::InvalidSubmission(
                "stored-source recovery requires the source-free durable handle".to_owned(),
            ));
        }
        let applied = engine.recover_validated_execution_apply(
            &prepared.plan.resolved_request,
            &prepared.role_run,
            &self.revision_history,
            &self.role_execution,
            journal,
        )?;
        let mut session = None;
        self.finalize_applied(
            engine,
            raw,
            prepared,
            None,
            &mut session,
            None,
            &applied,
            true,
        )
        .map_err(|error| HarnessError::Finalization {
            message: error.to_string(),
            applied: Box::new(applied),
        })
    }

    #[allow(
        clippy::too_many_lines,
        reason = "current finalization keeps durable Harness engine replay, disk/index evidence and the uniquely digested successor receipt auditable together"
    )]
    #[allow(
        clippy::too_many_arguments,
        reason = "file and storage evidence share one verified finalization boundary"
    )]
    fn finalize_applied(
        &self,
        engine: &HarnessEngine,
        raw_prepared_run: &[u8],
        prepared: &PreparedHarnessRun,
        contract: Option<&ContextApplyContract>,
        session: &mut Option<&mut dyn ContextCommitSession>,
        committed: Option<&ContextCommitReceipt>,
        applied: &AppliedHarnessBatch,
        recovered: bool,
    ) -> HarnessResult<(Self, HarnessApplyReceipt)> {
        let durable_applied = engine.inspect_validated_execution_apply(
            &prepared.plan.resolved_request,
            &prepared.role_run,
            &self.revision_history,
            &self.role_execution,
            &applied.lifecycle_receipt.journal_relative_path,
        )?;
        if durable_applied.resolved_plan_digest != applied.resolved_plan_digest
            || durable_applied.candidate_digest != applied.candidate_digest
            || durable_applied.task_evaluation_receipt_digest
                != applied.task_evaluation_receipt_digest
            || durable_applied.batch_id != applied.batch_id
        {
            return Err(HarnessError::InvalidSubmission(
                "durable Harness engine completion receipt differs from the immediate apply result"
                    .to_owned(),
            ));
        }
        engine.with_workspace_finalization_lock(|| {
            let applied = &durable_applied;
            let targets =
                FinalizationTarget::from_recovered_batch(&prepared.plan.frozen_targets, applied)?;
            let actual_workspace = FinalWorkspaceEvidence::read_from_workspace(
                &engine.workspace_root,
                &prepared.plan.frozen_targets,
            )?;
            validate_finalization_evidence(
                &prepared.plan.frozen_targets.workspace_locator_map_digest,
                &targets,
                &actual_workspace,
            )?;
            let changes =
                evaluated_execution_changes(self.evaluation.as_ref().ok_or_else(|| {
                    HarnessError::InvalidSubmission(
                        "finalization requires the accepted evaluation".to_owned(),
                    )
                })?)?;
            let context_commit_receipt = if let Some(contract) = contract {
                contract.check_session(session.as_deref())?;
                if contract.requires_commit() {
                    let proof = VerifiedContextCommit::new(
                        contract,
                        applied,
                        &actual_workspace,
                        &targets,
                        &changes,
                    )?;
                    let receipt = if let Some(receipt) = committed {
                        receipt.clone()
                    } else {
                        session
                            .as_deref_mut()
                            .ok_or_else(|| {
                                HarnessError::InvalidSubmission(
                                    "native finalization session missing".to_owned(),
                                )
                            })?
                            .commit(&proof)?
                    };
                    contract.check_session(session.as_deref())?;
                    receipt.validate_for(contract)?;
                    if receipt.observed_batch_digest() != serialized_digest(applied)? {
                        return Err(HarnessError::InvalidSubmission(
                            "context receipt differs from the actual observed batch".to_owned(),
                        ));
                    }
                    Some(receipt)
                } else {
                    if committed.is_some() {
                        return Err(HarnessError::InvalidSubmission(
                            "external target finalization forbids a context receipt".to_owned(),
                        ));
                    }
                    None
                }
            } else {
                if !native_target_paths(prepared)?.is_empty() || committed.is_some() {
                    return Err(HarnessError::InvalidSubmission(
                        "native finalization requires a durable effect contract".to_owned(),
                    ));
                }
                None
            };
            let finalization_digest = serialized_digest(&(
                &targets,
                &actual_workspace,
                &context_commit_receipt,
                applied,
                recovered,
                &prepared.plan.frozen_targets.identity_set_digest,
                &prepared.plan.frozen_targets.workspace_locator_map_digest,
            ))?;
            let candidate_digest = self.candidate_digest.clone().ok_or_else(|| {
                HarnessError::InvalidSubmission("current candidate digest is missing".to_owned())
            })?;
            let validation_receipt_digest =
                self.validation_receipt_digest.clone().ok_or_else(|| {
                    HarnessError::InvalidSubmission(
                        "current validation receipt is missing".to_owned(),
                    )
                })?;
            let harness_plan_digest = prepared.plan_identity.normalized_value_digest.clone();
            let batch_receipt_digest = serialized_digest(applied)?;
            let batch_journal_relative_path =
                applied.lifecycle_receipt.journal_relative_path.clone();
            let apply_receipt_digest = serialized_digest(&(
                HARNESS_SCHEMA_VERSION,
                &harness_plan_digest,
                &prepared.prepared_run_digest,
                &candidate_digest,
                &validation_receipt_digest,
                &finalization_digest,
                &batch_receipt_digest,
                &batch_journal_relative_path,
                recovered,
                &prepared.plan.frozen_targets.identity_set_digest,
                &prepared.plan.frozen_targets.workspace_locator_map_digest,
                applied,
                &targets,
                &actual_workspace,
                &context_commit_receipt,
            ))?;
            let receipt = HarnessApplyReceipt {
                version: HARNESS_SCHEMA_VERSION,
                harness_plan_digest,
                prepared_run_digest: prepared.prepared_run_digest.clone(),
                candidate_digest,
                validation_receipt_digest,
                finalization_digest: finalization_digest.clone(),
                batch_receipt_digest: batch_receipt_digest.clone(),
                batch_journal_relative_path,
                recovered,
                frozen_identity_set_digest: prepared
                    .plan
                    .frozen_targets
                    .identity_set_digest
                    .clone(),
                workspace_locator_map_digest: prepared
                    .plan
                    .frozen_targets
                    .workspace_locator_map_digest
                    .clone(),
                applied_batch: applied.clone(),
                finalization_targets: targets,
                final_workspace: actual_workspace,
                context_commit_receipt,
                apply_receipt_digest,
            };
            receipt.validate()?;
            let mut next = self.successor()?;
            next.state = HarnessExecutionState::Finalized;
            next.finalization_digest = Some(finalization_digest);
            next.batch_receipt_digest = Some(batch_receipt_digest);
            next.record_digest = next.calculate_digest()?;
            next.validate(prepared, raw_prepared_run)?;
            Ok((next, receipt))
        })
    }

    #[allow(
        clippy::too_many_lines,
        reason = "current record validation keeps the complete immutable execution-chain audit in one boundary"
    )]
    pub fn validate(
        &self,
        prepared: &PreparedHarnessRun,
        raw_prepared_run: &[u8],
    ) -> HarnessResult<()> {
        self.validate_durable_head_identity()?;
        prepared.validate_serialized_integrity(raw_prepared_run)?;
        let execution_events_per_round = u64::try_from(
            prepared.plan.resolved_request.plan.role_count()
                + usize::from(prepared.accepted_tool_plan.is_some()),
        )
        .map_err(|_| {
            HarnessError::InvalidSubmission(
                "current execution graph size exceeds the supported sequence range".to_owned(),
            )
        })?;
        let maximum_sequence = u64::from(prepared.plan.resolved_request.plan.max_revisions)
            .checked_mul(execution_events_per_round.checked_add(2).ok_or_else(|| {
                HarnessError::InvalidSubmission(
                    "current execution sequence bound overflow".to_owned(),
                )
            })?)
            .and_then(|value| value.checked_add(execution_events_per_round))
            .and_then(|value| value.checked_add(3))
            .ok_or_else(|| {
                HarnessError::InvalidSubmission(
                    "current execution sequence bound overflow".to_owned(),
                )
            })?;
        if self.sequence > maximum_sequence {
            return Err(HarnessError::InvalidSubmission(
                "current execution sequence exceeds the finite graph and revision bound".to_owned(),
            ));
        }
        if self.prepared_run_digest != prepared.prepared_run_digest
            || self.prepared_run_raw_digest != byte_digest(raw_prepared_run)
        {
            return Err(HarnessError::InvalidSubmission(
                "current execution record is not bound to the exact prepared-run bytes".to_owned(),
            ));
        }
        if self.role_execution.resolved_plan_digest
            != prepared.plan.resolved_request.resolved_plan_digest
            || self.role_execution.prepared_role_run_digest
                != prepared.role_run.prepared_role_run_digest
        {
            return Err(HarnessError::InvalidSubmission(
                "current record Harness engine execution is not bound to its embedded resolution/preparation"
                    .to_owned(),
            ));
        }
        for lifecycle in self
            .role_execution
            .role_results
            .iter()
            .map(|result| &result.lifecycle)
            .chain(
                self.revision_history
                    .iter()
                    .flat_map(|evaluation| &evaluation.role_executions)
                    .map(|binding| &binding.lifecycle),
            )
        {
            prepared.plan.lifecycle.validate_observation(
                lifecycle.started_at_millis,
                lifecycle.terminal_at_millis,
                lifecycle.closed_at_millis,
            )?;
        }
        let step = execution_step(
            &prepared.plan.resolved_request.plan,
            &prepared.role_run,
            &self.revision_history,
            self.role_execution.clone(),
        )?;
        if step.ready_role_invocations != self.ready_role_invocations
            || step.ready_tool_invocation != self.ready_tool_invocation
        {
            return Err(HarnessError::InvalidSubmission(
                "current ready invocations are not the exact current Harness engine execution step"
                    .to_owned(),
            ));
        }
        let active = matches!(
            self.state,
            HarnessExecutionState::Begun | HarnessExecutionState::Executing
        );
        if active {
            if self.evaluation.is_some()
                || self.exact_tool_evidence.is_some()
                || self.tool_evidence_digest.is_some()
                || self.candidate_digest.is_some()
                || self.run_evaluation_receipt_digest.is_some()
                || self.validation_receipt_digest.is_some()
                || self.finalization_digest.is_some()
                || self.batch_receipt_digest.is_some()
            {
                return Err(HarnessError::InvalidSubmission(
                    "active current execution contains terminal evidence".to_owned(),
                ));
            }
        } else {
            match (&prepared.accepted_tool_plan, &self.exact_tool_evidence) {
                (Some(plan), Some(evidence)) => {
                    evidence
                        .validate_tool_evidence(plan, self.role_execution.tool_evidence.as_ref())?;
                    if self.tool_evidence_digest.as_deref()
                        != Some(serialized_digest(evidence)?.as_str())
                    {
                        return Err(HarnessError::InvalidSubmission(
                            "current exact tool evidence digest does not match its embedded evidence"
                                .to_owned(),
                        ));
                    }
                }
                (None, None) => {
                    if self.tool_evidence_digest.is_some() {
                        return Err(HarnessError::InvalidSubmission(
                            "current record has a tool evidence digest without an accepted tool plan"
                                .to_owned(),
                        ));
                    }
                }
                _ => {
                    return Err(HarnessError::InvalidSubmission(
                        "current record must embed exact tool evidence iff a tool plan was accepted"
                            .to_owned(),
                    ));
                }
            }
            let expected = evaluate_execution_record(
                &prepared.plan.resolved_request.plan,
                &prepared.role_run,
                &self.revision_history,
                &self.role_execution,
            )?;
            if self.evaluation.as_ref() != Some(&expected)
                || self.candidate_digest != expected.candidate_digest
            {
                return Err(HarnessError::InvalidSubmission(
                    "current terminal evidence is not the exact deterministic Harness engine evaluation"
                        .to_owned(),
                ));
            }
            let expected_evaluation_receipt = serialized_digest(&(
                HARNESS_SCHEMA_VERSION,
                &self.run_identifier,
                &prepared.prepared_run_digest,
                &self.tool_evidence_digest,
                &self.candidate_digest,
                &self.role_execution.execution_record_digest,
                &expected,
            ))?;
            if self.run_evaluation_receipt_digest.as_deref()
                != Some(expected_evaluation_receipt.as_str())
            {
                return Err(HarnessError::InvalidSubmission(
                    "current evaluation receipt is not derived from the exact Harness engine evaluation"
                        .to_owned(),
                ));
            }
            let validated = matches!(
                self.state,
                HarnessExecutionState::Validated | HarnessExecutionState::Finalized
            );
            if validated {
                validate_accepted_execution_evaluation(&expected)?;
                let expected_validation_receipt = serialized_digest(&(
                    HARNESS_SCHEMA_VERSION,
                    &self.run_identifier,
                    &prepared.prepared_run_digest,
                    &expected_evaluation_receipt,
                    &self.candidate_digest,
                    &expected,
                ))?;
                if self.validation_receipt_digest.as_deref()
                    != Some(expected_validation_receipt.as_str())
                {
                    return Err(HarnessError::InvalidSubmission(
                        "current validation receipt is not derived from the accepted Harness engine evaluation"
                            .to_owned(),
                    ));
                }
            } else if self.validation_receipt_digest.is_some()
                || self.finalization_digest.is_some()
                || self.batch_receipt_digest.is_some()
            {
                return Err(HarnessError::InvalidSubmission(
                    "evaluated current record contains validation or finalization evidence"
                        .to_owned(),
                ));
            }
            if self.state == HarnessExecutionState::Validated
                && (self.finalization_digest.is_some() || self.batch_receipt_digest.is_some())
            {
                return Err(HarnessError::InvalidSubmission(
                    "validated current record contains finalization evidence".to_owned(),
                ));
            }
            if self.state == HarnessExecutionState::Finalized
                && (self.finalization_digest.is_none() || self.batch_receipt_digest.is_none())
            {
                return Err(HarnessError::InvalidSubmission(
                    "finalized current record is missing Harness engine apply evidence".to_owned(),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn validate_durable_head_identity(&self) -> HarnessResult<()> {
        require_version(
            "current execution record",
            self.version,
            HARNESS_SCHEMA_VERSION,
        )?;
        validate_identifier("current run identifier", &self.run_identifier)?;
        validate_digest("current prepared-run digest", &self.prepared_run_digest)?;
        validate_digest(
            "current prepared-run raw digest",
            &self.prepared_run_raw_digest,
        )?;
        if self.sequence == 0 && self.predecessor_record_digest.is_some()
            || self.sequence > 0 && self.predecessor_record_digest.is_none()
        {
            return Err(HarnessError::InvalidSubmission(
                "current execution record predecessor does not match its sequence".to_owned(),
            ));
        }
        for digest in self
            .predecessor_record_digest
            .iter()
            .chain(self.tool_evidence_digest.iter())
            .chain(self.candidate_digest.iter())
            .chain(self.run_evaluation_receipt_digest.iter())
            .chain(self.validation_receipt_digest.iter())
            .chain(self.finalization_digest.iter())
            .chain(self.batch_receipt_digest.iter())
        {
            validate_digest("current execution evidence digest", digest)?;
        }
        validate_digest("current execution record digest", &self.record_digest)?;
        if self.calculate_digest()? != self.record_digest {
            return Err(HarnessError::InvalidSubmission(
                "current execution record digest does not match its exact state".to_owned(),
            ));
        }
        validate_serialized_size(
            "current execution record",
            self,
            MAX_CURRENT_EXECUTION_RECORD_BYTES,
        )?;
        Ok(())
    }

    fn successor(&self) -> HarnessResult<Self> {
        let mut next = self.clone();
        next.sequence = self.sequence.checked_add(1).ok_or_else(|| {
            HarnessError::InvalidSubmission("current execution sequence overflow".to_owned())
        })?;
        next.predecessor_record_digest = Some(self.record_digest.clone());
        next.record_digest.clear();
        Ok(next)
    }

    fn calculate_digest(&self) -> HarnessResult<String> {
        serialized_digest(&(
            (
                self.version,
                &self.run_identifier,
                &self.prepared_run_digest,
                &self.prepared_run_raw_digest,
                self.sequence,
                &self.predecessor_record_digest,
                self.state,
                &self.revision_history,
                &self.role_execution,
            ),
            (
                &self.ready_role_invocations,
                &self.ready_tool_invocation,
                &self.evaluation,
                &self.exact_tool_evidence,
                &self.tool_evidence_digest,
                &self.candidate_digest,
                &self.run_evaluation_receipt_digest,
                &self.validation_receipt_digest,
                &self.finalization_digest,
                &self.batch_receipt_digest,
            ),
        ))
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "evaluation orchestration and execution digest ordering must remain auditable together"
)]
pub(super) fn evaluate_execution_record(
    plan: &ResolvedHarnessPlan,
    prepared: &PreparedRoleRun,
    revision_history: &[HarnessTaskEvaluation],
    record: &RoleExecutionRecord,
) -> HarnessResult<HarnessTaskEvaluation> {
    validate_execution_record(plan, prepared, revision_history, record)?;
    let execution_status = execution_status(record);
    let halted = execution_status != ExecutionStatus::Completed;
    let missing_context = record.role_results.iter().find_map(|result| {
        if let RoleExecutionOutcome::MissingContext { request } = &result.outcome {
            Some(request.clone())
        } else {
            None
        }
    });
    if (execution_status == ExecutionStatus::MissingContext) != missing_context.is_some() {
        return Err(HarnessError::InvalidSubmission(
            "missing context status does not match its structured request".to_owned(),
        ));
    }
    if !halted {
        let completed_roles = record
            .role_results
            .iter()
            .map(|result| result.role)
            .collect::<BTreeSet<_>>();
        if plan.roles().any(|role| !completed_roles.contains(&role)) {
            return Err(HarnessError::InvalidSubmission(
                "execution record is not terminal because planned roles are still pending"
                    .to_owned(),
            ));
        }
        let tool_required = plan
            .verification_requirements
            .iter()
            .any(|requirement| requirement.owner == VerificationOwner::Tool);
        if tool_required != record.tool_evidence.is_some() {
            return Err(HarnessError::InvalidSubmission(
                "execution record is missing its exact tool evidence set".to_owned(),
            ));
        }
    }

    let subject = if halted {
        None
    } else {
        Some(current_evaluation_subject(plan, record)?)
    };
    let requirements = if halted {
        Vec::new()
    } else {
        evaluate_requirements(plan, record, subject.as_ref().expect("subject must exist"))?
    };
    let EvaluatedOutputs {
        subject_status,
        artifact,
        blocking_findings,
        improvements,
        promotion_proposals,
    } = if halted {
        EvaluatedOutputs {
            subject_status: SubjectStatus::NotApplicable,
            artifact: None,
            blocking_findings: Vec::new(),
            improvements: Vec::new(),
            promotion_proposals: Vec::new(),
        }
    } else {
        evaluated_outputs(plan, record, &requirements)?
    };
    let candidate_digest = if halted || !plan.action.is_write() {
        None
    } else {
        Some(serialized_digest(&(
            &record.resolved_plan_digest,
            &record.prepared_role_run_digest,
            &record.revision_contract,
            &artifact,
        ))?)
    };
    let role_executions = record
        .role_results
        .iter()
        .map(|result| RoleExecutionBinding {
            role: result.role,
            context_id: result.context_id.clone(),
            lifecycle: result.lifecycle.clone(),
            result_digest: result.result_digest.clone(),
        })
        .collect::<Vec<_>>();
    let learning_observations = if halted {
        Vec::new()
    } else {
        learning_observations(plan, prepared, &requirements)?
    };
    let promotion_state = if promotion_proposals.is_empty() {
        PromotionState::None
    } else {
        PromotionState::RequiresSeparateCuration {
            proposals: promotion_proposals,
        }
    };
    let completion_state = if execution_status == ExecutionStatus::MissingContext {
        HarnessCompletionState::MissingContext
    } else if halted {
        HarnessCompletionState::ExecutionHalted
    } else if plan.action.is_write() {
        match subject_status {
            SubjectStatus::Accepted => HarnessCompletionState::ValidatedPendingApply,
            SubjectStatus::Rejected if record.revision_contract.revision < plan.max_revisions => {
                HarnessCompletionState::RevisionRequired
            }
            SubjectStatus::Rejected | SubjectStatus::NotApplicable => {
                HarnessCompletionState::Rejected
            }
        }
    } else if plan.action.is_review() {
        HarnessCompletionState::ReviewComplete
    } else if subject_status == SubjectStatus::Rejected {
        HarnessCompletionState::Rejected
    } else {
        match plan.action {
            HarnessAction::VaultRead => HarnessCompletionState::ReadComplete,
            HarnessAction::Investigation | HarnessAction::Design | HarnessAction::Ideation => {
                HarnessCompletionState::AnalysisComplete
            }
            HarnessAction::CodeWrite
            | HarnessAction::CodeReview
            | HarnessAction::DocumentWrite
            | HarnessAction::DocumentReview
            | HarnessAction::VaultCuration => {
                unreachable!("write and review actions are handled before this branch")
            }
        }
    };
    let assurance = ExecutionAssurance::Advisory;
    let mut evaluation = HarnessTaskEvaluation {
        version: HARNESS_SCHEMA_VERSION,
        resolved_plan_digest: record.resolved_plan_digest.clone(),
        prepared_role_run_digest: record.prepared_role_run_digest.clone(),
        revision: record.revision_contract.revision,
        revision_contract_digest: record.revision_contract.revision_contract_digest.clone(),
        execution_record: record.clone(),
        execution_status,
        subject_status,
        subject,
        missing_context,
        candidate_digest,
        artifact,
        requirements,
        blocking_findings,
        improvements,
        promotion_state,
        learning_observations,
        role_executions,
        completion_state,
        assurance,
        task_evaluation_receipt_digest: String::new(),
    };
    evaluation.task_evaluation_receipt_digest = task_evaluation_receipt_digest(&evaluation)?;
    validate_task_evaluation_receipt(&evaluation)?;
    Ok(evaluation)
}

pub(super) fn require_accepted_execution_evaluation(
    evaluation: HarnessTaskEvaluation,
) -> HarnessResult<HarnessTaskEvaluation> {
    validate_accepted_execution_evaluation(&evaluation)?;
    Ok(evaluation)
}

fn validate_accepted_execution_evaluation(evaluation: &HarnessTaskEvaluation) -> HarnessResult<()> {
    if evaluation.execution_status != ExecutionStatus::Completed {
        return Err(HarnessError::UnsupportedRuntime(format!(
            "execution ended as {:?}",
            evaluation.execution_status
        )));
    }
    if evaluation.requirements.iter().any(|result| !result.passed)
        || !evaluation.blocking_findings.is_empty()
        || evaluation.subject_status != SubjectStatus::Accepted
    {
        return Err(HarnessError::InvalidSubmission(
            "execution result does not satisfy every verification requirement".to_owned(),
        ));
    }
    Ok(())
}

fn task_evaluation_receipt_digest(evaluation: &HarnessTaskEvaluation) -> HarnessResult<String> {
    serialized_digest(&(
        (
            evaluation.version,
            &evaluation.resolved_plan_digest,
            &evaluation.prepared_role_run_digest,
            evaluation.revision,
            &evaluation.revision_contract_digest,
            &evaluation.execution_record,
            evaluation.execution_status,
            evaluation.subject_status,
            &evaluation.subject,
            &evaluation.missing_context,
        ),
        (
            &evaluation.candidate_digest,
            &evaluation.artifact,
            &evaluation.requirements,
            &evaluation.blocking_findings,
            &evaluation.improvements,
            &evaluation.promotion_state,
            &evaluation.learning_observations,
            &evaluation.role_executions,
            evaluation.completion_state,
            evaluation.assurance,
        ),
    ))
}

fn learning_observations(
    plan: &ResolvedHarnessPlan,
    prepared: &PreparedRoleRun,
    requirements: &[EvaluatedRequirement],
) -> HarnessResult<Vec<LearningObservation>> {
    let producer_bundle = prepared
        .role_metadata
        .iter()
        .position(|metadata| metadata.role == plan.primary_producer_role)
        .and_then(|index| prepared.role_bundles.get(index))
        .ok_or_else(|| {
            HarnessError::InvalidSubmission(
                "prepared input omits the primary producer bundle".to_owned(),
            )
        })?;
    let mut observations = Vec::with_capacity(plan.learning_sources.len());
    for source in &plan.learning_sources {
        let was_bound = producer_bundle.bound_documents().any(|document| {
            document.source == HarnessBoundDocumentSource::Learning
                && document.relative_path == source.repository_relative_path
                && document.content_digest == source.content_digest
        });
        if !was_bound {
            return Err(HarnessError::InvalidSubmission(format!(
                "learning source `{}` was not present in the primary producer input",
                source.repository_relative_path
            )));
        }
        let requirement = requirements
            .iter()
            .find(|result| result.requirement.unit == source.metadata.verification_unit)
            .ok_or_else(|| {
                HarnessError::InvalidPlan(format!(
                    "learning source `{}` has no evaluated verification unit",
                    source.repository_relative_path
                ))
            })?;
        observations.push(LearningObservation {
            learning_id: source.metadata.learning_id.clone(),
            repository_relative_path: source.repository_relative_path.clone(),
            content_digest: source.content_digest.clone(),
            verification_unit: source.metadata.verification_unit,
            outcome: if requirement.passed {
                LearningObservationOutcome::Passed
            } else {
                LearningObservationOutcome::Failed
            },
        });
    }
    Ok(observations)
}

fn execution_status(record: &RoleExecutionRecord) -> ExecutionStatus {
    record
        .role_results
        .iter()
        .find_map(|result| match result.outcome {
            RoleExecutionOutcome::Completed { .. } => None,
            RoleExecutionOutcome::MissingContext { .. } => Some(ExecutionStatus::MissingContext),
            RoleExecutionOutcome::Failed { .. } => Some(ExecutionStatus::Failed),
            RoleExecutionOutcome::Cancelled => Some(ExecutionStatus::Cancelled),
            RoleExecutionOutcome::TimedOut => Some(ExecutionStatus::TimedOut),
            RoleExecutionOutcome::Unsupported { .. } => Some(ExecutionStatus::Unsupported),
        })
        .unwrap_or(ExecutionStatus::Completed)
}

fn evaluate_requirements(
    plan: &ResolvedHarnessPlan,
    record: &RoleExecutionRecord,
    subject: &EvaluationSubject,
) -> HarnessResult<Vec<EvaluatedRequirement>> {
    let mut evaluated = Vec::with_capacity(plan.verification_requirements.len());
    for requirement in &plan.verification_requirements {
        if requirement.subject != subject.kind()
            && requirement.owner != VerificationOwner::Deterministic
        {
            return Err(HarnessError::InvalidPlan(format!(
                "verification requirement `{}` binds the wrong final subject",
                requirement.unit.as_str()
            )));
        }
        let result = match requirement.owner {
            VerificationOwner::Deterministic => EvaluatedRequirement {
                requirement: *requirement,
                passed: true,
                detail: "Harness engine validation passed".to_owned(),
                evidence: Vec::new(),
            },
            VerificationOwner::Tool => {
                let evidence = record.tool_evidence.as_ref().ok_or_else(|| {
                    HarnessError::InvalidSubmission(
                        "tool-owned verification requirement has no evidence set".to_owned(),
                    )
                })?;
                let result = evidence
                    .results
                    .iter()
                    .find(|result| result.unit == requirement.unit)
                    .ok_or_else(|| {
                        HarnessError::InvalidSubmission(format!(
                            "tool result is missing verification requirement `{}`",
                            requirement.unit.as_str()
                        ))
                    })?;
                EvaluatedRequirement {
                    requirement: *requirement,
                    passed: result.passed,
                    detail: result.detail.clone(),
                    evidence: result.evidence.clone(),
                }
            }
            VerificationOwner::Role { role } => {
                let role_result = record
                    .role_results
                    .iter()
                    .find(|result| result.role == role)
                    .ok_or_else(|| {
                        HarnessError::InvalidSubmission(format!(
                            "role {role:?} verification result is missing"
                        ))
                    })?;
                let result = completed_requirement_results(role_result)?
                    .iter()
                    .find(|result| result.unit == requirement.unit)
                    .ok_or_else(|| {
                        HarnessError::InvalidSubmission(format!(
                            "role {role:?} is missing verification requirement `{}`",
                            requirement.unit.as_str()
                        ))
                    })?
                    .clone();
                EvaluatedRequirement {
                    requirement: *requirement,
                    passed: result.passed,
                    detail: result.detail,
                    evidence: result.evidence,
                }
            }
        };
        evaluated.push(result);
    }
    Ok(evaluated)
}

fn completed_requirement_results(
    result: &RoleExecutionResult,
) -> HarnessResult<&[RequirementResult]> {
    let RoleExecutionOutcome::Completed { result } = &result.outcome else {
        return Err(HarnessError::InvalidSubmission(
            "verification requirements require a completed role".to_owned(),
        ));
    };
    match result {
        CompletedRoleResult::Verifier {
            requirement_results,
            ..
        }
        | CompletedRoleResult::Reviewer {
            requirement_results,
            ..
        }
        | CompletedRoleResult::Specialist {
            requirement_results,
            ..
        } => Ok(requirement_results),
        CompletedRoleResult::Writer { .. } => Err(HarnessError::InvalidSubmission(
            "producer role cannot own verification requirements".to_owned(),
        )),
    }
}

struct EvaluatedOutputs {
    subject_status: SubjectStatus,
    artifact: Option<EvaluatedArtifact>,
    blocking_findings: Vec<BlockingFinding>,
    improvements: Vec<ImprovementOpportunity>,
    promotion_proposals: Vec<PromotionProposal>,
}

fn evaluated_outputs(
    plan: &ResolvedHarnessPlan,
    record: &RoleExecutionRecord,
    requirements: &[EvaluatedRequirement],
) -> HarnessResult<EvaluatedOutputs> {
    let reviewer_result = record
        .role_results
        .iter()
        .find(|result| result.role == HarnessRole::Reviewer)
        .and_then(|execution| match &execution.outcome {
            RoleExecutionOutcome::Completed {
                result:
                    CompletedRoleResult::Reviewer {
                        blocking_findings,
                        improvements,
                        learning_candidates,
                        ..
                    },
            } => Some((
                execution,
                blocking_findings,
                improvements,
                learning_candidates,
            )),
            _ => None,
        });
    let (blocking_findings, improvements) = reviewer_result.map_or_else(
        || (Vec::new(), Vec::new()),
        |(_, findings, improvements, _)| (findings.clone(), improvements.clone()),
    );
    let subject_status =
        if requirements.iter().all(|result| result.passed) && blocking_findings.is_empty() {
            SubjectStatus::Accepted
        } else {
            SubjectStatus::Rejected
        };
    let mut promotion_proposals = Vec::new();
    let artifact = match plan.action {
        HarnessAction::CodeWrite | HarnessAction::DocumentWrite | HarnessAction::VaultCuration => {
            let producer = completed_role_result(record, HarnessRole::Writer)?;
            let CompletedRoleResult::Writer { artifact } = producer else {
                unreachable!("Writer result kind is validated before evaluation");
            };
            Some(EvaluatedArtifact::Writer {
                artifact: artifact.clone(),
            })
        }
        HarnessAction::Investigation
        | HarnessAction::Design
        | HarnessAction::Ideation
        | HarnessAction::VaultRead => {
            let producer = completed_role_result(record, HarnessRole::Specialist)?;
            let CompletedRoleResult::Specialist {
                artifact,
                promotion_proposals: specialist_promotions,
                ..
            } = producer
            else {
                unreachable!("Specialist result kind is validated before evaluation");
            };
            if subject_status == SubjectStatus::Accepted {
                promotion_proposals.extend(specialist_promotions.clone());
            }
            Some(EvaluatedArtifact::Specialist {
                artifact: artifact.clone(),
            })
        }
        HarnessAction::CodeReview | HarnessAction::DocumentReview => {
            reviewer_result.ok_or_else(|| {
                HarnessError::InvalidSubmission(
                    "review action requires a completed Reviewer result".to_owned(),
                )
            })?;
            let summary = match subject_status {
                SubjectStatus::Accepted => "No Findings",
                SubjectStatus::Rejected => {
                    "Rejected: see failed requirements and blocking findings."
                }
                SubjectStatus::NotApplicable => {
                    unreachable!("completed review subject status must be applicable")
                }
            };
            Some(EvaluatedArtifact::Review {
                summary: summary.to_owned(),
            })
        }
    };
    if let Some((reviewer_execution, _, _, learning_candidates)) = reviewer_result {
        promotion_proposals.extend(derive_reviewer_learning_promotions(
            plan,
            record,
            reviewer_execution,
            learning_candidates,
            requirements,
        )?);
    }
    validate_unique_promotion_identifiers(&promotion_proposals)?;
    Ok(EvaluatedOutputs {
        subject_status,
        artifact,
        blocking_findings,
        improvements,
        promotion_proposals,
    })
}

fn validate_unique_promotion_identifiers(proposals: &[PromotionProposal]) -> HarnessResult<()> {
    let mut identifiers = BTreeSet::new();
    if proposals
        .iter()
        .any(|proposal| !identifiers.insert(proposal.identifier.as_str()))
    {
        return Err(HarnessError::InvalidSubmission(
            "promotion proposal identifiers must be unique across role results".to_owned(),
        ));
    }
    Ok(())
}

fn derive_reviewer_learning_promotions(
    plan: &ResolvedHarnessPlan,
    record: &RoleExecutionRecord,
    reviewer_execution: &RoleExecutionResult,
    candidates: &[ReviewerLearningCandidate],
    requirements: &[EvaluatedRequirement],
) -> HarnessResult<Vec<PromotionProposal>> {
    candidates
        .iter()
        .map(|candidate| {
            let failed = requirements
                .iter()
                .find(|result| result.requirement.unit == candidate.failed_unit && !result.passed)
                .ok_or_else(|| {
                    HarnessError::InvalidSubmission(
                        "Reviewer learning candidate did not produce a failed evaluated requirement"
                            .to_owned(),
                    )
                })?;
            if failed.evidence != candidate.evidence {
                return Err(HarnessError::InvalidSubmission(
                    "Reviewer learning candidate evidence changed during evaluation".to_owned(),
                ));
            }
            let identity = (
                &record.resolved_plan_digest,
                &record.prepared_role_run_digest,
                &reviewer_execution.result_digest,
                candidate.failed_unit,
                &candidate.requirement_result_digest,
            );
            let learning_id = digest_identifier("learning-", &("reviewer-learning", &identity))?;
            let identifier =
                digest_identifier("proposal-", &("reviewer-learning-promotion", &identity))?;
            let proposal = PromotionProposal {
                identifier,
                owner: plan.owner.clone(),
                curation_kind: CurationKind::Knowledge,
                title: candidate.title.clone(),
                content: candidate.guidance.clone(),
                origin: PromotionProposalOrigin::ReviewerLearning {
                    learning_id,
                    source_action: plan.action,
                    source_intent: plan.intent,
                    failed_unit: candidate.failed_unit,
                    requirement_result_digest: candidate.requirement_result_digest.clone(),
                    evidence: candidate.evidence.clone(),
                },
            };
            validate_promotion_proposal_shape(&proposal)?;
            Ok(proposal)
        })
        .collect()
}

fn digest_identifier<T: Serialize>(prefix: &str, value: &T) -> HarnessResult<String> {
    let digest = serialized_digest(value)?;
    Ok(format!("{prefix}{}", &digest[..16]))
}

fn completed_role_result(
    record: &RoleExecutionRecord,
    role: HarnessRole,
) -> HarnessResult<&CompletedRoleResult> {
    let outcome = &record
        .role_results
        .iter()
        .find(|result| result.role == role)
        .ok_or_else(|| {
            HarnessError::InvalidSubmission(format!("completed {role:?} result is missing"))
        })?
        .outcome;
    let RoleExecutionOutcome::Completed { result } = outcome else {
        return Err(HarnessError::InvalidSubmission(format!(
            "{role:?} result is not completed"
        )));
    };
    Ok(result)
}

pub(super) fn evaluated_execution_changes(
    evaluation: &HarnessTaskEvaluation,
) -> HarnessResult<Vec<FileChange>> {
    let changes = match &evaluation.artifact {
        Some(EvaluatedArtifact::Writer {
            artifact: WriterArtifact::Changes { changes },
        }) => changes.clone(),
        Some(EvaluatedArtifact::Writer {
            artifact: WriterArtifact::Curation { entries, .. },
        }) => entries.iter().map(|entry| entry.change.clone()).collect(),
        _ => {
            return Err(HarnessError::InvalidSubmission(
                "only Writer file-change results can be applied".to_owned(),
            ));
        }
    };
    if changes.is_empty() {
        return Err(HarnessError::InvalidSubmission(
            "Writer result must contain at least one file change".to_owned(),
        ));
    }
    Ok(changes)
}

pub(super) fn career_verification_bindings(
    plan: &ResolvedHarnessPlan,
    record: &RoleExecutionRecord,
) -> HarnessResult<Vec<VerificationExecutionBinding>> {
    plan.verification_requirements
        .iter()
        .map(|requirement| {
            let result_digest = match requirement.owner {
                VerificationOwner::Deterministic => serialized_digest(&(
                    &record.resolved_plan_digest,
                    requirement.unit,
                    "core-validation-passed",
                ))?,
                VerificationOwner::Tool => record
                    .tool_evidence
                    .as_ref()
                    .map(|evidence| evidence.evidence_set_digest.clone())
                    .ok_or_else(|| {
                        HarnessError::InvalidSubmission(
                            "career review is missing tool verification evidence".to_owned(),
                        )
                    })?,
                VerificationOwner::Role { role } => record
                    .role_results
                    .iter()
                    .find(|result| result.role == role)
                    .map(|result| result.result_digest.clone())
                    .ok_or_else(|| {
                        HarnessError::InvalidSubmission(format!(
                            "career review is missing {role:?} verification evidence"
                        ))
                    })?,
            };
            Ok(VerificationExecutionBinding {
                unit: requirement.unit,
                owner: requirement.owner,
                result_digest,
            })
        })
        .collect()
}

pub(super) fn career_execution_review_binding(
    evidence_owner: Option<DataOwner>,
    receipt: &CareerExecutionReviewReceipt,
) -> CareerExecutionReviewBinding {
    CareerExecutionReviewBinding {
        evidence_owner,
        resolved_plan_digest: receipt.resolved.resolved_plan_digest.clone(),
        prepared_role_run_digest: receipt.prepared.prepared_role_run_digest.clone(),
        execution_record_digest: receipt
            .evaluation
            .execution_record
            .execution_record_digest
            .clone(),
        task_evaluation_receipt_digest: receipt.evaluation.task_evaluation_receipt_digest.clone(),
        reviewer_result_digest: receipt.reviewer_result_digest.clone(),
        verification_bindings: receipt.verification_bindings.clone(),
        artifact_set_digest: receipt.artifact_set_digest.clone(),
        evidence_bundle_digest: receipt.evidence_bundle_digest.clone(),
        receipt_digest: receipt.receipt_digest.clone(),
        role_executions: receipt.evaluation.role_executions.clone(),
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VerificationCheck {
    pub id: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Finding {
    pub severity: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "operation")]
pub enum FileChange {
    Create {
        path: String,
        content: String,
    },
    Update {
        path: String,
        expected_content_digest: String,
        content: String,
    },
    Delete {
        path: String,
        expected_content_digest: String,
    },
}

impl FileChange {
    pub(super) fn path(&self) -> &str {
        match self {
            Self::Create { path, .. } | Self::Update { path, .. } | Self::Delete { path, .. } => {
                path
            }
        }
    }

    fn content_len(&self) -> usize {
        match self {
            Self::Create { content, .. } | Self::Update { content, .. } => content.len(),
            Self::Delete { .. } => 0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CurationEntry {
    pub change: FileChange,
    pub provenance: Vec<String>,
    pub source_references: Vec<SourceBinding>,
    pub promotion_handoff_digest: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum SubmissionArtifact {
    Changes {
        changes: Vec<FileChange>,
    },
    Review {
        reviewed_targets: Vec<String>,
        summary: String,
    },
    Analysis {
        source_targets: Vec<String>,
        context_bundle_digest: Option<String>,
        output: String,
    },
    Curation {
        curation_kind: CurationKind,
        entries: Vec<CurationEntry>,
        reported_user_confirmation: bool,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReportedRoleContext {
    pub role: HarnessRole,
    pub context_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RoleTerminalState {
    Completed,
    MissingContext,
    Failed,
    Cancelled,
    TimedOut,
    Unsupported,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReportedRoleLifecycle {
    pub role: HarnessRole,
    pub context_id: String,
    pub started_at_millis: u64,
    pub context_ready_at_millis: u64,
    pub first_output_at_millis: Option<u64>,
    pub interrupt_requested_at_millis: Option<u64>,
    pub grace_deadline_at_millis: Option<u64>,
    pub terminal_at_millis: u64,
    pub closed_at_millis: u64,
    pub terminal_state: RoleTerminalState,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HarnessSubmission {
    pub resolved_plan_digest: String,
    pub prepared_role_run_digest: String,
    pub revision: u32,
    pub previous_candidate_digest: Option<String>,
    pub revision_feedback: Vec<Finding>,
    pub artifact: SubmissionArtifact,
    pub reported_producer_context_id: String,
    pub reported_reviewer_context_id: String,
    pub reported_role_contexts: Vec<ReportedRoleContext>,
    pub reported_role_lifecycles: Vec<ReportedRoleLifecycle>,
    pub verification_checks: Vec<VerificationCheck>,
    pub findings: Vec<Finding>,
    pub assurance: ExecutionAssurance,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HarnessEvaluationResult {
    pub resolved_plan_digest: String,
    pub prepared_role_run_digest: String,
    pub revision: u32,
    pub previous_candidate_digest: Option<String>,
    pub revision_feedback: Vec<Finding>,
    pub candidate_digest: String,
    pub validation_receipt_digest: String,
    pub artifact: SubmissionArtifact,
    pub reported_producer_context_id: String,
    pub reported_reviewer_context_id: String,
    pub reported_role_contexts: Vec<ReportedRoleContext>,
    pub reported_role_lifecycles: Vec<ReportedRoleLifecycle>,
    pub verification_checks: Vec<VerificationCheck>,
    pub findings: Vec<Finding>,
    pub accepted: bool,
    pub assurance: ExecutionAssurance,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CareerReviewReceipt {
    pub version: u32,
    pub resolved: ResolvedHarnessRequest,
    pub prepared: PreparedRoleRun,
    pub submission: HarnessSubmission,
    pub evaluation: HarnessEvaluationResult,
    pub artifact_set_digest: String,
    pub evidence_bundle_digest: Option<String>,
    pub receipt_digest: String,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CareerReviewBinding {
    pub evidence_owner: Option<DataOwner>,
    pub resolved_plan_digest: String,
    pub prepared_role_run_digest: String,
    pub candidate_digest: String,
    pub validation_receipt_digest: String,
    pub artifact_set_digest: String,
    pub evidence_bundle_digest: Option<String>,
    pub receipt_digest: String,
    pub reported_producer_context_id: String,
    pub reported_reviewer_context_id: String,
    pub reported_role_contexts: Vec<ReportedRoleContext>,
    pub reported_role_lifecycles: Vec<ReportedRoleLifecycle>,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CareerCompositionReceipt {
    pub version: u32,
    pub manifest_digest: String,
    pub career_output_surface: CareerOutputSurface,
    pub coverage: CareerCoverageMode,
    pub declared_evidence_owners: Vec<DataOwner>,
    pub artifact_targets: Vec<TargetBinding>,
    pub artifact_set_digest: String,
    pub holistic_review: CareerReviewBinding,
    pub evidence_reviews: Vec<CareerReviewBinding>,
    pub coverage_is_caller_attested: bool,
    pub assurance: ExecutionAssurance,
    pub composition_digest: String,
}

#[cfg(test)]
pub(super) fn submission_artifact_template(
    plan: &ResolvedHarnessPlan,
    context_bundle_digest: Option<String>,
) -> HarnessResult<SubmissionArtifact> {
    Ok(match plan.action {
        HarnessAction::CodeWrite | HarnessAction::DocumentWrite => SubmissionArtifact::Changes {
            changes: plan
                .targets
                .iter()
                .map(file_change_template)
                .collect::<HarnessResult<Vec<_>>>()?,
        },
        HarnessAction::CodeReview | HarnessAction::DocumentReview => SubmissionArtifact::Review {
            reviewed_targets: planned_target_paths(plan),
            summary: String::new(),
        },
        HarnessAction::Investigation
        | HarnessAction::Design
        | HarnessAction::Ideation
        | HarnessAction::VaultRead => SubmissionArtifact::Analysis {
            source_targets: planned_target_paths(plan),
            context_bundle_digest,
            output: String::new(),
        },
        HarnessAction::VaultCuration => SubmissionArtifact::Curation {
            curation_kind: plan.curation_kind.ok_or_else(|| {
                HarnessError::InvalidPlan("curation plan is missing its curation kind".to_owned())
            })?,
            entries: plan
                .targets
                .iter()
                .map(|target| {
                    Ok(CurationEntry {
                        change: file_change_template(target)?,
                        provenance: Vec::new(),
                        source_references: plan.curation_sources.clone(),
                        promotion_handoff_digest: plan
                            .promotion_handoff
                            .as_ref()
                            .map(|handoff| handoff.handoff_digest.clone()),
                    })
                })
                .collect::<HarnessResult<Vec<_>>>()?,
            reported_user_confirmation: true,
        },
    })
}

#[cfg(test)]
fn file_change_template(target: &TargetBinding) -> HarnessResult<FileChange> {
    Ok(match (&target.operation, &target.state) {
        (TargetOperation::Create, TargetState::Absent) => FileChange::Create {
            path: target.workspace_relative_path.clone(),
            content: String::new(),
        },
        (TargetOperation::Update, TargetState::Existing { content_digest }) => FileChange::Update {
            path: target.workspace_relative_path.clone(),
            expected_content_digest: content_digest.clone(),
            content: String::new(),
        },
        (TargetOperation::Delete, TargetState::Existing { content_digest }) => FileChange::Delete {
            path: target.workspace_relative_path.clone(),
            expected_content_digest: content_digest.clone(),
        },
        _ => {
            return Err(HarnessError::InvalidPlan(format!(
                "target operation does not match starting state for `{}`",
                target.workspace_relative_path
            )));
        }
    })
}

#[cfg(test)]
pub(super) fn validate_submission_artifact(
    plan: &ResolvedHarnessPlan,
    artifact: &SubmissionArtifact,
) -> HarnessResult<()> {
    match (plan.action, artifact) {
        (
            HarnessAction::CodeWrite | HarnessAction::DocumentWrite,
            SubmissionArtifact::Changes { changes },
        ) => validate_file_changes(&plan.targets, changes),
        (
            HarnessAction::CodeReview | HarnessAction::DocumentReview,
            SubmissionArtifact::Review {
                reviewed_targets,
                summary,
            },
        ) => {
            validate_target_coverage(plan, reviewed_targets)?;
            validate_artifact_text("review summary", summary)
        }
        (
            HarnessAction::Investigation
            | HarnessAction::Design
            | HarnessAction::Ideation
            | HarnessAction::VaultRead,
            SubmissionArtifact::Analysis {
                source_targets,
                context_bundle_digest: _,
                output,
            },
        ) => {
            validate_target_coverage(plan, source_targets)?;
            validate_artifact_text("analysis output", output)
        }
        (
            HarnessAction::VaultCuration,
            SubmissionArtifact::Curation {
                curation_kind,
                entries,
                reported_user_confirmation,
            },
        ) => validate_curation_entries(plan, *curation_kind, entries, *reported_user_confirmation),
        _ => Err(HarnessError::InvalidSubmission(
            "submission artifact kind does not match the planned action".to_owned(),
        )),
    }
}

#[cfg(test)]
pub(super) fn validate_submission_context_binding(
    prepared: &PreparedRoleRun,
    artifact: &SubmissionArtifact,
) -> HarnessResult<()> {
    let expected = prepared
        .context_bundle
        .as_ref()
        .map(|bundle| bundle.bundle_digest.as_str());
    let submitted = match artifact {
        SubmissionArtifact::Analysis {
            context_bundle_digest,
            ..
        } => context_bundle_digest.as_deref(),
        _ => None,
    };
    if submitted != expected {
        return Err(HarnessError::InvalidSubmission(
            "submission context bundle does not match the prepared execution".to_owned(),
        ));
    }
    Ok(())
}

fn validate_file_changes(targets: &[TargetBinding], changes: &[FileChange]) -> HarnessResult<()> {
    if changes.len() != targets.len() {
        return Err(HarnessError::InvalidSubmission(
            "file change count does not match the planned target count".to_owned(),
        ));
    }
    let mut ordered = changes.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|change| change.path());
    let mut total_bytes = 0_usize;
    for (target, change) in targets.iter().zip(ordered) {
        if change.path() != target.workspace_relative_path {
            return Err(HarnessError::InvalidSubmission(
                "file change paths do not match the planned targets".to_owned(),
            ));
        }
        match (target.operation, &target.state, change) {
            (TargetOperation::Create, TargetState::Absent, FileChange::Create { content, .. }) => {
                validate_artifact_text("created file content", content)?;
            }
            (
                TargetOperation::Update,
                TargetState::Existing { content_digest },
                FileChange::Update {
                    expected_content_digest,
                    content,
                    ..
                },
            ) if content_digest == expected_content_digest => {
                validate_artifact_text("updated file content", content)?;
            }
            (
                TargetOperation::Delete,
                TargetState::Existing { content_digest },
                FileChange::Delete {
                    expected_content_digest,
                    ..
                },
            ) if content_digest == expected_content_digest => {}
            _ => {
                return Err(HarnessError::InvalidSubmission(format!(
                    "file operation or starting digest does not match `{}`",
                    target.workspace_relative_path
                )));
            }
        }
        total_bytes = total_bytes
            .checked_add(change.content_len())
            .ok_or_else(|| HarnessError::InvalidSubmission("candidate size overflow".to_owned()))?;
        if total_bytes > MAX_CANDIDATE_BYTES {
            return Err(HarnessError::InvalidSubmission(format!(
                "candidate content exceeds the {MAX_CANDIDATE_BYTES} byte limit"
            )));
        }
    }
    Ok(())
}

fn validate_curation_entries(
    plan: &ResolvedHarnessPlan,
    kind: CurationKind,
    entries: &[CurationEntry],
    reported_user_confirmation: bool,
) -> HarnessResult<()> {
    if plan.curation_kind != Some(kind) {
        return Err(HarnessError::InvalidSubmission(
            "curation kind does not match the resolved request".to_owned(),
        ));
    }
    if !reported_user_confirmation {
        return Err(HarnessError::InvalidSubmission(
            "Vault curation requires explicit user confirmation".to_owned(),
        ));
    }
    let changes = entries
        .iter()
        .map(|entry| &entry.change)
        .collect::<Vec<_>>();
    if changes.len() != plan.targets.len() {
        return Err(HarnessError::InvalidSubmission(
            "curation entry count does not match the planned targets".to_owned(),
        ));
    }
    let owned_changes = changes.into_iter().cloned().collect::<Vec<_>>();
    validate_file_changes(&plan.targets, &owned_changes)?;
    let expected_learning_markdown = plan
        .promotion_handoff
        .as_ref()
        .filter(|handoff| {
            matches!(
                handoff.proposal.origin,
                PromotionProposalOrigin::ReviewerLearning { .. }
            )
        })
        .map(canonical_learning_markdown)
        .transpose()?;
    for entry in entries {
        validate_bounded_text_list("curation provenance", &entry.provenance, true)?;
        validate_source_bindings(&plan.curation_sources, &entry.source_references)?;
        let expected_handoff_digest = plan
            .promotion_handoff
            .as_ref()
            .map(|handoff| handoff.handoff_digest.as_str());
        if entry.promotion_handoff_digest.as_deref() != expected_handoff_digest {
            return Err(HarnessError::InvalidSubmission(
                "curation entry promotion handoff digest does not match the resolved plan"
                    .to_owned(),
            ));
        }
        if let Some(expected) = &expected_learning_markdown {
            let FileChange::Create { content, .. } = &entry.change else {
                return Err(HarnessError::InvalidSubmission(
                    "Reviewer learning curation must create a new Knowledge document".to_owned(),
                ));
            };
            if content != expected {
                return Err(HarnessError::InvalidSubmission(
                    "Reviewer learning curation content does not match canonical learning Markdown"
                        .to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_source_bindings(
    planned: &[SourceBinding],
    submitted: &[SourceBinding],
) -> HarnessResult<()> {
    if submitted.len() > MAX_TARGETS {
        return Err(HarnessError::InvalidSubmission(format!(
            "curation source bindings exceed the {MAX_TARGETS} item limit"
        )));
    }
    let mut planned = planned.to_vec();
    let mut submitted = submitted.to_vec();
    planned.sort_by(|left, right| {
        left.repository_relative_path
            .cmp(&right.repository_relative_path)
    });
    submitted.sort_by(|left, right| {
        left.repository_relative_path
            .cmp(&right.repository_relative_path)
    });
    if submitted != planned {
        return Err(HarnessError::InvalidSubmission(
            "curation source paths and content digests must match the resolved plan".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn validate_revision_history(
    resolved: &ResolvedHarnessRequest,
    prepared_role_run_digest: &str,
    runtime_capabilities: &RoleRuntimeCapabilities,
    history: &[HarnessEvaluationResult],
) -> HarnessResult<()> {
    if history.len() > resolved.plan.max_revisions as usize {
        return Err(HarnessError::InvalidSubmission(
            "revision history exceeds the plan limit".to_owned(),
        ));
    }
    let mut previous_result: Option<&HarnessEvaluationResult> = None;
    let mut observed_context_ids = BTreeSet::new();
    for (expected_revision, result) in history.iter().enumerate() {
        if result.resolved_plan_digest != resolved.resolved_plan_digest
            || result.prepared_role_run_digest != prepared_role_run_digest
            || result.revision as usize != expected_revision
        {
            return Err(HarnessError::InvalidSubmission(
                "revision history plan or sequence does not match the resolved plan".to_owned(),
            ));
        }
        let candidate_digest = serialized_digest(&(
            &result.resolved_plan_digest,
            &result.prepared_role_run_digest,
            result.revision,
            &result.previous_candidate_digest,
            &result.revision_feedback,
            &result.reported_producer_context_id,
            &result.artifact,
        ))?;
        if candidate_digest != result.candidate_digest {
            return Err(HarnessError::InvalidSubmission(
                "revision history candidate digest is invalid".to_owned(),
            ));
        }
        let receipt_digest = serialized_digest(&(
            &result.candidate_digest,
            &result.reported_reviewer_context_id,
            &result.reported_role_contexts,
            &result.reported_role_lifecycles,
            &result.verification_checks,
            &result.findings,
            result.assurance,
            result.accepted,
        ))?;
        if receipt_digest != result.validation_receipt_digest {
            return Err(HarnessError::InvalidSubmission(
                "revision history validation receipt digest is invalid".to_owned(),
            ));
        }
        validate_result_chain_fields(result, previous_result)?;
        validate_reported_context_ids(
            &resolved.plan,
            &result.reported_producer_context_id,
            &result.reported_reviewer_context_id,
            &result.reported_role_contexts,
        )?;
        for role_context in &result.reported_role_contexts {
            if !observed_context_ids.insert(role_context.context_id.as_str()) {
                return Err(HarnessError::InvalidSubmission(
                    "revision history role context IDs must be globally unique".to_owned(),
                ));
            }
        }
        validate_reported_role_lifecycles(
            &resolved.plan,
            runtime_capabilities,
            &result.reported_role_contexts,
            &result.reported_role_lifecycles,
        )?;
        validate_submission_artifact(&resolved.plan, &result.artifact)?;
        validate_checks(
            &resolved.plan.verification_requirements,
            &result.verification_checks,
        )?;
        validate_bounded_findings("revision history findings", &result.findings)?;
        if result.accepted != evaluation_is_accepted(&result.verification_checks, &result.findings)
            || result.assurance != ExecutionAssurance::Advisory
        {
            return Err(HarnessError::InvalidSubmission(
                "revision history acceptance or assurance is inconsistent".to_owned(),
            ));
        }
        previous_result = Some(result);
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn validate_submission_context_freshness(
    submission: &HarnessSubmission,
    history: &[HarnessEvaluationResult],
) -> HarnessResult<()> {
    let historical_context_ids = history
        .iter()
        .flat_map(|result| &result.reported_role_contexts)
        .map(|role_context| role_context.context_id.as_str())
        .collect::<BTreeSet<_>>();
    if submission
        .reported_role_contexts
        .iter()
        .any(|role_context| historical_context_ids.contains(role_context.context_id.as_str()))
    {
        return Err(HarnessError::InvalidSubmission(
            "revised submissions must use fresh role context IDs".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
fn validate_result_chain_fields(
    result: &HarnessEvaluationResult,
    previous_result: Option<&HarnessEvaluationResult>,
) -> HarnessResult<()> {
    validate_bounded_findings("revision feedback", &result.revision_feedback)?;
    match (result.revision, previous_result) {
        (0, None)
            if result.previous_candidate_digest.is_none()
                && result.revision_feedback.is_empty() =>
        {
            Ok(())
        }
        (revision, Some(previous))
            if revision > 0
                && result.previous_candidate_digest.as_deref()
                    == Some(previous.candidate_digest.as_str())
                && result.revision_feedback == revision_feedback_for(previous)
                && !previous.accepted =>
        {
            Ok(())
        }
        _ => Err(HarnessError::InvalidSubmission(
            "revision history does not form an exact candidate chain".to_owned(),
        )),
    }
}

#[cfg(test)]
pub(super) fn validate_revision_chain(
    submission: &HarnessSubmission,
    previous_result: Option<&HarnessEvaluationResult>,
) -> HarnessResult<()> {
    validate_bounded_findings("revision feedback", &submission.revision_feedback)?;
    match (
        submission.revision,
        &submission.previous_candidate_digest,
        submission.revision_feedback.is_empty(),
        previous_result,
    ) {
        (0, None, true, None) => Ok(()),
        (0, _, _, _) => Err(HarnessError::InvalidSubmission(
            "revision zero must not contain a previous candidate or revision feedback".to_owned(),
        )),
        (_, Some(digest), false, Some(previous))
            if digest == &previous.candidate_digest
                && submission.revision == previous.revision + 1
                && submission.revision_feedback == revision_feedback_for(previous)
                && !previous.accepted =>
        {
            validate_digest("previous candidate digest", digest)
        }
        (_, _, _, _) => Err(HarnessError::InvalidSubmission(
            "a later revision requires the exact previous validated candidate and revision feedback"
                .to_owned(),
        )),
    }
}

#[cfg(test)]
fn validate_target_coverage(plan: &ResolvedHarnessPlan, submitted: &[String]) -> HarnessResult<()> {
    let mut submitted = submitted.to_vec();
    submitted.sort();
    if !submitted.is_empty() {
        ensure_unique_strings("submitted target", &submitted)
            .map_err(|error| HarnessError::InvalidSubmission(error.to_string()))?;
    }
    if submitted != planned_target_paths(plan) {
        return Err(HarnessError::InvalidSubmission(
            "submission target coverage does not match the resolved plan".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
fn planned_target_paths(plan: &ResolvedHarnessPlan) -> Vec<String> {
    plan.targets
        .iter()
        .map(|target| target.workspace_relative_path.clone())
        .collect()
}

fn validate_artifact_text(field: &str, value: &str) -> HarnessResult<()> {
    if value.trim().is_empty() {
        return Err(HarnessError::InvalidSubmission(format!(
            "{field} must not be empty"
        )));
    }
    if value.len() > MAX_CANDIDATE_BYTES {
        return Err(HarnessError::InvalidSubmission(format!(
            "{field} exceeds the {MAX_CANDIDATE_BYTES} byte limit"
        )));
    }
    Ok(())
}

fn validate_bounded_text_list(field: &str, values: &[String], required: bool) -> HarnessResult<()> {
    if (required && values.is_empty()) || values.len() > MAX_SUBMISSION_LIST_ITEMS {
        return Err(HarnessError::InvalidSubmission(format!(
            "{field} must contain between {} and {MAX_SUBMISSION_LIST_ITEMS} values",
            usize::from(required)
        )));
    }
    let mut total = 0_usize;
    for value in values {
        if value.trim().is_empty() {
            return Err(HarnessError::InvalidSubmission(format!(
                "{field} must contain non-empty values"
            )));
        }
        total = total
            .checked_add(value.len())
            .ok_or_else(|| HarnessError::InvalidSubmission(format!("{field} size overflow")))?;
        if total > MAX_SUBMISSION_TEXT_BYTES {
            return Err(HarnessError::InvalidSubmission(format!(
                "{field} exceeds the {MAX_SUBMISSION_TEXT_BYTES} byte limit"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn validate_bounded_findings(field: &str, findings: &[Finding]) -> HarnessResult<()> {
    if findings.len() > MAX_SUBMISSION_LIST_ITEMS {
        return Err(HarnessError::InvalidSubmission(format!(
            "{field} exceeds the {MAX_SUBMISSION_LIST_ITEMS} item limit"
        )));
    }
    let mut total = 0_usize;
    for finding in findings {
        validate_runtime_text("finding severity", &finding.severity)?;
        if finding.message.trim().is_empty() {
            return Err(HarnessError::InvalidSubmission(
                "finding message must not be empty".to_owned(),
            ));
        }
        total = total
            .checked_add(finding.severity.len())
            .and_then(|value| value.checked_add(finding.message.len()))
            .ok_or_else(|| HarnessError::InvalidSubmission(format!("{field} size overflow")))?;
        if total > MAX_SUBMISSION_TEXT_BYTES {
            return Err(HarnessError::InvalidSubmission(format!(
                "{field} exceeds the {MAX_SUBMISSION_TEXT_BYTES} byte limit"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
fn validate_digest(field: &str, value: &str) -> HarnessResult<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(HarnessError::InvalidSubmission(format!(
            "{field} must be a 64-character hexadecimal SHA-256 digest"
        )));
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn evaluation_is_accepted(checks: &[VerificationCheck], findings: &[Finding]) -> bool {
    findings.is_empty() && checks.iter().all(|check| check.passed)
}

#[cfg(test)]
fn revision_feedback_for(result: &HarnessEvaluationResult) -> Vec<Finding> {
    let mut feedback = result.findings.clone();
    feedback.extend(
        result
            .verification_checks
            .iter()
            .filter(|check| !check.passed)
            .map(|check| Finding {
                severity: "verification".to_owned(),
                message: format!("{}: {}", check.id, check.detail),
            }),
    );
    feedback
}
