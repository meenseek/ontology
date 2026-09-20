//! The Core's bounded source I/O port. Providers expose named paths, never owner selection.
use std::{
    io::{Seek, SeekFrom},
    sync::Arc,
};

use super::*;

const MAX_SOURCE_CHILDREN: usize = 100_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceStoreIdentity {
    pub store_id: String,
}

impl SourceStoreIdentity {
    pub(super) fn validate(&self) -> HarnessResult<()> {
        validate_source_identity(&self.store_id)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case", deny_unknown_fields)]
pub enum StoredSourceState {
    Missing,
    Live {
        material_id: String,
        revision: u64,
        content_digest: String,
    },
    Deleted {
        material_id: String,
        revision: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceVersion {
    pub logical_path: String,
    pub state: StoredSourceState,
}

impl SourceVersion {
    pub fn validate(&self) -> HarnessResult<()> {
        validate_source_path(Path::new(&self.logical_path))?;
        match &self.state {
            StoredSourceState::Missing => Ok(()),
            StoredSourceState::Live {
                material_id,
                revision,
                content_digest,
            } => {
                validate_material_revision(material_id, *revision)?;
                if content_digest.len() != 64
                    || !content_digest
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    return Err(invalid_source(
                        "stored content digest must be lowercase SHA256",
                    ));
                }
                Ok(())
            }
            StoredSourceState::Deleted {
                material_id,
                revision,
            } => validate_material_revision(material_id, *revision),
        }
    }
}

/// An exact, sorted set. The filesystem contract has neither an identity nor versions.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceVersionSet {
    pub store_identity: Option<SourceStoreIdentity>,
    pub source_root_identity: Option<String>,
    pub versions: Vec<SourceVersion>,
}

impl SourceVersionSet {
    pub fn validate(&self) -> HarnessResult<()> {
        if let Some(identity) = &self.store_identity {
            identity.validate()?;
            let root = self.source_root_identity.as_deref().ok_or_else(|| {
                invalid_source("stored source identity requires its view-root identity")
            })?;
            validate_source_identity(root)?;
        } else if !self.versions.is_empty() || self.source_root_identity.is_some() {
            return Err(invalid_source("source versions require a store identity"));
        }
        let mut previous = None;
        let mut materials = BTreeSet::new();
        for version in &self.versions {
            version.validate()?;
            if previous.is_some_and(|path: &str| path >= version.logical_path.as_str()) {
                return Err(invalid_source(
                    "source versions must have unique sorted logical paths",
                ));
            }
            previous = Some(version.logical_path.as_str());
            match &version.state {
                StoredSourceState::Live { material_id, .. }
                | StoredSourceState::Deleted { material_id, .. } => {
                    if !materials.insert(material_id) {
                        return Err(invalid_source(
                            "source material identity is shared by different paths",
                        ));
                    }
                }
                StoredSourceState::Missing => {}
            }
        }
        Ok(())
    }

    pub(super) fn require_superset(&self, bound: &Self) -> HarnessResult<()> {
        self.validate()?;
        bound.validate()?;
        if self.store_identity != bound.store_identity
            || self.source_root_identity != bound.source_root_identity
            || bound.versions.iter().any(|version| {
                self.versions
                    .binary_search_by(|current| current.logical_path.cmp(&version.logical_path))
                    .ok()
                    .and_then(|index| self.versions.get(index))
                    != Some(version)
            })
        {
            return Err(invalid_source(
                "prepared source versions do not preserve the resolved source contract",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourcePathKind {
    Missing,
    RegularFile,
    Directory,
    Symlink,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceMetadata {
    pub kind: SourcePathKind,
    pub stored_version: Option<SourceVersion>,
}

impl SourceMetadata {
    pub(super) const fn exists(&self) -> bool {
        !matches!(self.kind, SourcePathKind::Missing)
    }
    pub(super) const fn is_file(&self) -> bool {
        matches!(self.kind, SourcePathKind::RegularFile)
    }
}

/// Implementations must own a canonical view root. Metadata and child enumeration must not
/// acquire source bodies. `open_file` may materialize only the exact named file in that view;
/// it must enforce the caller's byte limit and return a regular, unaliased file. No method may
/// route owners, expand scope, create source records, or mutate stored source revisions.
pub trait ContextSource: Send + Sync {
    fn view_root(&self) -> &Path;
    fn store_identity(&self) -> HarnessResult<Option<SourceStoreIdentity>>;
    fn metadata(&self, relative_path: &Path) -> HarnessResult<SourceMetadata>;
    /// Prepare an authorized, exact file write target in the disposable source view before
    /// its locator is bound. Preserve parent directory identities and stored source state.
    /// The default retains the provider's ordinary metadata behavior without reading a body.
    fn prepare_file_target(&self, relative_path: &Path) -> HarnessResult<SourceMetadata> {
        self.metadata(relative_path)
    }
    fn children(
        &self,
        relative_directory: &Path,
        max_entries: usize,
    ) -> HarnessResult<Vec<OsString>>;
    fn open_file(&self, relative_path: &Path, max_bytes: u64) -> HarnessResult<File>;
    /// Validate existence, regular-file type, and the no-symlink/no-hardlink contract without
    /// acquiring a body. Stored providers return the same exact metadata as `metadata`.
    fn validate_regular_file(&self, relative_path: &Path) -> HarnessResult<SourceMetadata>;
}

#[derive(Clone, Debug)]
pub struct FilesystemContextSource {
    root: PathBuf,
}

impl FilesystemContextSource {
    pub fn open(root: impl AsRef<Path>) -> HarnessResult<Self> {
        Ok(Self {
            root: canonical_directory(root.as_ref(), "source view root")?,
        })
    }

    fn file_metadata(&self, relative: &Path) -> HarnessResult<Option<Metadata>> {
        validate_source_path(relative)?;
        // Validate existing parents even when the final path is absent. Do not follow aliases.
        let mut current = self.root.clone();
        for component in relative.components() {
            current.push(component);
            match fs::symlink_metadata(&current) {
                Ok(metadata) => {
                    if current != self.root.join(relative) && metadata.file_type().is_symlink() {
                        return Err(HarnessError::PathEscapesRoot(current));
                    }
                    if current == self.root.join(relative) {
                        return Ok(Some(metadata));
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(source_read_error(&current, error)),
            }
        }
        Err(invalid_source("source path is empty"))
    }
}

impl ContextSource for FilesystemContextSource {
    fn view_root(&self) -> &Path {
        &self.root
    }
    fn store_identity(&self) -> HarnessResult<Option<SourceStoreIdentity>> {
        Ok(None)
    }
    fn metadata(&self, relative: &Path) -> HarnessResult<SourceMetadata> {
        let kind = match self.file_metadata(relative)? {
            None => SourcePathKind::Missing,
            Some(metadata) if metadata.file_type().is_symlink() => SourcePathKind::Symlink,
            Some(metadata) if metadata.is_file() => SourcePathKind::RegularFile,
            Some(metadata) if metadata.is_dir() => SourcePathKind::Directory,
            Some(_) => SourcePathKind::Other,
        };
        Ok(SourceMetadata {
            kind,
            stored_version: None,
        })
    }
    fn children(&self, relative: &Path, max_entries: usize) -> HarnessResult<Vec<OsString>> {
        validate_source_path(relative)?;
        if max_entries == 0 || max_entries > MAX_SOURCE_CHILDREN {
            return Err(invalid_source("source directory entry limit is invalid"));
        }
        let path = self.root.join(relative);
        ensure_no_symlink_components(&self.root, relative)?;
        let metadata = self
            .file_metadata(relative)?
            .ok_or_else(|| missing_source(&path))?;
        if !metadata.is_dir() {
            return Err(invalid_source("source child lookup requires a directory"));
        }
        #[cfg(unix)]
        {
            let directory = File::open(&path).map_err(|error| source_read_error(&path, error))?;
            let opened = directory
                .metadata()
                .map_err(|error| source_read_error(&path, error))?;
            if opened.dev() != metadata.dev() || opened.ino() != metadata.ino() {
                return Err(HarnessError::PathEscapesRoot(path));
            }
            ensure_no_symlink_components(&self.root, relative)?;
            read_directory_names(directory, max_entries)
                .map_err(|error| source_read_error(&path, error))
        }
        #[cfg(not(unix))]
        Err(invalid_source("secure source traversal requires Unix"))
    }
    fn open_file(&self, relative: &Path, max_bytes: u64) -> HarnessResult<File> {
        validate_source_path(relative)?;
        let file = open_verified_file(&self.root, relative)?;
        validate_open_source_file(&file, &self.root.join(relative), max_bytes)?;
        Ok(file)
    }
    fn validate_regular_file(&self, relative: &Path) -> HarnessResult<SourceMetadata> {
        let path = self.root.join(relative);
        let metadata = self
            .file_metadata(relative)?
            .ok_or_else(|| missing_source(&path))?;
        if metadata.file_type().is_symlink() {
            return Err(HarnessError::PathEscapesRoot(path));
        }
        if !metadata.is_file() {
            return Err(invalid_source("source must be a regular file"));
        }
        #[cfg(unix)]
        if metadata.nlink() != 1 {
            return Err(HarnessError::InvalidRequest(format!(
                "scoped file `{}` must have exactly one hard link",
                path.display()
            )));
        }
        ensure_no_symlink_components(&self.root, relative)?;
        Ok(SourceMetadata {
            kind: SourcePathKind::RegularFile,
            stored_version: None,
        })
    }
}

impl VaultRepository {
    pub(super) fn with_source(
        root: impl AsRef<Path>,
        source: Arc<dyn ContextSource>,
    ) -> HarnessResult<Self> {
        let root = canonical_directory(root.as_ref(), "Vault repository root")?;
        if source.view_root() != root {
            return Err(invalid_source(
                "source view root must equal the canonical repository root",
            ));
        }
        let store_identity = source.store_identity()?;
        if let Some(identity) = &store_identity {
            identity.validate()?;
        }
        let repository = Self {
            root,
            source,
            store_identity,
        };
        if repository.source_metadata(Path::new("vault"))?.kind != SourcePathKind::Directory {
            return Err(HarnessError::InvalidRepository(format!(
                "Vault repository root `{}` does not contain `vault`",
                repository.root.display()
            )));
        }
        Ok(repository)
    }

    fn check_source_identity(&self) -> HarnessResult<()> {
        if self.source.view_root() != self.root
            || self.source.store_identity()? != self.store_identity
        {
            return Err(invalid_source("source store identity or view root changed"));
        }
        Ok(())
    }

    pub(super) fn source_metadata(&self, relative: &Path) -> HarnessResult<SourceMetadata> {
        self.checked_source_metadata(relative, || self.source.metadata(relative))
    }

    pub(super) fn prepare_file_target(&self, relative: &Path) -> HarnessResult<SourceMetadata> {
        self.checked_source_metadata(relative, || self.source.prepare_file_target(relative))
    }

    fn checked_source_metadata(
        &self,
        relative: &Path,
        read: impl FnOnce() -> HarnessResult<SourceMetadata>,
    ) -> HarnessResult<SourceMetadata> {
        validate_source_path(relative)?;
        self.check_source_identity()?;
        let metadata = read()?;
        self.validate_source_metadata(relative, &metadata)?;
        self.check_source_identity()?;
        Ok(metadata)
    }

    fn validate_source_metadata(
        &self,
        relative: &Path,
        metadata: &SourceMetadata,
    ) -> HarnessResult<()> {
        match (
            &self.store_identity,
            &metadata.stored_version,
            metadata.kind,
        ) {
            (None, None, _) | (Some(_), None, SourcePathKind::Directory) => Ok(()),
            (Some(_), Some(version), kind) => {
                version.validate()?;
                if Path::new(&version.logical_path) != relative
                    || !matches!(
                        (&version.state, kind),
                        (
                            StoredSourceState::Missing | StoredSourceState::Deleted { .. },
                            SourcePathKind::Missing
                        ) | (StoredSourceState::Live { .. }, SourcePathKind::RegularFile)
                    )
                {
                    return Err(invalid_source(
                        "source metadata path, kind and stored state contradict each other",
                    ));
                }
                Ok(())
            }
            _ => Err(invalid_source(
                "source metadata must match the provider's stored identity contract",
            )),
        }
    }

    pub(super) fn validate_source_file(&self, relative: &Path) -> HarnessResult<()> {
        let expected = self.source_metadata(relative)?;
        self.require_regular_metadata(relative, &expected)?;
        let actual = self.source.validate_regular_file(relative)?;
        self.validate_source_metadata(relative, &actual)?;
        if actual != expected {
            return Err(invalid_source(
                "source metadata changed during regular-file validation",
            ));
        }
        self.check_source_identity()
    }

    fn require_regular_metadata(
        &self,
        relative: &Path,
        metadata: &SourceMetadata,
    ) -> HarnessResult<()> {
        match metadata.kind {
            SourcePathKind::RegularFile => Ok(()),
            SourcePathKind::Missing => Err(missing_source(&self.root.join(relative))),
            SourcePathKind::Symlink => Err(HarnessError::PathEscapesRoot(self.root.join(relative))),
            _ => Err(invalid_source("source must be a regular file")),
        }
    }

    pub(super) fn open_source_file(&self, relative: &Path, max_bytes: u64) -> HarnessResult<File> {
        let expected = self.source_metadata(relative)?;
        self.require_regular_metadata(relative, &expected)?;
        let path = self.root.join(relative);
        let mut file = self.source.open_file(relative, max_bytes)?;
        validate_open_source_file(&file, &path, max_bytes)?;
        // A provider cannot substitute an unrelated descriptor or generated bytes for this path.
        let named = open_verified_file(&self.root, relative)?;
        let actual = file
            .metadata()
            .map_err(|error| source_read_error(&path, error))?;
        let named_metadata = named
            .metadata()
            .map_err(|error| source_read_error(&path, error))?;
        if !repository::same_file(&actual, &named_metadata) {
            return Err(HarnessError::PathEscapesRoot(path));
        }
        if let Some(SourceVersion {
            state: StoredSourceState::Live { content_digest, .. },
            ..
        }) = &expected.stored_version
        {
            let copy = file
                .try_clone()
                .map_err(|error| source_read_error(&path, error))?;
            file.seek(SeekFrom::Start(0))
                .map_err(|error| source_read_error(&path, error))?;
            let digest = digest_file(copy, max_bytes, &path)?;
            if &digest != content_digest {
                return Err(HarnessError::PlanDrift {
                    expected: content_digest.clone(),
                    actual: digest,
                });
            }
            file.seek(SeekFrom::Start(0))
                .map_err(|error| source_read_error(&path, error))?;
        }
        if self.source_metadata(relative)? != expected {
            return Err(invalid_source(
                "source version changed while its body was opened",
            ));
        }
        Ok(file)
    }

    pub(super) fn source_children(&self, relative: &Path) -> HarnessResult<Vec<OsString>> {
        validate_source_path(relative)?;
        self.check_source_identity()?;
        let mut names = self.source.children(relative, MAX_SOURCE_CHILDREN)?;
        if names.len() > MAX_SOURCE_CHILDREN {
            return Err(invalid_source("source directory exceeds its child limit"));
        }
        for name in &names {
            let path = Path::new(name);
            if path.components().count() != 1
                || !matches!(path.components().next(), Some(Component::Normal(_)))
            {
                return Err(invalid_source("source directory returned a non-child path"));
            }
            validate_source_path(path)?;
        }
        names.sort();
        if names.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(invalid_source(
                "source directory returned duplicate children",
            ));
        }
        self.check_source_identity()?;
        Ok(names)
    }

    pub(super) fn existing_context_roots(
        &self,
        candidates: impl IntoIterator<Item = String>,
    ) -> HarnessResult<Vec<ContextRoot>> {
        candidates
            .into_iter()
            .filter_map(
                |candidate| match self.source_metadata(Path::new(&candidate)) {
                    Ok(metadata) if metadata.exists() => {
                        Some(self.require_context_path(&candidate))
                    }
                    Ok(_) => None,
                    Err(error) => Some(Err(error)),
                },
            )
            .collect()
    }

    fn bind_source_versions(
        &self,
        paths: &BTreeMap<String, Option<String>>,
    ) -> HarnessResult<SourceVersionSet> {
        self.check_source_identity()?;
        let mut set = SourceVersionSet {
            store_identity: self.store_identity.clone(),
            source_root_identity: self
                .store_identity
                .as_ref()
                .map(|_| requirements::workspace_root_identity(&self.root))
                .transpose()?,
            versions: Vec::new(),
        };
        if self.store_identity.is_some() {
            for (path, digest) in paths {
                let metadata = self.source_metadata(Path::new(path))?;
                let version = metadata
                    .stored_version
                    .ok_or_else(|| invalid_source("bound source path has no stored state"))?;
                let matches = match (&version.state, digest) {
                    (StoredSourceState::Live { content_digest, .. }, Some(expected)) => {
                        content_digest == expected
                    }
                    (StoredSourceState::Missing | StoredSourceState::Deleted { .. }, None) => true,
                    _ => false,
                };
                if !matches {
                    return Err(invalid_source(
                        "stored source state does not match the exact bound document or target",
                    ));
                }
                set.versions.push(version);
            }
        }
        set.validate()?;
        self.check_source_identity()?;
        Ok(set)
    }
}

impl HarnessEngine {
    /// Use the Core selector with a source provider whose owned view has this exact canonical root.
    pub fn with_source(
        vault_repository_root: impl AsRef<Path>,
        workspace_root: impl AsRef<Path>,
        source: Arc<dyn ContextSource>,
    ) -> HarnessResult<Self> {
        Self::with_source_and_router(
            vault_repository_root,
            workspace_root,
            source,
            HarnessRouter::default(),
        )
    }

    pub fn with_source_and_router(
        vault_repository_root: impl AsRef<Path>,
        workspace_root: impl AsRef<Path>,
        source: Arc<dyn ContextSource>,
        router: HarnessRouter,
    ) -> HarnessResult<Self> {
        #[cfg(unix)]
        {
            Ok(Self {
                router,
                vault: VaultRepository::with_source(vault_repository_root, source)?,
                workspace_root: canonical_directory(workspace_root.as_ref(), "workspace root")?,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = (vault_repository_root, workspace_root, source, router);
            Err(invalid_source(
                "the Harness source port currently requires Unix file safety",
            ))
        }
    }

    pub(super) fn uses_source_document(
        &self,
        source: HarnessBoundDocumentSource,
        path: &str,
    ) -> bool {
        bound_document_repository(source) == 1
            || (self.workspace_root == self.vault.root && path.starts_with("vault/"))
    }

    pub(super) fn plan_source_versions(
        &self,
        plan: &ResolvedHarnessPlan,
    ) -> HarnessResult<SourceVersionSet> {
        self.vault
            .bind_source_versions(&self.plan_source_paths(plan)?)
    }

    fn plan_source_paths(
        &self,
        plan: &ResolvedHarnessPlan,
    ) -> HarnessResult<BTreeMap<String, Option<String>>> {
        let mut paths = BTreeMap::new();
        for policy in plan
            .required_policies
            .iter()
            .chain(&plan.orchestrator_policies)
            .chain(
                plan.role_policy_bindings
                    .iter()
                    .flat_map(|binding| &binding.policies),
            )
        {
            insert_source_path(
                &mut paths,
                &policy.repository_relative_path,
                Some(&policy.content_digest),
            )?;
        }
        for source in plan.evidence_sources.iter().chain(&plan.curation_sources) {
            insert_source_path(
                &mut paths,
                &source.repository_relative_path,
                Some(&source.content_digest),
            )?;
        }
        for source in &plan.learning_sources {
            insert_source_path(
                &mut paths,
                &source.repository_relative_path,
                Some(&source.content_digest),
            )?;
        }
        for target in &plan.targets {
            if self.uses_source_document(
                HarnessBoundDocumentSource::Target,
                &target.workspace_relative_path,
            ) {
                let digest = match &target.state {
                    TargetState::Absent => None,
                    TargetState::Existing { content_digest } => Some(content_digest.as_str()),
                };
                insert_source_path(&mut paths, &target.workspace_relative_path, digest)?;
            }
        }
        for policy in &plan.workspace_policies {
            if self.uses_source_document(
                HarnessBoundDocumentSource::WorkspacePolicy,
                &policy.workspace_relative_path,
            ) {
                insert_source_path(
                    &mut paths,
                    &policy.workspace_relative_path,
                    Some(&policy.content_digest),
                )?;
            }
        }
        Ok(paths)
    }

    pub(super) fn prepared_source_versions(
        &self,
        resolved: &ResolvedHarnessRequest,
        context: Option<&HarnessContextBundle>,
        bundles: &[HarnessRoleBundle],
    ) -> HarnessResult<SourceVersionSet> {
        let mut paths = self.plan_source_paths(&resolved.plan)?;
        if let Some(context) = context {
            for document in &context.documents {
                insert_source_path(
                    &mut paths,
                    &document.repository_relative_path,
                    Some(&document.content_digest),
                )?;
            }
        }
        for document in bundles.iter().flat_map(HarnessRoleBundle::bound_documents) {
            if self.uses_source_document(document.source, &document.relative_path) {
                insert_source_path(
                    &mut paths,
                    &document.relative_path,
                    Some(&document.content_digest),
                )?;
            }
        }
        let versions = self.vault.bind_source_versions(&paths)?;
        versions.require_superset(&resolved.plan.source_versions)?;
        Ok(versions)
    }

    pub(super) fn push_role_document(
        &self,
        documents: &mut Vec<HarnessBoundDocument>,
        remaining: &mut usize,
        path: &str,
        digest: &str,
        limit: u64,
        source: HarnessBoundDocumentSource,
    ) -> HarnessResult<()> {
        let root = if bound_document_repository(source) == 1 {
            &self.vault.root
        } else {
            &self.workspace_root
        };
        if !self.uses_source_document(source, path) {
            return push_bound_document(documents, remaining, root, path, digest, limit, source);
        }
        if contains_bound_document(documents, source, path, digest, 1, usize::MAX) {
            return Ok(());
        }
        let file = self.vault.open_scoped_file(Path::new(path), limit)?;
        let document = read_bound_document_from_file(
            file,
            &root.join(path),
            path,
            digest,
            limit,
            *remaining,
            source,
        )?;
        *remaining = remaining
            .checked_sub(document.content.len())
            .ok_or_else(|| invalid_source("role content size overflow"))?;
        documents.push(document);
        Ok(())
    }
}

impl ResolvedHarnessPlan {
    #[must_use]
    pub const fn bound_source_versions(&self) -> &SourceVersionSet {
        &self.source_versions
    }
}
impl PreparedRoleRun {
    #[must_use]
    pub const fn bound_source_versions(&self) -> &SourceVersionSet {
        &self.source_versions
    }
}

fn insert_source_path(
    paths: &mut BTreeMap<String, Option<String>>,
    path: &str,
    digest: Option<&str>,
) -> HarnessResult<()> {
    let value = digest.map(str::to_owned);
    if paths.get(path).is_some_and(|previous| previous != &value) {
        return Err(invalid_source(
            "source path has contradictory document bindings",
        ));
    }
    paths.insert(path.to_owned(), value);
    Ok(())
}
fn validate_material_revision(identity: &str, revision: u64) -> HarnessResult<()> {
    validate_source_identity(identity)?;
    if revision == 0 {
        return Err(invalid_source("stored source revision must be positive"));
    }
    Ok(())
}
fn validate_source_identity(identity: &str) -> HarnessResult<()> {
    if identity.is_empty()
        || identity.len() > 128
        || !identity
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.'))
    {
        return Err(invalid_source(
            "stored source identity must be a bounded opaque identifier",
        ));
    }
    Ok(())
}
fn validate_source_path(path: &Path) -> HarnessResult<()> {
    validate_relative_path(path, "source path")?;
    let value = path
        .to_str()
        .ok_or_else(|| invalid_source("source path must be UTF-8"))?;
    if value.len() > MAX_TARGET_LENGTH
        || value.contains('\\')
        || value.chars().any(char::is_control)
        || value
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(invalid_source("source path must be bounded and normalized"));
    }
    Ok(())
}
fn validate_open_source_file(file: &File, path: &Path, limit: u64) -> HarnessResult<()> {
    ensure_single_hard_link(file, path)?;
    let metadata = file
        .metadata()
        .map_err(|error| source_read_error(path, error))?;
    if !metadata.is_file() {
        return Err(invalid_source("opened source must be a regular file"));
    }
    if metadata.len() > limit {
        return Err(HarnessError::InvalidRepository(format!(
            "file `{}` exceeds the {limit} byte limit",
            path.display()
        )));
    }
    Ok(())
}
fn invalid_source(message: &str) -> HarnessError {
    HarnessError::InvalidRepository(message.to_owned())
}
fn source_read_error(path: &Path, error: io::Error) -> HarnessError {
    HarnessError::FileRead {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}
fn missing_source(path: &Path) -> HarnessError {
    source_read_error(path, io::Error::from(io::ErrorKind::NotFound))
}
