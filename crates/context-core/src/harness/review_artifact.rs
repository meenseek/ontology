//! Explicit output evidence, separate from native sources and writable targets.
use super::*;

const MAX_ARTIFACTS: usize = 16;
const MAX_ARTIFACT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 24 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewArtifactFormat {
    Pdf,
    Png,
    Jpeg,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReviewArtifactBinding {
    pub workspace_relative_path: String,
    pub content_digest: String,
    pub format: ReviewArtifactFormat,
    pub size_bytes: u64,
}

/// Only Core can construct bytes verified against a prepared binding.
#[derive(Debug)]
pub struct FrozenReviewArtifact {
    binding: ReviewArtifactBinding,
    bytes: Vec<u8>,
}

impl FrozenReviewArtifact {
    #[must_use]
    pub fn binding(&self) -> &ReviewArtifactBinding {
        &self.binding
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

fn path_format(path: &str) -> HarnessResult<ReviewArtifactFormat> {
    let relative = Path::new(path);
    validate_single_line("review artifact path", path, MAX_TARGET_LENGTH)?;
    validate_relative_path(relative, "review artifact")?;
    if portable_path(relative) != path
        || !path.starts_with("review-artifacts/")
        || relative.components().any(|part| {
            matches!(part, Component::Normal(name) if name.to_string_lossy().starts_with('.'))
        })
    {
        return Err(HarnessError::InvalidRequest(
            "review artifacts must be ordinary files under workspace review-artifacts/".to_owned(),
        ));
    }
    match relative.extension().and_then(OsStr::to_str) {
        Some("pdf") => Ok(ReviewArtifactFormat::Pdf),
        Some("png") => Ok(ReviewArtifactFormat::Png),
        Some("jpg" | "jpeg") => Ok(ReviewArtifactFormat::Jpeg),
        _ => Err(HarnessError::InvalidRequest(
            "review artifacts support only .pdf, .png, .jpg and .jpeg".to_owned(),
        )),
    }
}

fn validate_paths(paths: &[String], targets: &[TargetBinding]) -> HarnessResult<()> {
    if paths.len() > MAX_ARTIFACTS {
        return Err(HarnessError::InvalidRequest(format!(
            "review artifacts exceed the {MAX_ARTIFACTS} file limit"
        )));
    }
    let mut unique = BTreeSet::new();
    for path in paths {
        path_format(path)?;
        if !unique.insert(path)
            || targets.iter().any(|target| {
                Path::new(path).starts_with(&target.workspace_relative_path)
                    || Path::new(&target.workspace_relative_path).starts_with(path)
            })
        {
            return Err(HarnessError::InvalidRequest(
                "review artifacts must be unique and disjoint from writable or review targets"
                    .to_owned(),
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_bindings(
    artifacts: &[ReviewArtifactBinding],
    targets: &[TargetBinding],
) -> HarnessResult<()> {
    let paths = artifacts
        .iter()
        .map(|a| a.workspace_relative_path.clone())
        .collect::<Vec<_>>();
    validate_paths(&paths, targets)?;
    let mut total = 0_u64;
    for artifact in artifacts {
        validate_plan_digest("review artifact digest", &artifact.content_digest)?;
        if path_format(&artifact.workspace_relative_path)? != artifact.format
            || artifact.size_bytes == 0
            || artifact.size_bytes > MAX_ARTIFACT_BYTES
        {
            return Err(HarnessError::InvalidPlan(
                "invalid review artifact format or size".to_owned(),
            ));
        }
        total += artifact.size_bytes;
    }
    if total > MAX_TOTAL_BYTES || paths.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(HarnessError::InvalidPlan(
            "review artifact bindings must be sorted and within the total byte limit".to_owned(),
        ));
    }
    Ok(())
}

impl HarnessEngine {
    fn read_review_artifact(
        &self,
        path: &str,
    ) -> HarnessResult<(FrozenReviewArtifact, fs::Metadata)> {
        let format = path_format(path)?;
        let absolute = self.workspace_root.join(path);
        let file = open_verified_file(&self.workspace_root, Path::new(path))?;
        ensure_single_hard_link(&file, &absolute)?;
        let identity = file.metadata().map_err(|error| HarnessError::FileRead {
            path: absolute.clone(),
            message: error.to_string(),
        })?;
        let (bytes, content_digest) =
            repository::read_file_bytes(file, MAX_ARTIFACT_BYTES, &absolute)?;
        let valid_signature = match format {
            ReviewArtifactFormat::Pdf => bytes.starts_with(b"%PDF-"),
            ReviewArtifactFormat::Png => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            ReviewArtifactFormat::Jpeg => bytes.starts_with(b"\xff\xd8\xff"),
        };
        if !valid_signature {
            return Err(HarnessError::InvalidRequest(format!(
                "review artifact `{path}` does not match its declared format"
            )));
        }
        let binding = ReviewArtifactBinding {
            workspace_relative_path: path.to_owned(),
            content_digest,
            format,
            size_bytes: bytes.len() as u64,
        };
        Ok((FrozenReviewArtifact { binding, bytes }, identity))
    }

    pub(super) fn bind_review_artifacts(
        &self,
        paths: &[String],
        targets: &[TargetBinding],
    ) -> HarnessResult<Vec<ReviewArtifactBinding>> {
        validate_paths(paths, targets)?;
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let mut paths = paths.to_vec();
        paths.sort();
        let mut total = 0_u64;
        let mut bindings = Vec::new();
        let mut identities = Vec::new();
        for target in targets {
            if matches!(target.state, TargetState::Existing { .. })
                && !self.uses_source_document(
                    HarnessBoundDocumentSource::Target,
                    &target.workspace_relative_path,
                )
            {
                let path = Path::new(&target.workspace_relative_path);
                let file = open_verified_file(&self.workspace_root, path)?;
                identities.push(file.metadata().map_err(|error| HarnessError::FileRead {
                    path: self.workspace_root.join(path),
                    message: error.to_string(),
                })?);
            }
        }
        for path in paths {
            let (artifact, identity) = self.read_review_artifact(&path)?;
            if identities
                .iter()
                .any(|other| repository::same_file(other, &identity))
            {
                return Err(HarnessError::InvalidRequest(
                    "review artifacts must identify distinct files outside all targets".to_owned(),
                ));
            }
            identities.push(identity);
            total += artifact.binding.size_bytes;
            if total > MAX_TOTAL_BYTES {
                return Err(HarnessError::InvalidRequest(
                    "review artifacts exceed the 24 MiB total limit".to_owned(),
                ));
            }
            bindings.push(artifact.binding);
        }
        Ok(bindings)
    }

    /// Freeze current bytes for Core-issued invocations before an adapter releases source reads.
    /// This does not authorize execution or acceptance of caller-generated invocations.
    pub fn freeze_review_artifacts(
        &self,
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
        invocations: &[RoleInvocationContract],
    ) -> HarnessResult<Vec<FrozenReviewArtifact>> {
        self.validate_prepared_run(resolved, prepared)?;
        for invocation in invocations {
            let bindings = invocation
                .segments
                .iter()
                .filter_map(|segment| match segment {
                    RoleInvocationSegment::BoundReviewArtifact { artifact } => Some(artifact),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if invocation.resolved_plan_digest != resolved.resolved_plan_digest
                || invocation.prepared_role_run_digest != prepared.prepared_role_run_digest
                || !resolved.plan.roles().any(|role| role == invocation.role)
                || bindings != resolved.plan.review_artifacts.iter().collect::<Vec<_>>()
            {
                return Err(HarnessError::InvalidSubmission(
                    "invocation review artifacts differ from the prepared scope".to_owned(),
                ));
            }
        }
        resolved
            .plan
            .review_artifacts
            .iter()
            .map(|binding| {
                let (artifact, _) = self.read_review_artifact(&binding.workspace_relative_path)?;
                if &artifact.binding != binding {
                    return Err(HarnessError::PlanDrift {
                        expected: binding.content_digest.clone(),
                        actual: artifact.binding.content_digest,
                    });
                }
                Ok(artifact)
            })
            .collect()
    }
}
