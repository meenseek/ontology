use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{
    HARNESS_SCHEMA_VERSION, HarnessEngine, HarnessError, HarnessPlan, HarnessResult,
    HarnessRuntimeCapabilities, PreparedRoleRun, RawNormalizedInputIdentity,
    RoleRuntimeCapabilities, ToolEvidenceSet, ToolTermination, VerificationOwner,
    VerificationRequirement, require_version, serialized_digest, validate_digest,
    validate_identifier, workspace_root_identity,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolStdin {
    pub bytes: Vec<u8>,
    pub bytes_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCheck {
    pub check_identifier: String,
    pub verification_requirement: VerificationRequirement,
    pub executable: String,
    pub executable_digest: String,
    pub argv: Vec<String>,
    pub cwd_workspace_relative: String,
    pub environment: BTreeMap<String, String>,
    pub inherit_environment: bool,
    pub stdin: ToolStdin,
    pub timeout_millis: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolExecutionPlan {
    pub version: u32,
    /// Digest of the exact engine-resolved Harness plan that owns these checks.
    pub resolved_plan_digest: String,
    pub workspace_root_identity: String,
    pub checks: Vec<ToolCheck>,
    /// Digest of this tool execution plan's current serialized contract.
    pub tool_plan_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCheckEvidence {
    pub check: ToolCheck,
    pub started_millis: u64,
    pub finished_millis: u64,
    pub exit_code: i32,
    pub stdout_digest: String,
    pub stderr_digest: String,
    pub semantic_result: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolExecutionEvidence {
    pub version: u32,
    pub tool_plan_digest: String,
    pub checks: Vec<ToolCheckEvidence>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedHarnessRun {
    pub version: u32,
    pub plan_identity: RawNormalizedInputIdentity,
    pub tool_plan_identity: Option<RawNormalizedInputIdentity>,
    pub plan: HarnessPlan,
    pub runtime_capabilities: HarnessRuntimeCapabilities,
    pub accepted_tool_plan: Option<ToolExecutionPlan>,
    pub role_run: PreparedRoleRun,
    /// Digest of the complete prepared Harness run, including raw input identities and role preparation.
    pub prepared_run_digest: String,
}

impl ToolExecutionPlan {
    pub fn build(
        resolved_plan_digest: String,
        workspace_root_identity: String,
        mut checks: Vec<ToolCheck>,
    ) -> HarnessResult<Self> {
        checks.sort_by(|left, right| left.check_identifier.cmp(&right.check_identifier));
        let tool_plan_digest = serialized_digest(&(
            HARNESS_SCHEMA_VERSION,
            &resolved_plan_digest,
            &workspace_root_identity,
            &checks,
        ))?;
        let plan = Self {
            version: HARNESS_SCHEMA_VERSION,
            resolved_plan_digest,
            workspace_root_identity,
            checks,
            tool_plan_digest,
        };
        plan.validate_shape()?;
        Ok(plan)
    }

    pub fn validate_for(&self, plan: &HarnessPlan) -> HarnessResult<()> {
        self.validate_shape()?;
        if self.resolved_plan_digest != plan.resolved_plan_digest {
            return Err(HarnessError::InvalidPlan(
                "tool plan resolved-plan digest does not match Harness plan".to_owned(),
            ));
        }
        if self.workspace_root_identity != plan.workspace_root_identity {
            return Err(HarnessError::InvalidPlan(
                "tool plan workspace identity does not match the frozen target workspace"
                    .to_owned(),
            ));
        }
        let expected = plan
            .requirements
            .iter()
            .filter_map(|requirement| {
                (requirement.owner == VerificationOwner::Tool).then_some(*requirement)
            })
            .collect::<BTreeSet<_>>();
        let actual = self
            .checks
            .iter()
            .map(|check| check.verification_requirement)
            .collect::<BTreeSet<_>>();
        if actual != expected {
            return Err(HarnessError::InvalidPlan(
                "tool plan checks must cover every and only Tool-owned verification unit"
                    .to_owned(),
            ));
        }
        for check in &self.checks {
            let executable = std::fs::read(&check.executable).map_err(|error| {
                HarnessError::InvalidPlan(format!(
                    "failed to read exact tool executable `{}`: {error}",
                    check.executable
                ))
            })?;
            if super::byte_digest(&executable) != check.executable_digest {
                return Err(HarnessError::InvalidPlan(format!(
                    "tool executable digest drifted for `{}`",
                    check.executable
                )));
            }
        }
        Ok(())
    }

    fn validate_shape(&self) -> HarnessResult<()> {
        require_version(
            "tool execution plan schema",
            self.version,
            HARNESS_SCHEMA_VERSION,
        )?;
        validate_digest("tool plan resolved-plan digest", &self.resolved_plan_digest)?;
        validate_identifier(
            "tool plan workspace root identity",
            &self.workspace_root_identity,
        )?;
        if self.checks.is_empty() || self.checks.len() > 128 {
            return Err(HarnessError::InvalidPlan(
                "tool plans require between one and 128 checks".to_owned(),
            ));
        }
        let mut identifiers = BTreeSet::new();
        let mut exact_recipes = BTreeSet::new();
        let mut previous = None;
        for check in &self.checks {
            validate_tool_check(check)?;
            if !identifiers.insert(check.check_identifier.as_str()) {
                return Err(HarnessError::InvalidPlan(
                    "tool check identifiers must be unique".to_owned(),
                ));
            }
            if previous.is_some_and(|value| value >= check.check_identifier.as_str()) {
                return Err(HarnessError::InvalidPlan(
                    "tool checks must be sorted by check identifier".to_owned(),
                ));
            }
            let recipe_digest = serialized_digest(&(
                check.verification_requirement,
                &check.executable,
                &check.executable_digest,
                &check.argv,
                &check.cwd_workspace_relative,
                &check.environment,
                check.inherit_environment,
                &check.stdin,
                check.timeout_millis,
            ))?;
            if !exact_recipes.insert(recipe_digest) {
                return Err(HarnessError::InvalidPlan(
                    "tool plans must not repeat the same exact recipe".to_owned(),
                ));
            }
            previous = Some(check.check_identifier.as_str());
        }
        let digest = serialized_digest(&(
            self.version,
            &self.resolved_plan_digest,
            &self.workspace_root_identity,
            &self.checks,
        ))?;
        if digest != self.tool_plan_digest {
            return Err(HarnessError::InvalidPlan(
                "tool execution plan digest does not match its exact recipes".to_owned(),
            ));
        }
        Ok(())
    }
}

impl ToolExecutionEvidence {
    pub fn validate_against(&self, plan: &ToolExecutionPlan) -> HarnessResult<()> {
        require_version(
            "tool execution evidence schema",
            self.version,
            HARNESS_SCHEMA_VERSION,
        )?;
        if self.tool_plan_digest != plan.tool_plan_digest || self.checks.len() != plan.checks.len()
        {
            return Err(HarnessError::InvalidSubmission(
                "tool evidence does not bind the exact accepted plan or check count".to_owned(),
            ));
        }
        for (expected, actual) in plan.checks.iter().zip(&self.checks) {
            if &actual.check != expected {
                return Err(HarnessError::InvalidSubmission(
                    "tool evidence contains a missing, extra, reordered, or tampered check"
                        .to_owned(),
                ));
            }
            if actual.finished_millis < actual.started_millis
                || actual
                    .finished_millis
                    .checked_sub(actual.started_millis)
                    .is_none_or(|elapsed| elapsed > expected.timeout_millis)
                || actual.exit_code < 0
            {
                return Err(HarnessError::InvalidSubmission(format!(
                    "tool check `{}` did not complete within its accepted recipe",
                    expected.check_identifier
                )));
            }
            let expected_semantic_result = if actual.exit_code == 0 {
                "passed"
            } else {
                "failed"
            };
            if actual.semantic_result.trim() != expected_semantic_result {
                return Err(HarnessError::InvalidSubmission(format!(
                    "tool check `{}` semantic result does not match its exit code",
                    expected.check_identifier
                )));
            }
            validate_digest("tool stdout digest", &actual.stdout_digest)?;
            validate_digest("tool stderr digest", &actual.stderr_digest)?;
        }
        Ok(())
    }

    pub fn validate_tool_evidence(
        &self,
        plan: &ToolExecutionPlan,
        core: Option<&ToolEvidenceSet>,
    ) -> HarnessResult<()> {
        self.validate_against(plan)?;
        let core = core.ok_or_else(|| {
            HarnessError::InvalidSubmission(
                "exact current tool evidence has no Harness engine tool evidence set".to_owned(),
            )
        })?;
        if core.executions.len() != self.checks.len() {
            return Err(HarnessError::InvalidSubmission(
                "Harness engine and exact current tool evidence counts differ".to_owned(),
            ));
        }
        for (exact, recorded_execution) in self.checks.iter().zip(&core.executions) {
            if recorded_execution.unit != exact.check.verification_requirement.unit
                || recorded_execution.command != exact.check.argv
                || recorded_execution.stdout_digest != exact.stdout_digest
                || recorded_execution.stderr_digest != exact.stderr_digest
                || !matches!(
                    recorded_execution.termination,
                    ToolTermination::Exited { code } if code == exact.exit_code
                )
            {
                return Err(HarnessError::InvalidSubmission(format!(
                    "Harness engine tool evidence for `{}` is not the exact accepted current recipe execution",
                    exact.check.check_identifier
                )));
            }
        }
        Ok(())
    }
}

impl PreparedHarnessRun {
    pub fn prepare(
        engine: &HarnessEngine,
        raw_plan_bytes: &[u8],
        plan: HarnessPlan,
        capabilities: HarnessRuntimeCapabilities,
        raw_tool_plan_bytes: Option<&[u8]>,
        tool_plan: Option<ToolExecutionPlan>,
    ) -> HarnessResult<Self> {
        plan.validate()?;
        engine.revalidate_resolved(&plan.resolved_request)?;
        plan.frozen_targets
            .revalidate_workspace(&engine.workspace_root)?;
        plan.validate_capabilities(&capabilities)?;
        match (plan.requires_tool_execution(), &tool_plan) {
            (true, Some(tool_plan)) => tool_plan.validate_for(&plan)?,
            (true, None) => {
                return Err(HarnessError::InvalidPlan(
                    "Tool-owned requirements require an accepted tool execution plan".to_owned(),
                ));
            }
            (false, None) => {}
            (false, Some(_)) => {
                return Err(HarnessError::InvalidPlan(
                    "tool plan is forbidden when no requirement is Tool-owned".to_owned(),
                ));
            }
        }
        if raw_tool_plan_bytes.is_some() != tool_plan.is_some() {
            return Err(HarnessError::InvalidPlan(
                "raw tool-plan bytes and decoded tool plan must be supplied together".to_owned(),
            ));
        }
        let decoded_plan: HarnessPlan = super::decode_current_json(raw_plan_bytes, "Harness plan")?;
        if decoded_plan != plan {
            return Err(HarnessError::InvalidPlan(
                "decoded current plan does not match the exact raw plan bytes".to_owned(),
            ));
        }
        if let (Some(bytes), Some(expected)) = (raw_tool_plan_bytes, tool_plan.as_ref()) {
            let decoded: ToolExecutionPlan =
                super::decode_current_json(bytes, "Harness tool plan")?;
            if &decoded != expected {
                return Err(HarnessError::InvalidPlan(
                    "decoded tool plan does not match the exact raw tool-plan bytes".to_owned(),
                ));
            }
        }
        let plan_identity =
            RawNormalizedInputIdentity::from_current_json(raw_plan_bytes, "Harness plan", &plan)?;
        let tool_plan_identity = raw_tool_plan_bytes
            .zip(tool_plan.as_ref())
            .map(|(bytes, expected)| {
                RawNormalizedInputIdentity::from_current_json(bytes, "Harness tool plan", expected)
            })
            .transpose()?;
        let role_run = prepare_role_run_from_bound_inputs(engine, &plan, &capabilities)?;
        let prepared_run_digest = serialized_digest(&(
            HARNESS_SCHEMA_VERSION,
            &plan_identity,
            &tool_plan_identity,
            &plan,
            &capabilities,
            &tool_plan,
            &role_run,
        ))?;
        Ok(Self {
            version: HARNESS_SCHEMA_VERSION,
            plan_identity,
            tool_plan_identity,
            plan,
            runtime_capabilities: capabilities,
            accepted_tool_plan: tool_plan,
            role_run,
            prepared_run_digest,
        })
    }

    pub(super) fn validate_serialized_integrity(
        &self,
        raw_prepared_bytes: &[u8],
    ) -> HarnessResult<()> {
        require_version("prepared run", self.version, HARNESS_SCHEMA_VERSION)?;
        let decoded: Self = super::decode_current_json(raw_prepared_bytes, "prepared Harness run")?;
        if decoded != *self {
            return Err(HarnessError::InvalidPlan(
                "downstream prepared-run bytes do not match the current prepared run".to_owned(),
            ));
        }
        self.plan.validate()?;
        self.role_run
            .source_versions
            .require_superset(self.plan.bound_source_versions())?;
        self.plan
            .validate_capabilities(&self.runtime_capabilities)?;
        self.validate_bound_inputs()?;
        let actual = serialized_digest(&(
            self.version,
            &self.plan_identity,
            &self.tool_plan_identity,
            &self.plan,
            &self.runtime_capabilities,
            &self.accepted_tool_plan,
            &self.role_run,
        ))?;
        if actual != self.prepared_run_digest {
            return Err(HarnessError::InvalidPlan(
                "prepared-run digest does not match the current execution contract".to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_bound_inputs(&self) -> HarnessResult<()> {
        self.plan_identity.validate()?;
        if self.plan_identity.normalized_value_digest != serialized_digest(&self.plan)? {
            return Err(HarnessError::InvalidPlan(
                "prepared plan identity does not bind the current plan".to_owned(),
            ));
        }
        match (
            self.plan.requires_tool_execution(),
            &self.tool_plan_identity,
            &self.accepted_tool_plan,
        ) {
            (true, Some(identity), Some(tool_plan)) => {
                identity.validate()?;
                if identity.normalized_value_digest != serialized_digest(tool_plan)? {
                    return Err(HarnessError::InvalidPlan(
                        "prepared tool-plan identity does not bind the accepted tool plan"
                            .to_owned(),
                    ));
                }
                tool_plan.validate_for(&self.plan)
            }
            (true, _, _) => Err(HarnessError::InvalidPlan(
                "Tool-owned requirements require both a tool-plan identity and an accepted tool execution plan"
                    .to_owned(),
            )),
            (false, None, None) => Ok(()),
            (false, _, _) => Err(HarnessError::InvalidPlan(
                "tool-plan identity and accepted tool plan are forbidden when no requirement is Tool-owned"
                    .to_owned(),
            )),
        }
    }

    #[must_use]
    pub fn bound_source_versions(&self) -> &super::SourceVersionSet {
        self.role_run.bound_source_versions()
    }

    pub fn validate_with_engine(
        &self,
        raw_prepared_bytes: &[u8],
        engine: &HarnessEngine,
    ) -> HarnessResult<()> {
        self.validate_recovery_with_engine(raw_prepared_bytes, engine)?;
        engine.revalidate_resolved(&self.plan.resolved_request)?;
        self.plan
            .frozen_targets
            .revalidate_workspace(&engine.workspace_root)?;
        let expected =
            prepare_role_run_from_bound_inputs(engine, &self.plan, &self.runtime_capabilities)?;
        if expected != self.role_run {
            return Err(HarnessError::InvalidSubmission(
                "current prepared run is not the exact current Harness engine preparation"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    pub fn validate_recovery_with_engine(
        &self,
        raw_prepared_bytes: &[u8],
        engine: &HarnessEngine,
    ) -> HarnessResult<()> {
        self.validate_serialized_integrity(raw_prepared_bytes)?;
        let canonical_workspace =
            engine
                .workspace_root
                .canonicalize()
                .map_err(|source| HarnessError::FileRead {
                    path: engine.workspace_root.clone(),
                    message: source.to_string(),
                })?;
        let actual = workspace_root_identity(&canonical_workspace)?;
        if actual != self.plan.workspace_root_identity {
            return Err(HarnessError::PlanDrift {
                expected: self.plan.workspace_root_identity.clone(),
                actual,
            });
        }
        Ok(())
    }
}

fn prepare_role_run_from_bound_inputs(
    engine: &HarnessEngine,
    plan: &HarnessPlan,
    capabilities: &HarnessRuntimeCapabilities,
) -> HarnessResult<PreparedRoleRun> {
    let role_capabilities = role_runtime_capabilities(capabilities);
    let context_bundle = if let Some((query, maximum_context_bytes)) =
        plan.resolved_request.contract.vault_read_parameters()
    {
        Some(engine.retrieve_context(&plan.resolved_request, query, maximum_context_bytes)?)
    } else {
        None
    };
    engine.prepare(
        &plan.resolved_request,
        &role_capabilities,
        context_bundle.as_ref(),
    )
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

fn validate_tool_check(check: &ToolCheck) -> HarnessResult<()> {
    validate_identifier("tool check identifier", &check.check_identifier)?;
    if !check.executable.starts_with('/')
        || check.executable.contains("/../")
        || matches!(
            check.executable.rsplit('/').next(),
            Some("sh" | "bash" | "zsh" | "fish" | "cmd" | "powershell" | "pwsh")
        )
    {
        return Err(HarnessError::InvalidPlan(
            "tool executables must be absolute direct executables, never shells or PATH-only names"
                .to_owned(),
        ));
    }
    validate_digest("tool executable digest", &check.executable_digest)?;
    if check.argv.is_empty()
        || check.argv[0] != check.executable
        || check.argv.iter().any(|argument| argument.contains('\0'))
        || check.cwd_workspace_relative.starts_with('/')
        || check
            .cwd_workspace_relative
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
        || check.timeout_millis == 0
    {
        return Err(HarnessError::InvalidPlan(
            "tool recipe argv, cwd, or timeout is not exact and portable".to_owned(),
        ));
    }
    if check.inherit_environment {
        return Err(HarnessError::InvalidPlan(
            "current tool recipes must disable inherited environment".to_owned(),
        ));
    }
    for (key, value) in &check.environment {
        if key.is_empty()
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            || value.contains('\0')
        {
            return Err(HarnessError::InvalidPlan(
                "tool environment must be an explicit valid map".to_owned(),
            ));
        }
    }
    validate_digest("tool stdin digest", &check.stdin.bytes_digest)?;
    if super::byte_digest(&check.stdin.bytes) != check.stdin.bytes_digest {
        return Err(HarnessError::InvalidPlan(
            "tool stdin digest does not match the exact bytes".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::VerificationUnit;
    use super::*;

    fn check(identifier: &str, unit: super::super::VerificationUnit) -> ToolCheck {
        ToolCheck {
            check_identifier: identifier.to_owned(),
            verification_requirement: VerificationRequirement {
                unit,
                owner: VerificationOwner::Tool,
                subject: super::super::EvaluationSubjectKind::ProducedArtifact,
            },
            executable: "/usr/bin/true".to_owned(),
            executable_digest: "a".repeat(64),
            argv: vec!["/usr/bin/true".to_owned()],
            cwd_workspace_relative: "workspace".to_owned(),
            environment: BTreeMap::new(),
            inherit_environment: false,
            stdin: ToolStdin {
                bytes: Vec::new(),
                bytes_digest: super::super::byte_digest(&[]),
            },
            timeout_millis: 100,
        }
    }

    #[test]
    fn evidence_rejects_reordered_or_tampered_multi_check_plan() {
        let plan = ToolExecutionPlan::build(
            "1".repeat(64),
            "workspace-1".to_owned(),
            vec![
                check("check-2", VerificationUnit::CodeCorrectness),
                check("check-1", VerificationUnit::TestsAndStaticAnalysis),
            ],
        )
        .unwrap();
        let mut evidence = ToolExecutionEvidence {
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
                    stdout_digest: "1".repeat(64),
                    stderr_digest: "2".repeat(64),
                    semantic_result: "passed".to_owned(),
                })
                .collect(),
        };
        evidence.validate_against(&plan).unwrap();
        evidence.checks.swap(0, 1);
        assert!(evidence.validate_against(&plan).is_err());
    }

    #[test]
    fn shell_and_path_only_recipes_are_rejected() {
        let mut recipe = check("check-1", VerificationUnit::TestsAndStaticAnalysis);
        recipe.executable = "cargo".to_owned();
        recipe.argv[0] = "cargo".to_owned();
        assert!(
            ToolExecutionPlan::build("plan-1".to_owned(), "workspace-1".to_owned(), vec![recipe])
                .is_err()
        );
    }
}
