use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use serde::{Deserialize, Serialize};

use super::{
    AppliedHarnessBatch, BatchApplyOutcomeReceipt, BatchTargetApplyStateReceipt, FileChange,
    HARNESS_SCHEMA_VERSION, HarnessCompletionState, HarnessEngine, HarnessError, HarnessResult,
    HarnessTaskEvaluation, MAX_APPLY_RECEIPT_BYTES, TargetOperation, VerifiedTargetStateReceipt,
    byte_digest, evaluated_execution_changes, require_version, serialized_digest, validate_digest,
    validate_serialized_size,
};
use super::{FrozenTargets, HarnessExecutionRecord, HarnessExecutionState, PreparedHarnessRun};

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FinalizationTarget {
    pub version: u32,
    pub workspace_relative_path: String,
    pub operation: FinalizationOperation,
    pub expected_current_digest: Option<String>,
    pub resulting_content_digest: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FinalizationOperation {
    Create,
    Update,
    Delete,
}

impl TryFrom<TargetOperation> for FinalizationOperation {
    type Error = HarnessError;

    fn try_from(operation: TargetOperation) -> HarnessResult<Self> {
        match operation {
            TargetOperation::Create => Ok(Self::Create),
            TargetOperation::Update => Ok(Self::Update),
            TargetOperation::Delete => Ok(Self::Delete),
            TargetOperation::Inspect => Err(HarnessError::InvalidSubmission(
                "read-only frozen targets cannot enter finalization".to_owned(),
            )),
        }
    }
}

fn finalization_operations(targets: &FrozenTargets) -> HarnessResult<Vec<FinalizationOperation>> {
    targets
        .targets
        .iter()
        .map(|target| FinalizationOperation::try_from(target.operation))
        .collect()
}

impl FinalizationTarget {
    pub fn from_frozen_targets(
        targets: &FrozenTargets,
        resulting_digests: &BTreeMap<String, Option<String>>,
    ) -> HarnessResult<Vec<Self>> {
        targets.validate()?;
        let operations = finalization_operations(targets)?;
        if resulting_digests.len() != targets.targets.len() {
            return Err(HarnessError::InvalidPlan(
                "current finalization results must cover exactly every frozen target".to_owned(),
            ));
        }
        targets
            .targets
            .iter()
            .zip(operations)
            .map(|(target, operation)| {
                let result = resulting_digests
                    .get(&target.workspace_relative_path)
                    .ok_or_else(|| {
                        HarnessError::InvalidPlan(format!(
                            "missing finalization result for `{}`",
                            target.workspace_relative_path
                        ))
                    })?
                    .clone();
                let view = Self {
                    version: HARNESS_SCHEMA_VERSION,
                    workspace_relative_path: target.workspace_relative_path.clone(),
                    operation,
                    expected_current_digest: target.content_digest.clone(),
                    resulting_content_digest: result,
                };
                view.validate()?;
                Ok(view)
            })
            .collect()
    }

    pub fn from_applied_batch(
        frozen: &FrozenTargets,
        applied: &AppliedHarnessBatch,
    ) -> HarnessResult<Vec<Self>> {
        frozen.validate()?;
        if applied.targets.len() != frozen.targets.len()
            || applied
                .targets
                .iter()
                .any(|target| target.state != BatchTargetApplyStateReceipt::Applied)
        {
            return Err(HarnessError::InvalidSubmission(
                "Harness engine apply receipt must contain exactly the fully-applied frozen target set"
                    .to_owned(),
            ));
        }
        let resulting = applied
            .targets
            .iter()
            .map(|target| {
                let repository = target.repository_apply.as_ref().ok_or_else(|| {
                    HarnessError::InvalidSubmission(format!(
                        "Harness engine apply receipt omitted repository evidence for `{}`",
                        target.workspace_relative_path
                    ))
                })?;
                let digest = match &repository.verified_target_state {
                    VerifiedTargetStateReceipt::Present { content_digest } => {
                        Some(content_digest.clone())
                    }
                    VerifiedTargetStateReceipt::Absent => None,
                    VerifiedTargetStateReceipt::Unverified => {
                        return Err(HarnessError::InvalidSubmission(format!(
                            "Harness engine apply receipt left `{}` unverified",
                            target.workspace_relative_path
                        )));
                    }
                };
                Ok((target.workspace_relative_path.clone(), digest))
            })
            .collect::<HarnessResult<BTreeMap<_, _>>>()?;
        Self::from_frozen_targets(frozen, &resulting)
    }

    pub fn from_recovered_batch(
        frozen: &FrozenTargets,
        applied: &AppliedHarnessBatch,
    ) -> HarnessResult<Vec<Self>> {
        frozen.validate()?;
        if applied.targets.len() != frozen.targets.len()
            || applied
                .targets
                .iter()
                .any(|target| target.state != BatchTargetApplyStateReceipt::Applied)
        {
            return Err(HarnessError::InvalidSubmission(
                "recovered Harness engine receipt must contain exactly the applied frozen target set"
                    .to_owned(),
            ));
        }
        let resulting = applied
            .targets
            .iter()
            .map(|target| {
                (
                    target.workspace_relative_path.clone(),
                    target.intended_content_digest.clone(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        Self::from_frozen_targets(frozen, &resulting)
    }

    pub fn validate(&self) -> HarnessResult<()> {
        require_version(
            "finalization target schema",
            self.version,
            HARNESS_SCHEMA_VERSION,
        )?;
        validate_relative_path(&self.workspace_relative_path)?;
        match self.operation {
            FinalizationOperation::Create => {
                if self.expected_current_digest.is_some() || self.resulting_content_digest.is_none()
                {
                    return Err(HarnessError::InvalidPlan(
                        "create finalization requires absent current state and a result digest"
                            .to_owned(),
                    ));
                }
            }
            FinalizationOperation::Update => {
                if self.expected_current_digest.is_none() || self.resulting_content_digest.is_none()
                {
                    return Err(HarnessError::InvalidPlan(
                        "update finalization requires current and result digests".to_owned(),
                    ));
                }
            }
            FinalizationOperation::Delete => {
                if self.expected_current_digest.is_none() || self.resulting_content_digest.is_some()
                {
                    return Err(HarnessError::InvalidPlan(
                        "delete finalization requires a current digest and absent result"
                            .to_owned(),
                    ));
                }
            }
        }
        if let Some(digest) = &self.expected_current_digest {
            validate_digest("expected current digest", digest)?;
        }
        if let Some(digest) = &self.resulting_content_digest {
            validate_digest("resulting content digest", digest)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FinalWorkspaceEvidence {
    pub version: u32,
    pub workspace_locator_map_digest: String,
    pub content_states: BTreeMap<String, Option<String>>,
}

impl FinalWorkspaceEvidence {
    pub fn validate(&self) -> HarnessResult<()> {
        require_version(
            "final workspace evidence schema",
            self.version,
            HARNESS_SCHEMA_VERSION,
        )?;
        validate_digest(
            "final workspace locator-map digest",
            &self.workspace_locator_map_digest,
        )?;
        for (path, digest) in &self.content_states {
            validate_relative_path(path)?;
            if let Some(digest) = digest {
                validate_digest("final workspace content digest", digest)?;
            }
        }
        Ok(())
    }

    pub fn read_from_workspace(
        workspace_root: &Path,
        frozen: &FrozenTargets,
    ) -> HarnessResult<Self> {
        frozen.validate()?;
        let root_identity = super::requirements::workspace_root_identity(workspace_root)?;
        if frozen
            .targets
            .iter()
            .any(|target| target.locator.root_identity != root_identity)
        {
            return Err(HarnessError::InvalidSubmission(
                "final workspace locators belong to another root".to_owned(),
            ));
        }
        Self::read_paths(
            workspace_root,
            frozen
                .targets
                .iter()
                .map(|target| target.workspace_relative_path.as_str()),
            frozen.workspace_locator_map_digest.clone(),
        )
    }

    pub fn read_paths<'a>(
        workspace_root: &Path,
        paths: impl IntoIterator<Item = &'a str>,
        workspace_locator_map_digest: String,
    ) -> HarnessResult<Self> {
        let canonical_root =
            workspace_root
                .canonicalize()
                .map_err(|source| HarnessError::FileRead {
                    path: workspace_root.to_path_buf(),
                    message: source.to_string(),
                })?;
        let content_states = paths
            .into_iter()
            .map(|relative_path| {
                validate_relative_path(relative_path)?;
                let digest =
                    super::repository::current_target_digest(&canonical_root, relative_path)?;
                Ok((relative_path.to_owned(), digest))
            })
            .collect::<HarnessResult<BTreeMap<_, _>>>()?;
        let evidence = Self {
            version: HARNESS_SCHEMA_VERSION,
            workspace_locator_map_digest,
            content_states,
        };
        evidence.validate()?;
        Ok(evidence)
    }
}

pub fn validate_finalization(
    targets: &[FinalizationTarget],
    actual_workspace: &BTreeMap<String, Option<String>>,
) -> HarnessResult<()> {
    if targets.is_empty() || actual_workspace.len() != targets.len() {
        return Err(HarnessError::InvalidSubmission(
            "finalization workspace evidence must cover exactly every target".to_owned(),
        ));
    }
    let mut paths = BTreeSet::new();
    for target in targets {
        target.validate()?;
        if !paths.insert(target.workspace_relative_path.as_str()) {
            return Err(HarnessError::InvalidPlan(
                "finalization targets must be unique".to_owned(),
            ));
        }
        let actual = actual_workspace
            .get(&target.workspace_relative_path)
            .ok_or_else(|| {
                HarnessError::InvalidSubmission(format!(
                    "missing final workspace evidence for `{}`",
                    target.workspace_relative_path
                ))
            })?;
        if actual != &target.resulting_content_digest {
            return Err(HarnessError::InvalidSubmission(format!(
                "final workspace state for `{}` does not match the exact result",
                target.workspace_relative_path
            )));
        }
    }
    Ok(())
}

pub fn validate_finalization_evidence(
    expected_workspace_locator_map_digest: &str,
    targets: &[FinalizationTarget],
    actual_workspace: &FinalWorkspaceEvidence,
) -> HarnessResult<()> {
    validate_digest(
        "expected workspace locator-map digest",
        expected_workspace_locator_map_digest,
    )?;
    actual_workspace.validate()?;
    if actual_workspace.workspace_locator_map_digest != expected_workspace_locator_map_digest {
        return Err(HarnessError::PlanDrift {
            expected: expected_workspace_locator_map_digest.to_owned(),
            actual: actual_workspace.workspace_locator_map_digest.clone(),
        });
    }
    validate_finalization(targets, &actual_workspace.content_states)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessApplyReceipt {
    pub version: u32,
    /// Digest of the complete serialized `HarnessPlan` artifact.
    pub harness_plan_digest: String,
    pub prepared_run_digest: String,
    pub candidate_digest: String,
    pub validation_receipt_digest: String,
    pub finalization_digest: String,
    pub batch_receipt_digest: String,
    pub batch_journal_relative_path: String,
    pub recovered: bool,
    pub frozen_identity_set_digest: String,
    pub workspace_locator_map_digest: String,
    pub applied_batch: AppliedHarnessBatch,
    pub finalization_targets: Vec<FinalizationTarget>,
    pub final_workspace: FinalWorkspaceEvidence,
    pub context_commit_receipt: Option<super::ContextCommitReceipt>,
    pub apply_receipt_digest: String,
}

impl HarnessApplyReceipt {
    pub fn validate(&self) -> HarnessResult<()> {
        require_version(
            "current apply receipt contract",
            self.version,
            HARNESS_SCHEMA_VERSION,
        )?;
        for (label, digest) in [
            ("current Harness plan digest", &self.harness_plan_digest),
            ("current prepared-run digest", &self.prepared_run_digest),
            ("current candidate digest", &self.candidate_digest),
            (
                "current validation receipt digest",
                &self.validation_receipt_digest,
            ),
            ("current finalization digest", &self.finalization_digest),
            (
                "current Harness engine apply receipt digest",
                &self.batch_receipt_digest,
            ),
            (
                "current frozen target identity-set digest",
                &self.frozen_identity_set_digest,
            ),
            (
                "current workspace locator-map digest",
                &self.workspace_locator_map_digest,
            ),
        ] {
            validate_digest(label, digest)?;
        }
        validate_relative_path(&self.batch_journal_relative_path)?;
        if serialized_digest(&self.applied_batch)? != self.batch_receipt_digest
            || self.applied_batch.lifecycle_receipt.journal_relative_path
                != self.batch_journal_relative_path
            || self.applied_batch.lifecycle_receipt.outcome != BatchApplyOutcomeReceipt::Applied
            || self.applied_batch.lifecycle_receipt.journal_retained
            || !self
                .applied_batch
                .lifecycle_receipt
                .orchestration_failures
                .is_empty()
        {
            return Err(HarnessError::InvalidPlan(
                "current apply receipt does not embed one fully applied Harness engine batch"
                    .to_owned(),
            ));
        }
        validate_finalization_evidence(
            &self.workspace_locator_map_digest,
            &self.finalization_targets,
            &self.final_workspace,
        )?;
        if let Some(receipt) = &self.context_commit_receipt {
            receipt.validate()?;
            if receipt.observed_batch_digest() != self.batch_receipt_digest {
                return Err(HarnessError::InvalidSubmission(
                    "context receipt does not bind the observed common batch".to_owned(),
                ));
            }
        }
        let finalization_digest = serialized_digest(&(
            &self.finalization_targets,
            &self.final_workspace,
            &self.context_commit_receipt,
            &self.applied_batch,
            self.recovered,
            &self.frozen_identity_set_digest,
            &self.workspace_locator_map_digest,
        ))?;
        if finalization_digest != self.finalization_digest {
            return Err(HarnessError::InvalidPlan(
                "current finalization digest does not match its embedded evidence".to_owned(),
            ));
        }
        let digest = serialized_digest(&(
            self.version,
            &self.harness_plan_digest,
            &self.prepared_run_digest,
            &self.candidate_digest,
            &self.validation_receipt_digest,
            &self.finalization_digest,
            &self.batch_receipt_digest,
            &self.batch_journal_relative_path,
            self.recovered,
            &self.frozen_identity_set_digest,
            &self.workspace_locator_map_digest,
            &self.applied_batch,
            &self.finalization_targets,
            &self.final_workspace,
            &self.context_commit_receipt,
        ))?;
        if digest != self.apply_receipt_digest {
            return Err(HarnessError::InvalidPlan(
                "current apply receipt digest does not match its cross-digests".to_owned(),
            ));
        }
        validate_serialized_size("current apply receipt", self, MAX_APPLY_RECEIPT_BYTES)?;
        Ok(())
    }

    #[allow(
        clippy::too_many_lines,
        reason = "current apply revalidation keeps Harness engine, final workspace and derived-index evidence in one auditable boundary"
    )]
    pub fn revalidate_from_disk_and_core(
        &self,
        engine: &HarnessEngine,
        raw_prepared_run: &[u8],
        prepared: &PreparedHarnessRun,
        record: &HarnessExecutionRecord,
    ) -> HarnessResult<()> {
        self.validate()?;
        prepared.validate_recovery_with_engine(raw_prepared_run, engine)?;
        record.validate(prepared, raw_prepared_run)?;
        if record.state != HarnessExecutionState::Finalized
            || self.harness_plan_digest != prepared.plan_identity.normalized_value_digest
            || self.prepared_run_digest != prepared.prepared_run_digest
            || self.frozen_identity_set_digest != prepared.plan.frozen_targets.identity_set_digest
            || self.workspace_locator_map_digest
                != prepared.plan.frozen_targets.workspace_locator_map_digest
            || record.candidate_digest.as_ref() != Some(&self.candidate_digest)
            || record.validation_receipt_digest.as_ref() != Some(&self.validation_receipt_digest)
            || record.finalization_digest.as_ref() != Some(&self.finalization_digest)
            || record.batch_receipt_digest.as_ref() != Some(&self.batch_receipt_digest)
        {
            return Err(HarnessError::InvalidSubmission(
                "current apply receipt is not bound to the exact prepared run and finalized record"
                    .to_owned(),
            ));
        }
        let evaluation = record.evaluation.as_ref().ok_or_else(|| {
            HarnessError::InvalidSubmission(
                "finalized current record is missing its Harness engine evaluation".to_owned(),
            )
        })?;
        let reproduced = engine.inspect_validated_execution_apply(
            &prepared.plan.resolved_request,
            &prepared.role_run,
            &record.revision_history,
            &record.role_execution,
            &self.batch_journal_relative_path,
        )?;
        if reproduced != self.applied_batch {
            return Err(HarnessError::InvalidSubmission(
                "current embedded apply evidence does not reproduce from the durable Harness engine journal"
                    .to_owned(),
            ));
        }
        validate_applied_batch(evaluation, &self.applied_batch)?;
        engine.with_workspace_finalization_lock(|| {
            let expected_targets = FinalizationTarget::from_recovered_batch(
                &prepared.plan.frozen_targets,
                &self.applied_batch,
            )?;
            if expected_targets != self.finalization_targets {
                return Err(HarnessError::InvalidSubmission(
                    "current finalization targets do not reproduce from the Harness engine apply receipt".to_owned(),
                ));
            }
            let actual_workspace = FinalWorkspaceEvidence::read_from_workspace(
                &engine.workspace_root,
                &prepared.plan.frozen_targets,
            )?;
            if actual_workspace != self.final_workspace {
                return Err(HarnessError::InvalidSubmission(
                    "current final workspace evidence no longer matches disk".to_owned(),
                ));
            }
            validate_finalization_evidence(
                &prepared.plan.frozen_targets.workspace_locator_map_digest,
                &expected_targets,
                &actual_workspace,
                )
        })
    }
}

fn validate_applied_batch(
    evaluation: &HarnessTaskEvaluation,
    applied: &AppliedHarnessBatch,
) -> HarnessResult<()> {
    let candidate = evaluation.candidate_digest.as_deref().ok_or_else(|| {
        HarnessError::InvalidSubmission("accepted evaluation has no write candidate".to_owned())
    })?;
    if applied.resolved_plan_digest != evaluation.resolved_plan_digest
        || applied.candidate_digest != candidate
        || applied.task_evaluation_receipt_digest != evaluation.task_evaluation_receipt_digest
        || applied.lifecycle_receipt.resolved_plan_digest != evaluation.resolved_plan_digest
        || applied.lifecycle_receipt.candidate_digest != candidate
        || applied.lifecycle_receipt.outcome != BatchApplyOutcomeReceipt::Applied
        || applied.lifecycle_receipt.journal_retained
        || !applied.lifecycle_receipt.orchestration_failures.is_empty()
        || !matches!(applied.completion_state, HarnessCompletionState::Applied)
    {
        return Err(HarnessError::InvalidSubmission(
            "applied batch is not the exact fully-confirmed evaluation result".to_owned(),
        ));
    }
    let mut changes = evaluated_execution_changes(evaluation)?;
    changes.sort_by(|left, right| left.path().cmp(right.path()));
    let mut targets = applied.targets.clone();
    targets.sort_by(|left, right| {
        left.workspace_relative_path
            .cmp(&right.workspace_relative_path)
    });
    if changes.len() != targets.len() {
        return Err(HarnessError::InvalidSubmission(
            "applied target count differs from the evaluated changes".to_owned(),
        ));
    }
    for (change, target) in changes.iter().zip(&targets) {
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
        if change.path() != target.workspace_relative_path
            || target.state != BatchTargetApplyStateReceipt::Applied
            || target.operation != operation
            || target.original_content_digest != original
            || target.intended_content_digest != intended
        {
            return Err(HarnessError::InvalidSubmission(
                "applied targets differ from the evaluated change set".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> HarnessResult<()> {
    if path.is_empty()
        || path.starts_with('/')
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(HarnessError::InvalidPlan(
            "path must be a normalized workspace-relative path".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_finalization_keeps_exact_common_create_update_delete_evidence() {
        let targets = vec![
            FinalizationTarget {
                version: HARNESS_SCHEMA_VERSION,
                workspace_relative_path: "vault/profile/a.md".into(),
                operation: FinalizationOperation::Create,
                expected_current_digest: None,
                resulting_content_digest: Some("a".repeat(64)),
            },
            FinalizationTarget {
                version: HARNESS_SCHEMA_VERSION,
                workspace_relative_path: "vault/profile/b.md".into(),
                operation: FinalizationOperation::Delete,
                expected_current_digest: Some("b".repeat(64)),
                resulting_content_digest: None,
            },
            FinalizationTarget {
                version: HARNESS_SCHEMA_VERSION,
                workspace_relative_path: "src/lib.rs".into(),
                operation: FinalizationOperation::Update,
                expected_current_digest: Some("d".repeat(64)),
                resulting_content_digest: Some("c".repeat(64)),
            },
        ];
        let actual = targets
            .iter()
            .map(|t| {
                (
                    t.workspace_relative_path.clone(),
                    t.resulting_content_digest.clone(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        validate_finalization(&targets, &actual).unwrap();
        let mut wrong = actual.clone();
        wrong.remove("vault/profile/b.md");
        assert!(validate_finalization(&targets, &wrong).is_err());
        let mut wrong = actual;
        wrong.insert("vault/profile/b.md".into(), Some("b".repeat(64)));
        assert!(validate_finalization(&targets, &wrong).is_err());
    }
}
