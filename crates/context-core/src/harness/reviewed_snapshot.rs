//! Preserve a reviewed historical artifact at its recorded evidence epoch.
//! Current access, native target state and commit CAS remain separate obligations.
use super::*;
use std::sync::Arc;

struct RecordedSource {
    current: Arc<dyn ContextSource>,
    versions: SourceVersionSet,
    inventory_cache: std::sync::Mutex<
        Option<(
            crate::career::CareerInventory,
            BTreeMap<String, crate::career::CareerOriginal>,
        )>,
    >,
}

impl RecordedSource {
    fn version(&self, path: &Path) -> Option<&SourceVersion> {
        self.versions
            .versions
            .iter()
            .find(|v| Path::new(&v.logical_path) == path)
    }
}

impl ContextSource for RecordedSource {
    fn view_root(&self) -> &Path {
        self.current.view_root()
    }
    fn store_identity(&self) -> HarnessResult<Option<SourceStoreIdentity>> {
        self.current.store_identity()
    }
    fn recorded_snapshot(&self) -> bool {
        true
    }
    fn current_source_versions(&self, paths: &[PathBuf]) -> HarnessResult<Vec<SourceVersion>> {
        self.current.current_source_versions(paths)
    }
    fn metadata(&self, path: &Path) -> HarnessResult<SourceMetadata> {
        match self.version(path) {
            Some(v) if matches!(v.state, StoredSourceState::Live { .. }) => {
                self.current.historical_metadata(v)
            }
            _ => self.current.metadata(path),
        }
    }
    fn prepare_file_target(&self, path: &Path) -> HarnessResult<SourceMetadata> {
        self.current.prepare_file_target(path)
    }
    fn children(&self, path: &Path, limit: usize) -> HarnessResult<Vec<OsString>> {
        if self.current.metadata(path)?.kind != SourcePathKind::Directory {
            return Err(HarnessError::InvalidSubmission(
                "recorded directory is unavailable".into(),
            ));
        }
        let prefix = path
            .to_str()
            .ok_or_else(|| HarnessError::InvalidSubmission("invalid recorded directory".into()))?;
        let prefix = if prefix.is_empty() {
            String::new()
        } else {
            format!("{prefix}/")
        };
        let children = self
            .versions
            .versions
            .iter()
            .filter(|v| matches!(v.state, StoredSourceState::Live { .. }))
            .filter_map(|v| v.logical_path.strip_prefix(&prefix))
            .filter_map(|v| v.split('/').next())
            .filter(|v| !v.is_empty())
            .map(OsString::from)
            .collect::<BTreeSet<_>>();
        if children.len() > limit {
            return Err(HarnessError::InvalidSubmission(
                "recorded children exceed limit".into(),
            ));
        }
        Ok(children.into_iter().collect())
    }
    fn open_file(&self, path: &Path, limit: u64) -> HarnessResult<File> {
        match self.version(path) {
            Some(v) if matches!(v.state, StoredSourceState::Live { .. }) => {
                self.current.open_historical_file(v, limit)
            }
            _ => self.current.open_file(path, limit),
        }
    }
    fn validate_regular_file(&self, path: &Path) -> HarnessResult<SourceMetadata> {
        let metadata = self.metadata(path)?;
        if metadata.kind != SourcePathKind::RegularFile {
            return Err(HarnessError::InvalidSubmission(
                "recorded source is not a regular file".into(),
            ));
        }
        Ok(metadata)
    }
    fn career_inventory(
        &self,
        configuration: &PolicyConfiguration,
        comparison: &crate::career::CareerComparison,
    ) -> HarnessResult<crate::career::CareerInventory> {
        let mut cache = self.inventory_cache.lock().map_err(|_| {
            HarnessError::InvalidSubmission("recorded inventory cache poisoned".into())
        })?;
        for version in &self.versions.versions {
            if matches!(version.state, StoredSourceState::Live { .. }) {
                self.current.historical_metadata(version)?;
            }
        }
        if let Some((inventory, originals)) = cache.as_ref() {
            comparison.validate_current(inventory, originals, true)?;
            return Ok(inventory.clone());
        }
        let mut originals = BTreeMap::new();
        let inventory = crate::career::discover(
            configuration,
            self.store_identity()?.map(|s| s.store_id),
            comparison.inventory.request.clone(),
            |paths| {
                paths
                    .iter()
                    .map(|path| {
                        let (content, _) = repository::read_text_file(
                            self.open_file(
                                Path::new(path),
                                crate::career::MAX_CAREER_SOURCE_BYTES as u64,
                            )?,
                            crate::career::MAX_CAREER_SOURCE_BYTES as u64,
                            &self.view_root().join(path),
                        )?;
                        let original = crate::career::CareerOriginal {
                            path: path.clone(),
                            content,
                            version: self.metadata(Path::new(path))?.stored_version,
                        };
                        originals.insert(path.clone(), original.clone());
                        Ok(original)
                    })
                    .collect()
            },
        )?;
        comparison.validate_current(&inventory, &originals, true)?;
        *cache = Some((inventory.clone(), originals));
        Ok(inventory)
    }
}

pub(super) fn historical_body(content: &str) -> bool {
    let Ok(Some(metadata)) = crate::frontmatter::ParsedFrontmatter::from_markdown(content) else {
        return false;
    };
    if !matches!(metadata.optional_bool("export"), Ok(Some(false))) {
        return false;
    }
    if metadata
        .optional_string("artifact_role")
        .ok()
        .flatten()
        .as_deref()
        != Some("historical_snapshot")
        || metadata
            .optional_string("artifact_status")
            .ok()
            .flatten()
            .as_deref()
            != Some("body_frozen")
        || metadata
            .optional_string("reuse_policy")
            .ok()
            .flatten()
            .as_deref()
            != Some("reference_only_no_copy_forward")
    {
        return false;
    }
    let Some(frontmatter) = content
        .strip_prefix("---\n")
        .and_then(|s| s.split_once("\n---\n").map(|v| v.0))
    else {
        return false;
    };
    let required = [
        "artifact_role: historical_snapshot",
        "export: false",
        "artifact_status: body_frozen",
        "reuse_policy: reference_only_no_copy_forward",
    ];
    required.iter().all(|required| {
        let key = required.split_once(':').unwrap().0;
        let declarations = frontmatter
            .lines()
            .filter(|line| {
                line.split_once(':')
                    .is_some_and(|(name, _)| name.trim() == key)
            })
            .collect::<Vec<_>>();
        declarations.as_slice() == [*required]
    })
}

pub(super) fn review_versions(
    prepared: &PreparedHarnessRun,
    holistic: &CareerExecutionReviewReceipt,
    owners: &[CareerExecutionReviewReceipt],
) -> HarnessResult<SourceVersionSet> {
    let mut result = prepared.role_run.source_versions.clone();
    let mut versions = result
        .versions
        .iter()
        .map(|v| (v.logical_path.clone(), v.clone()))
        .collect::<BTreeMap<_, _>>();
    for set in std::iter::once(&holistic.prepared.source_versions)
        .chain(owners.iter().map(|r| &r.prepared.source_versions))
    {
        if set.store_identity != result.store_identity
            || set.source_root_identity != result.source_root_identity
        {
            return Err(HarnessError::InvalidSubmission(
                "recorded reviews have different stores or roots".into(),
            ));
        }
        for version in &set.versions {
            if versions
                .get(&version.logical_path)
                .is_some_and(|old| old != version)
            {
                return Err(HarnessError::InvalidSubmission(
                    "recorded reviews have inconsistent source epochs".into(),
                ));
            }
            versions.insert(version.logical_path.clone(), version.clone());
        }
    }
    result.versions = versions.into_values().collect();
    result.validate()?;
    Ok(result)
}

pub(super) fn source_versions_from_reviews(
    holistic: &CareerExecutionReviewReceipt,
    owners: &[CareerExecutionReviewReceipt],
) -> HarnessResult<SourceVersionSet> {
    let mut result = holistic.prepared.source_versions.clone();
    let mut versions = result
        .versions
        .iter()
        .map(|v| (v.logical_path.clone(), v.clone()))
        .collect::<BTreeMap<_, _>>();
    for owner in owners {
        let source = &owner.prepared.source_versions;
        if source.store_identity != result.store_identity
            || source.source_root_identity != result.source_root_identity
        {
            return Err(HarnessError::InvalidSubmission(
                "recorded review store/root differs".into(),
            ));
        }
        for version in &source.versions {
            if versions
                .get(&version.logical_path)
                .is_some_and(|old| old != version)
            {
                return Err(HarnessError::InvalidSubmission(
                    "recorded review epochs differ".into(),
                ));
            }
            versions.insert(version.logical_path.clone(), version.clone());
        }
    }
    result.versions = versions.into_values().collect();
    result.validate()?;
    Ok(result)
}

impl HarnessEngine {
    pub fn with_recorded_career_sources(&self, versions: &SourceVersionSet) -> HarnessResult<Self> {
        versions.validate()?;
        if versions.store_identity.is_none()
            || versions.store_identity != self.vault.store_identity
            || versions.source_root_identity.as_deref()
                != Some(requirements::workspace_root_identity(&self.vault.root)?.as_str())
        {
            return Err(HarnessError::InvalidSubmission(
                "recorded source store/root is not the current authority".into(),
            ));
        }
        for version in &versions.versions {
            if matches!(version.state, StoredSourceState::Live { .. }) {
                self.vault.source.historical_metadata(version)?;
            }
        }
        let source = Arc::new(RecordedSource {
            current: self.vault.source.clone(),
            versions: versions.clone(),
            inventory_cache: std::sync::Mutex::new(None),
        });
        Self::with_source_and_router(
            &self.vault.root,
            &self.workspace_root,
            source,
            self.router.clone(),
            self.vault.policy_configuration.clone(),
        )
    }

    pub fn career_preservation_engine(
        &self,
        prepared: &PreparedHarnessRun,
        review_workspace: &Path,
        holistic: &CareerExecutionReviewReceipt,
        owners: &[CareerExecutionReviewReceipt],
    ) -> HarnessResult<Self> {
        let root = canonical_directory(review_workspace, "review workspace")?;
        let mut historical = true;
        for target in &holistic.resolved.plan.targets {
            let (content, _) = repository::read_text_file(
                open_verified_file(&root, Path::new(&target.workspace_relative_path))?,
                MAX_TARGET_BYTES,
                &root.join(&target.workspace_relative_path),
            )?;
            historical &= historical_body(&content);
        }
        if historical && !holistic.resolved.plan.targets.is_empty() {
            self.with_recorded_career_sources(&review_versions(prepared, holistic, owners)?)
        } else {
            Ok(self.clone())
        }
    }

    pub fn career_preservation_engine_from_reviews(
        &self,
        review_workspace: &Path,
        holistic: &CareerExecutionReviewReceipt,
        owners: &[CareerExecutionReviewReceipt],
    ) -> HarnessResult<Self> {
        let root = canonical_directory(review_workspace, "review workspace")?;
        for target in &holistic.resolved.plan.targets {
            let (content, _) = repository::read_text_file(
                open_verified_file(&root, Path::new(&target.workspace_relative_path))?,
                MAX_TARGET_BYTES,
                &root.join(&target.workspace_relative_path),
            )?;
            if !historical_body(&content) {
                return Ok(self.clone());
            }
        }
        if holistic.resolved.plan.targets.is_empty() {
            return Ok(self.clone());
        }
        self.with_recorded_career_sources(&source_versions_from_reviews(holistic, owners)?)
    }
}

#[cfg(test)]
mod tests {
    use super::historical_body;
    #[test]
    fn preservation_kind_requires_private_frozen_reference_metadata() {
        let body = "---\nexport: false\nartifact_role: historical_snapshot\nartifact_status: body_frozen\nreuse_policy: reference_only_no_copy_forward\n---\n\nAn already reviewed fictional snapshot.\n";
        assert!(historical_body(body));
        assert!(!historical_body(
            &body.replace("export: false", "export: true")
        ));
        assert!(!historical_body(&body.replace(
            "export: false",
            "other: \"header\nexport: false\ntrailer\""
        )));
        assert!(!historical_body(&body.replace("body_frozen", "draft")));
        assert!(!historical_body(
            &body.replace("export: false", "export: false\nexport: true")
        ));
        assert!(!historical_body(&body.replace(
            "artifact_role: historical_snapshot",
            "artifact_role: historical_snapshot\nartifact_role : current"
        )));
        assert!(!historical_body(
            &body.replace("export: false", "export: false\ninvalid: [")
        ));
    }
}
