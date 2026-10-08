//! Typed persistence effects. Only validated durable Core state can construct an effect contract.
use super::*;

fn invalid(message: &str) -> HarnessError {
    HarnessError::InvalidSubmission(message.to_owned())
}

/// One canonical context target, selected by the Core's frozen root and source bindings.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContextTarget {
    previous: SourceVersion,
    operation: FinalizationOperation,
    resulting_content_digest: Option<String>,
}
impl ContextTarget {
    pub fn previous(&self) -> &SourceVersion {
        &self.previous
    }
    pub fn operation(&self) -> FinalizationOperation {
        self.operation
    }
    pub fn resulting_content_digest(&self) -> Option<&str> {
        self.resulting_content_digest.as_deref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContextApplyContract {
    version: u32,
    attempt: HarnessApplyAttemptReceipt,
    store_identity: Option<SourceStoreIdentity>,
    source_root_identity: Option<String>,
    workspace_root_identity: String,
    targets: Vec<ContextTarget>,
    non_target_versions: Vec<SourceVersion>,
    contract_digest: String,
}
impl ContextApplyContract {
    pub fn version(&self) -> u32 {
        self.version
    }
    pub fn attempt(&self) -> &HarnessApplyAttemptReceipt {
        &self.attempt
    }
    pub fn store_identity(&self) -> Option<&SourceStoreIdentity> {
        self.store_identity.as_ref()
    }
    pub fn source_root_identity(&self) -> Option<&str> {
        self.source_root_identity.as_deref()
    }
    pub fn workspace_root_identity(&self) -> &str {
        &self.workspace_root_identity
    }
    pub fn targets(&self) -> &[ContextTarget] {
        &self.targets
    }
    pub fn non_target_versions(&self) -> &[SourceVersion] {
        &self.non_target_versions
    }
    pub fn contract_digest(&self) -> &str {
        &self.contract_digest
    }
    pub fn requires_commit(&self) -> bool {
        !self.targets.is_empty()
    }

    pub(super) fn from_durable(
        prepared: &PreparedHarnessRun,
        head: &HarnessExecutionRecord,
        attempt: &HarnessApplyAttemptReceipt,
    ) -> HarnessResult<Self> {
        attempt.validate(prepared)?;
        execution::validate_durable_apply_lineage(head, attempt)?;
        let resolved = &prepared.plan.resolved_request.plan.source_versions;
        let versions = &prepared.role_run.source_versions;
        versions.require_superset(resolved)?;
        let evaluation = head
            .evaluation
            .as_ref()
            .ok_or_else(|| invalid("context effect requires the actual accepted evaluation"))?;
        if head.candidate_digest.as_ref() != Some(&attempt.candidate_digest)
            || head.validation_receipt_digest.as_ref() != Some(&attempt.validation_receipt_digest)
        {
            return Err(invalid(
                "context effect differs from the durable accepted candidate",
            ));
        }
        let mut changes = evaluated_execution_changes(evaluation)?;
        changes.sort_by(|a, b| a.path().cmp(b.path()));
        let native_paths = native_target_paths(prepared)?;
        let mut targets = Vec::new();
        for change in &changes {
            if !native_paths.contains(change.path()) {
                continue;
            }
            let previous = resolved
                .versions
                .iter()
                .find(|v| v.logical_path == change.path())
                .ok_or_else(|| invalid("native target has no resolved stored-state binding"))?
                .clone();
            let (operation, original, resulting_content_digest) = match change {
                FileChange::Create { content, .. } => (
                    FinalizationOperation::Create,
                    None,
                    Some(byte_digest(content.as_bytes())),
                ),
                FileChange::Update {
                    expected_content_digest,
                    content,
                    ..
                } => (
                    FinalizationOperation::Update,
                    Some(expected_content_digest.as_str()),
                    Some(byte_digest(content.as_bytes())),
                ),
                FileChange::Delete {
                    expected_content_digest,
                    ..
                } => (
                    FinalizationOperation::Delete,
                    Some(expected_content_digest.as_str()),
                    None,
                ),
            };
            let matches = match (&previous.state, operation, original) {
                (
                    StoredSourceState::Missing | StoredSourceState::Deleted { .. },
                    FinalizationOperation::Create,
                    None,
                ) => true,
                (
                    StoredSourceState::Live { content_digest, .. },
                    FinalizationOperation::Update | FinalizationOperation::Delete,
                    Some(expected),
                ) => content_digest == expected,
                _ => false,
            };
            if !matches {
                return Err(invalid(
                    "native target original differs from its accepted operation",
                ));
            }
            targets.push(ContextTarget {
                previous,
                operation,
                resulting_content_digest,
            });
        }
        if targets.len() != native_paths.len() {
            return Err(invalid(
                "native target set is not exactly the accepted candidate",
            ));
        }
        let mut frozen_attempt = attempt.clone();
        frozen_attempt = frozen_attempt.replace_state(HarnessApplyAttemptState::PendingApply)?;
        let mut value = Self {
            version: HARNESS_SCHEMA_VERSION,
            attempt: frozen_attempt,
            store_identity: resolved.store_identity.clone(),
            source_root_identity: resolved.source_root_identity.clone(),
            workspace_root_identity: prepared.plan.workspace_root_identity.clone(),
            targets,
            non_target_versions: versions
                .versions
                .iter()
                .filter(|v| !native_paths.contains(&v.logical_path))
                .cloned()
                .collect(),
            contract_digest: String::new(),
        };
        value.contract_digest = serialized_digest(&value)?;
        Ok(value)
    }

    pub(super) fn check_session(
        &self,
        session: Option<&dyn ContextCommitSession>,
    ) -> HarnessResult<()> {
        match (&self.store_identity, session) {
            (Some(expected), Some(session)) if expected == session.store_identity() => Ok(()),
            (None, None) => Ok(()),
            (Some(_), None) if !self.requires_commit() => Ok(()),
            _ => Err(invalid(
                "context effect session is missing or belongs to another store",
            )),
        }
    }
}

pub(super) fn native_target_paths(
    prepared: &PreparedHarnessRun,
) -> HarnessResult<BTreeSet<String>> {
    let resolved = &prepared.plan.resolved_request.plan.source_versions;
    let versions = &prepared.role_run.source_versions;
    versions.require_superset(resolved)?;
    let mut paths = BTreeSet::new();
    if resolved.source_root_identity.as_ref() == Some(&prepared.plan.workspace_root_identity) {
        for target in &prepared.plan.frozen_targets.targets {
            if target.workspace_relative_path.starts_with("vault/") {
                let version = resolved
                    .versions
                    .iter()
                    .find(|v| v.logical_path == target.workspace_relative_path)
                    .ok_or_else(|| {
                        invalid("frozen native target is missing its resolved stored-state binding")
                    })?;
                version.validate()?;
                paths.insert(target.workspace_relative_path.clone());
            }
        }
    }
    Ok(paths)
}

/// Durable recovery links both the actual head and the actual current attempt to the immutable effect.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContextRecoveryContract {
    apply: ContextApplyContract,
    actual_head: HarnessExecutionRecord,
    actual_attempt: HarnessApplyAttemptReceipt,
    contract_digest: String,
}
impl ContextRecoveryContract {
    pub fn apply(&self) -> &ContextApplyContract {
        &self.apply
    }
    pub fn actual_head(&self) -> &HarnessExecutionRecord {
        &self.actual_head
    }
    pub fn actual_attempt(&self) -> &HarnessApplyAttemptReceipt {
        &self.actual_attempt
    }
    pub fn contract_digest(&self) -> &str {
        &self.contract_digest
    }
    pub fn stored_receipt(&self) -> Option<&ContextCommitReceipt> {
        match &self.actual_attempt.attempt_state {
            HarnessApplyAttemptState::AppliedFinalized { apply_receipt, .. }
            | HarnessApplyAttemptState::RecoveredFinalized { apply_receipt, .. } => {
                apply_receipt.context_commit_receipt.as_ref()
            }
            _ => None,
        }
    }
    pub(super) fn from_durable(
        prepared: &PreparedHarnessRun,
        head: &HarnessExecutionRecord,
        attempt: &HarnessApplyAttemptReceipt,
    ) -> HarnessResult<Self> {
        let mut value = Self {
            apply: ContextApplyContract::from_durable(prepared, head, attempt)?,
            actual_head: head.clone(),
            actual_attempt: attempt.clone(),
            contract_digest: String::new(),
        };
        value.contract_digest = serialized_digest(&value)?;
        Ok(value)
    }
    pub(super) fn validate_status(&self, status: &ContextRecoveryStatus) -> HarnessResult<()> {
        if !self.apply.requires_commit() {
            return if matches!(status, ContextRecoveryStatus::NotRequired) {
                Ok(())
            } else {
                Err(invalid("empty native target set forbids persistence state"))
            };
        }
        if matches!(status, ContextRecoveryStatus::Finalized(_)) && self.stored_receipt().is_none()
        {
            return Err(invalid(
                "finalized context state requires the persisted terminal Core receipt",
            ));
        }
        match status {
            ContextRecoveryStatus::NotRequired => {
                Err(invalid("native recovery requires actual persistence state"))
            }
            ContextRecoveryStatus::Committed(receipt)
            | ContextRecoveryStatus::Finalized(receipt) => {
                receipt.validate_for(&self.apply)?;
                if let Some(stored) = self.stored_receipt()
                    && stored != receipt
                {
                    return Err(invalid(
                        "persisted context receipt differs from the terminal Core receipt",
                    ));
                }
                if matches!(
                    self.actual_attempt.attempt_state,
                    HarnessApplyAttemptState::AbortedBeforeMutation { .. }
                        | HarnessApplyAttemptState::AbortedAfterRollback { .. }
                ) {
                    return Err(invalid(
                        "committed context cannot belong to an aborted Core attempt",
                    ));
                }
                Ok(())
            }
            _ if self.stored_receipt().is_some() => Err(invalid(
                "terminal native Core receipt has no matching committed database evidence",
            )),
            _ => Ok(()),
        }
    }
}

/// Borrowed accepted bytes and verified common evidence. A provider cannot construct this proof.
pub struct VerifiedContextCommit<'a> {
    contract: &'a ContextApplyContract,
    applied: &'a AppliedHarnessBatch,
    workspace: &'a FinalWorkspaceEvidence,
    changes: &'a [FileChange],
}
impl<'a> VerifiedContextCommit<'a> {
    pub fn contract(&self) -> &ContextApplyContract {
        self.contract
    }
    pub fn applied_batch(&self) -> &AppliedHarnessBatch {
        self.applied
    }
    pub fn workspace(&self) -> &FinalWorkspaceEvidence {
        self.workspace
    }
    pub fn target_content(&self, path: &str) -> Option<&[u8]> {
        if !self
            .contract
            .targets
            .iter()
            .any(|t| t.previous.logical_path == path)
        {
            return None;
        }
        self.changes
            .iter()
            .find(|c| c.path() == path)
            .and_then(|c| match c {
                FileChange::Create { content, .. } | FileChange::Update { content, .. } => {
                    Some(content.as_bytes())
                }
                FileChange::Delete { .. } => None,
            })
    }
    pub(super) fn new(
        contract: &'a ContextApplyContract,
        applied: &'a AppliedHarnessBatch,
        workspace: &'a FinalWorkspaceEvidence,
        targets: &[FinalizationTarget],
        changes: &'a [FileChange],
    ) -> HarnessResult<Self> {
        validate_finalization_evidence(
            &contract.attempt.workspace_locator_map_digest,
            targets,
            workspace,
        )?;
        if applied.batch_id != contract.attempt.expected_batch_identifier
            || applied.lifecycle_receipt.journal_relative_path
                != contract.attempt.expected_journal_relative_path
            || applied
                .lifecycle_receipt
                .completion_receipt_relative_path
                .as_ref()
                != Some(&contract.attempt.expected_completion_relative_path)
            || applied.candidate_digest != contract.attempt.candidate_digest
            || applied.resolved_plan_digest != contract.attempt.resolved_plan_digest
        {
            return Err(invalid(
                "verified context commit differs from the actual durable attempt batch",
            ));
        }
        for native in &contract.targets {
            let target = targets
                .iter()
                .find(|t| t.workspace_relative_path == native.previous.logical_path)
                .ok_or_else(|| invalid("native finalization target is absent"))?;
            if target.operation != native.operation
                || target.resulting_content_digest != native.resulting_content_digest
            {
                return Err(invalid(
                    "native finalization target differs from accepted candidate",
                ));
            }
            let change = changes
                .iter()
                .find(|c| c.path() == native.previous.logical_path)
                .ok_or_else(|| invalid("native accepted bytes are absent"))?;
            let content = match change {
                FileChange::Create { content, .. } | FileChange::Update { content, .. } => {
                    Some(byte_digest(content.as_bytes()))
                }
                FileChange::Delete { .. } => None,
            };
            if content != native.resulting_content_digest {
                return Err(invalid(
                    "native accepted bytes do not match verified final state",
                ));
            }
        }
        Ok(Self {
            contract,
            applied,
            workspace,
            changes,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextCommitReceipt {
    version: u32,
    store_identity: SourceStoreIdentity,
    contract_digest: String,
    attempt_identifier: String,
    batch_identifier: String,
    journal_relative_path: String,
    observed_batch_digest: String,
    previous: Vec<SourceVersion>,
    resulting: Vec<SourceVersion>,
    projection_metadata_digest: String,
    receipt_digest: String,
}
impl ContextCommitReceipt {
    /// Validate actual transaction rows against the Core-issued effect.
    /// Providers may construct this tentative receipt inside the same transaction that writes
    /// content, history, and projection, and persist the exact receipt before committing.
    /// The session's `commit` callback may return it successfully only after DB COMMIT succeeds.
    pub fn from_persisted(
        verified: &VerifiedContextCommit<'_>,
        previous: Vec<SourceVersion>,
        resulting: Vec<SourceVersion>,
        projection_metadata_digest: String,
    ) -> HarnessResult<Self> {
        let contract = verified.contract();
        let mut value = Self {
            version: HARNESS_SCHEMA_VERSION,
            store_identity: contract
                .store_identity
                .clone()
                .ok_or_else(|| invalid("context commit needs a store"))?,
            contract_digest: contract.contract_digest.clone(),
            attempt_identifier: contract.attempt.attempt_identifier.clone(),
            batch_identifier: contract.attempt.expected_batch_identifier.clone(),
            journal_relative_path: contract.attempt.expected_journal_relative_path.clone(),
            observed_batch_digest: serialized_digest(verified.applied_batch())?,
            previous,
            resulting,
            projection_metadata_digest,
            receipt_digest: String::new(),
        };
        value.receipt_digest = value.calculate_digest()?;
        value.validate_for(contract)?;
        Ok(value)
    }
    pub fn store_identity(&self) -> &SourceStoreIdentity {
        &self.store_identity
    }
    pub fn contract_digest(&self) -> &str {
        &self.contract_digest
    }
    pub fn attempt_identifier(&self) -> &str {
        &self.attempt_identifier
    }
    pub fn batch_identifier(&self) -> &str {
        &self.batch_identifier
    }
    pub fn journal_relative_path(&self) -> &str {
        &self.journal_relative_path
    }
    pub fn observed_batch_digest(&self) -> &str {
        &self.observed_batch_digest
    }
    pub fn previous(&self) -> &[SourceVersion] {
        &self.previous
    }
    pub fn resulting(&self) -> &[SourceVersion] {
        &self.resulting
    }
    pub fn projection_metadata_digest(&self) -> &str {
        &self.projection_metadata_digest
    }
    pub fn receipt_digest(&self) -> &str {
        &self.receipt_digest
    }
    fn calculate_digest(&self) -> HarnessResult<String> {
        serialized_digest(&(
            self.version,
            &self.store_identity,
            &self.contract_digest,
            &self.attempt_identifier,
            &self.batch_identifier,
            &self.journal_relative_path,
            &self.observed_batch_digest,
            &self.previous,
            &self.resulting,
            &self.projection_metadata_digest,
        ))
    }
    pub fn validate(&self) -> HarnessResult<()> {
        require_version(
            "context commit receipt",
            self.version,
            HARNESS_SCHEMA_VERSION,
        )?;
        self.store_identity.validate()?;
        for digest in [
            &self.contract_digest,
            &self.projection_metadata_digest,
            &self.observed_batch_digest,
            &self.receipt_digest,
        ] {
            validate_digest("context receipt digest", digest)?;
        }
        if self.receipt_digest != self.calculate_digest()?
            || self.previous.is_empty()
            || self.previous.len() != self.resulting.len()
        {
            return Err(invalid(
                "context receipt is incomplete or has an invalid digest",
            ));
        }
        for versions in [&self.previous, &self.resulting] {
            let mut last = None;
            let mut ids = BTreeSet::new();
            for version in versions {
                version.validate()?;
                if last.is_some_and(|p: &str| p >= version.logical_path.as_str()) {
                    return Err(invalid("context receipt paths must be sorted and unique"));
                }
                last = Some(version.logical_path.as_str());
                if let StoredSourceState::Live { material_id, .. }
                | StoredSourceState::Deleted { material_id, .. } = &version.state
                    && !ids.insert(material_id)
                {
                    return Err(invalid("context receipt duplicates a material identity"));
                }
            }
        }
        Ok(())
    }
    pub fn validate_for(&self, contract: &ContextApplyContract) -> HarnessResult<()> {
        self.validate()?;
        if !contract.requires_commit()
            || Some(&self.store_identity) != contract.store_identity.as_ref()
            || self.contract_digest != contract.contract_digest
            || self.attempt_identifier != contract.attempt.attempt_identifier
            || self.batch_identifier != contract.attempt.expected_batch_identifier
            || self.journal_relative_path != contract.attempt.expected_journal_relative_path
            || self.previous.len() != contract.targets.len()
        {
            return Err(invalid(
                "context receipt does not match the exact native effect contract",
            ));
        }
        for ((target, previous), resulting) in contract
            .targets
            .iter()
            .zip(&self.previous)
            .zip(&self.resulting)
        {
            if previous != &target.previous || resulting.logical_path != previous.logical_path {
                return Err(invalid(
                    "context receipt target states differ from the frozen native set",
                ));
            }
            let (new_id, new_revision, digest, deleted) = match &resulting.state {
                StoredSourceState::Live {
                    material_id,
                    revision,
                    content_digest,
                } => (material_id, *revision, Some(content_digest.as_str()), false),
                StoredSourceState::Deleted {
                    material_id,
                    revision,
                } => (material_id, *revision, None, true),
                StoredSourceState::Missing => {
                    return Err(invalid(
                        "a committed context target cannot lose its material identity",
                    ));
                }
            };
            if contract
                .non_target_versions
                .iter()
                .any(|version| match &version.state {
                    StoredSourceState::Live { material_id, .. }
                    | StoredSourceState::Deleted { material_id, .. } => material_id == new_id,
                    StoredSourceState::Missing => false,
                })
            {
                return Err(invalid(
                    "context receipt result aliases a non-target material identity",
                ));
            }
            match &previous.state {
                StoredSourceState::Missing if new_revision == 1 && !deleted => {}
                StoredSourceState::Live {
                    material_id,
                    revision,
                    ..
                }
                | StoredSourceState::Deleted {
                    material_id,
                    revision,
                } if material_id == new_id && revision.checked_add(1) == Some(new_revision) => {}
                _ => {
                    return Err(invalid(
                        "context receipt does not preserve material identity and exact next revision",
                    ));
                }
            }
            if deleted != (target.operation == FinalizationOperation::Delete)
                || digest != target.resulting_content_digest.as_deref()
            {
                return Err(invalid(
                    "context receipt result differs from the accepted target operation and bytes",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContextRecoveryStatus {
    NotRequired,
    Absent,
    Pending,
    Committed(ContextCommitReceipt),
    Finalized(ContextCommitReceipt),
    Aborted,
}
impl ContextRecoveryStatus {
    pub(super) fn committed_receipt(&self) -> Option<&ContextCommitReceipt> {
        match self {
            Self::Committed(receipt) | Self::Finalized(receipt) => Some(receipt),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContextTerminalProof {
    contract: ContextApplyContract,
    persisted_attempt: HarnessApplyAttemptReceipt,
    finalized_head: HarnessExecutionRecord,
}
impl ContextTerminalProof {
    pub fn contract(&self) -> &ContextApplyContract {
        &self.contract
    }
    pub fn persisted_attempt(&self) -> &HarnessApplyAttemptReceipt {
        &self.persisted_attempt
    }
    pub fn finalized_head(&self) -> &HarnessExecutionRecord {
        &self.finalized_head
    }
    pub(super) fn new(
        contract: ContextApplyContract,
        persisted_attempt: HarnessApplyAttemptReceipt,
        finalized_head: HarnessExecutionRecord,
    ) -> HarnessResult<Self> {
        let resulting = match &persisted_attempt.attempt_state {
            HarnessApplyAttemptState::AppliedFinalized {
                resulting_record, ..
            }
            | HarnessApplyAttemptState::RecoveredFinalized {
                resulting_record, ..
            } => resulting_record,
            _ => {
                return Err(invalid(
                    "context terminal cleanup needs a persisted terminal attempt",
                ));
            }
        };
        if resulting.as_ref() != &finalized_head
            || finalized_head.state != HarnessExecutionState::Finalized
        {
            return Err(invalid(
                "context terminal cleanup needs the read-back finalized head",
            ));
        }
        Ok(Self {
            contract,
            persisted_attempt,
            finalized_head,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContextAbortProof {
    contract: ContextApplyContract,
    persisted_attempt: HarnessApplyAttemptReceipt,
}
impl ContextAbortProof {
    pub fn contract(&self) -> &ContextApplyContract {
        &self.contract
    }
    pub fn persisted_attempt(&self) -> &HarnessApplyAttemptReceipt {
        &self.persisted_attempt
    }
    pub(super) fn new(
        contract: ContextApplyContract,
        persisted_attempt: HarnessApplyAttemptReceipt,
        status: &ContextRecoveryStatus,
    ) -> HarnessResult<Self> {
        if status.committed_receipt().is_some()
            || !matches!(
                persisted_attempt.attempt_state,
                HarnessApplyAttemptState::AbortedBeforeMutation { .. }
                    | HarnessApplyAttemptState::AbortedAfterRollback { .. }
            )
        {
            return Err(invalid(
                "context abort requires durable rollback/no-mutation proof and forbids committed effects",
            ));
        }
        Ok(Self {
            contract,
            persisted_attempt,
        })
    }
}

/// The command owns the locked storage session for the entire apply/recovery call.
/// Source-free identity selected from a validated, unapplied durable native run.
/// Callers cannot construct this contract or turn it into an apply contract.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContextPreApplyCloseContract {
    store_identity: SourceStoreIdentity,
    source_root_identity: String,
    run_identifier: String,
    prepared_run_digest: String,
}
impl ContextPreApplyCloseContract {
    pub fn store_identity(&self) -> &SourceStoreIdentity {
        &self.store_identity
    }
    pub fn run_identifier(&self) -> &str {
        &self.run_identifier
    }
    pub fn prepared_run_digest(&self) -> &str {
        &self.prepared_run_digest
    }
    pub fn check_view(&self, view: &Path) -> HarnessResult<()> {
        if requirements::workspace_root_identity(view)? != self.source_root_identity {
            return Err(invalid("pre-apply closure belongs to another source view"));
        }
        Ok(())
    }
    pub(super) fn from_durable(
        prepared: &PreparedHarnessRun,
        head: &HarnessExecutionRecord,
    ) -> HarnessResult<Self> {
        let versions = &prepared.plan.resolved_request.plan.source_versions;
        versions.validate()?;
        if prepared.plan.external_write_allowed
            || prepared.plan.frozen_targets.targets.is_empty()
            || prepared
                .plan
                .frozen_targets
                .targets
                .iter()
                .any(|target| !target.workspace_relative_path.starts_with("vault/"))
        {
            return Err(invalid("pre-apply closure requires a native-only run"));
        }
        Ok(Self {
            store_identity: versions
                .store_identity
                .clone()
                .ok_or_else(|| invalid("pre-apply closure requires a bound native store"))?,
            source_root_identity: versions
                .source_root_identity
                .clone()
                .ok_or_else(|| invalid("pre-apply closure requires a bound source view"))?,
            run_identifier: head.run_identifier.clone(),
            prepared_run_digest: prepared.prepared_run_digest.clone(),
        })
    }
}

pub trait ContextCommitSession {
    fn store_identity(&self) -> &SourceStoreIdentity;
    /// Under the retained exclusive store gate, reject every correlated effect,
    /// including finalized/aborted rows, and any outstanding native batch.
    fn verify_pre_apply_close(
        &mut self,
        contract: &ContextPreApplyCloseContract,
    ) -> HarnessResult<()>;
    fn begin(&mut self, contract: &ContextApplyContract) -> HarnessResult<()>;
    fn recover(
        &mut self,
        contract: &ContextRecoveryContract,
    ) -> HarnessResult<ContextRecoveryStatus>;
    /// Return the validated receipt successfully only after its database transaction commits.
    fn commit(
        &mut self,
        verified: &VerifiedContextCommit<'_>,
    ) -> HarnessResult<ContextCommitReceipt>;
    fn finalize(&mut self, proof: &ContextTerminalProof) -> HarnessResult<()>;
    fn abort(&mut self, proof: &ContextAbortProof) -> HarnessResult<()>;
}
