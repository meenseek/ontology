#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{collections::BTreeSet, fs, path::Path};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use super::{
    EvaluationSubjectKind, HARNESS_SCHEMA_VERSION, HarnessAction, HarnessError, HarnessResult,
    HarnessRole, ResolvedHarnessRequest, RoleRuntimeCapabilities, TargetBinding, TargetOperation,
    TargetState, VerificationOwner, VerificationRequirement, byte_digest,
    ensure_unique_verification_requirements, require_version, serialized_digest, validate_digest,
    validate_identifier, validate_role_runtime_capabilities,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawNormalizedInputIdentity {
    pub version: u32,
    pub raw_bytes_digest: String,
    pub normalized_value_digest: String,
}

impl RawNormalizedInputIdentity {
    pub fn from_current_json<T>(bytes: &[u8], label: &str, expected: &T) -> HarnessResult<Self>
    where
        T: DeserializeOwned + Serialize + PartialEq,
    {
        let decoded: T = super::decode_current_json(bytes, label)?;
        if &decoded != expected {
            return Err(HarnessError::InvalidPlan(format!(
                "decoded current {label} does not match the bound artifact"
            )));
        }
        Ok(Self {
            version: HARNESS_SCHEMA_VERSION,
            raw_bytes_digest: byte_digest(bytes),
            normalized_value_digest: serialized_digest(expected)?,
        })
    }

    pub fn validate(&self) -> HarnessResult<()> {
        require_version(
            "raw/normalized identity",
            self.version,
            HARNESS_SCHEMA_VERSION,
        )?;
        validate_digest("raw bytes digest", &self.raw_bytes_digest)?;
        validate_digest("normalized value digest", &self.normalized_value_digest)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TargetLocator {
    pub root_identity: String,
    pub canonical_parent: String,
    pub device: u64,
    pub inode: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenTarget {
    pub workspace_relative_path: String,
    pub operation: TargetOperation,
    pub content_digest: Option<String>,
    pub parent_directories_to_create: Vec<String>,
    pub locator: TargetLocator,
    pub human_evidence_locator: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenTargets {
    pub version: u32,
    pub identity_set_digest: String,
    pub workspace_locator_map_digest: String,
    pub targets: Vec<FrozenTarget>,
}

impl FrozenTargets {
    pub fn build(mut targets: Vec<FrozenTarget>) -> HarnessResult<Self> {
        targets.sort_by(|left, right| {
            left.workspace_relative_path
                .cmp(&right.workspace_relative_path)
        });
        validate_frozen_targets(&targets)?;
        let identities = targets
            .iter()
            .map(|target| {
                (
                    &target.workspace_relative_path,
                    target.operation,
                    &target.content_digest,
                )
            })
            .collect::<Vec<_>>();
        let locators = targets
            .iter()
            .map(|target| (&target.workspace_relative_path, &target.locator))
            .collect::<Vec<_>>();
        Ok(Self {
            version: HARNESS_SCHEMA_VERSION,
            identity_set_digest: serialized_digest(&identities)?,
            workspace_locator_map_digest: serialized_digest(&locators)?,
            targets,
        })
    }

    pub fn validate(&self) -> HarnessResult<()> {
        require_version("frozen targets", self.version, HARNESS_SCHEMA_VERSION)?;
        let rebuilt = Self::build(self.targets.clone())?;
        if rebuilt.targets != self.targets {
            return Err(HarnessError::InvalidPlan(
                "frozen targets must be sorted by workspace-relative path".to_owned(),
            ));
        }
        if rebuilt.identity_set_digest != self.identity_set_digest {
            return Err(HarnessError::InvalidPlan(
                "frozen target identity-set digest does not match target identities".to_owned(),
            ));
        }
        if rebuilt.workspace_locator_map_digest != self.workspace_locator_map_digest {
            return Err(HarnessError::InvalidPlan(
                "frozen target locator-map digest does not match workspace locators".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn from_resolved(workspace_root: &Path, targets: &[TargetBinding]) -> HarnessResult<Self> {
        let canonical_root =
            workspace_root
                .canonicalize()
                .map_err(|source| HarnessError::FileRead {
                    path: workspace_root.to_path_buf(),
                    message: source.to_string(),
                })?;
        let root_identity = workspace_root_identity(&canonical_root)?;
        let frozen = targets
            .iter()
            .map(|target| {
                let content_digest =
                    frozen_content_digest(target.operation, &target.state)?.map(ToOwned::to_owned);
                let locator = target_locator(&canonical_root, &root_identity, target)?;
                Ok(FrozenTarget {
                    workspace_relative_path: target.workspace_relative_path.clone(),
                    operation: target.operation,
                    content_digest,
                    parent_directories_to_create: target.parent_directories_to_create.clone(),
                    human_evidence_locator: canonical_root
                        .join(&target.workspace_relative_path)
                        .display()
                        .to_string(),
                    locator,
                })
            })
            .collect::<HarnessResult<Vec<_>>>()?;
        Self::build(frozen)
    }

    pub fn revalidate_workspace(&self, workspace_root: &Path) -> HarnessResult<()> {
        self.validate()?;
        let canonical_root =
            workspace_root
                .canonicalize()
                .map_err(|source| HarnessError::FileRead {
                    path: workspace_root.to_path_buf(),
                    message: source.to_string(),
                })?;
        let root_identity = workspace_root_identity(&canonical_root)?;
        for target in &self.targets {
            let binding = TargetBinding {
                workspace_relative_path: target.workspace_relative_path.clone(),
                state: match &target.content_digest {
                    None => TargetState::Absent,
                    Some(content_digest) => TargetState::Existing {
                        content_digest: content_digest.clone(),
                    },
                },
                operation: target.operation,
                parent_directories_to_create: target.parent_directories_to_create.clone(),
            };
            let actual = target_locator(&canonical_root, &root_identity, &binding)?;
            if actual != target.locator {
                return Err(HarnessError::PlanDrift {
                    expected: serialized_digest(&target.locator)?,
                    actual: serialized_digest(&actual)?,
                });
            }
            let path = canonical_root.join(&target.workspace_relative_path);
            let actual_content = match fs::read(&path) {
                Ok(bytes) => Some(byte_digest(&bytes)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(source) => {
                    return Err(HarnessError::FileRead {
                        path,
                        message: source.to_string(),
                    });
                }
            };
            if actual_content != target.content_digest {
                return Err(HarnessError::PlanDrift {
                    expected: target
                        .content_digest
                        .clone()
                        .unwrap_or_else(|| "absent".to_owned()),
                    actual: actual_content.unwrap_or_else(|| "absent".to_owned()),
                });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoleLifecycleLimits {
    pub max_role_execution_millis: u64,
    pub max_role_grace_millis: u64,
    pub max_role_close_millis: u64,
    pub max_total_role_millis: u64,
}

impl RoleLifecycleLimits {
    pub fn validate(&self) -> HarnessResult<()> {
        if self.max_role_execution_millis == 0
            || self.max_role_grace_millis == 0
            || self.max_role_close_millis == 0
        {
            return Err(HarnessError::InvalidPlan(
                "current role execution, grace, and close limits must all be non-zero".to_owned(),
            ));
        }
        let calculated = self
            .max_role_execution_millis
            .checked_add(self.max_role_grace_millis)
            .and_then(|value| value.checked_add(self.max_role_close_millis))
            .ok_or_else(|| {
                HarnessError::InvalidPlan("current role lifecycle bound overflows u64".to_owned())
            })?;
        if calculated != self.max_total_role_millis {
            return Err(HarnessError::InvalidPlan(
                "current total role bound must exactly equal execution + grace + close".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn validate_observation(
        &self,
        started_millis: u64,
        terminal_millis: u64,
        closed_millis: u64,
    ) -> HarnessResult<()> {
        self.validate()?;
        let terminal_deadline = started_millis
            .checked_add(self.max_role_execution_millis)
            .and_then(|value| value.checked_add(self.max_role_grace_millis))
            .ok_or_else(|| HarnessError::InvalidPlan("terminal deadline overflow".to_owned()))?;
        if terminal_millis < started_millis || terminal_millis > terminal_deadline {
            return Err(HarnessError::InvalidSubmission(
                "role terminal observation is outside execution + grace".to_owned(),
            ));
        }
        let close_deadline = terminal_millis
            .checked_add(self.max_role_close_millis)
            .ok_or_else(|| HarnessError::InvalidPlan("close deadline overflow".to_owned()))?;
        let total_deadline = started_millis
            .checked_add(self.max_total_role_millis)
            .ok_or_else(|| HarnessError::InvalidPlan("total role deadline overflow".to_owned()))?;
        if closed_millis < terminal_millis
            || closed_millis > close_deadline
            || closed_millis > total_deadline
        {
            return Err(HarnessError::InvalidSubmission(
                "role close observation is outside close or total bound".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "strict capability JSON preserves independent fail-closed runtime claims"
)]
pub struct HarnessRuntimeCapabilities {
    pub version: u32,
    pub available_roles: Vec<HarnessRole>,
    pub max_concurrent_roles: usize,
    pub separate_contexts: bool,
    pub file_reading: bool,
    pub tool_execution: bool,
    pub deterministic_validation: bool,
    pub max_role_bundle_bytes: usize,
    pub max_role_invocation_bytes: usize,
    pub lifecycle: RoleLifecycleLimits,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessPlan {
    pub version: u32,
    /// Digest of the exact engine-resolved Harness plan bound by this execution plan.
    pub resolved_plan_digest: String,
    pub authority_digest: String,
    pub workspace_root_identity: String,
    pub resolved_request: ResolvedHarnessRequest,
    pub action: HarnessAction,
    pub requirements: Vec<VerificationRequirement>,
    pub frozen_targets: FrozenTargets,
    pub lifecycle: RoleLifecycleLimits,
    pub source_write_allowed: bool,
    pub external_write_allowed: bool,
}

impl HarnessPlan {
    pub fn validate(&self) -> HarnessResult<()> {
        require_version("Harness plan", self.version, HARNESS_SCHEMA_VERSION)?;
        self.resolved_request.plan.validate()?;
        validate_digest("resolved plan digest", &self.resolved_plan_digest)?;
        validate_digest("current resolved authority digest", &self.authority_digest)?;
        validate_digest(
            "current workspace root identity",
            &self.workspace_root_identity,
        )?;
        if serialized_digest(&self.resolved_request)? != self.authority_digest
            || self.resolved_plan_digest != self.resolved_request.resolved_plan_digest
        {
            return Err(HarnessError::InvalidPlan(
                "current plan authority does not match its exact Harness engine-resolved request"
                    .to_owned(),
            ));
        }
        if self.resolved_request.plan.action != self.action
            || self.resolved_request.plan.verification_requirements != self.requirements
            || self.resolved_request.plan.source_write_allowed != self.source_write_allowed
            || self.resolved_request.plan.external_write_allowed != self.external_write_allowed
        {
            return Err(HarnessError::InvalidPlan(
                "current plan fields must be derived exactly from the embedded Harness engine resolution"
                    .to_owned(),
            ));
        }
        if self.requirements.is_empty() {
            return Err(HarnessError::InvalidPlan(
                "current plans require at least one verification requirement".to_owned(),
            ));
        }
        ensure_unique_verification_requirements(&self.requirements)?;
        self.frozen_targets.validate()?;
        if self.frozen_targets.targets.len() != self.resolved_request.plan.targets.len() {
            return Err(HarnessError::InvalidPlan(
                "current frozen target count differs from the Harness engine-resolved target set"
                    .to_owned(),
            ));
        }
        for (frozen, resolved) in self
            .frozen_targets
            .targets
            .iter()
            .zip(&self.resolved_request.plan.targets)
        {
            let content_digest = frozen_content_digest(resolved.operation, &resolved.state)?;
            if frozen.workspace_relative_path != resolved.workspace_relative_path
                || frozen.operation != resolved.operation
                || frozen.content_digest.as_deref() != content_digest
                || frozen.parent_directories_to_create != resolved.parent_directories_to_create
                || frozen.locator.root_identity != self.workspace_root_identity
            {
                return Err(HarnessError::InvalidPlan(
                    "current frozen targets are not the exact Harness engine-resolved path/operation/content set"
                        .to_owned(),
                ));
            }
        }
        self.lifecycle.validate()?;
        if self.external_write_allowed {
            return Err(HarnessError::InvalidPlan(
                "current Harness engine never authorizes external writes".to_owned(),
            ));
        }
        if self.source_write_allowed != self.action.is_write() {
            return Err(HarnessError::InvalidPlan(
                "current source-write authority must match the resolved write action".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn from_resolved(
        resolved_request: ResolvedHarnessRequest,
        workspace_root: &Path,
        lifecycle: RoleLifecycleLimits,
    ) -> HarnessResult<Self> {
        lifecycle.validate()?;
        let frozen_targets =
            FrozenTargets::from_resolved(workspace_root, &resolved_request.plan.targets)?;
        let canonical_workspace =
            workspace_root
                .canonicalize()
                .map_err(|source| HarnessError::FileRead {
                    path: workspace_root.to_path_buf(),
                    message: source.to_string(),
                })?;
        let plan = Self {
            version: HARNESS_SCHEMA_VERSION,
            resolved_plan_digest: resolved_request.resolved_plan_digest.clone(),
            authority_digest: serialized_digest(&resolved_request)?,
            workspace_root_identity: workspace_root_identity(&canonical_workspace)?,
            action: resolved_request.plan.action,
            requirements: resolved_request.plan.verification_requirements.clone(),
            source_write_allowed: resolved_request.plan.source_write_allowed,
            external_write_allowed: resolved_request.plan.external_write_allowed,
            resolved_request,
            frozen_targets,
            lifecycle,
        };
        plan.validate()?;
        Ok(plan)
    }

    #[must_use]
    pub fn bound_source_versions(&self) -> &super::SourceVersionSet {
        self.resolved_request.plan.bound_source_versions()
    }

    #[must_use]
    pub fn required_roles(&self) -> BTreeSet<HarnessRole> {
        self.resolved_request.plan.roles().collect()
    }

    #[must_use]
    pub fn requires_tool_execution(&self) -> bool {
        self.requirements
            .iter()
            .any(|requirement| requirement.owner == VerificationOwner::Tool)
    }

    #[must_use]
    pub fn requires_tool_capability(&self) -> bool {
        self.required_roles().contains(&HarnessRole::Verifier)
            || super::action_requires_tool_execution(self.action)
    }

    pub fn validate_capabilities(
        &self,
        capabilities: &HarnessRuntimeCapabilities,
    ) -> HarnessResult<()> {
        require_version(
            "runtime capabilities",
            capabilities.version,
            HARNESS_SCHEMA_VERSION,
        )?;
        capabilities.lifecycle.validate()?;
        if capabilities.lifecycle != self.lifecycle {
            return Err(HarnessError::InvalidPlan(
                "runtime lifecycle limits do not exactly match the current plan".to_owned(),
            ));
        }
        if !capabilities.deterministic_validation {
            return Err(HarnessError::UnsupportedRuntime(
                "runtime capabilities do not satisfy mechanically derived current requirements"
                    .to_owned(),
            ));
        }
        validate_role_runtime_capabilities(
            &self.resolved_request.plan,
            &RoleRuntimeCapabilities {
                available_roles: capabilities.available_roles.clone(),
                max_concurrent_roles: capabilities.max_concurrent_roles,
                separate_contexts: capabilities.separate_contexts,
                file_reading: capabilities.file_reading,
                tool_execution: capabilities.tool_execution,
                max_role_bundle_bytes: capabilities.max_role_bundle_bytes,
                max_role_invocation_bytes: capabilities.max_role_invocation_bytes,
                max_role_execution_millis: capabilities.lifecycle.max_role_execution_millis,
                max_role_grace_millis: capabilities.lifecycle.max_role_grace_millis,
            },
        )
    }
}

fn validate_frozen_targets(targets: &[FrozenTarget]) -> HarnessResult<()> {
    if targets.len() > 128 {
        return Err(HarnessError::InvalidPlan(
            "current frozen targets must contain at most 128 entries".to_owned(),
        ));
    }
    let mut paths = BTreeSet::new();
    for target in targets {
        if target.workspace_relative_path.is_empty()
            || target.workspace_relative_path.starts_with('/')
            || target
                .workspace_relative_path
                .split('/')
                .any(|part| part.is_empty() || matches!(part, "." | ".."))
            || !paths.insert(target.workspace_relative_path.as_str())
        {
            return Err(HarnessError::InvalidPlan(
                "current frozen target paths must be unique normalized relative paths".to_owned(),
            ));
        }
        validate_frozen_target_state(target.operation, target.content_digest.as_deref())?;
        for parent in &target.parent_directories_to_create {
            validate_relative_path(parent)?;
        }
        validate_identifier("frozen target root identity", &target.locator.root_identity)?;
        if target.locator.canonical_parent.is_empty() || target.human_evidence_locator.is_empty() {
            return Err(HarnessError::InvalidPlan(
                "current target locators must be non-empty".to_owned(),
            ));
        }
    }
    Ok(())
}

fn frozen_content_digest(
    operation: TargetOperation,
    state: &TargetState,
) -> HarnessResult<Option<&str>> {
    let content_digest = match state {
        TargetState::Absent => None,
        TargetState::Existing { content_digest } => Some(content_digest.as_str()),
    };
    validate_frozen_target_state(operation, content_digest)?;
    Ok(content_digest)
}

fn validate_frozen_target_state(
    operation: TargetOperation,
    content_digest: Option<&str>,
) -> HarnessResult<()> {
    match (operation, content_digest) {
        (TargetOperation::Create, None) => Ok(()),
        (
            TargetOperation::Update | TargetOperation::Delete | TargetOperation::Inspect,
            Some(content_digest),
        ) => validate_digest("frozen target content digest", content_digest),
        (TargetOperation::Create, Some(_)) => Err(HarnessError::InvalidPlan(
            "create targets must be frozen as absent".to_owned(),
        )),
        (TargetOperation::Update | TargetOperation::Delete | TargetOperation::Inspect, None) => {
            Err(HarnessError::InvalidPlan(
                "existing target operations require a frozen content digest".to_owned(),
            ))
        }
    }
}

fn validate_relative_path(value: &str) -> HarnessResult<()> {
    if value.is_empty()
        || Path::new(value).is_absolute()
        || Path::new(value)
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(HarnessError::InvalidPlan(
            "current target paths must be normalized workspace-relative paths".to_owned(),
        ));
    }
    Ok(())
}

fn target_locator(
    canonical_root: &Path,
    root_identity: &str,
    target: &TargetBinding,
) -> HarnessResult<TargetLocator> {
    let target_path = canonical_root.join(&target.workspace_relative_path);
    let mut ancestor = target_path.parent().ok_or_else(|| {
        HarnessError::InvalidPlan("current target has no workspace parent".to_owned())
    })?;
    while !ancestor.exists() {
        ancestor = ancestor.parent().ok_or_else(|| {
            HarnessError::InvalidPlan("current target has no existing parent ancestor".to_owned())
        })?;
    }
    let canonical_parent = ancestor
        .canonicalize()
        .map_err(|source| HarnessError::FileRead {
            path: ancestor.to_path_buf(),
            message: source.to_string(),
        })?;
    if !canonical_parent.starts_with(canonical_root) {
        return Err(HarnessError::InvalidPlan(
            "current target parent escapes the canonical workspace root".to_owned(),
        ));
    }
    let metadata = fs::metadata(&canonical_parent).map_err(|source| HarnessError::FileRead {
        path: canonical_parent.clone(),
        message: source.to_string(),
    })?;
    let (device, inode) = metadata_identity(&metadata);
    Ok(TargetLocator {
        root_identity: root_identity.to_owned(),
        canonical_parent: canonical_parent.display().to_string(),
        device,
        inode,
    })
}

pub(super) fn workspace_root_identity(canonical_root: &Path) -> HarnessResult<String> {
    let metadata = fs::metadata(canonical_root).map_err(|source| HarnessError::FileRead {
        path: canonical_root.to_path_buf(),
        message: source.to_string(),
    })?;
    let (device, inode) = metadata_identity(&metadata);
    serialized_digest(&(canonical_root.as_os_str().as_encoded_bytes(), device, inode))
}

#[cfg(unix)]
fn metadata_identity(metadata: &fs::Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}

#[cfg(not(unix))]
fn metadata_identity(_metadata: &fs::Metadata) -> (u64, u64) {
    (0, 0)
}

#[must_use]
pub fn requirement_subjects(
    requirements: &[VerificationRequirement],
) -> BTreeSet<EvaluationSubjectKind> {
    requirements
        .iter()
        .map(|requirement| requirement.subject)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_and_normalized_json_identities_are_distinct() {
        let expected = std::collections::BTreeMap::from([("a".to_owned(), 1_u32)]);
        let compact = RawNormalizedInputIdentity::from_current_json(
            br#"{"a":1}"#,
            "test identity",
            &expected,
        )
        .unwrap();
        let spaced = RawNormalizedInputIdentity::from_current_json(
            b"{ \"a\" : 1 }",
            "test identity",
            &expected,
        )
        .unwrap();
        assert_ne!(compact.raw_bytes_digest, spaced.raw_bytes_digest);
        assert_eq!(
            compact.normalized_value_digest,
            spaced.normalized_value_digest
        );
    }

    #[test]
    fn lifecycle_requires_bounded_close_and_checked_total() {
        let limits = RoleLifecycleLimits {
            max_role_execution_millis: 10,
            max_role_grace_millis: 2,
            max_role_close_millis: 3,
            max_total_role_millis: 15,
        };
        limits.validate_observation(5, 17, 20).unwrap();
        assert!(limits.validate_observation(5, 17, 21).is_err());
        assert!(
            RoleLifecycleLimits {
                max_role_execution_millis: u64::MAX,
                max_role_grace_millis: 1,
                max_role_close_millis: 1,
                max_total_role_millis: u64::MAX,
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn strict_current_types_reject_unknown_and_missing_fields() {
        let unknown = serde_json::json!({
            "version": 1,
            "raw_bytes_digest": "0".repeat(64),
            "normalized_value_digest": "1".repeat(64),
            "extra": true
        });
        assert!(serde_json::from_value::<RawNormalizedInputIdentity>(unknown).is_err());
        let missing = serde_json::json!({
            "version": 1,
            "raw_bytes_digest": "0".repeat(64)
        });
        assert!(serde_json::from_value::<RawNormalizedInputIdentity>(missing).is_err());
    }

    #[test]
    fn frozen_inspection_requires_an_existing_content_digest() {
        let target = FrozenTarget {
            workspace_relative_path: "README.md".to_owned(),
            operation: TargetOperation::Inspect,
            content_digest: Some("a".repeat(64)),
            parent_directories_to_create: Vec::new(),
            locator: TargetLocator {
                root_identity: "workspace-1".to_owned(),
                canonical_parent: "/workspace".to_owned(),
                device: 1,
                inode: 1,
            },
            human_evidence_locator: "/workspace/README.md".to_owned(),
        };
        FrozenTargets::build(vec![target.clone()])
            .expect("an inspected target with an exact digest must freeze");

        let mut missing_digest = target;
        missing_digest.content_digest = None;
        assert!(matches!(
            FrozenTargets::build(vec![missing_digest]),
            Err(HarnessError::InvalidPlan(message))
                if message == "existing target operations require a frozen content digest"
        ));
    }

    #[test]
    fn runtime_contract_rejects_missing_close_limit_without_a_default() {
        let value = serde_json::json!({
            "version": 1,
            "available_roles": [],
            "max_concurrent_roles": 1,
            "separate_contexts": true,
            "file_reading": true,
            "tool_execution": false,
            "deterministic_validation": true,
            "max_role_bundle_bytes": 1024,
            "max_role_invocation_bytes": 1024,
            "lifecycle": {
                "max_role_execution_millis": 10,
                "max_role_grace_millis": 1,
                "max_total_role_millis": 12
            }
        });
        assert!(serde_json::from_value::<HarnessRuntimeCapabilities>(value).is_err());
    }
}
