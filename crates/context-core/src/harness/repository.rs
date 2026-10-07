use super::*;
use crate::{atomic_file::copy_permission_metadata, document::validate_context_markdown};

/// This internal capability is selected by the validated source/workspace boundary.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum WorkspacePolicy {
    External,
    NativePrivate,
}
impl WorkspacePolicy {
    fn validate_platform(self) -> HarnessResult<()> {
        if self == Self::NativePrivate && !cfg!(any(target_os = "macos", target_os = "linux")) {
            return Err(HarnessError::InvalidRepository(
                "atomic private Create publication is unsupported on this platform".to_owned(),
            ));
        }
        Ok(())
    }
    fn create_file(self, path: &Path) -> std::io::Result<File> {
        let mut options = fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        if self == Self::NativePrivate {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        options.open(path)
    }
    fn create_directory(self, path: &Path) -> std::io::Result<()> {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        if self == Self::NativePrivate {
            use std::os::unix::fs::DirBuilderExt as _;
            builder.mode(0o700);
        }
        builder.create(path)
    }
    fn copy_permissions(self, source: &Path, target: &Path) -> std::io::Result<()> {
        if self == Self::External {
            copy_permission_metadata(source, target)
        } else {
            Ok(())
        }
    }
    fn publish_create(self, source: &Path, target: &Path) -> std::io::Result<()> {
        if self == Self::External {
            return fs::hard_link(source, target);
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            use std::{ffi::CString, os::unix::ffi::OsStrExt as _};
            let source = CString::new(source.as_os_str().as_bytes())?;
            let target = CString::new(target.as_os_str().as_bytes())?;
            // SAFETY: both NUL-terminated paths live through the syscall; exclusive
            // rename publishes one inode without replacing a foreign destination.
            #[cfg(target_os = "macos")]
            let result =
                unsafe { libc::renamex_np(source.as_ptr(), target.as_ptr(), libc::RENAME_EXCL) };
            #[cfg(target_os = "linux")]
            let result = unsafe {
                libc::renameat2(
                    libc::AT_FDCWD,
                    source.as_ptr(),
                    libc::AT_FDCWD,
                    target.as_ptr(),
                    libc::RENAME_NOREPLACE,
                )
            };
            if result == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "atomic private Create publication is unsupported",
        ))
    }
}

pub(super) fn validate_target_path_is_not_reserved(relative_target: &str) -> HarnessResult<()> {
    if path_is_within(relative_target, BATCH_CONTROL_DIRECTORY)
        || relative_target == ".llm-context-vault-harness.lock"
    {
        return Err(HarnessError::InvalidRequest(
            "target uses a reserved Harness lifecycle path".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn bind_target(
    workspace_root: &Path,
    relative_target: &str,
    action: HarnessAction,
    delete_requested: bool,
) -> HarnessResult<TargetBinding> {
    let relative = Path::new(relative_target);
    validate_relative_path(relative, "target")?;
    validate_target_path_is_not_reserved(relative_target)?;
    ensure_no_symlink_components(workspace_root, relative)?;
    let path = workspace_root.join(relative);
    let (state, operation) = if path.exists() {
        let file = open_verified_file(workspace_root, relative)?;
        ensure_single_hard_link(&file, &path)?;
        let state = TargetState::Existing {
            content_digest: digest_file(file, MAX_TARGET_BYTES, &path)?,
        };
        let operation = if action.is_review() || !action.is_write() {
            TargetOperation::Inspect
        } else if delete_requested {
            TargetOperation::Delete
        } else {
            TargetOperation::Update
        };
        (state, operation)
    } else {
        if delete_requested || !action.is_write() {
            return Err(HarnessError::InvalidRequest(format!(
                "target `{relative_target}` must exist for the requested operation"
            )));
        }
        ensure_existing_ancestor_within_root(workspace_root, &path)?;
        let operation = if action.is_write() {
            TargetOperation::Create
        } else {
            TargetOperation::Inspect
        };
        (TargetState::Absent, operation)
    };
    Ok(TargetBinding {
        workspace_relative_path: portable_path(relative),
        state,
        operation,
        parent_directories_to_create: if operation == TargetOperation::Create {
            missing_parent_directories(workspace_root, relative)?
        } else {
            Vec::new()
        },
    })
}

fn missing_parent_directories(root: &Path, relative: &Path) -> HarnessResult<Vec<String>> {
    let mut current = root.to_path_buf();
    let mut missing = Vec::new();
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    for component in parent.components() {
        let Component::Normal(part) = component else {
            return Err(HarnessError::InvalidRequest(
                "target parent contains an unsupported component".to_owned(),
            ));
        };
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(HarnessError::PathEscapesRoot(current));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let relative = current
                    .strip_prefix(root)
                    .map_err(|_| HarnessError::PathEscapesRoot(current.clone()))?;
                missing.push(portable_path(relative));
            }
            Err(error) => {
                return Err(HarnessError::FileRead {
                    path: current,
                    message: error.to_string(),
                });
            }
        }
    }
    Ok(missing)
}

pub(super) fn validate_curation_targets(
    owner: &DataOwner,
    kind: Option<CurationKind>,
    targets: &[String],
    workspace_root: &Path,
    vault_repository_root: &Path,
) -> HarnessResult<()> {
    if workspace_root != vault_repository_root {
        return Err(HarnessError::InvalidRequest(
            "Vault curation requires the Vault repository as the target workspace".to_owned(),
        ));
    }
    let kind = kind.ok_or_else(|| {
        HarnessError::InvalidRequest("Vault curation requires a curation kind".to_owned())
    })?;
    let allowed_root = curation_root(owner, kind)?;
    for target in targets {
        if !path_is_within(target, &allowed_root) {
            return Err(HarnessError::InvalidRequest(format!(
                "Vault curation target `{target}` is outside owner root `{allowed_root}`"
            )));
        }
    }
    Ok(())
}

pub(super) fn curation_root(owner: &DataOwner, kind: CurationKind) -> HarnessResult<String> {
    let category = match kind {
        CurationKind::Idea => "ideas",
        CurationKind::Knowledge => "knowledge",
        CurationKind::Fact => "facts",
        CurationKind::Decision => "decisions",
        CurationKind::Journal => "journal",
        CurationKind::Ontology => "ontology",
    };
    match (owner, kind) {
        (DataOwner::Personal, CurationKind::Idea) => Ok("vault/personal/projects/ideas".to_owned()),
        (DataOwner::Personal, _) => Ok(format!("vault/personal/{category}")),
        (DataOwner::CommonWork, _) => Err(HarnessError::InvalidRequest(
            "common work does not support Vault curation".to_owned(),
        )),
        (
            DataOwner::PersonalBusiness
            | DataOwner::PersonalProject { .. }
            | DataOwner::Profile
            | DataOwner::Company { .. }
            | DataOwner::CompanyProject { .. },
            CurationKind::Journal | CurationKind::Ontology,
        ) => Err(HarnessError::InvalidRequest(
            "journal and ontology curation require the personal owner".to_owned(),
        )),
        (DataOwner::PersonalProject { project }, _) => {
            Ok(format!("vault/personal/projects/{project}/{category}"))
        }
        (DataOwner::PersonalBusiness, _) => Ok(format!("vault/personal/business/{category}")),
        (DataOwner::Profile, _) => Ok(format!("vault/profile/{category}")),
        (DataOwner::Company { company }, _) => Ok(format!("vault/work/{company}/{category}")),
        (DataOwner::CompanyProject { company, project }, _) => Ok(format!(
            "vault/work/{company}/projects/{project}/{category}"
        )),
    }
}

pub(super) fn ensure_existing_ancestor_within_root(
    root: &Path,
    target: &Path,
) -> HarnessResult<()> {
    let mut current = target.parent();
    while let Some(path) = current {
        if path.exists() {
            let canonical = path
                .canonicalize()
                .map_err(|source| HarnessError::FileRead {
                    path: path.to_path_buf(),
                    message: source.to_string(),
                })?;
            if !canonical.starts_with(root) || !canonical.is_dir() {
                return Err(HarnessError::PathEscapesRoot(target.to_path_buf()));
            }
            return Ok(());
        }
        current = path.parent();
    }
    Err(HarnessError::PathEscapesRoot(target.to_path_buf()))
}

pub(super) fn ensure_no_symlink_components(root: &Path, relative: &Path) -> HarnessResult<()> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err(HarnessError::InvalidRequest(
                "path contains an unsupported component".to_owned(),
            ));
        };
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(HarnessError::PathEscapesRoot(current));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(HarnessError::FileRead {
                    path: current,
                    message: error.to_string(),
                });
            }
        }
    }
    Ok(())
}

pub(super) fn open_verified_file(root: &Path, relative: &Path) -> HarnessResult<File> {
    validate_relative_path(relative, "file path")?;
    ensure_no_symlink_components(root, relative)?;
    let path = root.join(relative);
    let file = File::open(&path).map_err(|source| HarnessError::FileRead {
        path: path.clone(),
        message: source.to_string(),
    })?;
    let opened_metadata = file.metadata().map_err(|source| HarnessError::FileRead {
        path: path.clone(),
        message: source.to_string(),
    })?;
    if !opened_metadata.is_file() {
        return Err(HarnessError::PathEscapesRoot(path));
    }

    let canonical = path
        .canonicalize()
        .map_err(|source| HarnessError::FileRead {
            path: path.clone(),
            message: source.to_string(),
        })?;
    if !canonical.starts_with(root) {
        return Err(HarnessError::PathEscapesRoot(path));
    }
    let canonical_file = File::open(&canonical).map_err(|source| HarnessError::FileRead {
        path: canonical.clone(),
        message: source.to_string(),
    })?;
    let canonical_metadata =
        canonical_file
            .metadata()
            .map_err(|source| HarnessError::FileRead {
                path: canonical,
                message: source.to_string(),
            })?;
    if !same_file(&opened_metadata, &canonical_metadata) {
        return Err(HarnessError::InvalidRepository(
            "file path changed while it was being opened".to_owned(),
        ));
    }
    Ok(file)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LockLifecycleConfirmation {
    Confirmed,
    Unconfirmed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct WorkspaceMutationLockFailure {
    pub(super) stage: String,
    pub(super) message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct WorkspaceMutationLockReport {
    pub(super) lock_file_creation_committed: bool,
    pub(super) lock_content_durability: LockLifecycleConfirmation,
    pub(super) creation_parent_durability: LockLifecycleConfirmation,
    pub(super) explicit_unlink_committed: bool,
    pub(super) verified_final_absence: LockLifecycleConfirmation,
    pub(super) unlink_parent_durability: LockLifecycleConfirmation,
    pub(super) failures: Vec<WorkspaceMutationLockFailure>,
}

impl WorkspaceMutationLockReport {
    fn created() -> Self {
        Self {
            lock_file_creation_committed: true,
            lock_content_durability: LockLifecycleConfirmation::Unconfirmed,
            creation_parent_durability: LockLifecycleConfirmation::Unconfirmed,
            explicit_unlink_committed: false,
            verified_final_absence: LockLifecycleConfirmation::Unconfirmed,
            unlink_parent_durability: LockLifecycleConfirmation::Unconfirmed,
            failures: Vec::new(),
        }
    }

    fn record_failure(&mut self, stage: &str, error: impl std::fmt::Display) {
        self.failures.push(WorkspaceMutationLockFailure {
            stage: stage.to_owned(),
            message: error.to_string(),
        });
    }

    fn acquisition_confirmed(&self) -> bool {
        self.lock_file_creation_committed
            && self.lock_content_durability == LockLifecycleConfirmation::Confirmed
            && self.creation_parent_durability == LockLifecycleConfirmation::Confirmed
            && self.failures.is_empty()
    }
}

pub(super) struct WorkspaceMutationLockAcquisition {
    pub(super) lock: Option<WorkspaceMutationLock>,
    pub(super) report: WorkspaceMutationLockReport,
}

pub(super) struct WorkspaceMutationLock {
    path: PathBuf,
    created_identity: Option<Metadata>,
    root_directory: File,
    armed: bool,
    report: WorkspaceMutationLockReport,
}

impl WorkspaceMutationLock {
    #[cfg(test)]
    pub(super) fn acquire_reported(root: &Path) -> HarnessResult<WorkspaceMutationLockAcquisition> {
        Self::acquire_reported_with_policy(WorkspacePolicy::External, root)
    }

    #[cfg(not(unix))]
    pub(super) fn acquire_reported_with_policy(
        policy: WorkspacePolicy,
        _root: &Path,
    ) -> HarnessResult<WorkspaceMutationLockAcquisition> {
        Err(HarnessError::InvalidRepository(
            "workspace mutation lock lifecycle reporting is unsupported on non-Unix platforms"
                .to_owned(),
        ))
    }

    #[cfg(unix)]
    pub(super) fn acquire_reported_with_policy(
        policy: WorkspacePolicy,
        root: &Path,
    ) -> HarnessResult<WorkspaceMutationLockAcquisition> {
        let root_directory = File::open(root).map_err(|source| HarnessError::FileRead {
            path: root.to_path_buf(),
            message: source.to_string(),
        })?;
        let path = root.join(".llm-context-vault-harness.lock");
        let mut file = policy.create_file(&path)
            .map_err(|source| HarnessError::FileWrite {
                path: path.clone(),
                message: format!(
                    "failed to acquire the workspace mutation lock; remove a stale lock only after confirming no apply is running: {source}"
                ),
            })?;
        let mut report = WorkspaceMutationLockReport::created();
        let created_identity = match file.metadata() {
            Ok(metadata) if metadata.is_file() => Some(metadata),
            Ok(_) => {
                report.record_failure(
                    "lock-created-identity",
                    "created lock descriptor is not a regular file",
                );
                None
            }
            Err(error) => {
                report.record_failure("lock-created-identity", error);
                None
            }
        };
        let content_written =
            match file.write_all(format!("pid={}\n", std::process::id()).as_bytes()) {
                Ok(()) => true,
                Err(error) => {
                    report.record_failure("lock-content-write", error);
                    false
                }
            };
        let content_synced = match file.sync_all() {
            Ok(()) => true,
            Err(error) => {
                report.record_failure("lock-content-durability", error);
                false
            }
        };
        if content_written && content_synced {
            report.lock_content_durability = LockLifecycleConfirmation::Confirmed;
        }
        match root_directory.sync_all() {
            Ok(()) => {
                report.creation_parent_durability = LockLifecycleConfirmation::Confirmed;
            }
            Err(error) => report.record_failure("lock-creation-parent-durability", error),
        }
        drop(file);
        let mut lock = Self {
            path,
            created_identity,
            root_directory,
            armed: true,
            report: report.clone(),
        };
        if let Err(error) = lock.current_path_matches_created_identity() {
            lock.report
                .record_failure("lock-acquisition-identity", error);
        }
        if lock.report.acquisition_confirmed() {
            let report = lock.report.clone();
            Ok(WorkspaceMutationLockAcquisition {
                lock: Some(lock),
                report,
            })
        } else {
            let report = lock.release();
            Ok(WorkspaceMutationLockAcquisition { lock: None, report })
        }
    }

    pub(super) fn release(mut self) -> WorkspaceMutationLockReport {
        match self.current_path_matches_created_identity() {
            Ok(()) => match fs::remove_file(&self.path) {
                Ok(()) => self.report.explicit_unlink_committed = true,
                Err(error) => self.report.record_failure("lock-explicit-unlink", error),
            },
            Err(error) => self.report.record_failure("lock-release-identity", error),
        }
        match fs::symlink_metadata(&self.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.report.verified_final_absence = LockLifecycleConfirmation::Confirmed;
            }
            Ok(_) => self.report.record_failure(
                "lock-final-absence-verification",
                "workspace mutation lock still exists",
            ),
            Err(error) => self
                .report
                .record_failure("lock-final-absence-verification", error),
        }
        match self.root_directory.sync_all() {
            Ok(()) => {
                self.report.unlink_parent_durability = LockLifecycleConfirmation::Confirmed;
            }
            Err(error) => self
                .report
                .record_failure("lock-unlink-parent-durability", error),
        }
        self.armed = false;
        self.report.clone()
    }

    fn current_path_matches_created_identity(&self) -> Result<(), String> {
        let created_identity = self
            .created_identity
            .as_ref()
            .ok_or_else(|| "created lock identity is unavailable".to_owned())?;
        let current = fs::symlink_metadata(&self.path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "lock pathname is absent".to_owned()
            } else {
                format!("lock pathname metadata cannot be read: {error}")
            }
        })?;
        if current.file_type().is_symlink() {
            return Err("lock pathname is a symbolic link".to_owned());
        }
        if !current.is_file() {
            return Err("lock pathname is not a regular file".to_owned());
        }
        if !same_file(created_identity, &current) {
            return Err("lock pathname no longer names the created lock file".to_owned());
        }
        Ok(())
    }
}

impl Drop for WorkspaceMutationLock {
    fn drop(&mut self) {
        if self.armed {
            if self.current_path_matches_created_identity().is_ok() {
                let _ = fs::remove_file(&self.path);
            }
            let _ = self.root_directory.sync_all();
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ApplyConfirmation {
    NotRequired,
    Confirmed,
    Unconfirmed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum VerifiedTargetState {
    Unverified,
    Present { content_digest: String },
    Absent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CreatedParentDirectoryState {
    NotCreated,
    Retained,
    RolledBack,
    Unconfirmed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct RepositoryApplyFailure {
    pub(super) stage: String,
    pub(super) message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RepositoryApplyReport {
    pub(super) workspace_relative_path: String,
    pub(super) operation: TargetOperation,
    pub(super) filesystem_mutation_occurred: bool,
    pub(super) target_mutation_committed: bool,
    pub(super) verified_target_state: VerifiedTargetState,
    pub(super) file_durability: ApplyConfirmation,
    pub(super) parent_directory_durability: ApplyConfirmation,
    pub(super) temporary_cleanup: ApplyConfirmation,
    pub(super) created_parent_directories: Vec<String>,
    pub(super) created_parent_directory_state: CreatedParentDirectoryState,
    pub(super) failures: Vec<RepositoryApplyFailure>,
}

impl RepositoryApplyReport {
    fn new(relative_path: &str, operation: TargetOperation) -> Self {
        Self {
            workspace_relative_path: relative_path.to_owned(),
            operation,
            filesystem_mutation_occurred: false,
            target_mutation_committed: false,
            verified_target_state: VerifiedTargetState::Unverified,
            file_durability: if operation == TargetOperation::Delete {
                ApplyConfirmation::NotRequired
            } else {
                ApplyConfirmation::Unconfirmed
            },
            parent_directory_durability: ApplyConfirmation::Unconfirmed,
            temporary_cleanup: if operation == TargetOperation::Delete {
                ApplyConfirmation::NotRequired
            } else {
                ApplyConfirmation::Unconfirmed
            },
            created_parent_directories: Vec::new(),
            created_parent_directory_state: CreatedParentDirectoryState::NotCreated,
            failures: Vec::new(),
        }
    }

    fn record_failure(&mut self, stage: &str, error: impl std::fmt::Display) {
        self.failures.push(RepositoryApplyFailure {
            stage: stage.to_owned(),
            message: error.to_string(),
        });
    }

    pub(super) fn is_fully_confirmed(&self) -> bool {
        let expected_state_confirmed = matches!(
            (&self.operation, &self.verified_target_state),
            (TargetOperation::Delete, VerifiedTargetState::Absent)
                | (
                    TargetOperation::Create | TargetOperation::Update,
                    VerifiedTargetState::Present { .. },
                )
        );
        let file_durability_confirmed = match self.operation {
            TargetOperation::Delete => self.file_durability == ApplyConfirmation::NotRequired,
            TargetOperation::Create | TargetOperation::Update => {
                self.file_durability == ApplyConfirmation::Confirmed
            }
            TargetOperation::Inspect => false,
        };
        let created_parent_state_confirmed = if self.created_parent_directories.is_empty() {
            self.created_parent_directory_state == CreatedParentDirectoryState::NotCreated
        } else {
            self.created_parent_directory_state == CreatedParentDirectoryState::Retained
        };
        self.filesystem_mutation_occurred
            && self.target_mutation_committed
            && expected_state_confirmed
            && file_durability_confirmed
            && self.parent_directory_durability == ApplyConfirmation::Confirmed
            && matches!(
                self.temporary_cleanup,
                ApplyConfirmation::NotRequired | ApplyConfirmation::Confirmed
            )
            && created_parent_state_confirmed
            && self.failures.is_empty()
    }
}

struct TemporaryFileGuard {
    path: PathBuf,
    armed: bool,
}

impl TemporaryFileGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }

    fn cleanup(&mut self, report: &mut RepositoryApplyReport) {
        if !self.armed {
            report.temporary_cleanup = ApplyConfirmation::Confirmed;
            return;
        }
        match fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => report.record_failure("temporary-file-cleanup", error),
        }
        let confirmed = match fs::symlink_metadata(&self.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Ok(_) => {
                report.record_failure(
                    "temporary-file-cleanup-verification",
                    "temporary file still exists",
                );
                false
            }
            Err(error) => {
                report.record_failure("temporary-file-cleanup-verification", error);
                false
            }
        };
        self.disarm();
        if confirmed {
            report.temporary_cleanup = ApplyConfirmation::Confirmed;
        }
    }
}

impl Drop for TemporaryFileGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

struct CreatedDirectoryIdentity {
    relative_path: String,
    metadata: Option<Metadata>,
}

struct CreatedDirectoryGuard {
    root: PathBuf,
    directories: Vec<CreatedDirectoryIdentity>,
    armed: bool,
}

impl CreatedDirectoryGuard {
    fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            directories: Vec::new(),
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }

    fn retain(&mut self, report: &mut RepositoryApplyReport) {
        let mut confirmed = true;
        for directory in &self.directories {
            let path = self.root.join(&directory.relative_path);
            match (&directory.metadata, fs::symlink_metadata(&path)) {
                (Some(created), Ok(current))
                    if current.is_dir()
                        && !current.file_type().is_symlink()
                        && same_file(created, &current) => {}
                (None, _) => {
                    confirmed = false;
                    report.record_failure(
                        "created-parent-retention-verification",
                        format!(
                            "created parent directory `{}` has no captured identity",
                            directory.relative_path
                        ),
                    );
                }
                (_, Ok(_)) => {
                    confirmed = false;
                    report.record_failure(
                        "created-parent-retention-verification",
                        format!(
                            "created parent directory `{}` changed identity",
                            directory.relative_path
                        ),
                    );
                }
                (_, Err(error)) => {
                    confirmed = false;
                    report.record_failure("created-parent-retention-verification", error);
                }
            }
        }
        self.disarm();
        report.created_parent_directory_state = if self.directories.is_empty() {
            CreatedParentDirectoryState::NotCreated
        } else if confirmed {
            CreatedParentDirectoryState::Retained
        } else {
            CreatedParentDirectoryState::Unconfirmed
        };
    }

    fn rollback(&mut self, report: &mut RepositoryApplyReport) {
        let mut confirmed = true;
        for directory in self.directories.iter().rev() {
            let path = self.root.join(&directory.relative_path);
            let identity_matches = match (&directory.metadata, fs::symlink_metadata(&path)) {
                (Some(created), Ok(current)) => {
                    current.is_dir()
                        && !current.file_type().is_symlink()
                        && same_file(created, &current)
                }
                (None, _) => false,
                (_, Err(error)) if error.kind() == std::io::ErrorKind::NotFound => false,
                (_, Err(error)) => {
                    report.record_failure("created-parent-rollback-identity", error);
                    false
                }
            };
            if !identity_matches {
                confirmed = false;
                report.record_failure(
                    "created-parent-rollback-identity",
                    format!(
                        "created parent directory `{}` is absent or changed identity",
                        directory.relative_path
                    ),
                );
                continue;
            }
            let removed = match fs::remove_dir(&path) {
                Ok(()) => true,
                Err(error) => {
                    confirmed = false;
                    report.record_failure("created-parent-rollback", error);
                    false
                }
            };
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && removed => {
                    if let Some(parent) = path.parent()
                        && let Err(error) = sync_directory(parent)
                    {
                        confirmed = false;
                        report.record_failure("created-parent-rollback-durability", error);
                    }
                }
                Ok(_) => {
                    confirmed = false;
                    report.record_failure(
                        "created-parent-rollback-verification",
                        format!(
                            "created parent directory `{}` still exists",
                            directory.relative_path
                        ),
                    );
                }
                Err(error) => {
                    confirmed = false;
                    report.record_failure("created-parent-rollback-verification", error);
                }
            }
        }
        self.disarm();
        report.created_parent_directory_state = if self.directories.is_empty() {
            CreatedParentDirectoryState::NotCreated
        } else if confirmed {
            CreatedParentDirectoryState::RolledBack
        } else {
            CreatedParentDirectoryState::Unconfirmed
        };
    }
}

impl Drop for CreatedDirectoryGuard {
    fn drop(&mut self) {
        if self.armed {
            for directory in self.directories.iter().rev() {
                let path = self.root.join(&directory.relative_path);
                if let (Some(created), Ok(current)) =
                    (&directory.metadata, fs::symlink_metadata(&path))
                    && current.is_dir()
                    && !current.file_type().is_symlink()
                    && same_file(created, &current)
                {
                    let _ = fs::remove_dir(path);
                }
            }
        }
    }
}

const BATCH_CONTROL_DIRECTORY: &str = ".llm-context-vault-harness";
const BATCH_DIRECTORY: &str = ".llm-context-vault-harness/batches";
const BATCH_COMPLETION_DIRECTORY: &str = ".llm-context-vault-harness/completed";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BatchJournalLocation {
    Active,
    Completion,
    ActiveAndCompletion,
}

pub(super) fn ensure_no_pending_batch_recovery(root: &Path) -> HarnessResult<()> {
    let batches_relative = Path::new(BATCH_DIRECTORY);
    ensure_no_symlink_components(root, batches_relative)?;
    let batches_path = root.join(batches_relative);
    let metadata = match fs::symlink_metadata(&batches_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(HarnessError::FileRead {
                path: batches_path,
                message: source.to_string(),
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(HarnessError::PathEscapesRoot(batches_path));
    }
    let entries = fs::read_dir(&batches_path).map_err(|source| HarnessError::FileRead {
        path: batches_path.clone(),
        message: source.to_string(),
    })?;
    let mut artifacts = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| HarnessError::FileRead {
            path: batches_path.clone(),
            message: source.to_string(),
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| HarnessError::FileRead {
            path: path.clone(),
            message: source.to_string(),
        })?;
        let artifact_path = if file_type.is_dir() && !file_type.is_symlink() {
            let journal = path.join("journal.json");
            match fs::symlink_metadata(&journal) {
                Ok(journal_metadata)
                    if journal_metadata.is_file() && !journal_metadata.file_type().is_symlink() =>
                {
                    journal
                }
                Ok(_) | Err(_) => path,
            }
        } else {
            path
        };
        let relative = artifact_path
            .strip_prefix(root)
            .map_err(|_| HarnessError::PathEscapesRoot(artifact_path.clone()))?;
        artifacts.push(portable_path(relative));
    }
    artifacts.sort();
    if artifacts.is_empty() {
        return Ok(());
    }
    Err(HarnessError::PendingBatchRecovery { artifacts })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BatchApplyOutcome {
    Applied,
    RolledBack,
    RecoveryRequired,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum BatchJournalPhase {
    Prepared,
    Applying,
    AppliedPendingCleanup,
    CleanupComplete,
    RollingBack,
    RolledBack,
    RecoveryRequired,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum BatchTargetApplyState {
    Pending,
    Staged,
    BackedUp,
    Applied,
    RolledBack,
    RecoveryRequired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct PersistedFileIdentity {
    device: u64,
    inode: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct BatchCreatedDirectory {
    relative_path: String,
    identity: Option<PersistedFileIdentity>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct BatchJournalTarget {
    workspace_relative_path: String,
    operation: TargetOperation,
    original_content_digest: Option<String>,
    intended_content_digest: Option<String>,
    staged_relative_path: Option<String>,
    backup_relative_path: Option<String>,
    planned_parent_directories: Vec<String>,
    state: BatchTargetApplyState,
    failures: Vec<RepositoryApplyFailure>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct BatchJournal {
    version: u32,
    batch_id: String,
    resolved_plan_digest: String,
    candidate_digest: String,
    phase: BatchJournalPhase,
    targets: Vec<BatchJournalTarget>,
    created_directories: Vec<BatchCreatedDirectory>,
    failures: Vec<RepositoryApplyFailure>,
    journal_digest: String,
}

#[derive(Clone, Copy)]
pub(super) struct BatchApplyInput<'a> {
    pub(super) change: &'a FileChange,
    pub(super) planned_parent_directories: &'a [String],
    pub(super) validate_context_markdown: bool,
}

#[derive(Clone, Debug)]
pub(super) struct BatchTargetApplyReport {
    pub(super) workspace_relative_path: String,
    pub(super) operation: TargetOperation,
    pub(super) original_content_digest: Option<String>,
    pub(super) intended_content_digest: Option<String>,
    pub(super) staged_relative_path: Option<String>,
    pub(super) backup_relative_path: Option<String>,
    pub(super) state: BatchTargetApplyState,
    pub(super) repository_apply: Option<RepositoryApplyReport>,
    pub(super) failures: Vec<RepositoryApplyFailure>,
}

#[derive(Clone, Debug)]
pub(super) struct BatchApplyReport {
    pub(super) batch_id: String,
    pub(super) resolved_plan_digest: String,
    pub(super) candidate_digest: String,
    pub(super) journal_relative_path: String,
    pub(super) completion_receipt_relative_path: Option<String>,
    pub(super) journal_retained: bool,
    pub(super) outcome: BatchApplyOutcome,
    pub(super) targets: Vec<BatchTargetApplyReport>,
    pub(super) failures: Vec<RepositoryApplyFailure>,
}

impl BatchJournal {
    fn new(
        batch_id: String,
        resolved_plan_digest: &str,
        candidate_digest: &str,
        inputs: &[BatchApplyInput<'_>],
    ) -> HarnessResult<Self> {
        let targets = inputs
            .iter()
            .enumerate()
            .map(|(index, input)| {
                let change = ApplyChange::from(input.change);
                let original_content_digest = change.expected_digest.map(str::to_owned);
                let intended_content_digest = change.content.map(byte_digest);
                let staged_relative_path = (change.operation != TargetOperation::Delete)
                    .then(|| format!("{BATCH_DIRECTORY}/{batch_id}/staged/{index:04}.new"));
                let backup_relative_path = (change.operation != TargetOperation::Create)
                    .then(|| format!("{BATCH_DIRECTORY}/{batch_id}/backups/{index:04}.bak"));
                BatchJournalTarget {
                    workspace_relative_path: change.relative_path.to_owned(),
                    operation: change.operation,
                    original_content_digest,
                    intended_content_digest,
                    staged_relative_path,
                    backup_relative_path,
                    planned_parent_directories: input.planned_parent_directories.to_vec(),
                    state: BatchTargetApplyState::Pending,
                    failures: Vec::new(),
                }
            })
            .collect();
        let mut journal = Self {
            version: HARNESS_SCHEMA_VERSION,
            batch_id,
            resolved_plan_digest: resolved_plan_digest.to_owned(),
            candidate_digest: candidate_digest.to_owned(),
            phase: BatchJournalPhase::Prepared,
            targets,
            created_directories: Vec::new(),
            failures: Vec::new(),
            journal_digest: String::new(),
        };
        journal.refresh_digest()?;
        Ok(journal)
    }

    fn refresh_digest(&mut self) -> HarnessResult<()> {
        self.journal_digest = serialized_digest(&(
            self.version,
            &self.batch_id,
            &self.resolved_plan_digest,
            &self.candidate_digest,
            self.phase,
            &self.targets,
            &self.created_directories,
            &self.failures,
        ))?;
        Ok(())
    }

    #[allow(
        clippy::too_many_lines,
        reason = "durable journal validation and rollback ordering invariants must remain auditable together"
    )]
    fn validate(&self) -> HarnessResult<()> {
        if self.version != HARNESS_SCHEMA_VERSION {
            return Err(HarnessError::InvalidSubmission(format!(
                "unsupported batch journal schema version `{}`",
                self.version
            )));
        }
        validate_batch_identifier(&self.batch_id)?;
        validate_plan_digest(
            "batch journal resolved-plan digest",
            &self.resolved_plan_digest,
        )?;
        validate_plan_digest("batch journal candidate digest", &self.candidate_digest)?;
        if self.targets.is_empty() || self.targets.len() > MAX_TARGETS {
            return Err(HarnessError::InvalidSubmission(format!(
                "batch journal must contain between one and {MAX_TARGETS} targets"
            )));
        }
        let expected = serialized_digest(&(
            self.version,
            &self.batch_id,
            &self.resolved_plan_digest,
            &self.candidate_digest,
            self.phase,
            &self.targets,
            &self.created_directories,
            &self.failures,
        ))?;
        if self.journal_digest != expected {
            return Err(HarnessError::InvalidSubmission(
                "batch journal digest is invalid".to_owned(),
            ));
        }
        let mut target_paths = BTreeSet::new();
        for (index, target) in self.targets.iter().enumerate() {
            validate_relative_path(
                Path::new(&target.workspace_relative_path),
                "batch journal target",
            )?;
            if path_is_within(&target.workspace_relative_path, BATCH_CONTROL_DIRECTORY)
                || target.workspace_relative_path == ".llm-context-vault-harness.lock"
            {
                return Err(HarnessError::InvalidSubmission(
                    "batch journal target uses a reserved Harness lifecycle path".to_owned(),
                ));
            }
            if !target_paths.insert(target.workspace_relative_path.as_str()) {
                return Err(HarnessError::InvalidSubmission(
                    "batch journal target paths must be unique".to_owned(),
                ));
            }
            match target.operation {
                TargetOperation::Create => {
                    if target.original_content_digest.is_some()
                        || target.intended_content_digest.is_none()
                        || target.staged_relative_path.is_none()
                        || target.backup_relative_path.is_some()
                    {
                        return Err(HarnessError::InvalidSubmission(
                            "batch create journal state is invalid".to_owned(),
                        ));
                    }
                }
                TargetOperation::Update => {
                    if target.original_content_digest.is_none()
                        || target.intended_content_digest.is_none()
                        || target.staged_relative_path.is_none()
                        || target.backup_relative_path.is_none()
                    {
                        return Err(HarnessError::InvalidSubmission(
                            "batch update journal state is invalid".to_owned(),
                        ));
                    }
                }
                TargetOperation::Delete => {
                    if target.original_content_digest.is_none()
                        || target.intended_content_digest.is_some()
                        || target.staged_relative_path.is_some()
                        || target.backup_relative_path.is_none()
                    {
                        return Err(HarnessError::InvalidSubmission(
                            "batch delete journal state is invalid".to_owned(),
                        ));
                    }
                }
                TargetOperation::Inspect => {
                    return Err(HarnessError::InvalidSubmission(
                        "batch journal cannot contain inspect targets".to_owned(),
                    ));
                }
            }
            if let Some(digest) = &target.original_content_digest {
                validate_plan_digest("batch original content digest", digest)?;
            }
            if let Some(digest) = &target.intended_content_digest {
                validate_plan_digest("batch intended content digest", digest)?;
            }
            let mut parent_paths = BTreeSet::new();
            for parent in &target.planned_parent_directories {
                validate_relative_path(Path::new(parent), "batch planned parent directory")?;
                let target_parent = Path::new(&target.workspace_relative_path)
                    .parent()
                    .unwrap_or_else(|| Path::new(""));
                if target.operation != TargetOperation::Create
                    || !target_parent.starts_with(Path::new(parent))
                {
                    return Err(HarnessError::InvalidSubmission(
                        "batch planned parent directory is not a create-target ancestor".to_owned(),
                    ));
                }
                if !parent_paths.insert(parent.as_str()) {
                    return Err(HarnessError::InvalidSubmission(
                        "batch planned parent directories must be unique".to_owned(),
                    ));
                }
            }
            if let Some(path) = &target.backup_relative_path {
                validate_batch_artifact_path(&self.batch_id, path, "backups")?;
                if path != &format!("{BATCH_DIRECTORY}/{}/backups/{index:04}.bak", self.batch_id) {
                    return Err(HarnessError::InvalidSubmission(
                        "batch backup path does not match its target index".to_owned(),
                    ));
                }
            }
            if let Some(path) = &target.staged_relative_path {
                validate_batch_artifact_path(&self.batch_id, path, "staged")?;
                if path != &format!("{BATCH_DIRECTORY}/{}/staged/{index:04}.new", self.batch_id) {
                    return Err(HarnessError::InvalidSubmission(
                        "batch staged path does not match its target index".to_owned(),
                    ));
                }
            }
        }
        let mut created_paths = BTreeSet::new();
        for directory in &self.created_directories {
            validate_relative_path(
                Path::new(&directory.relative_path),
                "batch created directory",
            )?;
            if !created_paths.insert(directory.relative_path.as_str())
                || !self.targets.iter().any(|target| {
                    target
                        .planned_parent_directories
                        .contains(&directory.relative_path)
                })
            {
                return Err(HarnessError::InvalidSubmission(
                    "batch created directory is duplicated or was not planned".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

pub(super) fn apply_single_change_reported(
    root: &Path,
    change: &FileChange,
    resolved_plan_digest: &str,
    planned_parent_directories: &[String],
    should_validate_context_markdown: bool,
) -> HarnessResult<RepositoryApplyReport> {
    let input = ApplyChange::from(change);
    let prepared = prepare_apply_paths(
        root,
        &input,
        resolved_plan_digest,
        planned_parent_directories,
        should_validate_context_markdown,
    )?;
    let report = RepositoryApplyReport::new(input.relative_path, input.operation);
    let created_directory_guard = CreatedDirectoryGuard::new(root);
    if input.operation == TargetOperation::Delete {
        return Ok(apply_delete_report(
            root,
            &prepared,
            created_directory_guard,
            report,
        ));
    }
    apply_content_change_reported(
        root,
        &input,
        &prepared,
        planned_parent_directories,
        created_directory_guard,
        report,
    )
}

pub(super) fn apply_change_batch_reported_with_policy(
    policy: WorkspacePolicy,
    root: &Path,
    inputs: &[BatchApplyInput<'_>],
    resolved_plan_digest: &str,
    candidate_digest: &str,
) -> HarnessResult<BatchApplyReport> {
    let batch_id = batch_identifier(resolved_plan_digest, candidate_digest)?;
    apply_change_batch_reported_with_hook(
        policy,
        root,
        inputs,
        resolved_plan_digest,
        candidate_digest,
        batch_id,
        false,
        |_| Ok(()),
    )
}

pub(super) fn apply_change_batch_reported_for_attempt_with_policy(
    policy: WorkspacePolicy,
    root: &Path,
    inputs: &[BatchApplyInput<'_>],
    resolved_plan_digest: &str,
    candidate_digest: &str,
    attempt_identifier: &str,
) -> HarnessResult<BatchApplyReport> {
    let (batch_id, _, _) =
        batch_paths_for_attempt(resolved_plan_digest, candidate_digest, attempt_identifier)?;
    apply_change_batch_reported_with_hook(
        policy,
        root,
        inputs,
        resolved_plan_digest,
        candidate_digest,
        batch_id,
        false,
        |_| Ok(()),
    )
}

#[cfg(test)]
pub(super) fn apply_change_batch_reported_with_test_hook_with_policy(
    policy: WorkspacePolicy,
    root: &Path,
    inputs: &[BatchApplyInput<'_>],
    resolved_plan_digest: &str,
    candidate_digest: &str,
    before_target_apply: impl FnMut(usize) -> HarnessResult<()>,
) -> HarnessResult<BatchApplyReport> {
    apply_change_batch_reported_with_hook(
        policy,
        root,
        inputs,
        resolved_plan_digest,
        candidate_digest,
        batch_identifier(resolved_plan_digest, candidate_digest)?,
        false,
        before_target_apply,
    )
}

#[cfg(test)]
pub(super) fn apply_change_batch_reported_for_attempt_with_test_hook_with_policy(
    policy: WorkspacePolicy,
    root: &Path,
    inputs: &[BatchApplyInput<'_>],
    resolved_plan_digest: &str,
    candidate_digest: &str,
    attempt_identifier: &str,
    before_target_apply: impl FnMut(usize) -> HarnessResult<()>,
) -> HarnessResult<BatchApplyReport> {
    let (batch_id, _, _) =
        batch_paths_for_attempt(resolved_plan_digest, candidate_digest, attempt_identifier)?;
    apply_change_batch_reported_with_hook(
        policy,
        root,
        inputs,
        resolved_plan_digest,
        candidate_digest,
        batch_id,
        false,
        before_target_apply,
    )
}

#[cfg(test)]
pub(super) fn apply_change_batch_reported_before_cleanup_for_test_with_policy(
    policy: WorkspacePolicy,
    root: &Path,
    inputs: &[BatchApplyInput<'_>],
    resolved_plan_digest: &str,
    candidate_digest: &str,
) -> HarnessResult<BatchApplyReport> {
    apply_change_batch_reported_with_hook(
        policy,
        root,
        inputs,
        resolved_plan_digest,
        candidate_digest,
        batch_identifier(resolved_plan_digest, candidate_digest)?,
        true,
        |_| Ok(()),
    )
}

#[allow(
    clippy::too_many_lines,
    reason = "durable journal transitions and rollback ordering must remain explicit in one operation"
)]
#[allow(
    clippy::too_many_arguments,
    reason = "the internal workspace capability accompanies the existing ordered mutation inputs"
)]
fn apply_change_batch_reported_with_hook(
    policy: WorkspacePolicy,
    root: &Path,
    inputs: &[BatchApplyInput<'_>],
    resolved_plan_digest: &str,
    candidate_digest: &str,
    batch_id: String,
    retain_before_cleanup: bool,
    mut before_target_apply: impl FnMut(usize) -> HarnessResult<()>,
) -> HarnessResult<BatchApplyReport> {
    policy.validate_platform()?;
    ensure_no_pending_batch_recovery(root)?;
    if inputs.is_empty() || inputs.len() > MAX_TARGETS {
        return Err(HarnessError::InvalidSubmission(format!(
            "batch apply requires between one and {MAX_TARGETS} changes"
        )));
    }
    let mut paths = BTreeSet::new();
    for input in inputs {
        let change = ApplyChange::from(input.change);
        if !paths.insert(change.relative_path) {
            return Err(HarnessError::InvalidSubmission(
                "batch apply paths must be unique".to_owned(),
            ));
        }
        prepare_apply_paths(
            root,
            &change,
            resolved_plan_digest,
            input.planned_parent_directories,
            input.validate_context_markdown,
        )?;
    }
    let mut journal = BatchJournal::new(batch_id, resolved_plan_digest, candidate_digest, inputs)?;
    let journal_relative_path = match initialize_batch_workspace(policy, root, &journal.batch_id) {
        Ok(path) => path,
        Err(error) => {
            let _ = cleanup_uncommitted_batch_workspace(root, &journal.batch_id);
            ensure_no_pending_batch_recovery(root)?;
            return Err(error);
        }
    };
    if let Err(error) = persist_batch_journal(policy, root, &journal_relative_path, &mut journal) {
        let _ = cleanup_uncommitted_batch_workspace(root, &journal.batch_id);
        ensure_no_pending_batch_recovery(root)?;
        return Err(error);
    }
    let mut repository_reports = vec![None; inputs.len()];

    for (index, input) in inputs.iter().enumerate() {
        if journal.targets[index].operation == TargetOperation::Delete {
            continue;
        }
        if let Err(error) = stage_batch_content(policy, root, input.change, &journal.targets[index])
        {
            journal.targets[index]
                .failures
                .push(RepositoryApplyFailure {
                    stage: "batch-stage".to_owned(),
                    message: error.to_string(),
                });
            return Ok(rollback_failed_batch(
                policy,
                root,
                &journal_relative_path,
                journal,
                &repository_reports,
                RepositoryApplyFailure {
                    stage: "batch-stage".to_owned(),
                    message: error.to_string(),
                },
            ));
        }
        journal.targets[index].state = BatchTargetApplyState::Staged;
        if let Err(error) =
            persist_batch_journal(policy, root, &journal_relative_path, &mut journal)
        {
            return Ok(rollback_failed_batch(
                policy,
                root,
                &journal_relative_path,
                journal,
                &repository_reports,
                RepositoryApplyFailure {
                    stage: "batch-journal-stage".to_owned(),
                    message: error.to_string(),
                },
            ));
        }
    }

    for index in 0..journal.targets.len() {
        if journal.targets[index].operation == TargetOperation::Create {
            continue;
        }
        if let Err(error) = create_batch_backup(policy, root, &journal.targets[index]) {
            journal.targets[index]
                .failures
                .push(RepositoryApplyFailure {
                    stage: "batch-backup".to_owned(),
                    message: error.to_string(),
                });
            return Ok(rollback_failed_batch(
                policy,
                root,
                &journal_relative_path,
                journal,
                &repository_reports,
                RepositoryApplyFailure {
                    stage: "batch-backup".to_owned(),
                    message: error.to_string(),
                },
            ));
        }
        journal.targets[index].state = BatchTargetApplyState::BackedUp;
        if let Err(error) =
            persist_batch_journal(policy, root, &journal_relative_path, &mut journal)
        {
            return Ok(rollback_failed_batch(
                policy,
                root,
                &journal_relative_path,
                journal,
                &repository_reports,
                RepositoryApplyFailure {
                    stage: "batch-journal-backup".to_owned(),
                    message: error.to_string(),
                },
            ));
        }
    }

    journal.phase = BatchJournalPhase::Applying;
    if let Err(error) = persist_batch_journal(policy, root, &journal_relative_path, &mut journal) {
        return Ok(rollback_failed_batch(
            policy,
            root,
            &journal_relative_path,
            journal,
            &repository_reports,
            RepositoryApplyFailure {
                stage: "batch-journal-applying".to_owned(),
                message: error.to_string(),
            },
        ));
    }

    for (index, input) in inputs.iter().enumerate() {
        let remaining_parent_directories =
            match remaining_batch_parent_directories(root, &journal.targets[index], &journal) {
                Ok(directories) => directories,
                Err(error) => {
                    return Ok(rollback_failed_batch(
                        policy,
                        root,
                        &journal_relative_path,
                        journal,
                        &repository_reports,
                        RepositoryApplyFailure {
                            stage: format!("batch-target-{index}-parent-revalidation"),
                            message: error.to_string(),
                        },
                    ));
                }
            };
        if let Err(error) = prepare_batch_parent_directories(
            policy,
            root,
            &journal_relative_path,
            &mut journal,
            &remaining_parent_directories,
        ) {
            return Ok(rollback_failed_batch(
                policy,
                root,
                &journal_relative_path,
                journal,
                &repository_reports,
                RepositoryApplyFailure {
                    stage: format!("batch-target-{index}-parent-preparation"),
                    message: error.to_string(),
                },
            ));
        }
        if let Err(error) = before_target_apply(index) {
            return Ok(rollback_failed_batch(
                policy,
                root,
                &journal_relative_path,
                journal,
                &repository_reports,
                RepositoryApplyFailure {
                    stage: format!("batch-target-{index}-pre-apply"),
                    message: error.to_string(),
                },
            ));
        }
        let expected_parent_identity =
            match verify_batch_created_parent_identities(root, &journal.targets[index], &journal) {
                Ok(identity) => identity,
                Err(error) => {
                    return Ok(rollback_failed_batch(
                        policy,
                        root,
                        &journal_relative_path,
                        journal,
                        &repository_reports,
                        RepositoryApplyFailure {
                            stage: format!("batch-target-{index}-parent-identity"),
                            message: error.to_string(),
                        },
                    ));
                }
            };
        let report = match apply_staged_change_reported(
            policy,
            root,
            input.change,
            &journal.targets[index],
            resolved_plan_digest,
            &[],
            expected_parent_identity.as_ref(),
            input.validate_context_markdown,
        ) {
            Ok(report) => report,
            Err(error) => {
                return Ok(rollback_failed_batch(
                    policy,
                    root,
                    &journal_relative_path,
                    journal,
                    &repository_reports,
                    RepositoryApplyFailure {
                        stage: format!("batch-target-{index}-apply"),
                        message: error.to_string(),
                    },
                ));
            }
        };
        let fully_confirmed = report.is_fully_confirmed();
        let created_directory_identity_result = if fully_confirmed {
            capture_batch_created_directories(root, &report, &mut journal)
        } else {
            Ok(())
        };
        repository_reports[index] = Some(report);
        if !fully_confirmed {
            return Ok(rollback_failed_batch(
                policy,
                root,
                &journal_relative_path,
                journal,
                &repository_reports,
                RepositoryApplyFailure {
                    stage: format!("batch-target-{index}-confirmation"),
                    message: "target mutation was not fully confirmed".to_owned(),
                },
            ));
        }
        if let Err(error) = created_directory_identity_result {
            return Ok(rollback_failed_batch(
                policy,
                root,
                &journal_relative_path,
                journal,
                &repository_reports,
                RepositoryApplyFailure {
                    stage: format!("batch-target-{index}-created-directory-identity"),
                    message: error.to_string(),
                },
            ));
        }
        journal.targets[index].state = BatchTargetApplyState::Applied;
        if let Err(error) =
            persist_batch_journal(policy, root, &journal_relative_path, &mut journal)
        {
            return Ok(rollback_failed_batch(
                policy,
                root,
                &journal_relative_path,
                journal,
                &repository_reports,
                RepositoryApplyFailure {
                    stage: format!("batch-target-{index}-journal"),
                    message: error.to_string(),
                },
            ));
        }
    }

    journal.phase = BatchJournalPhase::AppliedPendingCleanup;
    if let Err(error) = persist_batch_journal(policy, root, &journal_relative_path, &mut journal) {
        journal.failures.push(RepositoryApplyFailure {
            stage: "batch-journal-applied".to_owned(),
            message: error.to_string(),
        });
        journal.phase = BatchJournalPhase::RecoveryRequired;
        let _ = persist_batch_journal(policy, root, &journal_relative_path, &mut journal);
        return Ok(batch_report_from_journal(
            &journal_relative_path,
            true,
            BatchApplyOutcome::RecoveryRequired,
            journal,
            &repository_reports,
        ));
    }
    if retain_before_cleanup {
        return Ok(batch_report_from_journal(
            &journal_relative_path,
            true,
            BatchApplyOutcome::RecoveryRequired,
            journal,
            &repository_reports,
        ));
    }
    match finalize_applied_batch(policy, root, &journal_relative_path, &mut journal) {
        Ok(()) => Ok(batch_report_from_journal(
            &journal_relative_path,
            true,
            BatchApplyOutcome::Applied,
            journal,
            &repository_reports,
        )),
        Err(error) => {
            journal.failures.push(RepositoryApplyFailure {
                stage: "batch-cleanup".to_owned(),
                message: error.to_string(),
            });
            let _ = persist_batch_journal(policy, root, &journal_relative_path, &mut journal);
            Ok(batch_report_from_journal(
                &journal_relative_path,
                true,
                BatchApplyOutcome::RecoveryRequired,
                journal,
                &repository_reports,
            ))
        }
    }
}

/// Inspect only a completed, fully cleaned batch. This path never compensates files.
pub(super) fn inspect_completed_batch_reported(
    root: &Path,
    journal_relative_path: &str,
    rolled_back: bool,
) -> HarnessResult<BatchApplyReport> {
    validate_relative_path(Path::new(journal_relative_path), "batch journal path")?;
    let (journal, completion) = read_recovery_batch_journal(root, journal_relative_path)?;
    journal.validate()?;
    validate_batch_journal_path(&journal.batch_id, journal_relative_path)?;
    let completion = completion.ok_or_else(|| {
        HarnessError::InvalidRepository(
            "read-only batch inspection requires the actual completion receipt".to_owned(),
        )
    })?;
    let expected_phase = if rolled_back {
        BatchJournalPhase::RolledBack
    } else {
        BatchJournalPhase::CleanupComplete
    };
    if journal.phase != expected_phase {
        return Err(HarnessError::InvalidRepository(
            "read-only batch inspection found an incomplete or conflicting outcome".to_owned(),
        ));
    }
    let batch_path = root.join(format!("{BATCH_DIRECTORY}/{}", journal.batch_id));
    match fs::symlink_metadata(&batch_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        _ => {
            return Err(HarnessError::InvalidRepository(
                "batch staging/backup cleanup is not complete".to_owned(),
            ));
        }
    }
    for target in &journal.targets {
        let expected = if rolled_back {
            &target.original_content_digest
        } else {
            &target.intended_content_digest
        };
        if current_target_digest(root, &target.workspace_relative_path)? != *expected {
            return Err(HarnessError::InvalidRepository(
                "completed batch target bytes no longer match its proved outcome".to_owned(),
            ));
        }
        let expected_state = if rolled_back {
            BatchTargetApplyState::RolledBack
        } else {
            BatchTargetApplyState::Applied
        };
        if target.state != expected_state {
            return Err(HarnessError::InvalidRepository(
                "completed batch target state differs from its outcome".to_owned(),
            ));
        }
    }
    if rolled_back {
        for directory in &journal.created_directories {
            if directory.identity.is_some()
                && !matches!(fs::symlink_metadata(root.join(&directory.relative_path)),Err(error) if error.kind()==std::io::ErrorKind::NotFound)
            {
                return Err(HarnessError::InvalidRepository(
                    "rollback created-directory cleanup is incomplete".to_owned(),
                ));
            }
        }
    }
    let reports = vec![None; journal.targets.len()];
    let mut report = batch_report_from_journal(
        journal_relative_path,
        false,
        if rolled_back {
            BatchApplyOutcome::RolledBack
        } else {
            BatchApplyOutcome::Applied
        },
        journal,
        &reports,
    );
    report.completion_receipt_relative_path = Some(completion);
    Ok(report)
}

pub(super) fn recover_change_batch_reported_with_policy(
    policy: WorkspacePolicy,
    root: &Path,
    journal_relative_path: &str,
) -> HarnessResult<BatchApplyReport> {
    validate_relative_path(Path::new(journal_relative_path), "batch journal path")?;
    if !journal_relative_path.starts_with(&format!("{BATCH_DIRECTORY}/"))
        || !journal_relative_path.ends_with("/journal.json")
    {
        return Err(HarnessError::InvalidSubmission(
            "batch recovery path is outside the Harness batch journal directory".to_owned(),
        ));
    }
    let (mut journal, completion_receipt_relative_path) =
        read_recovery_batch_journal(root, journal_relative_path)?;
    journal.validate()?;
    validate_batch_journal_path(&journal.batch_id, journal_relative_path)?;
    let repository_reports = vec![None; journal.targets.len()];
    if let Some(completion_receipt_relative_path) = completion_receipt_relative_path {
        let outcome = match journal.phase {
            BatchJournalPhase::CleanupComplete => BatchApplyOutcome::Applied,
            BatchJournalPhase::RolledBack => BatchApplyOutcome::RolledBack,
            _ => {
                return Err(HarnessError::InvalidRepository(
                    "batch completion receipt has an incomplete phase".to_owned(),
                ));
            }
        };
        let mut report = batch_report_from_journal(
            journal_relative_path,
            false,
            outcome,
            journal,
            &repository_reports,
        );
        report.completion_receipt_relative_path = Some(completion_receipt_relative_path);
        return Ok(report);
    }
    if matches!(
        journal.phase,
        BatchJournalPhase::AppliedPendingCleanup | BatchJournalPhase::CleanupComplete
    ) {
        if let Err(error) = verify_all_intended_states(root, &journal) {
            journal.failures.push(RepositoryApplyFailure {
                stage: "batch-recovery-intended-state".to_owned(),
                message: error.to_string(),
            });
            let _ = persist_batch_journal(policy, root, journal_relative_path, &mut journal);
            return Ok(batch_report_from_journal(
                journal_relative_path,
                true,
                BatchApplyOutcome::RecoveryRequired,
                journal,
                &repository_reports,
            ));
        }
        return match finalize_applied_batch(policy, root, journal_relative_path, &mut journal) {
            Ok(()) => Ok(batch_report_from_journal(
                journal_relative_path,
                true,
                BatchApplyOutcome::Applied,
                journal,
                &repository_reports,
            )),
            Err(error) => {
                journal.failures.push(RepositoryApplyFailure {
                    stage: "batch-recovery-cleanup".to_owned(),
                    message: error.to_string(),
                });
                let _ = persist_batch_journal(policy, root, journal_relative_path, &mut journal);
                Ok(batch_report_from_journal(
                    journal_relative_path,
                    true,
                    BatchApplyOutcome::RecoveryRequired,
                    journal,
                    &repository_reports,
                ))
            }
        };
    }
    Ok(rollback_failed_batch(
        policy,
        root,
        journal_relative_path,
        journal,
        &repository_reports,
        RepositoryApplyFailure {
            stage: "batch-recovery".to_owned(),
            message: "recovery requested before the applied cleanup boundary".to_owned(),
        },
    ))
}

#[derive(Clone, Copy)]
struct ApplyChange<'a> {
    relative_path: &'a str,
    expected_digest: Option<&'a str>,
    content: Option<&'a [u8]>,
    operation: TargetOperation,
}

impl<'a> From<&'a FileChange> for ApplyChange<'a> {
    fn from(change: &'a FileChange) -> Self {
        match change {
            FileChange::Create { path, content } => Self {
                relative_path: path,
                expected_digest: None,
                content: Some(content.as_bytes()),
                operation: TargetOperation::Create,
            },
            FileChange::Update {
                path,
                expected_content_digest,
                content,
            } => Self {
                relative_path: path,
                expected_digest: Some(expected_content_digest),
                content: Some(content.as_bytes()),
                operation: TargetOperation::Update,
            },
            FileChange::Delete {
                path,
                expected_content_digest,
            } => Self {
                relative_path: path,
                expected_digest: Some(expected_content_digest),
                content: None,
                operation: TargetOperation::Delete,
            },
        }
    }
}

fn batch_identifier(resolved_plan_digest: &str, candidate_digest: &str) -> HarnessResult<String> {
    validate_plan_digest("batch resolved-plan digest", resolved_plan_digest)?;
    validate_plan_digest("batch candidate digest", candidate_digest)?;
    let plan = resolved_plan_digest.get(..12).ok_or_else(|| {
        HarnessError::InvalidSubmission("batch resolved-plan digest is too short".to_owned())
    })?;
    let candidate = candidate_digest.get(..12).ok_or_else(|| {
        HarnessError::InvalidSubmission("batch candidate digest is too short".to_owned())
    })?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| HarnessError::InvalidRepository(error.to_string()))?
        .as_nanos();
    Ok(format!(
        "batch-{plan}-{candidate}-{}-{nonce}",
        std::process::id()
    ))
}

pub(super) fn batch_paths_for_attempt(
    resolved_plan_digest: &str,
    candidate_digest: &str,
    attempt_identifier: &str,
) -> HarnessResult<(String, String, String)> {
    validate_plan_digest("batch resolved-plan digest", resolved_plan_digest)?;
    validate_plan_digest("batch candidate digest", candidate_digest)?;
    if !attempt_identifier.starts_with("attempt-")
        || attempt_identifier.len() > 96
        || !attempt_identifier
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(HarnessError::InvalidSubmission(
            "apply attempt ID must start with `attempt-` and use lowercase ASCII letters, digits, and hyphens"
                .to_owned(),
        ));
    }
    let plan = resolved_plan_digest.get(..12).ok_or_else(|| {
        HarnessError::InvalidSubmission("batch resolved-plan digest is too short".to_owned())
    })?;
    let candidate = candidate_digest.get(..12).ok_or_else(|| {
        HarnessError::InvalidSubmission("batch candidate digest is too short".to_owned())
    })?;
    let batch_id = format!("batch-{plan}-{candidate}-{attempt_identifier}");
    validate_batch_identifier(&batch_id)?;
    Ok((
        batch_id.clone(),
        format!("{BATCH_DIRECTORY}/{batch_id}/journal.json"),
        completion_receipt_relative_path(&batch_id),
    ))
}

fn initialize_batch_workspace(
    policy: WorkspacePolicy,
    root: &Path,
    batch_id: &str,
) -> HarnessResult<String> {
    validate_batch_identifier(batch_id)?;
    ensure_batch_directory(policy, root, BATCH_CONTROL_DIRECTORY)?;
    ensure_batch_directory(policy, root, BATCH_DIRECTORY)?;
    let batch_relative = format!("{BATCH_DIRECTORY}/{batch_id}");
    ensure_batch_directory(policy, root, &batch_relative)?;
    ensure_batch_directory(policy, root, &format!("{batch_relative}/staged"))?;
    ensure_batch_directory(policy, root, &format!("{batch_relative}/backups"))?;
    Ok(format!("{batch_relative}/journal.json"))
}

fn validate_batch_identifier(batch_id: &str) -> HarnessResult<()> {
    if !batch_id.starts_with("batch-")
        || batch_id.len() > 160
        || !batch_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(HarnessError::InvalidSubmission(
            "batch ID must use lowercase ASCII letters, digits, and hyphens".to_owned(),
        ));
    }
    Ok(())
}

fn ensure_batch_directory(
    policy: WorkspacePolicy,
    root: &Path,
    relative_path: &str,
) -> HarnessResult<()> {
    let relative = Path::new(relative_path);
    validate_relative_path(relative, "batch control directory")?;
    ensure_no_symlink_components(root, relative)?;
    let path = root.join(relative);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(HarnessError::PathEscapesRoot(path)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or_else(|| {
                HarnessError::InvalidRepository("batch control directory has no parent".to_owned())
            })?;
            policy
                .create_directory(&path)
                .map_err(|source| HarnessError::FileWrite {
                    path: path.clone(),
                    message: source.to_string(),
                })?;
            sync_directory(parent)
        }
        Err(source) => Err(HarnessError::FileRead {
            path,
            message: source.to_string(),
        }),
    }
}

fn validate_batch_artifact_path(
    batch_id: &str,
    relative_path: &str,
    category: &str,
) -> HarnessResult<()> {
    validate_relative_path(Path::new(relative_path), "batch artifact")?;
    let expected_prefix = format!("{BATCH_DIRECTORY}/{batch_id}/{category}/");
    if !relative_path.starts_with(&expected_prefix) {
        return Err(HarnessError::InvalidSubmission(
            "batch artifact path is outside its batch directory".to_owned(),
        ));
    }
    Ok(())
}

fn validate_batch_journal_path(batch_id: &str, relative_path: &str) -> HarnessResult<()> {
    let expected = format!("{BATCH_DIRECTORY}/{batch_id}/journal.json");
    if relative_path != expected {
        return Err(HarnessError::InvalidSubmission(
            "batch journal path does not match its batch ID".to_owned(),
        ));
    }
    Ok(())
}

fn persist_batch_journal(
    policy: WorkspacePolicy,
    root: &Path,
    journal_relative_path: &str,
    journal: &mut BatchJournal,
) -> HarnessResult<()> {
    validate_batch_journal_path(&journal.batch_id, journal_relative_path)?;
    journal.refresh_digest()?;
    let bytes = serde_json::to_vec_pretty(journal).map_err(|error| {
        HarnessError::InvalidSubmission(format!("failed to serialize batch journal: {error}"))
    })?;
    if bytes.len() > MAX_CANDIDATE_BYTES {
        return Err(HarnessError::InvalidSubmission(
            "batch journal exceeds the supported size".to_owned(),
        ));
    }
    let journal_relative = Path::new(journal_relative_path);
    ensure_no_symlink_components(root, journal_relative)?;
    let journal_path = root.join(journal_relative);
    let parent = journal_path
        .parent()
        .ok_or_else(|| HarnessError::InvalidRepository("batch journal has no parent".to_owned()))?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| HarnessError::InvalidRepository(error.to_string()))?
        .as_nanos();
    let temporary = parent.join(format!(
        ".journal-{}-{}-{nonce}.tmp",
        std::process::id(),
        journal.targets.len()
    ));
    let write_result = (|| {
        let mut file =
            policy
                .create_file(&temporary)
                .map_err(|source| HarnessError::FileWrite {
                    path: temporary.clone(),
                    message: source.to_string(),
                })?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|source| HarnessError::FileWrite {
                path: temporary.clone(),
                message: source.to_string(),
            })?;
        drop(file);
        fs::rename(&temporary, &journal_path).map_err(|source| HarnessError::FileWrite {
            path: journal_path.clone(),
            message: source.to_string(),
        })?;
        sync_directory(parent)
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

fn read_batch_journal(root: &Path, journal_relative_path: &str) -> HarnessResult<BatchJournal> {
    let relative = Path::new(journal_relative_path);
    let path = root.join(relative);
    let file = open_verified_file(root, relative)?;
    let metadata = file.metadata().map_err(|source| HarnessError::FileRead {
        path: path.clone(),
        message: source.to_string(),
    })?;
    if metadata.len() > MAX_CANDIDATE_BYTES as u64 {
        return Err(HarnessError::InvalidSubmission(
            "batch journal exceeds the supported size".to_owned(),
        ));
    }
    let capacity = usize::try_from(metadata.len()).map_err(|_| {
        HarnessError::InvalidSubmission(
            "batch journal size cannot be represented on this platform".to_owned(),
        )
    })?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(MAX_CANDIDATE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| HarnessError::FileRead {
            path,
            message: source.to_string(),
        })?;
    decode_current_json(&bytes, "batch journal")
}

fn create_batch_backup(
    policy: WorkspacePolicy,
    root: &Path,
    target: &BatchJournalTarget,
) -> HarnessResult<()> {
    let original = target.original_content_digest.as_deref().ok_or_else(|| {
        HarnessError::InvalidSubmission("batch backup target has no original digest".to_owned())
    })?;
    let backup_relative_path = target.backup_relative_path.as_deref().ok_or_else(|| {
        HarnessError::InvalidSubmission("batch backup path is missing".to_owned())
    })?;
    verify_target_precondition(
        root,
        Path::new(&target.workspace_relative_path),
        Some(original),
    )?;
    let backup_relative = Path::new(backup_relative_path);
    ensure_no_symlink_components(root, backup_relative)?;
    let source = root.join(&target.workspace_relative_path);
    let backup = root.join(backup_relative);
    let mut backup_file = policy
        .create_file(&backup)
        .map_err(|error| HarnessError::FileWrite {
            path: backup.clone(),
            message: error.to_string(),
        })?;
    std::io::copy(
        &mut File::open(&source).map_err(|error| HarnessError::FileRead {
            path: source.clone(),
            message: error.to_string(),
        })?,
        &mut backup_file,
    )
    .map_err(|error| HarnessError::FileWrite {
        path: backup.clone(),
        message: error.to_string(),
    })?;
    policy
        .copy_permissions(&source, &backup)
        .map_err(|error| HarnessError::FileWrite {
            path: backup.clone(),
            message: error.to_string(),
        })?;
    File::open(&backup)
        .and_then(|file| file.sync_all())
        .map_err(|error| HarnessError::FileWrite {
            path: backup.clone(),
            message: error.to_string(),
        })?;
    let backup_parent = backup
        .parent()
        .ok_or_else(|| HarnessError::InvalidRepository("batch backup has no parent".to_owned()))?;
    sync_directory(backup_parent)?;
    let backup_file = open_verified_file(root, backup_relative)?;
    let actual = digest_file(backup_file, MAX_TARGET_BYTES, &backup)?;
    if actual != original {
        return Err(HarnessError::PlanDrift {
            expected: original.to_owned(),
            actual,
        });
    }
    verify_target_precondition(
        root,
        Path::new(&target.workspace_relative_path),
        Some(original),
    )?;
    Ok(())
}

fn stage_batch_content(
    policy: WorkspacePolicy,
    root: &Path,
    change: &FileChange,
    target: &BatchJournalTarget,
) -> HarnessResult<()> {
    let input = ApplyChange::from(change);
    let content = input.content.ok_or_else(|| {
        HarnessError::InvalidSubmission("staged batch content is missing".to_owned())
    })?;
    let staged_relative_path = target.staged_relative_path.as_deref().ok_or_else(|| {
        HarnessError::InvalidSubmission("batch staged path is missing".to_owned())
    })?;
    let staged_relative = Path::new(staged_relative_path);
    ensure_no_symlink_components(root, staged_relative)?;
    let staged = root.join(staged_relative);
    let mut file = policy
        .create_file(&staged)
        .map_err(|source| HarnessError::FileWrite {
            path: staged.clone(),
            message: source.to_string(),
        })?;
    let stage_result = (|| {
        if input.operation == TargetOperation::Update {
            policy
                .copy_permissions(&root.join(input.relative_path), &staged)
                .map_err(|source| HarnessError::FileWrite {
                    path: staged.clone(),
                    message: source.to_string(),
                })?;
        }
        file.write_all(content)
            .and_then(|()| file.sync_all())
            .map_err(|source| HarnessError::FileWrite {
                path: staged.clone(),
                message: source.to_string(),
            })?;
        drop(file);
        sync_target_parent(&staged)?;
        let intended = target.intended_content_digest.as_deref().ok_or_else(|| {
            HarnessError::InvalidSubmission("batch intended digest is missing".to_owned())
        })?;
        let actual = open_verified_file(root, staged_relative)
            .and_then(|file| digest_file(file, MAX_TARGET_BYTES, &staged))?;
        if actual != intended {
            return Err(HarnessError::PlanDrift {
                expected: intended.to_owned(),
                actual,
            });
        }
        verify_target_precondition(root, Path::new(input.relative_path), input.expected_digest)
    })();
    if stage_result.is_err() {
        let _ = fs::remove_file(&staged);
    }
    stage_result
}

#[allow(
    clippy::too_many_lines,
    reason = "durable staged-write confirmation and rollback ordering must remain explicit together"
)]
#[allow(
    clippy::too_many_arguments,
    reason = "the internal workspace capability accompanies the existing ordered mutation inputs"
)]
fn apply_staged_change_reported(
    policy: WorkspacePolicy,
    root: &Path,
    change: &FileChange,
    target: &BatchJournalTarget,
    resolved_plan_digest: &str,
    planned_parent_directories: &[String],
    expected_parent_identity: Option<&PersistedFileIdentity>,
    should_validate_context_markdown: bool,
) -> HarnessResult<RepositoryApplyReport> {
    let input = ApplyChange::from(change);
    if input.operation == TargetOperation::Delete {
        return apply_single_change_reported(
            root,
            change,
            resolved_plan_digest,
            planned_parent_directories,
            should_validate_context_markdown,
        );
    }
    let prepared = prepare_apply_paths(
        root,
        &input,
        resolved_plan_digest,
        planned_parent_directories,
        should_validate_context_markdown,
    )?;
    let mut report = RepositoryApplyReport::new(input.relative_path, input.operation);
    let mut created_directory_guard = CreatedDirectoryGuard::new(root);
    let Some((canonical_parent, canonical_parent_file)) = prepare_content_parent(
        root,
        &prepared,
        planned_parent_directories,
        &mut created_directory_guard,
        &mut report,
    ) else {
        finalize_apply_report(
            root,
            &prepared.relative,
            prepared.expected_result_digest.as_deref(),
            None,
            None,
            &mut created_directory_guard,
            &mut report,
        );
        return Ok(report);
    };
    if let Some(expected_parent_identity) = expected_parent_identity {
        match canonical_parent_file.metadata() {
            Ok(metadata) if persisted_identity_matches(expected_parent_identity, &metadata) => {}
            Ok(_) => report.record_failure(
                "batch-created-parent-handle-identity",
                "target parent handle does not match the batch-created directory",
            ),
            Err(error) => report.record_failure("batch-created-parent-handle-identity", error),
        }
    }
    let staged_relative_path = target.staged_relative_path.as_deref().ok_or_else(|| {
        HarnessError::InvalidSubmission("batch staged path is missing".to_owned())
    })?;
    let staged_relative = Path::new(staged_relative_path);
    let staged = root.join(staged_relative);
    let intended = target.intended_content_digest.as_deref().ok_or_else(|| {
        HarnessError::InvalidSubmission("batch intended digest is missing".to_owned())
    })?;
    let staged_digest = open_verified_file(root, staged_relative)
        .and_then(|file| digest_file(file, MAX_TARGET_BYTES, &staged))?;
    if staged_digest == intended {
        report.file_durability = ApplyConfirmation::Confirmed;
    } else {
        report.record_failure(
            "batch-staged-content-verification",
            format!("unexpected staged content digest `{staged_digest}`"),
        );
    }
    if report.failures.is_empty()
        && let Err(error) = ensure_no_symlink_components(root, &prepared.parent_relative)
    {
        report.record_failure("parent-component-revalidation", error);
    }
    if report.failures.is_empty() {
        match prepared.parent.canonicalize() {
            Ok(current_parent) if current_parent == canonical_parent => {}
            Ok(current_parent) => report.record_failure(
                "parent-identity-revalidation",
                format!(
                    "expected `{}`, found `{}`",
                    canonical_parent.display(),
                    current_parent.display()
                ),
            ),
            Err(error) => report.record_failure("parent-identity-revalidation", error),
        }
    }
    if report.failures.is_empty()
        && let Err(error) =
            verify_target_precondition(root, &prepared.relative, input.expected_digest)
    {
        report.record_failure("target-precondition-revalidation", error);
    }
    if report.failures.is_empty() {
        let mutation = if input.operation == TargetOperation::Create {
            policy.publish_create(&staged, &prepared.target)
        } else {
            fs::rename(&staged, &prepared.target)
        };
        match mutation {
            Ok(()) => {
                report.filesystem_mutation_occurred = true;
                report.target_mutation_committed = true;
                if policy == WorkspacePolicy::NativePrivate
                    && let Err(error) = sync_target_parent(&staged)
                {
                    report.record_failure("staged-parent-durability", error);
                }
            }
            Err(error) => report.record_failure("target-mutation", error),
        }
    }
    finalize_apply_report(
        root,
        &prepared.relative,
        prepared.expected_result_digest.as_deref(),
        Some(&canonical_parent_file),
        None,
        &mut created_directory_guard,
        &mut report,
    );
    Ok(report)
}

fn remaining_batch_parent_directories(
    root: &Path,
    target: &BatchJournalTarget,
    journal: &BatchJournal,
) -> HarnessResult<Vec<String>> {
    if target.operation != TargetOperation::Create {
        if target.planned_parent_directories.is_empty() {
            return Ok(Vec::new());
        }
        return Err(HarnessError::InvalidSubmission(
            "non-create batch target has planned parent directories".to_owned(),
        ));
    }
    let mut remaining = Vec::new();
    for relative_path in &target.planned_parent_directories {
        let path = root.join(relative_path);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                remaining.push(relative_path.clone());
            }
            Err(source) => {
                return Err(HarnessError::FileRead {
                    path,
                    message: source.to_string(),
                });
            }
            Ok(metadata)
                if metadata.is_dir()
                    && !metadata.file_type().is_symlink()
                    && journal.created_directories.iter().any(|directory| {
                        directory.relative_path == *relative_path
                            && directory.identity.as_ref().is_some_and(|identity| {
                                persisted_identity_matches(identity, &metadata)
                            })
                    }) => {}
            Ok(_) => {
                return Err(HarnessError::InvalidRepository(format!(
                    "planned batch parent `{relative_path}` exists without a matching batch-created identity"
                )));
            }
        }
    }
    Ok(remaining)
}

fn prepare_batch_parent_directories(
    policy: WorkspacePolicy,
    root: &Path,
    journal_relative_path: &str,
    journal: &mut BatchJournal,
    directories: &[String],
) -> HarnessResult<()> {
    for relative_path in directories {
        if journal
            .created_directories
            .iter()
            .any(|directory| directory.relative_path == *relative_path)
        {
            return Err(HarnessError::InvalidRepository(format!(
                "batch parent `{relative_path}` was already recorded"
            )));
        }
        let path = root.join(relative_path);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(HarnessError::FileRead {
                    path,
                    message: source.to_string(),
                });
            }
            Ok(_) => {
                return Err(HarnessError::InvalidRepository(format!(
                    "planned batch parent `{relative_path}` appeared before Harness created it"
                )));
            }
        }
        journal.created_directories.push(BatchCreatedDirectory {
            relative_path: relative_path.clone(),
            identity: None,
        });
        persist_batch_journal(policy, root, journal_relative_path, journal)?;
        policy
            .create_directory(&path)
            .map_err(|source| HarnessError::FileWrite {
                path: path.clone(),
                message: source.to_string(),
            })?;
        let metadata = fs::symlink_metadata(&path).map_err(|source| HarnessError::FileRead {
            path: path.clone(),
            message: source.to_string(),
        })?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(HarnessError::PathEscapesRoot(path));
        }
        let parent = path.parent().ok_or_else(|| {
            HarnessError::InvalidRepository("batch parent has no parent directory".to_owned())
        })?;
        sync_directory(parent)?;
        let entry = journal
            .created_directories
            .iter_mut()
            .find(|directory| directory.relative_path == *relative_path)
            .ok_or_else(|| {
                HarnessError::InvalidRepository(
                    "created batch parent disappeared from the journal".to_owned(),
                )
            })?;
        entry.identity = Some(persisted_file_identity(&metadata));
        persist_batch_journal(policy, root, journal_relative_path, journal)?;
    }
    Ok(())
}

fn verify_batch_created_parent_identities(
    root: &Path,
    target: &BatchJournalTarget,
    journal: &BatchJournal,
) -> HarnessResult<Option<PersistedFileIdentity>> {
    let mut target_parent_identity = None;
    for relative_path in &target.planned_parent_directories {
        let recorded = journal
            .created_directories
            .iter()
            .find(|directory| directory.relative_path == *relative_path)
            .and_then(|directory| directory.identity.as_ref())
            .ok_or_else(|| {
                HarnessError::InvalidRepository(format!(
                    "batch-created parent `{relative_path}` has no confirmed identity"
                ))
            })?;
        let path = root.join(relative_path);
        let metadata = fs::symlink_metadata(&path).map_err(|source| HarnessError::FileRead {
            path: path.clone(),
            message: source.to_string(),
        })?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || !persisted_identity_matches(recorded, &metadata)
        {
            return Err(HarnessError::InvalidRepository(format!(
                "batch-created parent `{relative_path}` changed identity before target apply"
            )));
        }
        target_parent_identity = Some(recorded.clone());
    }
    Ok(target_parent_identity)
}

fn capture_batch_created_directories(
    root: &Path,
    report: &RepositoryApplyReport,
    journal: &mut BatchJournal,
) -> HarnessResult<()> {
    let mut identity_failure = false;
    for relative_path in &report.created_parent_directories {
        if journal
            .created_directories
            .iter()
            .any(|directory| directory.relative_path == *relative_path)
        {
            continue;
        }
        let path = root.join(relative_path);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                journal.created_directories.push(BatchCreatedDirectory {
                    relative_path: relative_path.clone(),
                    identity: Some(persisted_file_identity(&metadata)),
                });
            }
            Ok(_) => {
                identity_failure = true;
                journal.created_directories.push(BatchCreatedDirectory {
                    relative_path: relative_path.clone(),
                    identity: None,
                });
                journal.failures.push(RepositoryApplyFailure {
                    stage: "batch-created-directory-identity".to_owned(),
                    message: format!("created path `{relative_path}` is not a directory"),
                });
            }
            Err(error) => {
                identity_failure = true;
                journal.created_directories.push(BatchCreatedDirectory {
                    relative_path: relative_path.clone(),
                    identity: None,
                });
                journal.failures.push(RepositoryApplyFailure {
                    stage: "batch-created-directory-identity".to_owned(),
                    message: error.to_string(),
                });
            }
        }
    }
    if identity_failure {
        Err(HarnessError::InvalidRepository(
            "one or more created batch directory identities could not be confirmed".to_owned(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn persisted_file_identity(metadata: &Metadata) -> PersistedFileIdentity {
    PersistedFileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

#[cfg(not(unix))]
fn persisted_file_identity(_metadata: &Metadata) -> PersistedFileIdentity {
    PersistedFileIdentity {
        device: 0,
        inode: 0,
    }
}

#[cfg(unix)]
fn persisted_identity_matches(identity: &PersistedFileIdentity, metadata: &Metadata) -> bool {
    identity.device == metadata.dev() && identity.inode == metadata.ino()
}

#[cfg(not(unix))]
fn persisted_identity_matches(_identity: &PersistedFileIdentity, _metadata: &Metadata) -> bool {
    false
}

fn rollback_failed_batch(
    policy: WorkspacePolicy,
    root: &Path,
    journal_relative_path: &str,
    mut journal: BatchJournal,
    repository_reports: &[Option<RepositoryApplyReport>],
    failure: RepositoryApplyFailure,
) -> BatchApplyReport {
    journal.failures.push(failure);
    journal.phase = BatchJournalPhase::RollingBack;
    let mut recovery_required =
        persist_batch_journal(policy, root, journal_relative_path, &mut journal)
            .map(|()| false)
            .unwrap_or_else(|error| {
                journal.failures.push(RepositoryApplyFailure {
                    stage: "batch-rollback-journal".to_owned(),
                    message: error.to_string(),
                });
                true
            });
    for index in (0..journal.targets.len()).rev() {
        if let Err(error) = rollback_batch_target(root, &journal.targets[index]) {
            recovery_required = true;
            journal.targets[index].state = BatchTargetApplyState::RecoveryRequired;
            journal.targets[index]
                .failures
                .push(RepositoryApplyFailure {
                    stage: "batch-target-rollback".to_owned(),
                    message: error.to_string(),
                });
        } else {
            journal.targets[index].state = BatchTargetApplyState::RolledBack;
        }
        if let Err(error) = persist_batch_journal(policy, root, journal_relative_path, &mut journal)
        {
            recovery_required = true;
            journal.failures.push(RepositoryApplyFailure {
                stage: "batch-target-rollback-journal".to_owned(),
                message: error.to_string(),
            });
        }
    }
    if let Err(error) = rollback_batch_created_directories(root, &journal.created_directories) {
        recovery_required = true;
        journal.failures.push(RepositoryApplyFailure {
            stage: "batch-directory-rollback".to_owned(),
            message: error.to_string(),
        });
    }
    if recovery_required {
        journal.phase = BatchJournalPhase::RecoveryRequired;
        let _ = persist_batch_journal(policy, root, journal_relative_path, &mut journal);
        return batch_report_from_journal(
            journal_relative_path,
            true,
            BatchApplyOutcome::RecoveryRequired,
            journal,
            repository_reports,
        );
    }
    journal.phase = BatchJournalPhase::RolledBack;
    if let Err(error) = persist_batch_journal(policy, root, journal_relative_path, &mut journal)
        .and_then(|()| prepare_batch_cleanup(root, &journal, journal_relative_path))
    {
        journal.failures.push(RepositoryApplyFailure {
            stage: "batch-rollback-cleanup".to_owned(),
            message: error.to_string(),
        });
        journal.phase = BatchJournalPhase::RecoveryRequired;
        let _ = persist_batch_journal(policy, root, journal_relative_path, &mut journal);
        return batch_report_from_journal(
            journal_relative_path,
            true,
            BatchApplyOutcome::RecoveryRequired,
            journal,
            repository_reports,
        );
    }
    batch_report_from_journal(
        journal_relative_path,
        true,
        BatchApplyOutcome::RolledBack,
        journal,
        repository_reports,
    )
}

fn rollback_batch_target(root: &Path, target: &BatchJournalTarget) -> HarnessResult<()> {
    let current = current_target_digest(root, &target.workspace_relative_path)?;
    match target.operation {
        TargetOperation::Create => {
            let intended = target.intended_content_digest.as_deref().ok_or_else(|| {
                HarnessError::InvalidSubmission(
                    "batch create target has no intended digest".to_owned(),
                )
            })?;
            match current.as_deref() {
                None => Ok(()),
                Some(actual) if actual == intended => {
                    let path = root.join(&target.workspace_relative_path);
                    fs::remove_file(&path).map_err(|source| HarnessError::FileWrite {
                        path: path.clone(),
                        message: source.to_string(),
                    })?;
                    sync_target_parent(&path)?;
                    if current_target_digest(root, &target.workspace_relative_path)?.is_none() {
                        Ok(())
                    } else {
                        Err(HarnessError::InvalidRepository(
                            "created batch target still exists after rollback".to_owned(),
                        ))
                    }
                }
                Some(actual) => Err(HarnessError::PlanDrift {
                    expected: intended.to_owned(),
                    actual: actual.to_owned(),
                }),
            }
        }
        TargetOperation::Update | TargetOperation::Delete => {
            let original = target.original_content_digest.as_deref().ok_or_else(|| {
                HarnessError::InvalidSubmission(
                    "batch rollback target has no original digest".to_owned(),
                )
            })?;
            if current.as_deref() == Some(original) {
                remove_batch_backup_if_present(root, target)?;
                return Ok(());
            }
            let state_matches_applied = match target.operation {
                TargetOperation::Update => {
                    current.as_deref() == target.intended_content_digest.as_deref()
                }
                TargetOperation::Delete => current.is_none(),
                TargetOperation::Create | TargetOperation::Inspect => false,
            };
            if !state_matches_applied {
                return Err(HarnessError::PlanDrift {
                    expected: target
                        .intended_content_digest
                        .clone()
                        .unwrap_or_else(|| "absent".to_owned()),
                    actual: current.unwrap_or_else(|| "absent".to_owned()),
                });
            }
            restore_batch_backup(root, target)?;
            let restored = current_target_digest(root, &target.workspace_relative_path)?;
            if restored.as_deref() != Some(original) {
                return Err(HarnessError::PlanDrift {
                    expected: original.to_owned(),
                    actual: restored.unwrap_or_else(|| "absent".to_owned()),
                });
            }
            Ok(())
        }
        TargetOperation::Inspect => Err(HarnessError::InvalidSubmission(
            "inspect target cannot be rolled back".to_owned(),
        )),
    }
}

pub(super) fn current_target_digest(
    root: &Path,
    relative_path: &str,
) -> HarnessResult<Option<String>> {
    let relative = Path::new(relative_path);
    ensure_no_symlink_components(root, relative)?;
    let path = root.join(relative);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(HarnessError::FileRead {
            path,
            message: source.to_string(),
        }),
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            let file = open_verified_file(root, relative)?;
            digest_file(file, MAX_TARGET_BYTES, &path).map(Some)
        }
        Ok(_) => Err(HarnessError::PathEscapesRoot(path)),
    }
}

fn remove_batch_backup_if_present(root: &Path, target: &BatchJournalTarget) -> HarnessResult<()> {
    let Some(relative_path) = target.backup_relative_path.as_deref() else {
        return Ok(());
    };
    let path = root.join(relative_path);
    match fs::remove_file(&path) {
        Ok(()) => sync_target_parent(&path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(HarnessError::FileWrite {
            path,
            message: source.to_string(),
        }),
    }
}

fn remove_batch_staged_if_present(root: &Path, target: &BatchJournalTarget) -> HarnessResult<()> {
    let Some(relative_path) = target.staged_relative_path.as_deref() else {
        return Ok(());
    };
    let path = root.join(relative_path);
    match fs::remove_file(&path) {
        Ok(()) => sync_target_parent(&path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(HarnessError::FileWrite {
            path,
            message: source.to_string(),
        }),
    }
}

fn restore_batch_backup(root: &Path, target: &BatchJournalTarget) -> HarnessResult<()> {
    let relative_path = target.backup_relative_path.as_deref().ok_or_else(|| {
        HarnessError::InvalidSubmission("batch backup path is missing".to_owned())
    })?;
    let backup = root.join(relative_path);
    let target_path = root.join(&target.workspace_relative_path);
    let backup_relative = Path::new(relative_path);
    let original = target.original_content_digest.as_deref().ok_or_else(|| {
        HarnessError::InvalidSubmission("batch original digest is missing".to_owned())
    })?;
    let backup_digest = open_verified_file(root, backup_relative)
        .and_then(|file| digest_file(file, MAX_TARGET_BYTES, &backup))?;
    if backup_digest != original {
        return Err(HarnessError::PlanDrift {
            expected: original.to_owned(),
            actual: backup_digest,
        });
    }
    fs::rename(&backup, &target_path).map_err(|source| HarnessError::FileWrite {
        path: target_path.clone(),
        message: source.to_string(),
    })?;
    sync_target_parent(&target_path)?;
    if let Some(parent) = backup.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

fn rollback_batch_created_directories(
    root: &Path,
    directories: &[BatchCreatedDirectory],
) -> HarnessResult<()> {
    for directory in directories.iter().rev() {
        let path = root.join(&directory.relative_path);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(HarnessError::FileRead {
                    path,
                    message: source.to_string(),
                });
            }
            Ok(metadata)
                if metadata.is_dir()
                    && !metadata.file_type().is_symlink()
                    && directory.identity.as_ref().is_some_and(|identity| {
                        persisted_identity_matches(identity, &metadata)
                    }) =>
            {
                fs::remove_dir(&path).map_err(|source| HarnessError::FileWrite {
                    path: path.clone(),
                    message: format!(
                        "created batch directory could not be removed safely: {source}"
                    ),
                })?;
                sync_target_parent(&path)?;
            }
            Ok(_) => {
                return Err(HarnessError::InvalidRepository(format!(
                    "created batch directory `{}` changed identity",
                    directory.relative_path
                )));
            }
        }
    }
    Ok(())
}

fn verify_all_intended_states(root: &Path, journal: &BatchJournal) -> HarnessResult<()> {
    for target in &journal.targets {
        let current = current_target_digest(root, &target.workspace_relative_path)?;
        if current != target.intended_content_digest {
            return Err(HarnessError::InvalidRepository(format!(
                "batch target `{}` no longer matches the intended state: expected {}, found {}",
                target.workspace_relative_path,
                target
                    .intended_content_digest
                    .clone()
                    .unwrap_or_else(|| "absent".to_owned()),
                current.unwrap_or_else(|| "absent".to_owned())
            )));
        }
    }
    Ok(())
}

fn finalize_applied_batch(
    policy: WorkspacePolicy,
    root: &Path,
    journal_relative_path: &str,
    journal: &mut BatchJournal,
) -> HarnessResult<()> {
    verify_all_intended_states(root, journal)?;
    for target in &journal.targets {
        remove_batch_backup_if_present(root, target)?;
        remove_batch_staged_if_present(root, target)?;
    }
    journal.phase = BatchJournalPhase::CleanupComplete;
    persist_batch_journal(policy, root, journal_relative_path, journal)?;
    prepare_batch_cleanup(root, journal, journal_relative_path)
}

fn prepare_batch_cleanup(
    root: &Path,
    journal: &BatchJournal,
    journal_relative_path: &str,
) -> HarnessResult<()> {
    for target in &journal.targets {
        remove_batch_backup_if_present(root, target)?;
        remove_batch_staged_if_present(root, target)?;
    }
    let backup_directory = root.join(format!("{BATCH_DIRECTORY}/{}/backups", journal.batch_id));
    remove_empty_batch_directory_strict(&backup_directory)?;
    let staged_directory = root.join(format!("{BATCH_DIRECTORY}/{}/staged", journal.batch_id));
    remove_empty_batch_directory_strict(&staged_directory)?;
    let batch_directory = root.join(format!("{BATCH_DIRECTORY}/{}", journal.batch_id));
    ensure_batch_directory_contains_only_journal(&batch_directory, journal_relative_path, true)
}

pub(super) fn complete_batch_cleanup_after_lock_release_with_policy(
    policy: WorkspacePolicy,
    root: &Path,
    mut report: BatchApplyReport,
) -> BatchApplyReport {
    if report.outcome == BatchApplyOutcome::RecoveryRequired {
        return report;
    }
    let journal_relative_path = report.journal_relative_path.clone();
    let completion_receipt_relative_path = completion_receipt_relative_path(&report.batch_id);
    let (journal, journal_location) = match read_completed_or_active_batch_journal(
        root,
        &journal_relative_path,
        &completion_receipt_relative_path,
    ) {
        Ok(value) => value,
        Err(error) => {
            report.outcome = BatchApplyOutcome::RecoveryRequired;
            report.journal_retained = recovery_record_exists(
                root,
                &journal_relative_path,
                &completion_receipt_relative_path,
            );
            report.failures.push(RepositoryApplyFailure {
                stage: "batch-post-lock-journal-read".to_owned(),
                message: error.to_string(),
            });
            return report;
        }
    };
    if let Err(error) = journal
        .validate()
        .and_then(|()| validate_batch_journal_path(&journal.batch_id, &journal_relative_path))
        .and_then(|()| {
            let expected_phase = match report.outcome {
                BatchApplyOutcome::Applied => BatchJournalPhase::CleanupComplete,
                BatchApplyOutcome::RolledBack => BatchJournalPhase::RolledBack,
                BatchApplyOutcome::RecoveryRequired => unreachable!(),
            };
            if journal.phase == expected_phase {
                Ok(())
            } else {
                Err(HarnessError::InvalidRepository(format!(
                    "completed batch cleanup expected phase `{expected_phase:?}`, found `{:?}`",
                    journal.phase
                )))
            }
        })
    {
        report.outcome = BatchApplyOutcome::RecoveryRequired;
        report.journal_retained = true;
        report.failures.push(RepositoryApplyFailure {
            stage: "batch-post-lock-journal-validation".to_owned(),
            message: error.to_string(),
        });
        return report;
    }
    if let Err(error) = move_batch_to_completion_receipt(
        policy,
        root,
        &journal,
        &journal_relative_path,
        &completion_receipt_relative_path,
        journal_location,
    ) {
        report.failures.push(RepositoryApplyFailure {
            stage: "batch-post-lock-cleanup".to_owned(),
            message: error.to_string(),
        });
        report.outcome = BatchApplyOutcome::RecoveryRequired;
        report.journal_retained = recovery_record_exists(
            root,
            &journal_relative_path,
            &completion_receipt_relative_path,
        );
        report.completion_receipt_relative_path = root
            .join(&completion_receipt_relative_path)
            .exists()
            .then_some(completion_receipt_relative_path);
        return report;
    }
    report.journal_retained = false;
    report.completion_receipt_relative_path = Some(completion_receipt_relative_path);
    report
}

fn move_batch_to_completion_receipt(
    policy: WorkspacePolicy,
    root: &Path,
    journal: &BatchJournal,
    journal_relative_path: &str,
    completion_receipt_relative_path: &str,
    journal_location: BatchJournalLocation,
) -> HarnessResult<()> {
    let batch_directory = root.join(format!("{BATCH_DIRECTORY}/{}", journal.batch_id));
    if journal_location == BatchJournalLocation::Active {
        ensure_batch_directory_contains_only_journal(
            &batch_directory,
            journal_relative_path,
            true,
        )?;
        ensure_batch_directory(policy, root, BATCH_COMPLETION_DIRECTORY)?;
        let journal_path = root.join(journal_relative_path);
        let completion_path = root.join(completion_receipt_relative_path);
        match fs::symlink_metadata(&completion_path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(HarnessError::FileRead {
                    path: completion_path,
                    message: source.to_string(),
                });
            }
            Ok(_) => {
                return Err(HarnessError::InvalidRepository(
                    "batch completion receipt already exists before journal promotion".to_owned(),
                ));
            }
        }
        fs::rename(&journal_path, &completion_path).map_err(|source| HarnessError::FileWrite {
            path: completion_path.clone(),
            message: source.to_string(),
        })?;
        // Confirm the durable new name before confirming removal of the old name. A process
        // termination between these fsync calls must leave at least the completion receipt.
        sync_target_parent(&completion_path)?;
        sync_target_parent(&journal_path)?;
    } else if journal_location == BatchJournalLocation::ActiveAndCompletion {
        ensure_batch_directory_contains_only_journal(
            &batch_directory,
            journal_relative_path,
            true,
        )?;
        let journal_path = root.join(journal_relative_path);
        fs::remove_file(&journal_path).map_err(|source| HarnessError::FileWrite {
            path: journal_path.clone(),
            message: source.to_string(),
        })?;
        sync_target_parent(&journal_path)?;
    }
    remove_empty_batch_directory_strict(&batch_directory)?;
    remove_empty_batch_parent_directory(&root.join(BATCH_DIRECTORY))?;
    Ok(())
}

fn completion_receipt_relative_path(batch_id: &str) -> String {
    format!("{BATCH_COMPLETION_DIRECTORY}/{batch_id}.json")
}

fn read_completed_or_active_batch_journal(
    root: &Path,
    journal_relative_path: &str,
    completion_receipt_relative_path: &str,
) -> HarnessResult<(BatchJournal, BatchJournalLocation)> {
    let active = read_optional_batch_journal(root, journal_relative_path)?;
    let completion = read_optional_batch_journal(root, completion_receipt_relative_path)?;
    match (active, completion) {
        (Some(active), None) => Ok((active, BatchJournalLocation::Active)),
        (None, Some(completion)) => Ok((completion, BatchJournalLocation::Completion)),
        (Some(active), Some(completion)) if active == completion => {
            Ok((completion, BatchJournalLocation::ActiveAndCompletion))
        }
        (Some(_), Some(_)) => Err(HarnessError::InvalidRepository(
            "active journal and completion receipt disagree".to_owned(),
        )),
        (None, None) => Err(HarnessError::FileRead {
            path: root.join(journal_relative_path),
            message: "active journal and completion receipt are both absent".to_owned(),
        }),
    }
}

fn read_optional_batch_journal(
    root: &Path,
    relative_path: &str,
) -> HarnessResult<Option<BatchJournal>> {
    match fs::symlink_metadata(root.join(relative_path)) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            read_batch_journal(root, relative_path).map(Some)
        }
        Ok(_) => Err(HarnessError::PathEscapesRoot(root.join(relative_path))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(HarnessError::FileRead {
            path: root.join(relative_path),
            message: source.to_string(),
        }),
    }
}

fn read_recovery_batch_journal(
    root: &Path,
    journal_relative_path: &str,
) -> HarnessResult<(BatchJournal, Option<String>)> {
    let batch_id = Path::new(journal_relative_path)
        .parent()
        .and_then(Path::file_name)
        .and_then(OsStr::to_str)
        .ok_or_else(|| {
            HarnessError::InvalidSubmission("batch journal has no batch ID".to_owned())
        })?;
    validate_batch_identifier(batch_id)?;
    let completion_receipt_relative_path = completion_receipt_relative_path(batch_id);
    read_completed_or_active_batch_journal(
        root,
        journal_relative_path,
        &completion_receipt_relative_path,
    )
    .map(|(journal, location)| {
        (
            journal,
            (location != BatchJournalLocation::Active).then_some(completion_receipt_relative_path),
        )
    })
}

fn recovery_record_exists(
    root: &Path,
    journal_relative_path: &str,
    completion_receipt_relative_path: &str,
) -> bool {
    [journal_relative_path, completion_receipt_relative_path]
        .iter()
        .any(|relative_path| {
            fs::symlink_metadata(root.join(relative_path))
                .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        })
}

fn cleanup_uncommitted_batch_workspace(root: &Path, batch_id: &str) -> HarnessResult<()> {
    validate_batch_identifier(batch_id)?;
    let batch_relative = format!("{BATCH_DIRECTORY}/{batch_id}");
    remove_empty_batch_directory_strict(&root.join(format!("{batch_relative}/backups")))?;
    remove_empty_batch_directory_strict(&root.join(format!("{batch_relative}/staged")))?;
    let journal_path = root.join(format!("{batch_relative}/journal.json"));
    ensure_batch_directory_contains_only_journal(
        &root.join(&batch_relative),
        &format!("{batch_relative}/journal.json"),
        false,
    )?;
    match fs::symlink_metadata(&journal_path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            fs::remove_file(&journal_path).map_err(|source| HarnessError::FileWrite {
                path: journal_path.clone(),
                message: source.to_string(),
            })?;
            sync_target_parent(&journal_path)?;
        }
        Ok(_) => return Err(HarnessError::PathEscapesRoot(journal_path)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(HarnessError::FileRead {
                path: journal_path,
                message: source.to_string(),
            });
        }
    }
    remove_empty_batch_directory_strict(&root.join(&batch_relative))?;
    remove_empty_batch_parent_directory(&root.join(BATCH_DIRECTORY))?;
    remove_empty_batch_parent_directory(&root.join(BATCH_CONTROL_DIRECTORY))
}

fn ensure_batch_directory_contains_only_journal(
    batch_directory: &Path,
    journal_relative_path: &str,
    journal_required: bool,
) -> HarnessResult<()> {
    let entries = match fs::read_dir(batch_directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !journal_required => {
            return Ok(());
        }
        Err(source) => {
            return Err(HarnessError::FileRead {
                path: batch_directory.to_path_buf(),
                message: source.to_string(),
            });
        }
    };
    let expected_journal = Path::new(journal_relative_path)
        .file_name()
        .ok_or_else(|| HarnessError::InvalidSubmission("batch journal has no name".to_owned()))?;
    let mut journal_found = false;
    for entry in entries {
        let entry = entry.map_err(|source| HarnessError::FileRead {
            path: batch_directory.to_path_buf(),
            message: source.to_string(),
        })?;
        if entry.file_name() != expected_journal {
            return Err(HarnessError::InvalidRepository(format!(
                "unexpected batch artifact remains before journal cleanup: `{}`",
                entry.path().display()
            )));
        }
        let file_type = entry.file_type().map_err(|source| HarnessError::FileRead {
            path: entry.path(),
            message: source.to_string(),
        })?;
        if !file_type.is_file() || file_type.is_symlink() {
            return Err(HarnessError::PathEscapesRoot(entry.path()));
        }
        journal_found = true;
    }
    if journal_required && !journal_found {
        return Err(HarnessError::InvalidRepository(
            "batch journal disappeared before cleanup".to_owned(),
        ));
    }
    Ok(())
}

fn remove_empty_batch_directory_strict(path: &Path) -> HarnessResult<()> {
    match fs::remove_dir(path) {
        Ok(()) => {
            if let Some(parent) = path.parent() {
                sync_directory(parent)?;
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(HarnessError::FileWrite {
            path: path.to_path_buf(),
            message: source.to_string(),
        }),
    }
}

fn remove_empty_batch_parent_directory(path: &Path) -> HarnessResult<()> {
    match fs::remove_dir(path) {
        Ok(()) => {
            if let Some(parent) = path.parent() {
                sync_directory(parent)?;
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => Ok(()),
        Err(source) => Err(HarnessError::FileWrite {
            path: path.to_path_buf(),
            message: source.to_string(),
        }),
    }
}

fn sync_target_parent(path: &Path) -> HarnessResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| HarnessError::InvalidRepository("target path has no parent".to_owned()))?;
    sync_directory(parent)
}

fn batch_report_from_journal(
    journal_relative_path: &str,
    journal_retained: bool,
    outcome: BatchApplyOutcome,
    journal: BatchJournal,
    repository_reports: &[Option<RepositoryApplyReport>],
) -> BatchApplyReport {
    let targets = journal
        .targets
        .iter()
        .enumerate()
        .map(|(index, target)| BatchTargetApplyReport {
            workspace_relative_path: target.workspace_relative_path.clone(),
            operation: target.operation,
            original_content_digest: target.original_content_digest.clone(),
            intended_content_digest: target.intended_content_digest.clone(),
            staged_relative_path: target.staged_relative_path.clone(),
            backup_relative_path: target.backup_relative_path.clone(),
            state: target.state,
            repository_apply: repository_reports.get(index).cloned().flatten(),
            failures: target.failures.clone(),
        })
        .collect();
    BatchApplyReport {
        batch_id: journal.batch_id,
        resolved_plan_digest: journal.resolved_plan_digest,
        candidate_digest: journal.candidate_digest,
        journal_relative_path: journal_relative_path.to_owned(),
        completion_receipt_relative_path: None,
        journal_retained,
        outcome,
        targets,
        failures: journal.failures,
    }
}

struct PreparedApplyPaths {
    relative: PathBuf,
    target: PathBuf,
    parent: PathBuf,
    parent_relative: PathBuf,
    initial_parent_file: Option<File>,
    expected_result_digest: Option<String>,
    temporary_name: Option<String>,
}

fn prepare_apply_paths(
    root: &Path,
    input: &ApplyChange<'_>,
    resolved_plan_digest: &str,
    planned_parent_directories: &[String],
    should_validate_context_markdown: bool,
) -> HarnessResult<PreparedApplyPaths> {
    let relative = Path::new(input.relative_path);
    validate_relative_path(relative, "applied file path")?;
    if let (true, Some(content)) = (should_validate_context_markdown, input.content) {
        let markdown = std::str::from_utf8(content).map_err(|error| {
            HarnessError::InvalidSubmission(format!(
                "Vault Markdown content is not valid UTF-8: {error}"
            ))
        })?;
        validate_context_markdown(markdown).map_err(|error| {
            HarnessError::InvalidSubmission(format!("Vault Markdown is invalid: {error}"))
        })?;
    }
    ensure_no_symlink_components(root, relative)?;
    let target = root.join(relative);
    let parent = target
        .parent()
        .ok_or_else(|| {
            HarnessError::InvalidSubmission("applied file has no parent directory".to_owned())
        })?
        .to_path_buf();
    let parent_relative = relative.parent().unwrap_or_else(|| Path::new(""));
    let expected_parent_directories = if input.operation == TargetOperation::Create {
        ensure_existing_ancestor_within_root(root, &target)?;
        missing_parent_directories(root, relative)?
    } else {
        Vec::new()
    };
    if expected_parent_directories != planned_parent_directories {
        return Err(HarnessError::PlanDrift {
            expected: planned_parent_directories.join(","),
            actual: expected_parent_directories.join(","),
        });
    }
    ensure_no_symlink_components(root, parent_relative)?;
    verify_target_precondition(root, relative, input.expected_digest)?;
    let initial_parent_file = open_initial_parent(root, &parent, input.operation)?;
    let temporary_name = temporary_name(&target, resolved_plan_digest, input.operation)?;
    Ok(PreparedApplyPaths {
        relative: relative.to_path_buf(),
        target,
        parent,
        parent_relative: parent_relative.to_path_buf(),
        initial_parent_file,
        expected_result_digest: input.content.map(byte_digest),
        temporary_name,
    })
}

fn open_initial_parent(
    root: &Path,
    parent: &Path,
    operation: TargetOperation,
) -> HarnessResult<Option<File>> {
    if operation == TargetOperation::Create {
        return Ok(None);
    }
    let canonical = parent
        .canonicalize()
        .map_err(|source| HarnessError::FileRead {
            path: parent.to_path_buf(),
            message: source.to_string(),
        })?;
    if !canonical.starts_with(root) || !canonical.is_dir() {
        return Err(HarnessError::PathEscapesRoot(parent.to_path_buf()));
    }
    File::open(&canonical)
        .map(Some)
        .map_err(|source| HarnessError::FileRead {
            path: canonical,
            message: source.to_string(),
        })
}

fn temporary_name(
    target: &Path,
    resolved_plan_digest: &str,
    operation: TargetOperation,
) -> HarnessResult<Option<String>> {
    if operation == TargetOperation::Delete {
        return Ok(None);
    }
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            HarnessError::InvalidSubmission("applied file name is not valid UTF-8".to_owned())
        })?;
    let plan_prefix = resolved_plan_digest.get(..12).ok_or_else(|| {
        HarnessError::InvalidSubmission(
            "resolved-plan digest is too short for a temporary file name".to_owned(),
        )
    })?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| HarnessError::InvalidRepository(error.to_string()))?
        .as_nanos();
    Ok(Some(format!(
        ".{file_name}.harness-{plan_prefix}-{}-{nonce}.tmp",
        std::process::id()
    )))
}

fn apply_delete_report(
    root: &Path,
    prepared: &PreparedApplyPaths,
    mut created_directory_guard: CreatedDirectoryGuard,
    mut report: RepositoryApplyReport,
) -> RepositoryApplyReport {
    match fs::remove_file(&prepared.target) {
        Ok(()) => {
            report.filesystem_mutation_occurred = true;
            report.target_mutation_committed = true;
        }
        Err(error) => report.record_failure("target-delete", error),
    }
    finalize_apply_report(
        root,
        &prepared.relative,
        prepared.expected_result_digest.as_deref(),
        prepared.initial_parent_file.as_ref(),
        None,
        &mut created_directory_guard,
        &mut report,
    );
    report
}

fn apply_content_change_reported(
    root: &Path,
    input: &ApplyChange<'_>,
    prepared: &PreparedApplyPaths,
    planned_parent_directories: &[String],
    mut created_directory_guard: CreatedDirectoryGuard,
    mut report: RepositoryApplyReport,
) -> HarnessResult<RepositoryApplyReport> {
    let content = input.content.ok_or_else(|| {
        HarnessError::InvalidSubmission("create or update content is missing".to_owned())
    })?;
    let temporary_name = prepared.temporary_name.as_deref().ok_or_else(|| {
        HarnessError::InvalidSubmission("temporary file name is missing".to_owned())
    })?;
    let Some((canonical_parent, canonical_parent_file)) = prepare_content_parent(
        root,
        prepared,
        planned_parent_directories,
        &mut created_directory_guard,
        &mut report,
    ) else {
        finalize_apply_report(
            root,
            &prepared.relative,
            prepared.expected_result_digest.as_deref(),
            None,
            None,
            &mut created_directory_guard,
            &mut report,
        );
        return Ok(report);
    };
    let Some((temporary, mut temporary_guard)) = write_temporary_content(
        input,
        prepared,
        &canonical_parent,
        temporary_name,
        content,
        &mut report,
    ) else {
        finalize_apply_report(
            root,
            &prepared.relative,
            prepared.expected_result_digest.as_deref(),
            Some(&canonical_parent_file),
            None,
            &mut created_directory_guard,
            &mut report,
        );
        return Ok(report);
    };
    revalidate_and_commit_content(
        root,
        input,
        prepared,
        &canonical_parent,
        &temporary,
        &mut temporary_guard,
        &mut report,
    );
    finalize_apply_report(
        root,
        &prepared.relative,
        prepared.expected_result_digest.as_deref(),
        Some(&canonical_parent_file),
        Some(&mut temporary_guard),
        &mut created_directory_guard,
        &mut report,
    );
    Ok(report)
}

fn prepare_content_parent(
    root: &Path,
    prepared: &PreparedApplyPaths,
    planned_parent_directories: &[String],
    created_directory_guard: &mut CreatedDirectoryGuard,
    report: &mut RepositoryApplyReport,
) -> Option<(PathBuf, File)> {
    let parents_ready = create_parent_directories_reported(
        root,
        &prepared.parent_relative,
        created_directory_guard,
        report,
    );
    if report.created_parent_directories != planned_parent_directories {
        report.record_failure(
            "created-parent-plan-drift",
            format!(
                "expected `{}`, created `{}`",
                planned_parent_directories.join(","),
                report.created_parent_directories.join(",")
            ),
        );
    }
    if !parents_ready || !report.failures.is_empty() {
        return None;
    }
    let canonical_parent = match prepared.parent.canonicalize() {
        Ok(canonical) if canonical.starts_with(root) && canonical.is_dir() => canonical,
        Ok(canonical) => {
            report.record_failure(
                "parent-canonicalization",
                format!(
                    "parent resolved outside the workspace: {}",
                    canonical.display()
                ),
            );
            return None;
        }
        Err(error) => {
            report.record_failure("parent-canonicalization", error);
            return None;
        }
    };
    let canonical_parent_file = match File::open(&canonical_parent) {
        Ok(file) => file,
        Err(error) => {
            report.record_failure("parent-directory-handle", error);
            return None;
        }
    };
    Some((canonical_parent, canonical_parent_file))
}

fn write_temporary_content(
    input: &ApplyChange<'_>,
    prepared: &PreparedApplyPaths,
    canonical_parent: &Path,
    temporary_name: &str,
    content: &[u8],
    report: &mut RepositoryApplyReport,
) -> Option<(PathBuf, TemporaryFileGuard)> {
    let temporary = canonical_parent.join(temporary_name);
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    let mut temporary_file = match options.open(&temporary) {
        Ok(file) => {
            report.filesystem_mutation_occurred = true;
            file
        }
        Err(error) => {
            report.record_failure("temporary-file-create", error);
            return None;
        }
    };
    let temporary_guard = TemporaryFileGuard::new(temporary.clone());
    if input.operation == TargetOperation::Update
        && let Err(error) = copy_permission_metadata(&prepared.target, &temporary)
    {
        report.record_failure("temporary-file-permissions", error);
    }
    if report.failures.is_empty() {
        match temporary_file
            .write_all(content)
            .and_then(|()| temporary_file.sync_all())
        {
            Ok(()) => report.file_durability = ApplyConfirmation::Confirmed,
            Err(error) => report.record_failure("temporary-file-durability", error),
        }
    }
    drop(temporary_file);
    Some((temporary, temporary_guard))
}

fn revalidate_and_commit_content(
    root: &Path,
    input: &ApplyChange<'_>,
    prepared: &PreparedApplyPaths,
    canonical_parent: &Path,
    temporary: &Path,
    temporary_guard: &mut TemporaryFileGuard,
    report: &mut RepositoryApplyReport,
) {
    if report.failures.is_empty()
        && let Err(error) = ensure_no_symlink_components(root, &prepared.parent_relative)
    {
        report.record_failure("parent-component-revalidation", error);
    }
    if report.failures.is_empty() {
        match prepared.parent.canonicalize() {
            Ok(current_parent) if current_parent == canonical_parent => {}
            Ok(current_parent) => report.record_failure(
                "parent-identity-revalidation",
                format!(
                    "expected `{}`, found `{}`",
                    canonical_parent.display(),
                    current_parent.display()
                ),
            ),
            Err(error) => report.record_failure("parent-identity-revalidation", error),
        }
    }
    if report.failures.is_empty()
        && let Err(error) =
            verify_target_precondition(root, &prepared.relative, input.expected_digest)
    {
        report.record_failure("target-precondition-revalidation", error);
    }
    if report.failures.is_empty() {
        let mutation_result = if input.operation == TargetOperation::Create {
            fs::hard_link(temporary, &prepared.target)
        } else {
            fs::rename(temporary, &prepared.target)
        };
        match mutation_result {
            Ok(()) => {
                report.filesystem_mutation_occurred = true;
                report.target_mutation_committed = true;
                if input.operation == TargetOperation::Update {
                    temporary_guard.disarm();
                }
            }
            Err(error) => report.record_failure("target-mutation", error),
        }
    }
}

fn finalize_apply_report(
    root: &Path,
    relative: &Path,
    expected_result_digest: Option<&str>,
    canonical_parent: Option<&File>,
    temporary_guard: Option<&mut TemporaryFileGuard>,
    created_directory_guard: &mut CreatedDirectoryGuard,
    report: &mut RepositoryApplyReport,
) {
    if let Some(temporary_guard) = temporary_guard {
        temporary_guard.cleanup(report);
    } else if report.operation != TargetOperation::Delete {
        report.temporary_cleanup = ApplyConfirmation::NotRequired;
    }
    if let Some(canonical_parent) = canonical_parent {
        match canonical_parent.sync_all() {
            Ok(()) => report.parent_directory_durability = ApplyConfirmation::Confirmed,
            Err(error) => report.record_failure("parent-directory-durability", error),
        }
    }
    verify_final_target_state(root, relative, expected_result_digest, report);
    if report.target_mutation_committed
        || !matches!(report.verified_target_state, VerifiedTargetState::Absent)
    {
        created_directory_guard.retain(report);
    } else {
        created_directory_guard.rollback(report);
    }
}

fn verify_final_target_state(
    root: &Path,
    relative: &Path,
    expected_result_digest: Option<&str>,
    report: &mut RepositoryApplyReport,
) {
    if let Err(error) = ensure_no_symlink_components(root, relative) {
        report.record_failure("final-target-component-verification", error);
        return;
    }
    let target = root.join(relative);
    match fs::symlink_metadata(&target) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            report.verified_target_state = VerifiedTargetState::Absent;
            if expected_result_digest.is_some() {
                report.record_failure(
                    "final-target-verification",
                    "target is absent after a content mutation",
                );
            }
        }
        Err(error) => report.record_failure("final-target-verification", error),
        Ok(metadata) if !metadata.is_file() => {
            report.record_failure("final-target-verification", "target is not a regular file");
        }
        Ok(_) => match open_verified_file(root, relative)
            .and_then(|file| digest_file(file, MAX_TARGET_BYTES, &target))
        {
            Ok(content_digest) => {
                report.verified_target_state = VerifiedTargetState::Present {
                    content_digest: content_digest.clone(),
                };
                if expected_result_digest.is_none_or(|expected| expected != content_digest) {
                    report.record_failure(
                        "final-target-verification",
                        format!("unexpected resulting content digest `{content_digest}`"),
                    );
                }
            }
            Err(error) => report.record_failure("final-target-verification", error),
        },
    }
}

fn verify_target_precondition(
    root: &Path,
    relative: &Path,
    expected_digest: Option<&str>,
) -> HarnessResult<()> {
    let target = root.join(relative);
    match expected_digest {
        Some(expected) => {
            let file = open_verified_file(root, relative)?;
            let actual = digest_file(file, MAX_TARGET_BYTES, &target)?;
            if actual != expected {
                return Err(HarnessError::PlanDrift {
                    expected: expected.to_owned(),
                    actual,
                });
            }
        }
        None if target.exists() => {
            return Err(HarnessError::PlanDrift {
                expected: "absent".to_owned(),
                actual: "existing".to_owned(),
            });
        }
        None => {}
    }
    Ok(())
}

fn create_parent_directories_reported(
    root: &Path,
    relative_parent: &Path,
    guard: &mut CreatedDirectoryGuard,
    report: &mut RepositoryApplyReport,
) -> bool {
    let mut current = root.to_path_buf();
    for component in relative_parent.components() {
        let Component::Normal(part) = component else {
            report.record_failure(
                "created-parent-validation",
                "parent directory contains an unsupported path component",
            );
            return false;
        };
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                report.record_failure(
                    "created-parent-validation",
                    format!("parent path is not a directory: {}", current.display()),
                );
                return false;
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let relative = match current.strip_prefix(root) {
                    Ok(relative) => portable_path(relative),
                    Err(error) => {
                        report.record_failure("created-parent-validation", error);
                        return false;
                    }
                };
                if let Err(error) = fs::create_dir(&current) {
                    report.record_failure("created-parent-create", error);
                    return false;
                }
                report.filesystem_mutation_occurred = true;
                report.created_parent_directories.push(relative.clone());
                let metadata = match fs::symlink_metadata(&current) {
                    Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                        Some(metadata)
                    }
                    Ok(_) => {
                        report.record_failure(
                            "created-parent-identity",
                            "created parent path is not a directory",
                        );
                        None
                    }
                    Err(error) => {
                        report.record_failure("created-parent-identity", error);
                        None
                    }
                };
                let identity_confirmed = metadata.is_some();
                guard.directories.push(CreatedDirectoryIdentity {
                    relative_path: relative,
                    metadata,
                });
                if !identity_confirmed {
                    return false;
                }
                if let Some(parent) = current.parent()
                    && let Err(error) = sync_directory(parent)
                {
                    report.record_failure("created-parent-durability", error);
                    return false;
                }
            }
            Err(error) => {
                report.record_failure("created-parent-inspection", error);
                return false;
            }
        }
    }
    true
}

fn sync_directory(path: &Path) -> HarnessResult<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| HarnessError::FileWrite {
            path: path.to_path_buf(),
            message: source.to_string(),
        })
}

#[cfg(unix)]
pub(super) fn same_file(first: &Metadata, second: &Metadata) -> bool {
    first.dev() == second.dev() && first.ino() == second.ino()
}

#[cfg(not(unix))]
pub(super) fn same_file(_first: &Metadata, _second: &Metadata) -> bool {
    false
}

pub(super) fn digest_file(mut file: File, max_bytes: u64, path: &Path) -> HarnessResult<String> {
    let before = file.metadata().map_err(|source| HarnessError::FileRead {
        path: path.to_path_buf(),
        message: source.to_string(),
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    let mut total = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| HarnessError::FileRead {
                path: path.to_path_buf(),
                message: source.to_string(),
            })?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or_else(|| HarnessError::InvalidRepository("file size overflow".to_owned()))?;
        if total > max_bytes {
            return Err(HarnessError::InvalidRepository(format!(
                "file `{}` exceeds the {max_bytes} byte limit",
                path.display()
            )));
        }
        hasher.update(&buffer[..read]);
    }
    let after = file.metadata().map_err(|source| HarnessError::FileRead {
        path: path.to_path_buf(),
        message: source.to_string(),
    })?;
    if !same_file_version(&before, &after) {
        return Err(HarnessError::InvalidRepository(format!(
            "file `{}` changed while it was being hashed",
            path.display()
        )));
    }
    Ok(finalize_content_digest(hasher))
}

pub(super) fn read_text_file(
    mut file: File,
    max_bytes: u64,
    path: &Path,
) -> HarnessResult<(String, String)> {
    let before = file.metadata().map_err(|source| HarnessError::FileRead {
        path: path.to_path_buf(),
        message: source.to_string(),
    })?;
    let mut bytes = Vec::new();
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| HarnessError::FileRead {
                path: path.to_path_buf(),
                message: source.to_string(),
            })?;
        if read == 0 {
            break;
        }
        let next = bytes
            .len()
            .checked_add(read)
            .ok_or_else(|| HarnessError::InvalidRepository("text file size overflow".to_owned()))?;
        if next as u64 > max_bytes {
            return Err(HarnessError::InvalidRepository(format!(
                "file `{}` exceeds the {max_bytes} byte limit",
                path.display()
            )));
        }
        hasher.update(&buffer[..read]);
        bytes.extend_from_slice(&buffer[..read]);
    }
    let after = file.metadata().map_err(|source| HarnessError::FileRead {
        path: path.to_path_buf(),
        message: source.to_string(),
    })?;
    if !same_file_version(&before, &after) {
        return Err(HarnessError::InvalidRepository(format!(
            "file `{}` changed while it was being read",
            path.display()
        )));
    }
    let content = String::from_utf8(bytes).map_err(|error| HarnessError::FileRead {
        path: path.to_path_buf(),
        message: format!("file is not valid UTF-8: {error}"),
    })?;
    let content_digest = finalize_content_digest(hasher);
    Ok((content, content_digest))
}

fn finalize_content_digest(hasher: Sha256) -> String {
    fn lowercase_hex_digit(value: u8) -> char {
        match value {
            0..=9 => char::from(b'0' + value),
            10..=15 => char::from(b'a' + value - 10),
            _ => '?',
        }
    }

    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(lowercase_hex_digit(byte >> 4));
        encoded.push(lowercase_hex_digit(byte & 0x0f));
    }
    encoded
}

pub(super) fn markdown_title(content: &str, path: &Path) -> String {
    content
        .lines()
        .find_map(|line| line.strip_prefix("# ").map(str::trim))
        .filter(|title| !title.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "Untitled".to_owned())
}

#[cfg(unix)]
pub(super) fn same_file_version(first: &Metadata, second: &Metadata) -> bool {
    same_file(first, second)
        && first.len() == second.len()
        && first.ctime() == second.ctime()
        && first.ctime_nsec() == second.ctime_nsec()
}

#[cfg(not(unix))]
pub(super) fn same_file_version(_first: &Metadata, _second: &Metadata) -> bool {
    false
}

pub(super) fn canonical_directory(path: &Path, label: &str) -> HarnessResult<PathBuf> {
    let canonical = path.canonicalize().map_err(|source| {
        HarnessError::InvalidRepository(format!(
            "{label} `{}` cannot be resolved: {source}",
            path.display()
        ))
    })?;
    if !canonical.is_dir() {
        return Err(HarnessError::InvalidRepository(format!(
            "{label} `{}` is not a directory",
            path.display()
        )));
    }
    Ok(canonical)
}

#[cfg(test)]
pub(super) fn apply_change_batch_reported(
    root: &Path,
    inputs: &[BatchApplyInput<'_>],
    resolved_plan_digest: &str,
    candidate_digest: &str,
) -> HarnessResult<BatchApplyReport> {
    apply_change_batch_reported_with_policy(
        WorkspacePolicy::External,
        root,
        inputs,
        resolved_plan_digest,
        candidate_digest,
    )
}

#[cfg(test)]
pub(super) fn apply_change_batch_reported_before_cleanup_for_test(
    root: &Path,
    inputs: &[BatchApplyInput<'_>],
    resolved_plan_digest: &str,
    candidate_digest: &str,
) -> HarnessResult<BatchApplyReport> {
    apply_change_batch_reported_before_cleanup_for_test_with_policy(
        WorkspacePolicy::External,
        root,
        inputs,
        resolved_plan_digest,
        candidate_digest,
    )
}

#[cfg(test)]
pub(super) fn apply_change_batch_reported_for_attempt_with_test_hook(
    root: &Path,
    inputs: &[BatchApplyInput<'_>],
    resolved_plan_digest: &str,
    candidate_digest: &str,
    attempt_identifier: &str,
    before_target_apply: impl FnMut(usize) -> HarnessResult<()>,
) -> HarnessResult<BatchApplyReport> {
    apply_change_batch_reported_for_attempt_with_test_hook_with_policy(
        WorkspacePolicy::External,
        root,
        inputs,
        resolved_plan_digest,
        candidate_digest,
        attempt_identifier,
        before_target_apply,
    )
}

#[cfg(test)]
pub(super) fn apply_change_batch_reported_with_test_hook(
    root: &Path,
    inputs: &[BatchApplyInput<'_>],
    resolved_plan_digest: &str,
    candidate_digest: &str,
    before_target_apply: impl FnMut(usize) -> HarnessResult<()>,
) -> HarnessResult<BatchApplyReport> {
    apply_change_batch_reported_with_test_hook_with_policy(
        WorkspacePolicy::External,
        root,
        inputs,
        resolved_plan_digest,
        candidate_digest,
        before_target_apply,
    )
}

#[cfg(test)]
pub(super) fn complete_batch_cleanup_after_lock_release(
    root: &Path,
    report: BatchApplyReport,
) -> BatchApplyReport {
    complete_batch_cleanup_after_lock_release_with_policy(WorkspacePolicy::External, root, report)
}

#[cfg(test)]
pub(super) fn recover_change_batch_reported(
    root: &Path,
    journal_relative_path: &str,
) -> HarnessResult<BatchApplyReport> {
    recover_change_batch_reported_with_policy(
        WorkspacePolicy::External,
        root,
        journal_relative_path,
    )
}
