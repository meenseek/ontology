//! Reuse accepted semantic reviews to preserve their exact bytes, without inventing role runs.
use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedCareerPreservation {
    pub(super) review_workspace_root: PathBuf,
    pub(super) manifest: CareerCompositionManifest,
    pub(super) holistic: Box<CareerExecutionReviewReceipt>,
    pub(super) owners: Vec<CareerExecutionReviewReceipt>,
    pub(super) composition: CareerExecutionCompositionReceipt,
    pub(super) changes: Vec<FileChange>,
    pub(super) current_source_versions: SourceVersionSet,
}

fn invalid(message: &str) -> HarnessError {
    HarnessError::InvalidSubmission(message.to_owned())
}

impl HarnessEngine {
    /// Preflight the complete consumer before any new role invocation or source effect.
    pub fn prepare_reviewed_career_preservation(
        &self,
        prepared: &PreparedHarnessRun,
        review_workspace: &Path,
        manifest: &CareerCompositionManifest,
        holistic: CareerExecutionReviewReceipt,
        owners: Vec<CareerExecutionReviewReceipt>,
    ) -> HarnessResult<ReviewedCareerPreservation> {
        let review_workspace_root = canonical_directory(review_workspace, "review workspace")?;
        if review_workspace_root == self.workspace_root || self.workspace_root != self.vault.root {
            return Err(invalid(
                "career preservation requires separate review and native workspaces",
            ));
        }
        let review_engine = Self {
            router: self.router.clone(),
            vault: self.vault.clone(),
            workspace_root: review_workspace_root.clone(),
        };
        let composition =
            review_engine.compose_career_execution_reviews(manifest, &holistic, &owners)?;
        let mut changes = Vec::new();
        for target in &composition.artifact_targets {
            let (content, digest) = repository::read_text_file(
                open_verified_file(
                    &review_workspace_root,
                    Path::new(&target.workspace_relative_path),
                )?,
                MAX_TARGET_BYTES,
                &review_workspace_root.join(&target.workspace_relative_path),
            )?;
            let TargetState::Existing { content_digest, .. } = &target.state else {
                return Err(invalid(
                    "reviewed career targets must contain existing UTF-8 bytes",
                ));
            };
            if &digest != content_digest {
                return Err(invalid("reviewed career bytes changed before preservation"));
            }
            let native = prepared
                .plan
                .resolved_request
                .plan
                .targets
                .iter()
                .find(|t| t.workspace_relative_path == target.workspace_relative_path)
                .ok_or_else(|| invalid("reviewed career target is absent from the native plan"))?;
            changes.push(match &native.state {
                TargetState::Absent => FileChange::Create {
                    path: target.workspace_relative_path.clone(),
                    content,
                },
                TargetState::Existing { content_digest, .. } => FileChange::Update {
                    path: target.workspace_relative_path.clone(),
                    expected_content_digest: content_digest.clone(),
                    content,
                },
            });
        }
        let mut proof = ReviewedCareerPreservation {
            review_workspace_root,
            manifest: manifest.normalized(),
            holistic: Box::new(holistic),
            owners,
            composition,
            changes,
            current_source_versions: SourceVersionSet::default(),
        };
        proof.current_source_versions = self.preservation_source_versions(prepared, &proof)?;
        self.revalidate_reviewed_career_preservation(prepared, &proof)?;
        Ok(proof)
    }

    fn preservation_source_versions(
        &self,
        prepared: &PreparedHarnessRun,
        proof: &ReviewedCareerPreservation,
    ) -> HarnessResult<SourceVersionSet> {
        let paths = std::iter::once(&prepared.role_run.source_versions)
            .chain(std::iter::once(&proof.holistic.prepared.source_versions))
            .chain(proof.owners.iter().map(|r| &r.prepared.source_versions))
            .flat_map(|s| &s.versions)
            .map(|v| PathBuf::from(&v.logical_path))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let versions = SourceVersionSet {
            store_identity: self.vault.store_identity.clone(),
            source_root_identity: prepared
                .role_run
                .source_versions
                .source_root_identity
                .clone(),
            versions: self.vault.source.source_versions(&paths)?,
        };
        versions.validate()?;
        Ok(versions)
    }

    pub(super) fn revalidate_reviewed_career_preservation(
        &self,
        prepared: &PreparedHarnessRun,
        proof: &ReviewedCareerPreservation,
    ) -> HarnessResult<()> {
        let native = &prepared.plan.resolved_request;
        self.revalidate_resolved(native)?;
        proof.validate_plan(prepared)?;
        let review_engine = Self {
            router: self.router.clone(),
            vault: self.vault.clone(),
            workspace_root: canonical_directory(&proof.review_workspace_root, "review workspace")?,
        };
        if self.preservation_source_versions(prepared, proof)? != proof.current_source_versions {
            return Err(invalid(
                "preservation source version changed since preparation",
            ));
        }
        let actual = review_engine.compose_career_execution_reviews(
            &proof.manifest,
            &proof.holistic,
            &proof.owners,
        )?;
        if actual != proof.composition {
            return Err(invalid("career preservation composition changed"));
        }
        // Current native reviewer policies must already be covered by the accepted reviews.
        let reviewed = std::iter::once(proof.holistic.as_ref())
            .chain(&proof.owners)
            .flat_map(|r| &r.resolved.plan.role_policy_bindings)
            .filter(|b| b.role == HarnessRole::Reviewer)
            .flat_map(|b| &b.policies)
            .collect::<Vec<_>>();
        for binding in native
            .plan
            .role_policy_bindings
            .iter()
            .filter(|b| b.role == HarnessRole::Reviewer)
        {
            for policy in &binding.policies {
                if !reviewed.contains(&policy) {
                    return Err(invalid(
                        "native preservation has an unreviewed semantic policy",
                    ));
                }
            }
        }
        Ok(())
    }
}

impl ReviewedCareerPreservation {
    pub(super) fn validate_plan(&self, prepared: &PreparedHarnessRun) -> HarnessResult<()> {
        let resolved = &prepared.plan.resolved_request;
        let plan = &resolved.plan;
        if plan.action != HarnessAction::DocumentWrite
            || plan.owner != DataOwner::Personal
            || !plan.intent.is_career_artifact()
            || !resolved.context_grants.is_empty()
            || prepared.accepted_tool_plan.is_some()
            || resolved.career_composition_manifest.as_ref() != Some(&self.manifest)
        {
            return Err(invalid(
                "career preservation requires the exact grantless personal career write plan",
            ));
        }
        if self.holistic.resolved.plan.source_versions.store_identity
            != plan.source_versions.store_identity
            || self
                .holistic
                .resolved
                .plan
                .source_versions
                .source_root_identity
                != plan.source_versions.source_root_identity
        {
            return Err(invalid(
                "career preservation source store or root differs from the reviewed source",
            ));
        }
        let evidence = self
            .owners
            .iter()
            .map(|r| {
                let [grant] = r.resolved.context_grants.as_slice() else {
                    return Err(invalid(
                        "career preservation owner receipt must have one grant",
                    ));
                };
                Ok((grant.owner.clone(), r.clone()))
            })
            .collect::<HarnessResult<Vec<_>>>()?;
        let targets =
            validate_career_execution_receipt_set(&self.manifest, &self.holistic, &evidence)?;
        if targets != self.composition.artifact_targets
            || serialized_digest(&self.manifest)? != self.composition.manifest_digest
            || self.changes.len() != targets.len()
            || plan.targets.len() != targets.len()
        {
            return Err(invalid(
                "career preservation manifest, coverage or target set differs",
            ));
        }
        for receipt in std::iter::once(self.holistic.as_ref()).chain(&self.owners) {
            let evaluation = execution::evaluate_execution_record(
                &receipt.resolved.plan,
                &receipt.prepared,
                &[],
                &receipt.evaluation.execution_record,
            )?;
            execution::require_accepted_execution_evaluation(evaluation.clone())?;
            if evaluation != receipt.evaluation || !evaluation.improvements.is_empty() {
                return Err(invalid(
                    "career preservation requires the actual accepted reviews without unresolved improvements",
                ));
            }
        }
        let paths = self
            .changes
            .iter()
            .map(FileChange::path)
            .collect::<BTreeSet<_>>();
        if paths.len() != targets.len() {
            return Err(invalid("duplicate preservation target"));
        }
        for reviewed in targets {
            let change = self
                .changes
                .iter()
                .find(|c| c.path() == reviewed.workspace_relative_path)
                .ok_or_else(|| invalid("reviewed target bytes are absent"))?;
            let (content, operation, original) = match change {
                FileChange::Create { content, .. } => (content, TargetOperation::Create, None),
                FileChange::Update {
                    content,
                    expected_content_digest,
                    ..
                } => (
                    content,
                    TargetOperation::Update,
                    Some(expected_content_digest),
                ),
                FileChange::Delete { .. } => {
                    return Err(invalid("career preservation cannot delete a target"));
                }
            };
            if !matches!(&reviewed.state, TargetState::Existing { content_digest, .. } if *content_digest == byte_digest(content.as_bytes()))
            {
                return Err(invalid(
                    "career preservation changed the reviewed UTF-8 bytes",
                ));
            }
            let native = plan
                .targets
                .iter()
                .find(|t| t.workspace_relative_path == change.path())
                .ok_or_else(|| invalid("unplanned preservation target"))?;
            if native.operation != operation
                || match (&native.state, original) {
                    (TargetState::Absent, None) => false,
                    (TargetState::Existing { content_digest, .. }, Some(expected)) => {
                        content_digest != expected
                    }
                    _ => true,
                }
            {
                return Err(invalid(
                    "preservation original or operation differs from the native plan",
                ));
            }
        }
        Ok(())
    }

    /// A deterministic acceptance derived from genuine earlier reviews; no new role result.
    pub(super) fn evaluation(
        &self,
        prepared: &PreparedHarnessRun,
        record: &RoleExecutionRecord,
    ) -> HarnessResult<HarnessTaskEvaluation> {
        self.validate_plan(prepared)?;
        let initial = execution::begin_execution_record(
            &prepared.plan.resolved_request.plan,
            &prepared.role_run,
            &[],
        )?
        .record;
        if &initial != record {
            return Err(invalid(
                "preservation execution must be the genuine empty initial role record",
            ));
        }
        if !record.role_results.is_empty() || !record.accepted_role_order.is_empty() {
            return Err(invalid(
                "reviewed preservation must not pretend to execute new roles",
            ));
        }
        let plan = &prepared.plan.resolved_request.plan;
        let candidate = serialized_digest(&(&self.changes, &self.composition.composition_digest))?;
        let requirements = plan.verification_requirements.iter().map(|requirement| {
            if requirement.owner == VerificationOwner::Deterministic {
                return Ok(EvaluatedRequirement { requirement: *requirement, passed: true,
                    detail: "Core verified exact reviewed bytes, native scope and stored-state bindings".into(), evidence: Vec::new() });
            }
            let original = self.holistic.evaluation.requirements.iter()
                .find(|r| r.requirement.unit == requirement.unit && r.passed);
            if let Some(original) = original {
                return Ok(EvaluatedRequirement { requirement: *requirement, passed: true,
                    detail: format!("Reused accepted holistic requirement {} from receipt {}: {}",
                        requirement.unit.as_str(), self.holistic.receipt_digest, original.detail),
                    evidence: original.evidence.clone() });
            }
            // A grantless holistic review intentionally delegates foreign lineage.
            // When there are no Personal originals, every declared owner must supply it.
            if requirement.unit == VerificationUnit::CareerEvidenceLineage
                && self.manifest.evidence_source_paths_for(&DataOwner::Personal).is_empty()
                && !self.owners.is_empty() {
                let results = self.owners.iter().map(|owner| owner.evaluation.requirements.iter()
                    .find(|r| r.requirement.unit == VerificationUnit::CareerEvidenceLineage && r.passed)
                    .ok_or_else(|| invalid("a declared owner has no accepted lineage verification")))
                    .collect::<HarnessResult<Vec<_>>>()?;
                return Ok(EvaluatedRequirement { requirement: *requirement, passed: true,
                    detail: "Reused the accepted lineage requirement of every declared evidence owner; no Personal originals were declared".into(),
                    evidence: results.into_iter().flat_map(|r| r.evidence.clone()).collect() });
            }
            Err(HarnessError::InvalidSubmission(format!("native preservation has an unreviewed verification requirement: {}", requirement.unit.as_str())))
        }).collect::<HarnessResult<Vec<_>>>()?;
        let mut evaluation = HarnessTaskEvaluation {
            version: HARNESS_SCHEMA_VERSION,
            resolved_plan_digest: plan_digest(plan)?,
            prepared_role_run_digest: prepared.role_run.prepared_role_run_digest.clone(),
            revision: record.revision_contract.revision,
            revision_contract_digest: record.revision_contract.revision_contract_digest.clone(),
            execution_record: record.clone(),
            execution_status: ExecutionStatus::Completed,
            subject_status: SubjectStatus::Accepted,
            subject: Some(EvaluationSubject::ProducedArtifact {
                artifact_digest: candidate.clone(),
            }),
            missing_context: None,
            candidate_digest: Some(candidate),
            artifact: Some(EvaluatedArtifact::ReviewedCareer {
                changes: self.changes.clone(),
                composition_digest: self.composition.composition_digest.clone(),
            }),
            requirements,
            blocking_findings: Vec::new(),
            improvements: Vec::new(),
            promotion_state: PromotionState::None,
            learning_observations: Vec::new(),
            role_executions: Vec::new(),
            completion_state: HarnessCompletionState::ValidatedPendingApply,
            assurance: ExecutionAssurance::Advisory,
            task_evaluation_receipt_digest: String::new(),
        };
        evaluation.task_evaluation_receipt_digest =
            execution::task_evaluation_receipt_digest(&evaluation)?;
        Ok(evaluation)
    }
}

fn plan_digest(plan: &ResolvedHarnessPlan) -> HarnessResult<String> {
    serialized_digest(plan)
}

/// Ignore other owners' comparison edits, never the reviewed body or this owner's inputs.
pub(crate) fn owner_manifest_equivalent(
    before: &CareerCompositionManifest,
    after: &CareerCompositionManifest,
    owner: &DataOwner,
) -> HarnessResult<bool> {
    let before = &before.normalized();
    let after = &after.normalized();
    let common = before.version == HARNESS_SCHEMA_VERSION
        && after.version == HARNESS_SCHEMA_VERSION
        && before.career_output_surface == after.career_output_surface
        && before.artifact_targets == after.artifact_targets
        && before.coverage == after.coverage
        && before.complete_coverage_confirmation_reported
            == after.complete_coverage_confirmation_reported
        && before.evidence_owners == after.evidence_owners
        && before.canonical_evidence_owners == after.canonical_evidence_owners
        && before.excluded_evidence_owners == after.excluded_evidence_owners
        && before
            .claim_lineage
            .iter()
            .filter(|c| &c.evidence_owner == owner)
            .eq(after
                .claim_lineage
                .iter()
                .filter(|c| &c.evidence_owner == owner));
    if !common {
        return Ok(false);
    }
    let (Some(before_comparison), Some(after_comparison)) = (&before.comparison, &after.comparison)
    else {
        return Ok(before.comparison.is_none() && after.comparison.is_none());
    };
    let own = |manifest: &CareerCompositionManifest| {
        manifest
            .claim_lineage
            .iter()
            .filter(|c| &c.evidence_owner == owner)
            .cloned()
            .collect::<Vec<_>>()
    };
    let candidates = |comparison: &crate::career::CareerComparison| {
        comparison
            .inventory
            .candidates
            .iter()
            .filter(|c| &c.owner == owner)
            .cloned()
            .collect::<Vec<_>>()
    };
    let mut left = before_comparison.review_view(owner)?;
    let mut right = after_comparison.review_view(owner)?;
    // These two hashes cover unrelated owners. The remaining view and exact owned
    // source identities/versions retain every actual decision input.
    left.comparison_digest.clear();
    right.comparison_digest.clear();
    left.inventory_digest.clear();
    right.inventory_digest.clear();
    Ok(before.career_output_surface == after.career_output_surface
        && before.artifact_targets == after.artifact_targets
        && before.coverage == after.coverage
        && before.complete_coverage_confirmation_reported
            == after.complete_coverage_confirmation_reported
        && before.evidence_owners == after.evidence_owners
        && before.canonical_evidence_owners == after.canonical_evidence_owners
        && before.excluded_evidence_owners == after.excluded_evidence_owners
        && own(before) == own(after)
        && left == right
        && before_comparison.inventory.request == after_comparison.inventory.request
        && before_comparison.inventory.router_sources == after_comparison.inventory.router_sources
        && candidates(before_comparison) == candidates(after_comparison))
}

impl HarnessEngine {
    pub(super) fn attest_owner_for_manifest(
        &self,
        receipt: &CareerExecutionReviewReceipt,
        manifest: &CareerCompositionManifest,
    ) -> HarnessResult<()> {
        let manifest = &manifest.normalized();
        let [grant] = receipt.resolved.context_grants.as_slice() else {
            return Err(invalid("owner reuse requires one original evidence grant"));
        };
        let old = receipt
            .resolved
            .career_composition_manifest
            .as_ref()
            .ok_or_else(|| invalid("owner reuse requires the original manifest"))?;
        if !owner_manifest_equivalent(old, manifest, &grant.owner)? {
            return Err(invalid("owner review inputs changed; re-review this owner"));
        }
        if old == manifest {
            let current = self.attest_career_execution_review_unlocked(
                &receipt.resolved,
                &receipt.prepared,
                &receipt.evaluation.execution_record,
            )?;
            return if current == *receipt {
                Ok(())
            } else {
                Err(invalid("owner receipt is invalid or stale"))
            };
        }
        self.vault
            .policy_configuration
            .validate_resolved(&receipt.resolved)?;
        // Bind the CURRENT complete inventory while retaining the old genuine execution.
        let current = self.resolve_contract(
            receipt.resolved.contract.clone(),
            receipt.resolved.decision_trace.clone(),
            receipt.resolved.request_provenance.clone(),
            Some(manifest),
        )?;
        let prepared = self.prepare(
            &current,
            &receipt.prepared.runtime_capabilities,
            receipt.prepared.context_bundle.as_ref(),
        )?;
        if prepared.source_versions.store_identity
            != receipt.prepared.source_versions.store_identity
            || prepared.source_versions.source_root_identity
                != receipt.prepared.source_versions.source_root_identity
        {
            return Err(invalid("owner review store or source root changed"));
        }
        // Reproduce the original plan and preparation through the same Core
        // builders. Only the already checked comparison metadata may differ;
        // role controls, runtime, workflow and every source binding stay exact.
        let original = &receipt.resolved;
        let mut old_plan = self.router.build_plan(HarnessPlanInputs {
            request: &current.request,
            contract: &current.contract,
            decision_trace: &current.decision_trace,
            request_provenance: &current.request_provenance,
            intent: current.intent,
            context_grants: &current.context_grants,
            career_composition_manifest: Some(old),
            evidence_source_paths: &current.evidence_source_paths,
            targets: current.plan.targets.clone(),
            workspace_policies: current.plan.workspace_policies.clone(),
            vault: &self.vault,
        })?;
        old_plan.review_artifacts = current.plan.review_artifacts.clone();
        old_plan.source_versions = self.plan_source_versions(&old_plan, old.comparison.as_ref())?;
        old_plan.validate()?;
        let mut canonical_original = current.clone();
        canonical_original.career_composition_manifest = Some(old.clone());
        canonical_original.resolved_plan_digest = serialized_digest(&old_plan)?;
        canonical_original.plan = old_plan;
        if canonical_original != *original
            || self.prepare_revalidated(
                &canonical_original,
                &receipt.prepared.runtime_capabilities,
                receipt.prepared.context_bundle.as_ref(),
            )? != receipt.prepared
        {
            return Err(invalid(
                "original owner plan or preparation is not canonical",
            ));
        }
        if current.plan.targets != receipt.resolved.plan.targets
            || current.plan.review_artifacts != receipt.resolved.plan.review_artifacts
            || current.plan.verification_requirements
                != receipt.resolved.plan.verification_requirements
            || current.plan.role_policy_bindings != receipt.resolved.plan.role_policy_bindings
            || current.plan.required_policies != receipt.resolved.plan.required_policies
            || current.plan.allowed_context_roots != receipt.resolved.plan.allowed_context_roots
            || current.plan.denied_context_roots != receipt.resolved.plan.denied_context_roots
            || current.plan.evidence_sources != receipt.resolved.plan.evidence_sources
            || current.plan.learning_sources != receipt.resolved.plan.learning_sources
        {
            return Err(invalid("owner policy, evidence, scope or artifact changed"));
        }
        let docs = |p: &PreparedRoleRun| {
            p.role_bundles
                .iter()
                .map(|b| (b.role, b.bound_documents().cloned().collect::<Vec<_>>()))
                .collect::<Vec<_>>()
        };
        let used_paths = receipt
            .prepared
            .role_bundles
            .iter()
            .flat_map(HarnessRoleBundle::bound_documents)
            .filter(|d| {
                d.source != HarnessBoundDocumentSource::Target
                    && d.source != HarnessBoundDocumentSource::WorkspacePolicy
            })
            .map(|d| d.relative_path.as_str())
            .collect::<BTreeSet<_>>();
        let owned_versions = |p: &PreparedRoleRun| {
            p.source_versions
                .versions
                .iter()
                .filter(|v| used_paths.contains(v.logical_path.as_str()))
                .cloned()
                .collect::<Vec<_>>()
        };
        if owned_versions(&prepared) != owned_versions(&receipt.prepared) {
            return Err(invalid(
                "owner policy or evidence identity/revision changed",
            ));
        }
        if docs(&prepared) != docs(&receipt.prepared) {
            return Err(invalid("actual owner review documents changed"));
        }
        let evaluation = execution::evaluate_execution_record(
            &receipt.resolved.plan,
            &receipt.prepared,
            &[],
            &receipt.evaluation.execution_record,
        )?;
        execution::require_accepted_execution_evaluation(evaluation.clone())?;
        let record = &receipt.evaluation.execution_record;
        let reviewer = record
            .role_results
            .iter()
            .find(|r| r.role == HarnessRole::Reviewer)
            .ok_or_else(|| invalid("owner receipt has no actual reviewer"))?;
        let bindings = career_verification_bindings(&receipt.resolved.plan, record)?;
        let artifacts = serialized_digest(&receipt.resolved.plan.targets)?;
        let sources = (!receipt.resolved.plan.evidence_sources.is_empty())
            .then(|| serialized_digest(&receipt.resolved.plan.evidence_sources))
            .transpose()?;
        let digest = serialized_digest(&(
            HARNESS_SCHEMA_VERSION,
            &receipt.resolved,
            &receipt.prepared,
            record,
            &evaluation,
            &reviewer.result_digest,
            &bindings,
            &artifacts,
            &sources,
        ))?;
        if evaluation != receipt.evaluation
            || reviewer.result_digest != receipt.reviewer_result_digest
            || bindings != receipt.verification_bindings
            || artifacts != receipt.artifact_set_digest
            || sources != receipt.evidence_bundle_digest
            || digest != receipt.receipt_digest
        {
            return Err(invalid("original owner execution receipt was altered"));
        }
        Ok(())
    }

    /// Source validation and exact reference construction without a model invocation.
    pub fn career_review_handoff(
        &self,
        manifest: &CareerCompositionManifest,
        receipts: &[CareerExecutionReviewReceipt],
    ) -> HarnessResult<String> {
        for receipt in receipts {
            self.attest_owner_for_manifest(receipt, manifest)?;
        }
        manifest.holistic_task_statement(receipts)
    }
}
