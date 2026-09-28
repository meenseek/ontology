use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fmt::{self, Display, Formatter, Write as _},
    fs::{self, File, Metadata},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};
#[cfg(unix)]
use std::{
    os::{
        fd::{FromRawFd, IntoRawFd},
        unix::{ffi::OsStringExt, fs::MetadataExt},
    },
    ptr::NonNull,
};

#[cfg(unix)]
use errno::{Errno, errno, set_errno};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};

use crate::{
    document::{contains_raw_conversation_path, is_skipped_directory_name},
    frontmatter::{ParsedFrontmatter, split_markdown_frontmatter},
};

/// The only executable Harness record schema.
///
/// Every other value is rejected. Transient runs are rebuilt from canonical inputs.
pub const HARNESS_SCHEMA_VERSION: u32 = 8;
const AI_COLLABORATION_VALUES_PATH: &str = "vault/personal/decisions/ai-collaboration-values.md";
const SOLO_MVP_IDEA_REGISTRY_PATH: &str =
    "vault/personal/projects/ideas/idea-discovery-registry.md";
const MAX_COMPANY_LENGTH: usize = 128;
const MAX_TARGET_LENGTH: usize = 4_096;
const MAX_OBJECTIVE_LENGTH: usize = 16_384;
const MAX_TARGETS: usize = 64;
const MAX_POLICY_BYTES: u64 = 5 * 1024 * 1024;
const MAX_TARGET_BYTES: u64 = 100 * 1024 * 1024;
const MAX_CANDIDATE_BYTES: usize = 10 * 1024 * 1024;
const MAX_SUBMISSION_LIST_ITEMS: usize = 128;
const MAX_SUBMISSION_TEXT_BYTES: usize = 16 * 1024;
const MAX_REVISIONS: u32 = 5;
pub(crate) const MAX_PLANNED_ROLES: usize = 4;
const MAX_RUNTIME_CONCURRENT_ROLES: usize = 5;
const MAX_ROLE_EXECUTION_MILLIS: u64 = 24 * 60 * 60 * 1_000;
const MAX_ROLE_GRACE_MILLIS: u64 = 5 * 60 * 1_000;
const MAX_RETRIEVAL_QUERY_LENGTH: usize = 1_024;
const MAX_RETRIEVAL_BYTES: usize = 1024 * 1024;
const MAX_RETRIEVAL_FILES: usize = 1_000;
const MAX_RETRIEVAL_SECTIONS: usize = 4_096;
const MAX_RETRIEVAL_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_RETRIEVAL_SCAN_BYTES: usize = 20 * 1024 * 1024;
const MAX_LEARNING_SOURCES: usize = 64;
const MAX_LEARNING_FILE_BYTES: u64 = 512 * 1024;
const MAX_LEARNING_SCAN_FILES: usize = 1_000;
const MAX_LEARNING_SCAN_BYTES: usize = 20 * 1024 * 1024;
const MAX_ROLE_BUNDLE_BYTES: usize = 8 * 1024 * 1024;
const MAX_ROLE_CONTROL_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompanyDomainRoute {
    pub(crate) signals: Vec<String>,
    pub(crate) actions: Vec<HarnessAction>,
    pub(crate) target_kind: CompanyDomainRouteTarget,
    pub(crate) policies: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CompanyDomainRouteTarget {
    Any,
    Rust,
}

impl CompanyDomainRouteTarget {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Rust => "rust",
        }
    }

    fn matches(self, request: &HarnessRequest) -> bool {
        match self {
            Self::Any => true,
            Self::Rust => request.targets.iter().any(|target| is_rust_target(target)),
        }
    }
}

#[derive(Debug, Deserialize)]
struct CompanyRoutingFrontmatter {
    domain_routes: Vec<CompanyDomainRoute>,
}
const MAX_ROLE_INVOCATION_BYTES: usize = 32 * 1024 * 1024;
// An evaluation embeds its role record and at most one derived copy of the record's disjoint
// output fields; 4 MiB covers the fixed evaluation structure. A current head can contain the
// finite revision history plus current evaluation, current role record, every bounded invocation,
// and 32 MiB of fixed/tool evidence. The terminal attempt adds one bounded apply receipt and
// 16 MiB of wrapper evidence. Durable JSON is compact, so the outer ceiling closes these layers.
const MAX_ROLE_EXECUTION_RECORD_BYTES: usize = 16 * 1024 * 1024;
const MAX_TASK_EVALUATION_BYTES: usize = 2 * MAX_ROLE_EXECUTION_RECORD_BYTES + 4 * 1024 * 1024;
const MAX_CURRENT_EXECUTION_RECORD_BYTES: usize = (MAX_REVISIONS as usize + 1)
    * MAX_TASK_EVALUATION_BYTES
    + MAX_ROLE_EXECUTION_RECORD_BYTES
    + MAX_PLANNED_ROLES * MAX_ROLE_INVOCATION_BYTES
    + 32 * 1024 * 1024;
const MAX_APPLY_RECEIPT_BYTES: usize = 64 * 1024 * 1024;
const MAX_APPLY_ATTEMPT_BYTES: usize =
    MAX_CURRENT_EXECUTION_RECORD_BYTES + MAX_APPLY_RECEIPT_BYTES + 16 * 1024 * 1024;
const MAX_DURABLE_RUN_FILE_BYTES: u64 = MAX_APPLY_ATTEMPT_BYTES as u64;

fn ensure_single_hard_link(file: &File, path: &Path) -> HarnessResult<()> {
    #[cfg(unix)]
    {
        let metadata = file.metadata().map_err(|source| HarnessError::FileRead {
            path: path.to_path_buf(),
            message: source.to_string(),
        })?;
        if metadata.nlink() != 1 {
            return Err(HarnessError::InvalidRequest(format!(
                "scoped file `{}` must have exactly one hard link",
                path.display()
            )));
        }
    }
    #[cfg(not(unix))]
    let _ = (file, path);
    Ok(())
}

pub(crate) fn verified_repository_file(
    repository_root: &Path,
    relative_path: &Path,
) -> HarnessResult<File> {
    let root = canonical_directory(repository_root, "repository root")?;
    let file = open_verified_file(&root, relative_path)?;
    ensure_single_hard_link(&file, &root.join(relative_path))?;
    Ok(file)
}

#[cfg(unix)]
struct RetrievalDirectoryStream {
    pointer: Option<NonNull<libc::DIR>>,
}

#[cfg(unix)]
impl RetrievalDirectoryStream {
    fn from_file(file: File) -> io::Result<Self> {
        let descriptor = file.into_raw_fd();
        // SAFETY: `descriptor` is open and uniquely owned here. `fdopendir` takes ownership only
        // on success; the failure branch reconstructs the `File` exactly once.
        let pointer = unsafe { libc::fdopendir(descriptor) };
        if let Some(pointer) = NonNull::new(pointer) {
            return Ok(Self {
                pointer: Some(pointer),
            });
        }
        let error = io::Error::last_os_error();
        // SAFETY: `fdopendir` returned null and therefore did not consume `descriptor`.
        drop(unsafe { File::from_raw_fd(descriptor) });
        Err(error)
    }

    fn pointer(&self) -> io::Result<*mut libc::DIR> {
        self.pointer
            .map(NonNull::as_ptr)
            .ok_or_else(|| io::Error::other("directory stream is already closed"))
    }

    fn close(mut self) -> io::Result<()> {
        let Some(pointer) = self.pointer.take() else {
            return Ok(());
        };
        // SAFETY: this guard uniquely owns the live stream, and taking it prevents double close.
        if unsafe { libc::closedir(pointer.as_ptr()) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

#[cfg(unix)]
impl Drop for RetrievalDirectoryStream {
    fn drop(&mut self) {
        if let Some(pointer) = self.pointer.take() {
            // SAFETY: a remaining pointer is a uniquely owned live stream; Drop is fallback only.
            unsafe {
                libc::closedir(pointer.as_ptr());
            }
        }
    }
}

#[cfg(unix)]
fn read_directory_names(file: File, max_entries: usize) -> io::Result<Vec<OsString>> {
    let stream = RetrievalDirectoryStream::from_file(file)?;
    let read_result = (|| -> io::Result<Vec<OsString>> {
        let pointer = stream.pointer()?;
        let mut names = Vec::new();
        loop {
            set_errno(Errno(0));
            // SAFETY: `pointer` is a live, uniquely owned `DIR*`; the returned record is copied
            // before the next `readdir` call and is never retained.
            let entry = unsafe { libc::readdir(pointer) };
            if entry.is_null() {
                let error = errno();
                if error.0 == 0 {
                    break;
                }
                return Err(io::Error::from_raw_os_error(error.0));
            }

            // SAFETY: `entry` is non-null from `readdir`; `addr_of!` computes field addresses
            // without creating an outliving reference to the transient record.
            let record_length = usize::from(unsafe { (*entry).d_reclen });
            let record_address = entry.cast::<u8>() as usize;
            let name_address = unsafe { std::ptr::addr_of!((*entry).d_name).cast::<u8>() as usize };
            let name_offset = name_address.checked_sub(record_address).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "directory record name offset is invalid",
                )
            })?;
            let name_bound = record_length.checked_sub(name_offset).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "directory record length is invalid",
                )
            })?;
            if name_bound == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "directory record has no name storage",
                ));
            }
            // SAFETY: the slice is bounded by this record's `d_reclen` minus its actual `d_name`
            // offset, and it is consumed before another `readdir` call can invalidate the record.
            let record_name =
                unsafe { std::slice::from_raw_parts(name_address as *const u8, name_bound) };
            let name_length = record_name
                .iter()
                .position(|byte| *byte == 0)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "directory record name is not terminated",
                    )
                })?;
            if name_length == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "directory record name is empty",
                ));
            }
            let name = &record_name[..name_length];
            if name == b"." || name == b".." {
                continue;
            }
            if names.len() >= max_entries {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "source directory exceeds its child limit",
                ));
            }
            names.push(OsString::from_vec(name.to_vec()));
        }
        Ok(names)
    })();
    let close_result = stream.close();
    match (read_result, close_result) {
        (Ok(names), Ok(())) => Ok(names),
        (Err(read_error), Ok(())) => Err(read_error),
        (Ok(_), Err(close_error)) => Err(close_error),
        (Err(read_error), Err(close_error)) => Err(io::Error::other(format!(
            "directory read failed: {read_error}; directory close also failed: {close_error}"
        ))),
    }
}
const DEPENDENCY_FILE_NAMES: &[&str] = &[
    ".terraform.lock.hcl",
    "build.gradle",
    "build.gradle.kts",
    "bun.lock",
    "bun.lockb",
    "cargo.lock",
    "cargo.toml",
    "composer.json",
    "composer.lock",
    "conda-lock.yml",
    "conanfile.py",
    "conanfile.txt",
    "deno.lock",
    "directory.packages.props",
    "environment.yml",
    "environment.yaml",
    "gemfile",
    "gemfile.lock",
    "gems.locked",
    "go.mod",
    "go.sum",
    "go.work",
    "go.work.sum",
    "gradle.lockfile",
    "libs.versions.toml",
    "mix.exs",
    "mix.lock",
    "npm-shrinkwrap.json",
    "package-lock.json",
    "package.json",
    "package.resolved",
    "package.swift",
    "packages.config",
    "packages.lock.json",
    "pipfile",
    "pipfile.lock",
    "pnpm-lock.yaml",
    "poetry.lock",
    "pom.xml",
    "pubspec.lock",
    "pubspec.yaml",
    "pyproject.toml",
    "rust-toolchain.toml",
    "setup.cfg",
    "setup.py",
    "uv.lock",
    "vcpkg-configuration.json",
    "vcpkg.json",
    "yarn.lock",
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessAction {
    CodeWrite,
    CodeReview,
    DocumentWrite,
    DocumentReview,
    Investigation,
    Design,
    Ideation,
    VaultRead,
    VaultCuration,
}

impl HarnessAction {
    #[must_use]
    pub const fn is_write(self) -> bool {
        matches!(
            self,
            Self::CodeWrite | Self::DocumentWrite | Self::VaultCuration
        )
    }

    #[must_use]
    pub const fn is_review(self) -> bool {
        matches!(self, Self::CodeReview | Self::DocumentReview)
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CodeWrite => "code-write",
            Self::CodeReview => "code-review",
            Self::DocumentWrite => "document-write",
            Self::DocumentReview => "document-review",
            Self::Investigation => "investigation",
            Self::Design => "design",
            Self::Ideation => "ideation",
            Self::VaultRead => "vault-read",
            Self::VaultCuration => "vault-curation",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum DataOwner {
    Profile,
    CommonWork,
    PersonalBusiness,
    Personal,
    PersonalProject { project: String },
    Company { company: String },
    CompanyProject { company: String, project: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CurationKind {
    Idea,
    Knowledge,
    Fact,
    Decision,
    Journal,
    Ontology,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextGrantPurpose {
    CareerWritingEvidence,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextAccess {
    ReadOnly,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CareerOutputSurface {
    General,
    Resume,
    CareerDescription,
    Portfolio,
    ProfessionalProfile,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum HarnessIntent {
    General,
    CareerArtifact { surface: CareerOutputSurface },
    PolicyMaintenance,
    OutputAdapter { surface: CareerOutputSurface },
    SoloMvpIdeation,
}

impl HarnessIntent {
    #[must_use]
    pub const fn career_surface(self) -> Option<CareerOutputSurface> {
        match self {
            Self::CareerArtifact { surface } => Some(surface),
            Self::General
            | Self::PolicyMaintenance
            | Self::OutputAdapter { .. }
            | Self::SoloMvpIdeation => None,
        }
    }

    #[must_use]
    pub const fn output_surface(self) -> Option<CareerOutputSurface> {
        match self {
            Self::CareerArtifact { surface } | Self::OutputAdapter { surface } => Some(surface),
            Self::General | Self::PolicyMaintenance | Self::SoloMvpIdeation => None,
        }
    }

    #[must_use]
    pub const fn route_segment(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::CareerArtifact { .. } => "career-artifact",
            Self::PolicyMaintenance => "policy-maintenance",
            Self::OutputAdapter { .. } => "output-adapter",
            Self::SoloMvpIdeation => "solo-mvp-ideation",
        }
    }

    #[must_use]
    pub const fn is_career_artifact(self) -> bool {
        matches!(self, Self::CareerArtifact { .. })
    }

    #[must_use]
    pub const fn is_output_adapter(self) -> bool {
        matches!(self, Self::OutputAdapter { .. })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CareerCoverageMode {
    Selected,
    Complete,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CareerOwnerExclusion {
    pub owner: DataOwner,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CareerClaimLineage {
    pub claim_id: String,
    pub evidence_owner: DataOwner,
    pub evidence_source_paths: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CareerCompositionManifest {
    pub version: u32,
    pub career_output_surface: CareerOutputSurface,
    pub artifact_targets: Vec<String>,
    pub coverage: CareerCoverageMode,
    pub complete_coverage_confirmation_reported: bool,
    pub evidence_owners: Vec<DataOwner>,
    pub canonical_evidence_owners: Vec<DataOwner>,
    pub excluded_evidence_owners: Vec<CareerOwnerExclusion>,
    pub claim_lineage: Vec<CareerClaimLineage>,
}

impl CareerCompositionManifest {
    fn normalized(&self) -> Self {
        let mut normalized = self.clone();
        normalized.artifact_targets.sort();
        normalized.evidence_owners.sort();
        normalized.canonical_evidence_owners.sort();
        normalized.excluded_evidence_owners.sort_by(|left, right| {
            left.owner
                .cmp(&right.owner)
                .then_with(|| left.reason.cmp(&right.reason))
        });
        for lineage in &mut normalized.claim_lineage {
            lineage.evidence_source_paths.sort();
        }
        normalized.claim_lineage.sort_by(|left, right| {
            left.claim_id
                .cmp(&right.claim_id)
                .then_with(|| left.evidence_owner.cmp(&right.evidence_owner))
                .then_with(|| left.evidence_source_paths.cmp(&right.evidence_source_paths))
        });
        normalized
    }

    fn validate(
        &self,
        request: &HarnessRequest,
        career_output_surface: Option<CareerOutputSurface>,
    ) -> HarnessResult<()> {
        if self.version != HARNESS_SCHEMA_VERSION {
            return Err(HarnessError::InvalidRequest(format!(
                "career composition manifest version must be {HARNESS_SCHEMA_VERSION}"
            )));
        }
        if career_output_surface != Some(self.career_output_surface) {
            return Err(HarnessError::InvalidRequest(
                "career composition manifest surface does not match the request".to_owned(),
            ));
        }
        let mut artifact_targets = self.artifact_targets.clone();
        artifact_targets.sort();
        if artifact_targets != request.targets {
            return Err(HarnessError::InvalidRequest(
                "career composition manifest targets do not match the request".to_owned(),
            ));
        }
        if self.evidence_owners.is_empty() || self.evidence_owners.len() > MAX_TARGETS {
            return Err(HarnessError::InvalidRequest(format!(
                "career composition manifest must declare between one and {MAX_TARGETS} evidence owners"
            )));
        }
        validate_evidence_owners("career manifest evidence owner", &self.evidence_owners)?;

        self.validate_coverage()?;

        if self.claim_lineage.is_empty() || self.claim_lineage.len() > MAX_TARGETS {
            return Err(HarnessError::InvalidRequest(format!(
                "career composition manifest must declare between one and {MAX_TARGETS} claim lineages"
            )));
        }
        let mut claim_ids = BTreeSet::new();
        let mut lineage_owners = BTreeSet::new();
        for lineage in &self.claim_lineage {
            validate_single_line("career claim ID", &lineage.claim_id, MAX_TARGET_LENGTH)?;
            if !claim_ids.insert(lineage.claim_id.clone()) {
                return Err(HarnessError::InvalidRequest(
                    "career composition manifest claim IDs must be unique".to_owned(),
                ));
            }
            validate_evidence_owner(&lineage.evidence_owner)?;
            lineage_owners.insert(lineage.evidence_owner.clone());
            validate_evidence_source_paths(&lineage.evidence_source_paths)?;
        }
        if lineage_owners
            != self
                .evidence_owners
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
        {
            return Err(HarnessError::InvalidRequest(
                "career claim-lineage owners must exactly match declared evidence owners"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_coverage(&self) -> HarnessResult<()> {
        let active_owners = match self.coverage {
            CareerCoverageMode::Selected => {
                if self.complete_coverage_confirmation_reported {
                    return Err(HarnessError::InvalidRequest(
                        "selected career coverage must not report complete-coverage confirmation"
                            .to_owned(),
                    ));
                }
                if !self.canonical_evidence_owners.is_empty()
                    || !self.excluded_evidence_owners.is_empty()
                {
                    return Err(HarnessError::InvalidRequest(
                        "selected career coverage must not declare canonical or excluded owners"
                            .to_owned(),
                    ));
                }
                self.evidence_owners.clone()
            }
            CareerCoverageMode::Complete => self.complete_coverage_owners()?,
        };
        let declared_owners = self
            .evidence_owners
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if active_owners.into_iter().collect::<BTreeSet<_>>() != declared_owners {
            return Err(HarnessError::InvalidRequest(
                "career manifest evidence owners do not match its declared coverage".to_owned(),
            ));
        }
        Ok(())
    }

    fn complete_coverage_owners(&self) -> HarnessResult<Vec<DataOwner>> {
        if !self.complete_coverage_confirmation_reported {
            return Err(HarnessError::InvalidRequest(
                "complete career coverage requires explicit reported user confirmation".to_owned(),
            ));
        }
        if self.canonical_evidence_owners.is_empty() {
            return Err(HarnessError::InvalidRequest(
                "complete career coverage requires a caller-attested canonical owner set"
                    .to_owned(),
            ));
        }
        validate_evidence_owners(
            "canonical career evidence owner",
            &self.canonical_evidence_owners,
        )?;
        let canonical = self
            .canonical_evidence_owners
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut excluded = BTreeSet::new();
        for exclusion in &self.excluded_evidence_owners {
            validate_evidence_owner(&exclusion.owner)?;
            validate_single_line(
                "career owner exclusion reason",
                &exclusion.reason,
                MAX_SUBMISSION_TEXT_BYTES,
            )?;
            if !canonical.contains(&exclusion.owner) || !excluded.insert(exclusion.owner.clone()) {
                return Err(HarnessError::InvalidRequest(
                    "career owner exclusions must be unique members of the canonical owner set"
                        .to_owned(),
                ));
            }
        }
        Ok(canonical.difference(&excluded).cloned().collect())
    }

    fn evidence_source_paths_for(&self, owner: &DataOwner) -> Vec<String> {
        self.claim_lineage
            .iter()
            .filter(|lineage| &lineage.evidence_owner == owner)
            .flat_map(|lineage| lineage.evidence_source_paths.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

impl CareerOutputSurface {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Resume => "resume",
            Self::CareerDescription => "career-description",
            Self::Portfolio => "portfolio",
            Self::ProfessionalProfile => "professional-profile",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ContextGrant {
    pub owner: DataOwner,
    pub purpose: ContextGrantPurpose,
    pub access: ContextAccess,
}

impl DataOwner {
    fn validate(&self) -> HarnessResult<()> {
        match self {
            Self::PersonalProject { project } => validate_partition_id("project", project)?,
            Self::Company { company } => validate_company_id(company)?,
            Self::CompanyProject { company, project } => {
                validate_company_id(company)?;
                validate_partition_id("project", project)?;
            }
            Self::Profile | Self::CommonWork | Self::PersonalBusiness | Self::Personal => {}
        }
        Ok(())
    }

    #[must_use]
    pub fn route_segment(&self) -> String {
        match self {
            Self::Profile => "profile".to_owned(),
            Self::CommonWork => "common-work".to_owned(),
            Self::PersonalBusiness => "personal-business".to_owned(),
            Self::Personal => "personal".to_owned(),
            Self::PersonalProject { project } => format!("personal-project-{project}"),
            Self::Company { company } => format!("company-{company}"),
            Self::CompanyProject { company, project } => {
                format!("company-{company}-project-{project}")
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HarnessRequest {
    pub action: HarnessAction,
    pub owner: DataOwner,
    pub targets: Vec<String>,
    pub objective: String,
    pub curation_kind: Option<CurationKind>,
    pub explicit_user_confirmation_reported: bool,
    pub curation_sources: Vec<String>,
    pub delete_targets: Vec<String>,
}

impl HarnessRequest {
    fn validate(&self) -> HarnessResult<()> {
        self.owner.validate()?;
        if self.owner == DataOwner::CommonWork
            && (self.action != HarnessAction::DocumentWrite
                || self.curation_kind.is_some()
                || !self.curation_sources.is_empty()
                || !self.delete_targets.is_empty())
        {
            return Err(HarnessError::InvalidRequest(
                "common work maintenance only updates existing Markdown documents".to_owned(),
            ));
        }
        self.validate_targets()?;
        self.validate_action_target_kinds()?;
        self.validate_curation_sources()?;
        validate_multiline("objective", &self.objective, MAX_OBJECTIVE_LENGTH)?;
        self.validate_code_owner()?;
        self.validate_vault_operation()
    }

    fn validate_targets(&self) -> HarnessResult<()> {
        if self.action == HarnessAction::VaultRead && !self.targets.is_empty() {
            return Err(HarnessError::InvalidRequest(
                "Vault read uses bounded retrieval and does not accept file targets".to_owned(),
            ));
        }
        let file_targets_required = matches!(
            self.action,
            HarnessAction::CodeWrite
                | HarnessAction::CodeReview
                | HarnessAction::DocumentWrite
                | HarnessAction::DocumentReview
                | HarnessAction::VaultCuration
        );
        if (file_targets_required && self.targets.is_empty()) || self.targets.len() > MAX_TARGETS {
            return Err(HarnessError::InvalidRequest(format!(
                "this action requires between one and {MAX_TARGETS} targets"
            )));
        }
        if !self.targets.is_empty() {
            ensure_unique_strings("target", &self.targets)
                .map_err(|error| HarnessError::InvalidRequest(error.to_string()))?;
        }
        if !self.delete_targets.is_empty() {
            ensure_unique_strings("delete target", &self.delete_targets)
                .map_err(|error| HarnessError::InvalidRequest(error.to_string()))?;
            if !self.action.is_write() {
                return Err(HarnessError::InvalidRequest(
                    "delete targets are only valid for write actions".to_owned(),
                ));
            }
            for target in &self.delete_targets {
                if self.targets.binary_search(target).is_err() {
                    return Err(HarnessError::InvalidRequest(format!(
                        "delete target `{target}` is not in the target set"
                    )));
                }
            }
        }
        for target in &self.targets {
            validate_single_line("target", target, MAX_TARGET_LENGTH)?;
            validate_relative_path(Path::new(target), "target")?;
        }
        for (index, target) in self.targets.iter().enumerate() {
            for other in self.targets.iter().skip(index + 1) {
                let (ancestor, descendant) = if Path::new(other).starts_with(Path::new(target)) {
                    (target, other)
                } else if Path::new(target).starts_with(Path::new(other)) {
                    (other, target)
                } else {
                    continue;
                };
                return Err(HarnessError::InvalidRequest(format!(
                    "target `{ancestor}` cannot be an ancestor of target `{descendant}`"
                )));
            }
        }
        Ok(())
    }

    fn validate_curation_sources(&self) -> HarnessResult<()> {
        if self.curation_sources.len() > MAX_TARGETS {
            return Err(HarnessError::InvalidRequest(format!(
                "curation sources must not exceed {MAX_TARGETS} paths"
            )));
        }
        ensure_unique_strings("curation source", &self.curation_sources)
            .map_err(|error| HarnessError::InvalidRequest(error.to_string()))?;
        for source in &self.curation_sources {
            validate_single_line("curation source", source, MAX_TARGET_LENGTH)?;
            validate_relative_path(Path::new(source), "curation source")?;
        }
        Ok(())
    }

    fn validate_action_target_kinds(&self) -> HarnessResult<()> {
        for target in &self.targets {
            let is_document = is_document_target(target);
            let mismatch = match self.action {
                HarnessAction::CodeWrite | HarnessAction::CodeReview => is_document,
                HarnessAction::DocumentWrite | HarnessAction::DocumentReview => !is_document,
                _ => false,
            };
            if mismatch {
                return Err(HarnessError::InvalidRequest(format!(
                    "target `{target}` is incompatible with the `{}` action",
                    self.action.as_str()
                )));
            }
        }
        Ok(())
    }

    fn validate_code_owner(&self) -> HarnessResult<()> {
        if matches!(
            self.action,
            HarnessAction::CodeWrite | HarnessAction::CodeReview
        ) {
            match self.owner {
                DataOwner::Personal | DataOwner::PersonalBusiness => {
                    return Err(HarnessError::InvalidRequest(
                        "personal code work requires a personal-project owner".to_owned(),
                    ));
                }
                DataOwner::Company { .. } => {
                    return Err(HarnessError::InvalidRequest(
                        "company code work requires a company project".to_owned(),
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn validate_vault_operation(&self) -> HarnessResult<()> {
        match self.action {
            HarnessAction::VaultCuration => {
                let kind = self.curation_kind.ok_or_else(|| {
                    HarnessError::InvalidRequest(
                        "Vault curation requires a curation kind".to_owned(),
                    )
                })?;
                if !self.explicit_user_confirmation_reported {
                    return Err(HarnessError::InvalidRequest(
                        "Vault curation requires explicit user confirmation".to_owned(),
                    ));
                }
                validate_private_content_owner(&self.owner, kind)?;
                if is_personal_idea_curation(self.action, &self.owner, Some(kind))
                    && !self
                        .targets
                        .iter()
                        .any(|target| target == SOLO_MVP_IDEA_REGISTRY_PATH)
                    && !self
                        .curation_sources
                        .iter()
                        .any(|source| source == SOLO_MVP_IDEA_REGISTRY_PATH)
                {
                    return Err(HarnessError::InvalidRequest(format!(
                        "personal Idea curation must bind `{SOLO_MVP_IDEA_REGISTRY_PATH}` as its target or curation source"
                    )));
                }
            }
            HarnessAction::VaultRead => {
                let kind = self.curation_kind.ok_or_else(|| {
                    HarnessError::InvalidRequest("Vault read requires a content kind".to_owned())
                })?;
                if !self.curation_sources.is_empty() || !self.delete_targets.is_empty() {
                    return Err(HarnessError::InvalidRequest(
                        "Vault read does not accept curation sources or delete targets".to_owned(),
                    ));
                }
                validate_private_content_owner(&self.owner, kind)?;
                let confirmation_required = kind == CurationKind::Journal;
                if self.explicit_user_confirmation_reported != confirmation_required {
                    return Err(HarnessError::InvalidRequest(
                        "journal read requires explicit confirmation; other Vault reads do not"
                            .to_owned(),
                    ));
                }
            }
            _ => {
                if self.curation_kind.is_some()
                    || self.explicit_user_confirmation_reported
                    || !self.curation_sources.is_empty()
                {
                    return Err(HarnessError::InvalidRequest(
                        "content kind, confirmation, and curation sources are only valid for Vault operations"
                            .to_owned(),
                    ));
                }
            }
        }
        Ok(())
    }
}

fn validate_private_content_owner(owner: &DataOwner, kind: CurationKind) -> HarnessResult<()> {
    if matches!(kind, CurationKind::Journal | CurationKind::Ontology)
        && owner != &DataOwner::Personal
    {
        return Err(HarnessError::InvalidRequest(
            "journal and ontology operations require the personal owner".to_owned(),
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessRole {
    Writer,
    Verifier,
    Reviewer,
    Specialist,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkflowSubjectSource {
    TaskContract,
    FrozenTargets,
    PrimaryProducer,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkflowRoleNode {
    pub role: HarnessRole,
    pub dependencies: Vec<HarnessRole>,
    pub subject_source: WorkflowSubjectSource,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VerificationUnit {
    CompletionContract,
    ScopeCompliance,
    TerminologyAndReadability,
    CodeCorrectness,
    TestsAndStaticAnalysis,
    SourceOwnership,
    FactAndClaimBoundary,
    EvidenceAndUncertainty,
    IdeaNotPromotedToDecision,
    SoloMvpIdeationContract,
    RetrievalScopeAndDigest,
    CurationProvenance,
    SourceAndOntologyBoundary,
    CareerEvidenceLineage,
    CareerSurfaceContract,
    CareerPerspectiveRouting,
    CareerPublicSafety,
    CareerOutputSurfaceSelection,
    ResumeFirstScreenAndArtifact,
    CareerDescriptionCaseStructure,
    PortfolioLocalBuildAndPublicCopy,
    ProfessionalProfileArtifact,
    CareerHolisticCoherence,
    OutputAdapterContract,
    OutputAdapterSurfaceSelection,
    ResumeOutputAdapterArtifact,
    CareerDescriptionOutputAdapterArtifact,
    ProfessionalProfileOutputAdapterArtifact,
}

impl VerificationUnit {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CompletionContract => "completion-contract",
            Self::ScopeCompliance => "scope-compliance",
            Self::TerminologyAndReadability => "terminology-and-readability",
            Self::CodeCorrectness => "code-correctness",
            Self::TestsAndStaticAnalysis => "tests-and-static-analysis",
            Self::SourceOwnership => "source-ownership",
            Self::FactAndClaimBoundary => "fact-and-claim-boundary",
            Self::EvidenceAndUncertainty => "evidence-and-uncertainty",
            Self::IdeaNotPromotedToDecision => "idea-not-promoted-to-decision",
            Self::SoloMvpIdeationContract => "solo-mvp-ideation-contract",
            Self::RetrievalScopeAndDigest => "retrieval-scope-and-digest",
            Self::CurationProvenance => "curation-provenance",
            Self::SourceAndOntologyBoundary => "source-and-ontology-boundary",
            Self::CareerEvidenceLineage => "career-evidence-lineage",
            Self::CareerSurfaceContract => "career-surface-contract",
            Self::CareerPerspectiveRouting => "career-perspective-routing",
            Self::CareerPublicSafety => "career-public-safety",
            Self::CareerOutputSurfaceSelection => "career-output-surface-selection",
            Self::ResumeFirstScreenAndArtifact => "resume-first-screen-and-artifact",
            Self::CareerDescriptionCaseStructure => "career-description-case-structure",
            Self::PortfolioLocalBuildAndPublicCopy => "portfolio-local-build-and-public-copy",
            Self::ProfessionalProfileArtifact => "professional-profile-artifact",
            Self::CareerHolisticCoherence => "career-holistic-coherence",
            Self::OutputAdapterContract => "output-adapter-contract",
            Self::OutputAdapterSurfaceSelection => "output-adapter-surface-selection",
            Self::ResumeOutputAdapterArtifact => "resume-output-adapter-artifact",
            Self::CareerDescriptionOutputAdapterArtifact => {
                "career-description-output-adapter-artifact"
            }
            Self::ProfessionalProfileOutputAdapterArtifact => {
                "professional-profile-output-adapter-artifact"
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum VerificationOwner {
    Deterministic,
    Tool,
    Role { role: HarnessRole },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvaluationSubjectKind {
    TaskContract,
    FrozenTargets,
    ProducedArtifact,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationRequirement {
    pub unit: VerificationUnit,
    pub owner: VerificationOwner,
    pub subject: EvaluationSubjectKind,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionAssurance {
    Advisory,
    Enforced,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessExecutionProfile {
    Standard,
    Strict,
}

impl HarnessExecutionProfile {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Strict => "strict",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContextRoot {
    pub repository_relative_path: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct PolicyBinding {
    pub id: String,
    pub repository_relative_path: String,
    pub content_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RolePolicyBinding {
    pub role: HarnessRole,
    pub policies: Vec<PolicyBinding>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkspacePolicyBinding {
    pub workspace_relative_path: String,
    pub content_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SourceBinding {
    pub repository_relative_path: String,
    pub content_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearningMetadata {
    pub learning_id: String,
    pub owner: DataOwner,
    pub action: HarnessAction,
    pub intent: HarnessIntent,
    pub verification_unit: VerificationUnit,
    pub source_task_evaluation_receipt_digest: String,
    pub promotion_handoff_digest: String,
}

impl LearningMetadata {
    fn validate(&self) -> HarnessResult<()> {
        validate_learning_identifier(&self.learning_id)?;
        self.owner.validate()?;
        validate_plan_digest(
            "learning source task evaluation receipt digest",
            &self.source_task_evaluation_receipt_digest,
        )?;
        validate_plan_digest(
            "learning promotion handoff digest",
            &self.promotion_handoff_digest,
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct LearningDocumentFrontmatter {
    title: String,
    scope: String,
    export: bool,
    learning: LearningMetadata,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LearningSourceBinding {
    pub repository_relative_path: String,
    pub content_digest: String,
    pub metadata: LearningMetadata,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HarnessContextDocument {
    pub repository_relative_path: String,
    pub content_digest: String,
    pub title: String,
    pub content: String,
    pub start_line: usize,
    pub end_line: usize,
    pub matched_terms: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HarnessContextBundle {
    pub resolved_plan_digest: String,
    pub query: String,
    pub max_bytes: usize,
    pub documents: Vec<HarnessContextDocument>,
    pub total_content_bytes: usize,
    pub bundle_digest: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessBoundDocumentSource {
    Target,
    Policy,
    WorkspacePolicy,
    Learning,
    Evidence,
    Curation,
    Retrieval,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HarnessBoundDocument {
    pub source: HarnessBoundDocumentSource,
    pub relative_path: String,
    pub content_digest: String,
    pub content: String,
    pub start_line: usize,
    pub end_line: usize,
    pub matched_terms: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum HarnessRoleSegment {
    ControlHead {
        content: String,
        content_digest: String,
    },
    BoundDocument {
        document: HarnessBoundDocument,
    },
    ControlTail {
        content: String,
        content_digest: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HarnessRoleBundle {
    pub resolved_plan_digest: String,
    pub role: HarnessRole,
    pub segments: Vec<HarnessRoleSegment>,
    pub total_content_bytes: usize,
    pub serialized_bytes: usize,
    pub bundle_digest: String,
}

impl HarnessRoleBundle {
    fn bound_documents(&self) -> impl Iterator<Item = &HarnessBoundDocument> {
        self.segments.iter().filter_map(|segment| match segment {
            HarnessRoleSegment::BoundDocument { document } => Some(document),
            HarnessRoleSegment::ControlHead { .. } | HarnessRoleSegment::ControlTail { .. } => None,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoleTaskScope {
    pub action: HarnessAction,
    pub owner: DataOwner,
    pub intent: HarnessIntent,
    pub context_grants: Vec<ContextGrant>,
    pub career_manifest_digest: Option<String>,
    pub career_claim_lineage: Vec<CareerClaimLineage>,
    pub evidence_sources: Vec<SourceBinding>,
    pub learning_sources: Vec<LearningSourceBinding>,
    pub task_statement: String,
    pub allowed_context_roots: Vec<ContextRoot>,
    pub denied_context_roots: Vec<ContextRoot>,
    pub retrieval_roots: Vec<ContextRoot>,
    pub context_bundle_digest: Option<String>,
    pub promotion_handoff: Option<PromotionHandoff>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "role")]
pub enum RoleTaskContract {
    Writer {
        targets: Vec<TargetBinding>,
        curation_kind: Option<CurationKind>,
    },
    Specialist {
        source_targets: Vec<TargetBinding>,
        verification_requirements: Vec<VerificationRequirement>,
    },
    Verifier {
        targets: Vec<TargetBinding>,
        verification_requirements: Vec<VerificationRequirement>,
    },
    Reviewer {
        targets: Vec<TargetBinding>,
        verification_requirements: Vec<VerificationRequirement>,
    },
}

impl RoleTaskContract {
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
/// Static prepare-time metadata for one Harness role.
pub struct PreparedRoleMetadata {
    pub resolved_plan_digest: String,
    pub contract_digest: String,
    pub decision_trace_digest: String,
    pub request_provenance: RequestProvenanceBinding,
    pub execution_order: usize,
    pub role: HarnessRole,
    pub independent_context_required: bool,
    /// Digest of the prepared static role bundle; this is not the final role-input digest.
    pub role_bundle_digest: String,
    pub required_policies: Vec<PolicyBinding>,
    pub workspace_policies: Vec<WorkspacePolicyBinding>,
    pub scope: RoleTaskScope,
    pub task: RoleTaskContract,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreparedRoleRun {
    pub source_versions: SourceVersionSet,
    pub resolved_plan_digest: String,
    pub runtime_capabilities_digest: String,
    /// Digest of the complete role preparation, before the outer prepared Harness run is bound.
    pub prepared_role_run_digest: String,
    pub assurance: ExecutionAssurance,
    pub runtime_capabilities: RoleRuntimeCapabilities,
    pub context_bundle: Option<HarnessContextBundle>,
    pub role_bundles: Vec<HarnessRoleBundle>,
    pub role_metadata: Vec<PreparedRoleMetadata>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoleRuntimeCapabilities {
    pub available_roles: Vec<HarnessRole>,
    pub max_concurrent_roles: usize,
    pub separate_contexts: bool,
    pub file_reading: bool,
    pub tool_execution: bool,
    pub max_role_bundle_bytes: usize,
    pub max_role_invocation_bytes: usize,
    pub max_role_execution_millis: u64,
    pub max_role_grace_millis: u64,
}

fn validate_role_runtime_capabilities(
    plan: &ResolvedHarnessPlan,
    capabilities: &RoleRuntimeCapabilities,
) -> HarnessResult<()> {
    if capabilities.max_role_bundle_bytes == 0
        || capabilities.max_role_bundle_bytes > MAX_ROLE_BUNDLE_BYTES
    {
        return Err(HarnessError::UnsupportedRuntime(format!(
            "runtime role bundle limit must be between 1 and {MAX_ROLE_BUNDLE_BYTES} bytes"
        )));
    }
    if capabilities.max_role_invocation_bytes == 0
        || capabilities.max_role_invocation_bytes > MAX_ROLE_INVOCATION_BYTES
    {
        return Err(HarnessError::UnsupportedRuntime(format!(
            "runtime role invocation limit must be between 1 and {MAX_ROLE_INVOCATION_BYTES} bytes"
        )));
    }
    if capabilities.max_role_execution_millis == 0
        || capabilities.max_role_execution_millis > MAX_ROLE_EXECUTION_MILLIS
    {
        return Err(HarnessError::UnsupportedRuntime(format!(
            "runtime role execution limit must be between 1 and {MAX_ROLE_EXECUTION_MILLIS} milliseconds"
        )));
    }
    if capabilities.max_role_grace_millis == 0
        || capabilities.max_role_grace_millis > MAX_ROLE_GRACE_MILLIS
    {
        return Err(HarnessError::UnsupportedRuntime(format!(
            "runtime role grace limit must be between 1 and {MAX_ROLE_GRACE_MILLIS} milliseconds"
        )));
    }
    if capabilities.max_concurrent_roles == 0
        || capabilities.max_concurrent_roles > MAX_RUNTIME_CONCURRENT_ROLES
    {
        return Err(HarnessError::UnsupportedRuntime(format!(
            "runtime concurrency must be between one and {MAX_RUNTIME_CONCURRENT_ROLES}"
        )));
    }
    if capabilities.max_concurrent_roles < plan.required_concurrent_roles {
        return Err(HarnessError::UnsupportedRuntime(format!(
            "runtime concurrency is {}, but the plan requires {}",
            capabilities.max_concurrent_roles, plan.required_concurrent_roles
        )));
    }
    let available = capabilities
        .available_roles
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if available.len() != capabilities.available_roles.len() {
        return Err(HarnessError::UnsupportedRuntime(
            "runtime role list contains duplicates".to_owned(),
        ));
    }
    let missing = plan
        .roles()
        .filter(|role| !available.contains(role))
        .map(|role| format!("{role:?}"))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(HarnessError::UnsupportedRuntime(format!(
            "runtime does not provide required roles: {}",
            missing.join(", ")
        )));
    }
    if !capabilities.file_reading {
        return Err(HarnessError::UnsupportedRuntime(
            "runtime cannot read bound policy and target files".to_owned(),
        ));
    }
    if (plan.contains_role(HarnessRole::Verifier) || action_requires_tool_execution(plan.action))
        && !capabilities.tool_execution
    {
        return Err(HarnessError::UnsupportedRuntime(
            "runtime cannot execute required verification tools".to_owned(),
        ));
    }
    if plan.role_count() > 1 && !capabilities.separate_contexts {
        return Err(HarnessError::UnsupportedRuntime(
            "runtime cannot provide separate role contexts".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
const fn default_max_role_bundle_bytes() -> usize {
    MAX_ROLE_BUNDLE_BYTES
}

#[cfg(test)]
const fn default_max_role_invocation_bytes() -> usize {
    16 * 1024 * 1024
}

#[cfg(test)]
const fn default_max_role_execution_millis() -> u64 {
    60 * 60 * 1_000
}

#[cfg(test)]
const fn default_max_role_grace_millis() -> u64 {
    5 * 60 * 1_000
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AppliedHarnessChange {
    pub resolved_plan_digest: String,
    pub candidate_digest: String,
    pub validation_receipt_digest: String,
    pub workspace_relative_path: String,
    pub operation: TargetOperation,
    pub resulting_content_digest: Option<String>,
    pub created_parent_directories: Vec<String>,
    pub completion_state: HarnessCompletionState,
    pub lifecycle_receipt: HarnessApplyLifecycleReceipt,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BatchApplyOutcomeReceipt {
    Applied,
    RolledBack,
    RecoveryRequired,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BatchTargetApplyStateReceipt {
    Pending,
    Staged,
    BackedUp,
    Applied,
    RolledBack,
    RecoveryRequired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BatchTargetApplyReceipt {
    pub workspace_relative_path: String,
    pub operation: TargetOperation,
    pub original_content_digest: Option<String>,
    pub intended_content_digest: Option<String>,
    pub staged_relative_path: Option<String>,
    pub backup_relative_path: Option<String>,
    pub state: BatchTargetApplyStateReceipt,
    pub repository_apply: Option<RepositoryApplyReceipt>,
    pub failures: Vec<LifecycleFailureReceipt>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HarnessBatchApplyLifecycleReceipt {
    pub batch_id: String,
    pub resolved_plan_digest: String,
    pub candidate_digest: String,
    pub journal_relative_path: String,
    pub completion_receipt_relative_path: Option<String>,
    pub journal_retained: bool,
    pub outcome: BatchApplyOutcomeReceipt,
    pub lock: WorkspaceMutationLockReceipt,
    pub targets: Vec<BatchTargetApplyReceipt>,
    pub failure_history: Vec<LifecycleFailureReceipt>,
    pub orchestration_failures: Vec<LifecycleFailureReceipt>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AppliedHarnessBatch {
    pub resolved_plan_digest: String,
    pub candidate_digest: String,
    /// Digest of the accepted task evaluation that authorized this engine batch.
    pub task_evaluation_receipt_digest: String,
    pub batch_id: String,
    pub targets: Vec<BatchTargetApplyReceipt>,
    pub completion_state: HarnessCompletionState,
    pub lifecycle_receipt: HarnessBatchApplyLifecycleReceipt,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LifecycleConfirmationReceipt {
    NotRequired,
    Confirmed,
    Unconfirmed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LifecycleFailureReceipt {
    pub stage: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkspaceMutationLockReceipt {
    pub lock_file_creation_committed: bool,
    pub lock_content_durability: LifecycleConfirmationReceipt,
    pub creation_parent_durability: LifecycleConfirmationReceipt,
    pub explicit_unlink_committed: bool,
    pub verified_final_absence: LifecycleConfirmationReceipt,
    pub unlink_parent_durability: LifecycleConfirmationReceipt,
    pub failures: Vec<LifecycleFailureReceipt>,
}

impl WorkspaceMutationLockReceipt {
    fn is_fully_confirmed(&self) -> bool {
        self.lock_file_creation_committed
            && self.lock_content_durability == LifecycleConfirmationReceipt::Confirmed
            && self.creation_parent_durability == LifecycleConfirmationReceipt::Confirmed
            && self.release_cleanup_confirmed()
            && self.failures.is_empty()
    }

    fn release_cleanup_confirmed(&self) -> bool {
        self.explicit_unlink_committed
            && self.verified_final_absence == LifecycleConfirmationReceipt::Confirmed
            && self.unlink_parent_durability == LifecycleConfirmationReceipt::Confirmed
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub enum VerifiedTargetStateReceipt {
    Unverified,
    Present { content_digest: String },
    Absent,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CreatedParentDirectoryStateReceipt {
    NotCreated,
    Retained,
    RolledBack,
    Unconfirmed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RepositoryApplyReceipt {
    pub workspace_relative_path: String,
    pub operation: TargetOperation,
    pub filesystem_mutation_occurred: bool,
    pub target_mutation_committed: bool,
    pub verified_target_state: VerifiedTargetStateReceipt,
    pub file_durability: LifecycleConfirmationReceipt,
    pub parent_directory_durability: LifecycleConfirmationReceipt,
    pub temporary_cleanup: LifecycleConfirmationReceipt,
    pub created_parent_directories: Vec<String>,
    pub created_parent_directory_state: CreatedParentDirectoryStateReceipt,
    pub failures: Vec<LifecycleFailureReceipt>,
}

impl RepositoryApplyReceipt {
    #[cfg(test)]
    fn is_fully_confirmed(&self) -> bool {
        let expected_state_confirmed = matches!(
            (&self.operation, &self.verified_target_state),
            (TargetOperation::Delete, VerifiedTargetStateReceipt::Absent)
                | (
                    TargetOperation::Create | TargetOperation::Update,
                    VerifiedTargetStateReceipt::Present { .. }
                )
        );
        let file_durability_confirmed = match self.operation {
            TargetOperation::Delete => {
                self.file_durability == LifecycleConfirmationReceipt::NotRequired
            }
            TargetOperation::Create | TargetOperation::Update => {
                self.file_durability == LifecycleConfirmationReceipt::Confirmed
            }
            TargetOperation::Inspect => false,
        };
        let created_parent_state_confirmed = if self.created_parent_directories.is_empty() {
            self.created_parent_directory_state == CreatedParentDirectoryStateReceipt::NotCreated
        } else {
            self.created_parent_directory_state == CreatedParentDirectoryStateReceipt::Retained
        };
        self.filesystem_mutation_occurred
            && self.target_mutation_committed
            && expected_state_confirmed
            && file_durability_confirmed
            && self.parent_directory_durability == LifecycleConfirmationReceipt::Confirmed
            && matches!(
                self.temporary_cleanup,
                LifecycleConfirmationReceipt::NotRequired | LifecycleConfirmationReceipt::Confirmed
            )
            && created_parent_state_confirmed
            && self.failures.is_empty()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HarnessApplyLifecycleReceipt {
    pub lock: WorkspaceMutationLockReceipt,
    pub target_apply: Option<RepositoryApplyReceipt>,
    pub orchestration_failures: Vec<LifecycleFailureReceipt>,
}

impl HarnessApplyLifecycleReceipt {
    #[cfg(test)]
    fn is_fully_confirmed(&self) -> bool {
        self.orchestration_failures.is_empty()
            && self.lock.is_fully_confirmed()
            && self
                .target_apply
                .as_ref()
                .is_some_and(RepositoryApplyReceipt::is_fully_confirmed)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub enum TargetState {
    Absent,
    Existing { content_digest: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetOperation {
    Create,
    Update,
    Delete,
    Inspect,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TargetBinding {
    pub workspace_relative_path: String,
    pub state: TargetState,
    pub operation: TargetOperation,
    pub parent_directories_to_create: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResolvedHarnessPlan {
    pub source_versions: SourceVersionSet,
    pub version: u32,
    pub route_id: String,
    pub request_digest: String,
    pub contract_digest: String,
    pub decision_trace_digest: String,
    pub request_provenance: RequestProvenanceBinding,
    pub execution_profile: HarnessExecutionProfile,
    pub action: HarnessAction,
    pub owner: DataOwner,
    pub intent: HarnessIntent,
    pub context_grants: Vec<ContextGrant>,
    pub career_manifest_digest: Option<String>,
    pub evidence_sources: Vec<SourceBinding>,
    pub learning_sources: Vec<LearningSourceBinding>,
    pub curation_kind: Option<CurationKind>,
    pub curation_sources: Vec<SourceBinding>,
    pub promotion_handoff: Option<PromotionHandoff>,
    pub targets: Vec<TargetBinding>,
    pub workflow: Vec<WorkflowRoleNode>,
    pub primary_producer_role: HarnessRole,
    pub allowed_context_roots: Vec<ContextRoot>,
    pub denied_context_roots: Vec<ContextRoot>,
    pub retrieval_roots: Vec<ContextRoot>,
    pub required_policies: Vec<PolicyBinding>,
    pub orchestrator_policies: Vec<PolicyBinding>,
    pub role_policy_bindings: Vec<RolePolicyBinding>,
    pub workspace_policies: Vec<WorkspacePolicyBinding>,
    pub verification_requirements: Vec<VerificationRequirement>,
    pub required_concurrent_roles: usize,
    pub max_revisions: u32,
    pub minimum_assurance: ExecutionAssurance,
    pub source_write_allowed: bool,
    pub external_write_allowed: bool,
}

impl ResolvedHarnessPlan {
    fn roles(&self) -> impl Iterator<Item = HarnessRole> + '_ {
        self.workflow.iter().map(|node| node.role)
    }

    fn role_count(&self) -> usize {
        self.workflow.len()
    }

    fn contains_role(&self, role: HarnessRole) -> bool {
        self.workflow.iter().any(|node| node.role == role)
    }

    fn role_position(&self, role: HarnessRole) -> Option<usize> {
        self.workflow.iter().position(|node| node.role == role)
    }

    fn workflow_node(&self, role: HarnessRole) -> Option<&WorkflowRoleNode> {
        self.workflow.iter().find(|node| node.role == role)
    }

    fn require_executable_version(&self) -> HarnessResult<()> {
        if self.version != HARNESS_SCHEMA_VERSION {
            return Err(HarnessError::UnsupportedPlanVersion {
                found: self.version,
                current: HARNESS_SCHEMA_VERSION,
            });
        }
        Ok(())
    }

    fn validate(&self) -> HarnessResult<()> {
        self.source_versions.validate()?;
        self.validate_limits_and_permissions()?;
        self.request_provenance.validate()?;
        validate_plan_digest("contract digest", &self.contract_digest)?;
        validate_plan_digest("decision trace digest", &self.decision_trace_digest)?;
        if self.request_provenance.decision_trace_digest != self.decision_trace_digest {
            return Err(HarnessError::InvalidPlan(
                "request provenance does not match the plan decision trace".to_owned(),
            ));
        }
        self.validate_promotion_handoff()?;
        self.validate_curation_source_bindings()?;
        self.validate_evidence_source_bindings()?;
        self.validate_learning_source_bindings()?;
        self.validate_target_bindings()?;
        self.validate_roles_and_context()?;
        ensure_unique_verification_requirements(&self.verification_requirements)?;
        self.validate_verification_requirements()?;
        ensure_unique_policy_ids(&self.required_policies)?;
        ensure_unique_policy_ids(&self.orchestrator_policies)?;
        self.validate_role_policy_bindings()?;
        ensure_unique_workspace_policy_paths(&self.workspace_policies)
    }

    fn validate_limits_and_permissions(&self) -> HarnessResult<()> {
        self.require_executable_version()?;
        if self.workflow.is_empty() || self.workflow.len() > MAX_PLANNED_ROLES {
            return Err(HarnessError::InvalidPlan(format!(
                "a plan must contain between one and {MAX_PLANNED_ROLES} roles"
            )));
        }
        if self.required_concurrent_roles == 0 || self.required_concurrent_roles > MAX_PLANNED_ROLES
        {
            return Err(HarnessError::InvalidPlan(format!(
                "required_concurrent_roles must be between one and {MAX_PLANNED_ROLES}"
            )));
        }
        if self.required_concurrent_roles > self.workflow.len() {
            return Err(HarnessError::InvalidPlan(
                "required_concurrent_roles must not exceed the planned role count".to_owned(),
            ));
        }
        self.validate_workflow()?;
        if !self.contains_role(self.primary_producer_role) {
            return Err(HarnessError::InvalidPlan(
                "primary_producer_role must be one of the planned roles".to_owned(),
            ));
        }
        let roles = self.roles().collect::<Vec<_>>();
        if self.primary_producer_role != primary_producer_role_for_action(self.action, &roles)? {
            return Err(HarnessError::InvalidPlan(
                "primary_producer_role does not match the planned action".to_owned(),
            ));
        }
        if self.max_revisions > MAX_REVISIONS {
            return Err(HarnessError::InvalidPlan(format!(
                "max_revisions must not exceed {MAX_REVISIONS}"
            )));
        }
        if !self.action.is_write() && self.max_revisions != 0 {
            return Err(HarnessError::InvalidPlan(
                "non-write plans must not permit candidate revisions".to_owned(),
            ));
        }
        if self.required_policies.is_empty() || self.verification_requirements.is_empty() {
            return Err(HarnessError::InvalidPlan(
                "policies and verification checks must not be empty".to_owned(),
            ));
        }
        if self.action.is_write() != self.source_write_allowed {
            return Err(HarnessError::InvalidPlan(
                "source write permission must match the action".to_owned(),
            ));
        }
        if self.external_write_allowed {
            return Err(HarnessError::InvalidPlan(
                "external writes are not supported by the common harness".to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_workflow(&self) -> HarnessResult<()> {
        let mut seen_roles = BTreeSet::new();
        for (node_position, node) in self.workflow.iter().enumerate() {
            if !seen_roles.insert(node.role) {
                return Err(HarnessError::InvalidPlan(
                    "workflow roles must be unique".to_owned(),
                ));
            }
            let mut dependency_roles = BTreeSet::new();
            let mut previous_position = None;
            for dependency in &node.dependencies {
                if !dependency_roles.insert(*dependency) {
                    return Err(HarnessError::InvalidPlan(format!(
                        "workflow role {:?} dependencies must be unique",
                        node.role
                    )));
                }
                let dependency_position = self.role_position(*dependency).ok_or_else(|| {
                    HarnessError::InvalidPlan(format!(
                        "workflow role {:?} depends on an unplanned role",
                        node.role
                    ))
                })?;
                if dependency_position >= node_position {
                    return Err(HarnessError::InvalidPlan(format!(
                        "workflow role {:?} dependencies must occur earlier in workflow order",
                        node.role
                    )));
                }
                if previous_position.is_some_and(|previous| previous >= dependency_position) {
                    return Err(HarnessError::InvalidPlan(format!(
                        "workflow role {:?} dependencies must use workflow order",
                        node.role
                    )));
                }
                previous_position = Some(dependency_position);
            }
            if !self.action.is_review()
                && node.role != self.primary_producer_role
                && node.dependencies.as_slice() != [self.primary_producer_role]
            {
                return Err(HarnessError::InvalidPlan(format!(
                    "workflow role {:?} must depend only on the primary producer",
                    node.role
                )));
            }
            match node.subject_source {
                WorkflowSubjectSource::TaskContract
                    if !self.action.is_review()
                        && node.role == self.primary_producer_role
                        && node.dependencies.is_empty() => {}
                WorkflowSubjectSource::FrozenTargets
                    if self.action.is_review() && node.dependencies.is_empty() => {}
                WorkflowSubjectSource::PrimaryProducer
                    if !self.action.is_review()
                        && node.role != self.primary_producer_role
                        && node.dependencies.as_slice() == [self.primary_producer_role] => {}
                _ => {
                    return Err(HarnessError::InvalidPlan(format!(
                        "workflow role {:?} has an invalid subject source",
                        node.role
                    )));
                }
            }
        }
        Ok(())
    }

    fn validate_curation_source_bindings(&self) -> HarnessResult<()> {
        let sources_required = self.action == HarnessAction::VaultCuration
            && matches!(
                self.curation_kind,
                Some(CurationKind::Knowledge | CurationKind::Fact | CurationKind::Ontology)
            );
        if (sources_required && self.curation_sources.is_empty())
            || (self.action != HarnessAction::VaultCuration && !self.curation_sources.is_empty())
        {
            return Err(HarnessError::InvalidPlan(
                "curation source bindings do not match the planned action and kind".to_owned(),
            ));
        }
        if is_personal_idea_curation(self.action, &self.owner, self.curation_kind)
            && !self
                .targets
                .iter()
                .any(|target| target.workspace_relative_path == SOLO_MVP_IDEA_REGISTRY_PATH)
            && !self
                .curation_sources
                .iter()
                .any(|source| source.repository_relative_path == SOLO_MVP_IDEA_REGISTRY_PATH)
        {
            return Err(HarnessError::InvalidPlan(format!(
                "personal Idea curation must bind `{SOLO_MVP_IDEA_REGISTRY_PATH}` as its target or curation source"
            )));
        }
        Ok(())
    }

    fn validate_promotion_handoff(&self) -> HarnessResult<()> {
        let Some(handoff) = &self.promotion_handoff else {
            return Ok(());
        };
        handoff.validate()?;
        if self.action != HarnessAction::VaultCuration
            || self.owner != handoff.proposal.owner
            || self.curation_kind != Some(handoff.proposal.curation_kind)
        {
            return Err(HarnessError::InvalidPlan(
                "promotion handoff must match a Vault curation plan owner and kind".to_owned(),
            ));
        }
        if self
            .targets
            .iter()
            .any(|target| target.operation == TargetOperation::Delete)
        {
            return Err(HarnessError::InvalidPlan(
                "promotion-backed curation cannot delete a Vault target".to_owned(),
            ));
        }
        if matches!(
            handoff.proposal.origin,
            PromotionProposalOrigin::ReviewerLearning { .. }
        ) {
            if self.curation_kind != Some(CurationKind::Knowledge)
                || self
                    .targets
                    .iter()
                    .any(|target| target.operation != TargetOperation::Create)
            {
                return Err(HarnessError::InvalidPlan(
                    "Reviewer learning curation requires a new Knowledge target".to_owned(),
                ));
            }
            if self.owner == DataOwner::Profile
                && self.targets.iter().any(|target| {
                    target.workspace_relative_path == "vault/profile/rules"
                        || target
                            .workspace_relative_path
                            .starts_with("vault/profile/rules/")
                })
            {
                return Err(HarnessError::InvalidPlan(
                    "Reviewer learning cannot directly create a profile rule".to_owned(),
                ));
            }
        }
        Ok(())
    }

    fn validate_evidence_source_bindings(&self) -> HarnessResult<()> {
        if self.evidence_sources.is_empty() {
            return Ok(());
        }
        if self.context_grants.len() != 1 || !self.intent.is_career_artifact() {
            return Err(HarnessError::InvalidPlan(
                "evidence source bindings require exactly one career-writing evidence grant"
                    .to_owned(),
            ));
        }
        let roots = self
            .evidence_sources
            .iter()
            .map(|source| ContextRoot {
                repository_relative_path: source.repository_relative_path.clone(),
            })
            .collect::<Vec<_>>();
        ensure_unique_paths("evidence source", &roots)?;
        for root in roots {
            if !self.allowed_context_roots.contains(&root) {
                return Err(HarnessError::InvalidPlan(format!(
                    "evidence source `{}` is outside allowed context paths",
                    root.repository_relative_path
                )));
            }
        }
        Ok(())
    }

    fn validate_learning_source_bindings(&self) -> HarnessResult<()> {
        if self.owner == DataOwner::CommonWork {
            return if self.learning_sources.is_empty() {
                Ok(())
            } else {
                Err(HarnessError::InvalidPlan(
                    "common work maintenance cannot bind learning sources".to_owned(),
                ))
            };
        }
        if self.learning_sources.len() > MAX_LEARNING_SOURCES {
            return Err(HarnessError::InvalidPlan(format!(
                "learning source bindings must not exceed {MAX_LEARNING_SOURCES} items"
            )));
        }
        let knowledge_root = curation_root(&self.owner, CurationKind::Knowledge)?;
        let planned_units = self
            .verification_requirements
            .iter()
            .map(|requirement| requirement.unit)
            .collect::<BTreeSet<_>>();
        let mut previous_path: Option<&str> = None;
        let mut learning_ids = BTreeSet::new();
        for source in &self.learning_sources {
            validate_relative_path(
                Path::new(&source.repository_relative_path),
                "learning source path",
            )?;
            validate_plan_digest("learning source digest", &source.content_digest)?;
            source.metadata.validate()?;
            if !path_is_within(&source.repository_relative_path, &knowledge_root)
                || source.metadata.owner != self.owner
                || source.metadata.action != self.action
                || source.metadata.intent != self.intent
                || !planned_units.contains(&source.metadata.verification_unit)
            {
                return Err(HarnessError::InvalidPlan(format!(
                    "learning source `{}` does not match the plan owner, action, intent, unit, or Knowledge root",
                    source.repository_relative_path
                )));
            }
            if previous_path
                .is_some_and(|previous| previous >= source.repository_relative_path.as_str())
            {
                return Err(HarnessError::InvalidPlan(
                    "learning source bindings must use unique repository-path order".to_owned(),
                ));
            }
            previous_path = Some(&source.repository_relative_path);
            if !learning_ids.insert(source.metadata.learning_id.clone()) {
                return Err(HarnessError::InvalidPlan(format!(
                    "learning identifier `{}` is duplicated in the plan",
                    source.metadata.learning_id
                )));
            }
        }
        Ok(())
    }

    fn validate_target_bindings(&self) -> HarnessResult<()> {
        for target in &self.targets {
            let valid_state = matches!(
                (target.operation, &target.state),
                (TargetOperation::Create, TargetState::Absent)
                    | (
                        TargetOperation::Update
                            | TargetOperation::Delete
                            | TargetOperation::Inspect,
                        TargetState::Existing { .. }
                    )
            );
            if !valid_state {
                return Err(HarnessError::InvalidPlan(format!(
                    "target operation does not match starting state for `{}`",
                    target.workspace_relative_path
                )));
            }
            if target.operation != TargetOperation::Create
                && !target.parent_directories_to_create.is_empty()
            {
                return Err(HarnessError::InvalidPlan(format!(
                    "non-create target `{}` cannot create parent directories",
                    target.workspace_relative_path
                )));
            }
            ensure_unique_strings(
                "target parent directory",
                &target.parent_directories_to_create,
            )?;
            for directory in &target.parent_directories_to_create {
                validate_relative_path(Path::new(directory), "target parent directory")?;
                if !path_is_within(&target.workspace_relative_path, directory)
                    || directory == &target.workspace_relative_path
                {
                    return Err(HarnessError::InvalidPlan(format!(
                        "planned parent directory `{directory}` does not contain `{}`",
                        target.workspace_relative_path
                    )));
                }
            }
            let is_inspection = target.operation == TargetOperation::Inspect;
            if (self.action.is_write() && is_inspection)
                || (!self.action.is_write() && !is_inspection)
            {
                return Err(HarnessError::InvalidPlan(format!(
                    "target operation does not match action for `{}`",
                    target.workspace_relative_path
                )));
            }
        }
        Ok(())
    }

    fn validate_roles_and_context(&self) -> HarnessResult<()> {
        if self.action.is_write() && !self.contains_role(HarnessRole::Writer) {
            return Err(HarnessError::InvalidPlan(
                "write actions require one writer role".to_owned(),
            ));
        }
        if !self.action.is_write() && self.contains_role(HarnessRole::Writer) {
            return Err(HarnessError::InvalidPlan(
                "non-write actions must not contain a writer role".to_owned(),
            ));
        }
        validate_intent(
            &HarnessRequest {
                action: self.action,
                owner: self.owner.clone(),
                targets: self
                    .targets
                    .iter()
                    .map(|target| target.workspace_relative_path.clone())
                    .collect(),
                objective: "plan validation".to_owned(),
                curation_kind: self.curation_kind,
                explicit_user_confirmation_reported: false,
                curation_sources: Vec::new(),
                delete_targets: Vec::new(),
            },
            self.intent,
            &self.context_grants,
        )?;
        ensure_unique_paths("allowed context root", &self.allowed_context_roots)?;
        ensure_unique_paths("denied context root", &self.denied_context_roots)?;
        ensure_unique_paths("retrieval root", &self.retrieval_roots)?;
        ensure_disjoint_context_roots(&self.allowed_context_roots, &self.denied_context_roots)?;
        if self.action != HarnessAction::VaultRead && !self.retrieval_roots.is_empty() {
            return Err(HarnessError::InvalidPlan(
                "retrieval roots are only valid for Vault read".to_owned(),
            ));
        }
        for retrieval_root in &self.retrieval_roots {
            if !self.allowed_context_roots.iter().any(|allowed| {
                path_is_within(
                    &retrieval_root.repository_relative_path,
                    &allowed.repository_relative_path,
                )
            }) {
                return Err(HarnessError::InvalidPlan(format!(
                    "retrieval root `{}` is outside allowed context paths",
                    retrieval_root.repository_relative_path
                )));
            }
        }
        Ok(())
    }

    fn validate_verification_requirements(&self) -> HarnessResult<()> {
        let produced_subject = match self.action {
            HarnessAction::CodeReview | HarnessAction::DocumentReview => {
                EvaluationSubjectKind::FrozenTargets
            }
            _ => EvaluationSubjectKind::ProducedArtifact,
        };
        for requirement in &self.verification_requirements {
            match requirement.owner {
                VerificationOwner::Role { role } if !self.contains_role(role) => {
                    return Err(HarnessError::InvalidPlan(format!(
                        "verification requirement `{}` is owned by an unplanned role",
                        requirement.unit.as_str()
                    )));
                }
                VerificationOwner::Deterministic
                    if requirement.subject != EvaluationSubjectKind::TaskContract =>
                {
                    return Err(HarnessError::InvalidPlan(format!(
                        "Harness engine verification requirement `{}` must bind the task contract",
                        requirement.unit.as_str()
                    )));
                }
                VerificationOwner::Tool | VerificationOwner::Role { .. }
                    if requirement.subject != produced_subject =>
                {
                    return Err(HarnessError::InvalidPlan(format!(
                        "verification requirement `{}` binds the wrong evaluation subject",
                        requirement.unit.as_str()
                    )));
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn validate_role_policy_bindings(&self) -> HarnessResult<()> {
        if self.role_policy_bindings.len() != self.role_count() {
            return Err(HarnessError::InvalidPlan(
                "every planned role must have one policy binding set".to_owned(),
            ));
        }
        let expected_roles = self.roles().collect::<BTreeSet<_>>();
        let actual_roles = self
            .role_policy_bindings
            .iter()
            .map(|binding| binding.role)
            .collect::<BTreeSet<_>>();
        if actual_roles.len() != self.role_policy_bindings.len() || actual_roles != expected_roles {
            return Err(HarnessError::InvalidPlan(
                "role policy bindings must exactly cover planned roles".to_owned(),
            ));
        }
        let required = self
            .required_policies
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let orchestrator = self
            .orchestrator_policies
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if !orchestrator.is_subset(&required) {
            return Err(HarnessError::InvalidPlan(
                "orchestrator policies must be a subset of the plan policy union".to_owned(),
            ));
        }
        let mut actual = orchestrator.clone();
        for role_binding in &self.role_policy_bindings {
            ensure_unique_policy_ids(&role_binding.policies)?;
            for policy in &role_binding.policies {
                if !required.contains(policy) {
                    return Err(HarnessError::InvalidPlan(format!(
                        "role {:?} binds policy `{}` outside the plan union",
                        role_binding.role, policy.id
                    )));
                }
                if orchestrator.contains(policy) {
                    return Err(HarnessError::InvalidPlan(format!(
                        "orchestrator policy `{}` must not be repeated in role bundles",
                        policy.id
                    )));
                }
                actual.insert(policy.clone());
            }
        }
        if actual != required {
            return Err(HarnessError::InvalidPlan(
                "plan policy union must equal the orchestrator and role policy union".to_owned(),
            ));
        }
        Ok(())
    }

    #[must_use]
    pub fn user_summary(&self) -> String {
        let owner = match &self.owner {
            DataOwner::Profile => "공통 운영 규칙".to_owned(),
            DataOwner::CommonWork => "업무 공통 규칙".to_owned(),
            DataOwner::PersonalBusiness => "개인 사업".to_owned(),
            DataOwner::Personal => "개인 자료".to_owned(),
            DataOwner::PersonalProject { project } => format!("개인 프로젝트: {project}"),
            DataOwner::Company { company } => format!("회사: {company}"),
            DataOwner::CompanyProject { company, project } => {
                format!("회사: {company}, 프로젝트: {project}")
            }
        };
        let action = match self.action {
            HarnessAction::CodeWrite => "코드 작성",
            HarnessAction::CodeReview => "코드 리뷰",
            HarnessAction::DocumentWrite => "문서 작성",
            HarnessAction::DocumentReview => "문서 리뷰",
            HarnessAction::Investigation => "조사",
            HarnessAction::Design => "설계",
            HarnessAction::Ideation => "아이디에이션",
            HarnessAction::VaultRead => "Vault 조회",
            HarnessAction::VaultCuration => "Vault 정리",
        };
        let intent = match self.intent {
            HarnessIntent::General => "일반 작업".to_owned(),
            HarnessIntent::CareerArtifact { surface } => {
                format!("경력 산출물: {}", surface.as_str())
            }
            HarnessIntent::PolicyMaintenance => "정책 정비".to_owned(),
            HarnessIntent::OutputAdapter { surface } => {
                format!("출력 어댑터: {}", surface.as_str())
            }
            HarnessIntent::SoloMvpIdeation => "1인 MVP 아이디에이션".to_owned(),
        };
        let targets = if self.targets.is_empty() {
            "파일 대상 없음".to_owned()
        } else {
            self.targets
                .iter()
                .map(|target| target.workspace_relative_path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!(
            "작업: {action}\n자료 범위: {owner}\n요청 의도: {intent}\n대상: {targets}\n작성자와 리뷰어: 분리 요청\n외부 변경: 허용하지 않음\n실행 보장: 계획과 제출 형식만 확인하며 실제 역할 독립성은 확인하지 않음"
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResolvedHarnessRequest {
    pub request: HarnessRequest,
    pub contract: ResolvedTaskContract,
    pub decision_trace: DecisionTrace,
    pub request_provenance: RequestProvenanceBinding,
    pub intent: HarnessIntent,
    pub context_grants: Vec<ContextGrant>,
    pub career_composition_manifest: Option<CareerCompositionManifest>,
    pub evidence_source_paths: Vec<String>,
    pub resolved_plan_digest: String,
    pub plan: ResolvedHarnessPlan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessRouter {
    max_revisions: u32,
    execution_profile: HarnessExecutionProfile,
}

struct HarnessPlanInputs<'a> {
    request: &'a HarnessRequest,
    contract: &'a ResolvedTaskContract,
    decision_trace: &'a DecisionTrace,
    request_provenance: &'a RequestProvenanceBinding,
    intent: HarnessIntent,
    context_grants: &'a [ContextGrant],
    career_composition_manifest: Option<&'a CareerCompositionManifest>,
    evidence_source_paths: &'a [String],
    targets: Vec<TargetBinding>,
    workspace_policies: Vec<WorkspacePolicyBinding>,
    vault: &'a VaultRepository,
}

struct PlanContextBindings {
    allowed_context_roots: Vec<ContextRoot>,
    evidence_sources: Vec<SourceBinding>,
    denied_context_roots: Vec<ContextRoot>,
    required_policies: Vec<PolicyBinding>,
    orchestrator_policies: Vec<PolicyBinding>,
    role_policy_bindings: Vec<RolePolicyBinding>,
    retrieval_roots: Vec<ContextRoot>,
    curation_sources: Vec<SourceBinding>,
}

impl HarnessRouter {
    pub fn new(max_revisions: u32) -> HarnessResult<Self> {
        Self::with_execution_profile(max_revisions, HarnessExecutionProfile::Standard)
    }

    pub fn with_execution_profile(
        max_revisions: u32,
        execution_profile: HarnessExecutionProfile,
    ) -> HarnessResult<Self> {
        if max_revisions > MAX_REVISIONS {
            return Err(HarnessError::InvalidPlan(format!(
                "max_revisions must not exceed {MAX_REVISIONS}"
            )));
        }
        Ok(Self {
            max_revisions,
            execution_profile,
        })
    }

    #[allow(
        clippy::too_many_lines,
        reason = "plan orchestration and digest field ordering must remain auditable together"
    )]
    fn build_plan(&self, inputs: HarnessPlanInputs<'_>) -> HarnessResult<ResolvedHarnessPlan> {
        let HarnessPlanInputs {
            request,
            contract,
            decision_trace,
            request_provenance,
            intent,
            context_grants,
            career_composition_manifest,
            evidence_source_paths,
            targets,
            workspace_policies,
            vault,
        } = inputs;
        request.validate()?;
        validate_context_grants(request, context_grants)?;
        validate_intent(request, intent, context_grants)?;
        let career_output_surface = intent.career_surface();
        if let Some(manifest) = career_composition_manifest {
            manifest.validate(request, career_output_surface)?;
        }
        let career_manifest_digest = career_composition_manifest
            .map(serialized_digest)
            .transpose()?;
        let promotion_handoff = contract.promotion_handoff().cloned();
        let contract_digest = serialized_digest(contract)?;
        let decision_trace_digest = serialized_digest(decision_trace)?;
        if decision_trace_digest != request_provenance.decision_trace_digest {
            return Err(HarnessError::InvalidPlan(
                "resolved decision trace does not match request provenance".to_owned(),
            ));
        }
        let request_digest = serialized_digest(&(
            &contract_digest,
            &decision_trace_digest,
            request_provenance,
            &career_manifest_digest,
        ))?;
        validate_review_targets(request.action, &targets)?;
        let roles = roles_for(request, self.execution_profile);
        let primary_producer_role = primary_producer_role_for_action(request.action, &roles)?;
        let workflow = workflow_for(request.action, &roles, primary_producer_role);
        let verification_requirements = plan_verification_requirements(
            request,
            intent,
            &roles,
            career_composition_manifest.is_some() && context_grants.is_empty(),
        );
        let PlanContextBindings {
            allowed_context_roots,
            evidence_sources,
            denied_context_roots,
            required_policies,
            orchestrator_policies,
            role_policy_bindings,
            retrieval_roots,
            curation_sources,
        } = bind_plan_context(
            vault,
            request,
            intent,
            context_grants,
            evidence_source_paths,
            &roles,
        )?;
        let learning_sources = vault.learning_sources(
            &request.owner,
            request.action,
            intent,
            &verification_requirements,
        )?;
        let required_concurrent_roles = required_concurrent_roles_for(request.action, &roles);

        let plan = ResolvedHarnessPlan {
            source_versions: SourceVersionSet::default(),
            version: HARNESS_SCHEMA_VERSION,
            route_id: plan_route_id(request, intent, context_grants),
            request_digest,
            contract_digest,
            decision_trace_digest,
            request_provenance: request_provenance.clone(),
            execution_profile: self.execution_profile,
            action: request.action,
            owner: request.owner.clone(),
            intent,
            context_grants: context_grants.to_vec(),
            career_manifest_digest,
            evidence_sources,
            learning_sources,
            curation_kind: request.curation_kind,
            curation_sources,
            promotion_handoff,
            targets,
            workflow,
            primary_producer_role,
            allowed_context_roots,
            denied_context_roots,
            retrieval_roots,
            required_policies,
            orchestrator_policies,
            role_policy_bindings,
            workspace_policies,
            verification_requirements,
            required_concurrent_roles,
            max_revisions: if request.action.is_write() {
                self.max_revisions
            } else {
                0
            },
            minimum_assurance: ExecutionAssurance::Advisory,
            source_write_allowed: request.action.is_write(),
            external_write_allowed: false,
        };
        plan.validate()?;
        Ok(plan)
    }
}

fn validate_plan_digest(field: &str, value: &str) -> HarnessResult<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(HarnessError::InvalidPlan(format!(
            "{field} must be a 64-character hexadecimal digest"
        )));
    }
    Ok(())
}

fn validate_learning_identifier(identifier: &str) -> HarnessResult<()> {
    let Some(suffix) = identifier.strip_prefix("learning-") else {
        return Err(HarnessError::InvalidPlan(
            "learning identifier must use the `learning-` prefix".to_owned(),
        ));
    };
    if suffix.len() != 16
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(HarnessError::InvalidPlan(
            "learning identifier must end with 16 lowercase hexadecimal characters".to_owned(),
        ));
    }
    Ok(())
}

fn plan_route_id(
    request: &HarnessRequest,
    intent: HarnessIntent,
    context_grants: &[ContextGrant],
) -> String {
    format!(
        "{}-{}-{}{}{}",
        request.owner.route_segment(),
        request.action.as_str(),
        intent.route_segment(),
        intent
            .output_surface()
            .map_or_else(String::new, |surface| format!("-{}", surface.as_str())),
        context_grants.first().map_or_else(String::new, |grant| {
            format!("-with-{}", grant.owner.route_segment())
        })
    )
}

fn plan_verification_requirements(
    request: &HarnessRequest,
    intent: HarnessIntent,
    roles: &[HarnessRole],
    holistic_career_review: bool,
) -> Vec<VerificationRequirement> {
    let mut requirements = verification_requirements_for(request, intent, roles);
    if holistic_career_review && request.action == HarnessAction::DocumentReview {
        requirements
            .retain(|requirement| requirement.unit != VerificationUnit::CareerEvidenceLineage);
        requirements.push(VerificationRequirement {
            unit: VerificationUnit::CareerHolisticCoherence,
            owner: VerificationOwner::Role {
                role: HarnessRole::Reviewer,
            },
            subject: EvaluationSubjectKind::FrozenTargets,
        });
    }
    requirements
}

fn bind_plan_context(
    vault: &VaultRepository,
    request: &HarnessRequest,
    intent: HarnessIntent,
    context_grants: &[ContextGrant],
    evidence_source_paths: &[String],
    roles: &[HarnessRole],
) -> HarnessResult<PlanContextBindings> {
    let mut allowed_context_roots = vault.allowed_context_roots(request)?;
    let evidence_sources = match context_grants {
        [grant] if !evidence_source_paths.is_empty() => {
            let sources = vault.bind_evidence_sources(grant, evidence_source_paths)?;
            allowed_context_roots.extend(sources.iter().map(|source| ContextRoot {
                repository_relative_path: source.repository_relative_path.clone(),
            }));
            sources
        }
        _ if evidence_source_paths.is_empty() => {
            for grant in context_grants {
                allowed_context_roots.extend(vault.context_grant_roots(grant)?);
            }
            Vec::new()
        }
        _ => {
            return Err(HarnessError::InvalidRequest(
                "evidence source paths require exactly one evidence grant".to_owned(),
            ));
        }
    };
    if intent.is_career_artifact() {
        allowed_context_roots
            .retain(|root| root.repository_relative_path != "vault/personal/projects/ideas");
    }
    allowed_context_roots.sort_by(|left, right| {
        left.repository_relative_path
            .cmp(&right.repository_relative_path)
    });
    allowed_context_roots.dedup();
    let denied_context_roots = VaultRepository::denied_context_roots(request, context_grants);
    let (required_policies, orchestrator_policies, role_policy_bindings) =
        vault.required_policies(request, intent, context_grants, roles)?;
    let retrieval_roots = vault.retrieval_roots(request)?;
    let curation_sources =
        vault.bind_curation_sources(request, &allowed_context_roots, &denied_context_roots)?;
    Ok(PlanContextBindings {
        allowed_context_roots,
        evidence_sources,
        denied_context_roots,
        required_policies,
        orchestrator_policies,
        role_policy_bindings,
        retrieval_roots,
        curation_sources,
    })
}

fn validate_review_targets(action: HarnessAction, targets: &[TargetBinding]) -> HarnessResult<()> {
    if action.is_review()
        && targets
            .iter()
            .any(|target| matches!(target.state, TargetState::Absent))
    {
        return Err(HarnessError::InvalidRequest(
            "review actions require an existing target file".to_owned(),
        ));
    }
    Ok(())
}

impl Default for HarnessRouter {
    fn default() -> Self {
        Self {
            max_revisions: 2,
            execution_profile: HarnessExecutionProfile::Standard,
        }
    }
}

fn validate_intent(
    request: &HarnessRequest,
    intent: HarnessIntent,
    context_grants: &[ContextGrant],
) -> HarnessResult<()> {
    if request.owner == DataOwner::CommonWork && intent != HarnessIntent::PolicyMaintenance {
        return Err(HarnessError::InvalidRequest(
            "common work maintenance requires policy-maintenance intent".to_owned(),
        ));
    }
    let document_or_analysis = matches!(
        request.action,
        HarnessAction::DocumentWrite
            | HarnessAction::DocumentReview
            | HarnessAction::Investigation
            | HarnessAction::Design
    );
    match intent {
        HarnessIntent::General => {
            if !context_grants.is_empty() {
                return Err(HarnessError::InvalidRequest(
                    "general intent cannot bind career-writing evidence grants".to_owned(),
                ));
            }
        }
        HarnessIntent::CareerArtifact { .. } => {
            if request.owner != DataOwner::Personal || !document_or_analysis {
                return Err(HarnessError::InvalidRequest(
                    "career-artifact intent requires a personal document, investigation, or design action"
                        .to_owned(),
                ));
            }
        }
        HarnessIntent::PolicyMaintenance => {
            if !context_grants.is_empty()
                || !matches!(
                    request.owner,
                    DataOwner::Profile | DataOwner::Personal | DataOwner::CommonWork
                )
                || !matches!(
                    request.action,
                    HarnessAction::CodeWrite
                        | HarnessAction::CodeReview
                        | HarnessAction::DocumentWrite
                        | HarnessAction::DocumentReview
                        | HarnessAction::Investigation
                        | HarnessAction::Design
                )
            {
                return Err(HarnessError::InvalidRequest(
                    "policy-maintenance intent requires profile or personal maintenance work without evidence grants"
                        .to_owned(),
                ));
            }
        }
        HarnessIntent::OutputAdapter { .. } => {
            if request.owner != DataOwner::Profile
                || !context_grants.is_empty()
                || !matches!(
                    request.action,
                    HarnessAction::CodeWrite
                        | HarnessAction::CodeReview
                        | HarnessAction::DocumentWrite
                        | HarnessAction::DocumentReview
                        | HarnessAction::Investigation
                        | HarnessAction::Design
                )
            {
                return Err(HarnessError::InvalidRequest(
                    "output-adapter intent requires profile-owned code, document, investigation, or design work without evidence grants"
                        .to_owned(),
                ));
            }
        }
        HarnessIntent::SoloMvpIdeation => {
            if request.action != HarnessAction::Ideation
                || request.owner != DataOwner::Personal
                || !context_grants.is_empty()
                || !request
                    .targets
                    .iter()
                    .any(|target| target == SOLO_MVP_IDEA_REGISTRY_PATH)
            {
                return Err(HarnessError::InvalidRequest(format!(
                    "solo-mvp-ideation intent requires personal ideation without context grants and with `{SOLO_MVP_IDEA_REGISTRY_PATH}` in the exact target set"
                )));
            }
        }
    }
    Ok(())
}

fn uses_solo_mvp_ideation_contract(request: &HarnessRequest, intent: HarnessIntent) -> bool {
    intent == HarnessIntent::SoloMvpIdeation
        || is_personal_idea_curation(request.action, &request.owner, request.curation_kind)
}

fn is_personal_idea_curation(
    action: HarnessAction,
    owner: &DataOwner,
    curation_kind: Option<CurationKind>,
) -> bool {
    action == HarnessAction::VaultCuration
        && owner == &DataOwner::Personal
        && curation_kind == Some(CurationKind::Idea)
}

fn validate_context_grants(
    request: &HarnessRequest,
    context_grants: &[ContextGrant],
) -> HarnessResult<()> {
    if context_grants.is_empty() {
        return Ok(());
    }
    if context_grants.len() != 1 {
        return Err(HarnessError::InvalidRequest(
            "a career-writing plan supports exactly one evidence grant".to_owned(),
        ));
    }
    if request.owner != DataOwner::Personal
        || !matches!(
            request.action,
            HarnessAction::DocumentWrite
                | HarnessAction::DocumentReview
                | HarnessAction::Investigation
                | HarnessAction::Design
        )
    {
        return Err(HarnessError::InvalidRequest(
            "cross-scope evidence grants require a personal document, investigation, or design action"
                .to_owned(),
        ));
    }
    let grant = &context_grants[0];
    grant.owner.validate()?;
    if !matches!(
        grant.owner,
        DataOwner::PersonalProject { .. }
            | DataOwner::Company { .. }
            | DataOwner::CompanyProject { .. }
    ) || grant.purpose != ContextGrantPurpose::CareerWritingEvidence
        || grant.access != ContextAccess::ReadOnly
    {
        return Err(HarnessError::InvalidRequest(
            "evidence grants are limited to one read-only personal project, company, or company project for career writing"
                .to_owned(),
        ));
    }
    Ok(())
}

fn validate_evidence_owner(owner: &DataOwner) -> HarnessResult<()> {
    owner.validate()?;
    if !matches!(
        owner,
        DataOwner::PersonalProject { .. }
            | DataOwner::Company { .. }
            | DataOwner::CompanyProject { .. }
    ) {
        return Err(HarnessError::InvalidRequest(
            "career evidence owners must be a personal project, company, or company project"
                .to_owned(),
        ));
    }
    Ok(())
}

fn validate_evidence_owners(field: &str, owners: &[DataOwner]) -> HarnessResult<()> {
    let mut unique = BTreeSet::new();
    for owner in owners {
        validate_evidence_owner(owner)?;
        if !unique.insert(owner.clone()) {
            return Err(HarnessError::InvalidRequest(format!(
                "{field} values must be unique"
            )));
        }
    }
    for owner in owners {
        if let DataOwner::CompanyProject { company, .. } = owner
            && owners.contains(&DataOwner::Company {
                company: company.clone(),
            })
        {
            return Err(HarnessError::InvalidRequest(format!(
                "{field} cannot combine a company with one of its project partitions"
            )));
        }
    }
    Ok(())
}

fn validate_evidence_source_paths(paths: &[String]) -> HarnessResult<()> {
    if paths.is_empty() || paths.len() > MAX_TARGETS {
        return Err(HarnessError::InvalidRequest(format!(
            "career evidence sources must contain between one and {MAX_TARGETS} paths"
        )));
    }
    ensure_unique_strings("career evidence source", paths)
        .map_err(|error| HarnessError::InvalidRequest(error.to_string()))?;
    for path in paths {
        validate_single_line("career evidence source", path, MAX_TARGET_LENGTH)?;
        validate_relative_path(Path::new(path), "career evidence source")?;
        if Path::new(path).extension().and_then(OsStr::to_str) != Some("md") {
            return Err(HarnessError::InvalidRequest(
                "career evidence sources must be Markdown files".to_owned(),
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum PolicyCapability {
    AgentHarness,
    AgentOperatingPreferences,
    ContextScopeRouting,
    MandatoryPreflight,
    CommonReviewQuality,
    CommonCodeQuality,
    CodeReview,
    RustCodeStyle,
    Dependency,
    Issue,
    CommonDocumentQuality,
    ContextVaultOperatingModel,
    ContextDocumentStability,
    ProfileIndex,
    PersonalIndex,
    AiCollaborationValues,
    SoloMvpIdeaDiscovery,
    PersonalBusinessIndex,
    WorkCompanyRegistry,
    WorkAgentGuide,
    WorkAgentOperatingPreferences,
    CompanyIndex(String),
    CompanyRouting(String),
    CompanyDomainRule {
        company: String,
        id: String,
        path: String,
    },
    GrantedCompanyIndex(String),
    GrantedPersonalProjectSource(String),
    CareerResumeSourceMap,
    CareerClaimTokenOutputSystem,
    CareerOutputAssembly,
    CareerContributionAudit,
    CareerPerspectiveAndPublicSafety,
    CareerPortfolioCasebook,
    CareerResumeCaseView,
    CareerResumePdfBaseline,
    CareerCaseDocumentContract,
}

impl PolicyCapability {
    fn dependencies(&self) -> Vec<Self> {
        match self {
            Self::MandatoryPreflight => vec![Self::AgentHarness, Self::ContextScopeRouting],
            Self::CommonReviewQuality
            | Self::CommonCodeQuality
            | Self::CommonDocumentQuality
            | Self::AgentHarness
            | Self::AgentOperatingPreferences
            | Self::ContextScopeRouting
            | Self::ContextVaultOperatingModel
            | Self::ProfileIndex
            | Self::PersonalIndex
            | Self::PersonalBusinessIndex
            | Self::WorkCompanyRegistry
            | Self::GrantedPersonalProjectSource(_)
            | Self::CareerResumeSourceMap => Vec::new(),
            Self::AiCollaborationValues | Self::SoloMvpIdeaDiscovery => {
                vec![Self::PersonalIndex]
            }
            Self::CodeReview | Self::RustCodeStyle | Self::Dependency | Self::Issue => {
                vec![Self::CommonCodeQuality]
            }
            Self::ContextDocumentStability => vec![Self::ContextVaultOperatingModel],
            Self::WorkAgentGuide | Self::WorkAgentOperatingPreferences => {
                vec![Self::WorkCompanyRegistry]
            }
            Self::CompanyIndex(_) | Self::GrantedCompanyIndex(_) => vec![
                Self::WorkCompanyRegistry,
                Self::WorkAgentGuide,
                Self::WorkAgentOperatingPreferences,
            ],
            Self::CompanyRouting(company) => vec![Self::CompanyIndex(company.clone())],
            Self::CompanyDomainRule { company, .. } => {
                vec![
                    Self::CompanyRouting(company.clone()),
                    Self::CommonCodeQuality,
                ]
            }
            Self::CareerClaimTokenOutputSystem
            | Self::CareerContributionAudit
            | Self::CareerPerspectiveAndPublicSafety
            | Self::CareerPortfolioCasebook
            | Self::CareerResumeCaseView
            | Self::CareerResumePdfBaseline
            | Self::CareerCaseDocumentContract => vec![Self::CareerResumeSourceMap],
            Self::CareerOutputAssembly => vec![
                Self::CareerResumeSourceMap,
                Self::CareerClaimTokenOutputSystem,
            ],
        }
    }

    fn binding_location(&self) -> (&str, String) {
        match self {
            Self::AgentHarness => (
                "agent-harness",
                "vault/profile/rules/agent-harness.md".to_owned(),
            ),
            Self::AgentOperatingPreferences => (
                "agent-operating-preferences",
                "vault/profile/preferences/agent-operating-preferences.md".to_owned(),
            ),
            Self::ContextScopeRouting => (
                "context-scope-routing",
                "vault/profile/preferences/context-scope-routing.md".to_owned(),
            ),
            Self::MandatoryPreflight => (
                "mandatory-preflight",
                "vault/profile/rules/mandatory-preflight.md".to_owned(),
            ),
            Self::CommonReviewQuality => (
                "common-review-quality",
                "vault/profile/rules/common-review-quality.md".to_owned(),
            ),
            Self::CommonCodeQuality => (
                "common-code-quality",
                "vault/profile/rules/common-code-quality.md".to_owned(),
            ),
            Self::CodeReview => (
                "code-review",
                "vault/work/common/rules/code-review.md".to_owned(),
            ),
            Self::RustCodeStyle => (
                "rust-code-style",
                "vault/work/common/rules/rust-code-style.md".to_owned(),
            ),
            Self::Dependency => (
                "dependency",
                "vault/work/common/rules/dependency.md".to_owned(),
            ),
            Self::Issue => ("issue", "vault/work/common/rules/issue.md".to_owned()),
            Self::CommonDocumentQuality => (
                "common-document-quality",
                "vault/profile/rules/common-document-quality.md".to_owned(),
            ),
            Self::ContextVaultOperatingModel => (
                "context-vault-operating-model",
                "vault/profile/preferences/context-vault-operating-model.md".to_owned(),
            ),
            Self::ContextDocumentStability => (
                "context-document-stability",
                "vault/profile/rules/context-doc-stability-review.md".to_owned(),
            ),
            Self::ProfileIndex => ("profile-index", "vault/profile/index.md".to_owned()),
            Self::PersonalIndex => ("personal-index", "vault/personal/index.md".to_owned()),
            Self::AiCollaborationValues => (
                "ai-collaboration-values",
                AI_COLLABORATION_VALUES_PATH.to_owned(),
            ),
            Self::SoloMvpIdeaDiscovery => (
                "solo-mvp-idea-discovery",
                "vault/personal/decisions/solo-mvp-idea-discovery.md".to_owned(),
            ),
            Self::PersonalBusinessIndex => (
                "personal-business-index",
                "vault/personal/business/index.md".to_owned(),
            ),
            Self::WorkCompanyRegistry => (
                "work-company-registry",
                "vault/work/common/router/company-registry.md".to_owned(),
            ),
            Self::WorkAgentGuide => (
                "work-agent-guide",
                "vault/work/common/router/agent-guide.md".to_owned(),
            ),
            Self::WorkAgentOperatingPreferences => (
                "work-agent-operating-preferences",
                "vault/work/common/preferences/work-agent-operating-preferences.md".to_owned(),
            ),
            Self::CompanyIndex(company) => {
                ("company-index", format!("vault/work/{company}/index.md"))
            }
            Self::CompanyRouting(company) => ("company-routing", company_routing_path(company)),
            Self::CompanyDomainRule { id, path, .. } => (id, path.clone()),
            Self::GrantedCompanyIndex(company) => (
                "granted-company-index",
                format!("vault/work/{company}/index.md"),
            ),
            Self::GrantedPersonalProjectSource(path) => {
                ("granted-personal-project-source", path.clone())
            }
            Self::CareerResumeSourceMap
            | Self::CareerClaimTokenOutputSystem
            | Self::CareerOutputAssembly
            | Self::CareerContributionAudit
            | Self::CareerPerspectiveAndPublicSafety
            | Self::CareerPortfolioCasebook
            | Self::CareerResumeCaseView
            | Self::CareerResumePdfBaseline
            | Self::CareerCaseDocumentContract => self.career_binding_location(),
        }
    }

    fn career_binding_location(&self) -> (&'static str, String) {
        let (id, path) = match self {
            Self::CareerResumeSourceMap => (
                "career-resume-source-map",
                "vault/personal/writing/resume-source-map.md",
            ),
            Self::CareerClaimTokenOutputSystem => (
                "career-claim-token-output-system",
                "vault/personal/writing/claim-token-output-system.md",
            ),
            Self::CareerOutputAssembly => (
                "career-output-assembly",
                "vault/personal/writing/career-output-assembly.md",
            ),
            Self::CareerContributionAudit => (
                "career-contribution-audit",
                "vault/personal/writing/contribution-audit.md",
            ),
            Self::CareerPerspectiveAndPublicSafety => (
                "career-perspective-and-public-safety",
                "vault/personal/writing/career-technical-portfolio-strategy.md",
            ),
            Self::CareerPortfolioCasebook => (
                "career-portfolio-casebook",
                "vault/personal/writing/portfolio-casebook.md",
            ),
            Self::CareerResumeCaseView => (
                "career-resume-case-view",
                "vault/personal/writing/resume-case-view.md",
            ),
            Self::CareerResumePdfBaseline => (
                "career-resume-pdf-baseline",
                "vault/personal/decisions/resume-pdf-final-plan.md",
            ),
            Self::CareerCaseDocumentContract => (
                "career-case-document-contract",
                "vault/personal/writing/case-docs/index.md",
            ),
            _ => unreachable!("only career policy capabilities use career binding locations"),
        };
        (id, path.to_owned())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PolicyRoleMask(u8);

impl PolicyRoleMask {
    const WRITER: u8 = 1 << 0;
    const VERIFIER: u8 = 1 << 1;
    const REVIEWER: u8 = 1 << 2;
    const SPECIALIST: u8 = 1 << 3;

    const ALL: Self = Self(Self::WRITER | Self::VERIFIER | Self::REVIEWER | Self::SPECIALIST);
    const WRITE_REVIEW: Self = Self(Self::WRITER | Self::VERIFIER | Self::REVIEWER);
    const REVIEW: Self = Self(Self::VERIFIER | Self::REVIEWER);
    const AUTHOR_AND_REVIEWER: Self = Self(Self::WRITER | Self::SPECIALIST | Self::REVIEWER);

    const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    const fn intersect(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    const fn is_empty(self) -> bool {
        self.0 == 0
    }

    fn for_roles(roles: &[HarnessRole]) -> Self {
        roles.iter().fold(Self::default(), |mask, role| {
            mask.union(Self(Self::bit(*role)))
        })
    }

    const fn bit(role: HarnessRole) -> u8 {
        match role {
            HarnessRole::Writer => Self::WRITER,
            HarnessRole::Verifier => Self::VERIFIER,
            HarnessRole::Reviewer => Self::REVIEWER,
            HarnessRole::Specialist => Self::SPECIALIST,
        }
    }

    const fn contains(self, role: HarnessRole) -> bool {
        self.0 & Self::bit(role) != 0
    }
}

fn add_policy_requirement(
    requirements: &mut BTreeMap<PolicyCapability, PolicyRoleMask>,
    capability: PolicyCapability,
    roles: PolicyRoleMask,
) {
    requirements
        .entry(capability)
        .and_modify(|current| *current = current.union(roles))
        .or_insert(roles);
}

const fn stability_policy_roles(action: HarnessAction) -> PolicyRoleMask {
    match action {
        HarnessAction::DocumentWrite | HarnessAction::DocumentReview => {
            PolicyRoleMask::WRITE_REVIEW
        }
        HarnessAction::Investigation | HarnessAction::Design => PolicyRoleMask::AUTHOR_AND_REVIEWER,
        HarnessAction::CodeWrite | HarnessAction::CodeReview => PolicyRoleMask::REVIEW,
        HarnessAction::Ideation | HarnessAction::VaultRead | HarnessAction::VaultCuration => {
            PolicyRoleMask::REVIEW
        }
    }
}

fn close_policy_requirements(
    requirements: &mut BTreeMap<PolicyCapability, PolicyRoleMask>,
) -> HarnessResult<()> {
    fn visit(
        capability: &PolicyCapability,
        mask: PolicyRoleMask,
        requirements: &mut BTreeMap<PolicyCapability, PolicyRoleMask>,
        visiting: &mut BTreeSet<PolicyCapability>,
    ) -> HarnessResult<()> {
        if !visiting.insert(capability.clone()) {
            return Err(HarnessError::InvalidPlan(
                "policy capability dependency cycle detected".to_owned(),
            ));
        }
        for dependency in capability.dependencies() {
            add_policy_requirement(requirements, dependency.clone(), mask);
            visit(&dependency, mask, requirements, visiting)?;
        }
        visiting.remove(capability);
        Ok(())
    }

    let roots = requirements.clone();
    for (capability, mask) in roots {
        visit(&capability, mask, requirements, &mut BTreeSet::new())?;
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MarkdownSection {
    content: String,
    start_line: usize,
    end_line: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MarkdownFence {
    marker: char,
    length: usize,
}

fn markdown_sections(
    content: &str,
    maximum_sections: usize,
) -> HarnessResult<Vec<MarkdownSection>> {
    if content.is_empty() {
        return Ok(Vec::new());
    }
    if maximum_sections == 0 {
        return Err(HarnessError::InvalidRepository(format!(
            "retrieval scan contains more than {MAX_RETRIEVAL_SECTIONS} Markdown sections"
        )));
    }
    let lines = content.split_inclusive('\n').collect::<Vec<_>>();
    let frontmatter_end = if ParsedFrontmatter::from_markdown(content)
        .map_err(|error| {
            HarnessError::InvalidRepository(format!(
                "retrieval Markdown frontmatter is invalid: {error}"
            ))
        })?
        .is_some()
    {
        let (_, body) = split_markdown_frontmatter(content);
        let prefix_bytes = content.len().checked_sub(body.len()).ok_or_else(|| {
            HarnessError::InvalidRepository(
                "retrieval Markdown frontmatter boundary overflow".to_owned(),
            )
        })?;
        Some(
            content[..prefix_bytes]
                .split_inclusive('\n')
                .count()
                .saturating_sub(1),
        )
    } else {
        None
    };
    let mut starts = vec![0_usize];
    let mut fence: Option<MarkdownFence> = None;
    let mut saw_heading = false;
    for (index, line) in lines.iter().enumerate() {
        if frontmatter_end.is_some_and(|end| index <= end) {
            continue;
        }
        let Some(trimmed) = markdown_block_line(line.trim_end_matches(['\r', '\n'])) else {
            continue;
        };
        if let Some(open) = fence {
            if is_markdown_closing_fence(trimmed, open) {
                fence = None;
            }
            continue;
        }
        if let Some(open) = markdown_opening_fence(trimmed) {
            fence = Some(open);
            continue;
        }
        if is_markdown_atx_heading(trimmed) {
            if saw_heading {
                if starts.len() == maximum_sections {
                    return Err(HarnessError::InvalidRepository(format!(
                        "retrieval scan contains more than {MAX_RETRIEVAL_SECTIONS} Markdown sections"
                    )));
                }
                starts.push(index);
            } else {
                saw_heading = true;
            }
        }
    }
    Ok(starts
        .iter()
        .enumerate()
        .map(|(index, start)| {
            let end = starts.get(index + 1).copied().unwrap_or(lines.len());
            MarkdownSection {
                content: lines[*start..end].concat(),
                start_line: start + 1,
                end_line: end,
            }
        })
        .collect())
}

fn markdown_block_line(line: &str) -> Option<&str> {
    let leading_spaces = line.bytes().take_while(|byte| *byte == b' ').count();
    if leading_spaces > 3 || line.as_bytes().get(leading_spaces) == Some(&b'\t') {
        return None;
    }
    Some(&line[leading_spaces..])
}

fn markdown_opening_fence(line: &str) -> Option<MarkdownFence> {
    let marker = line.chars().next()?;
    let length = line.chars().take_while(|value| *value == marker).count();
    if !matches!(marker, '`' | '~') || length < 3 {
        return None;
    }
    if marker == '`' && line[length..].contains('`') {
        return None;
    }
    Some(MarkdownFence { marker, length })
}

fn is_markdown_closing_fence(line: &str, open: MarkdownFence) -> bool {
    let length = line
        .chars()
        .take_while(|value| *value == open.marker)
        .count();
    length >= open.length && line[length..].trim_matches([' ', '\t']).is_empty()
}

fn is_markdown_atx_heading(line: &str) -> bool {
    let markers = line.chars().take_while(|value| *value == '#').count();
    (1..=6).contains(&markers) && line.chars().nth(markers).is_some_and(char::is_whitespace)
}

fn parse_learning_document(
    content: &str,
    relative_path: &Path,
) -> HarnessResult<Option<LearningDocumentFrontmatter>> {
    let (raw_frontmatter, body) = split_markdown_frontmatter(content);
    let Some(raw_frontmatter) = raw_frontmatter else {
        if content.starts_with("---\n")
            && content
                .lines()
                .skip(1)
                .any(markdown_line_declares_learning_key)
        {
            return Err(HarnessError::InvalidRepository(format!(
                "learning document `{}` has unterminated frontmatter",
                relative_path.display()
            )));
        }
        return Ok(None);
    };
    let parsed = ParsedFrontmatter::parse(Some(raw_frontmatter))
        .map_err(|error| invalid_learning_document(relative_path, error))?
        .ok_or_else(|| {
            HarnessError::InvalidRepository(format!(
                "learning document `{}` has no frontmatter",
                relative_path.display()
            ))
        })?;
    if !parsed.keys().any(|key| key == "learning") {
        return Ok(None);
    }
    let frontmatter = parsed
        .deserialize::<LearningDocumentFrontmatter>()
        .map_err(|error| invalid_learning_document(relative_path, error))?;
    frontmatter
        .learning
        .validate()
        .map_err(|error| invalid_learning_document(relative_path, error))?;
    validate_single_line(
        "learning document title",
        &frontmatter.title,
        MAX_OBJECTIVE_LENGTH,
    )
    .map_err(|error| invalid_learning_document(relative_path, error))?;
    if frontmatter.scope != learning_owner_scope(&frontmatter.learning.owner) {
        return Err(HarnessError::InvalidRepository(format!(
            "learning document `{}` scope does not match its owner",
            relative_path.display()
        )));
    }
    if frontmatter.export {
        return Err(HarnessError::InvalidRepository(format!(
            "learning document `{}` must use `export: false`",
            relative_path.display()
        )));
    }
    let body_prefix = format!("# {}\n\n", frontmatter.title);
    let guidance = body.strip_prefix(&body_prefix).ok_or_else(|| {
        HarnessError::InvalidRepository(format!(
            "learning document `{}` body does not match its canonical title",
            relative_path.display()
        ))
    })?;
    validate_multiline(
        "learning document guidance",
        guidance,
        MAX_SUBMISSION_TEXT_BYTES,
    )
    .map_err(|error| invalid_learning_document(relative_path, error))?;
    Ok(Some(frontmatter))
}

fn markdown_line_declares_learning_key(line: &str) -> bool {
    let Some((key, _value)) = line.trim().split_once(':') else {
        return false;
    };
    matches!(key.trim(), "learning" | "'learning'" | "\"learning\"")
}

fn invalid_learning_document(relative_path: &Path, error: impl Display) -> HarnessError {
    HarnessError::InvalidRepository(format!(
        "learning document `{}` is invalid: {error}",
        relative_path.display()
    ))
}

const fn learning_owner_scope(owner: &DataOwner) -> &'static str {
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

#[derive(Clone)]
struct VaultRepository {
    root: PathBuf,
    source: std::sync::Arc<dyn ContextSource>,
    store_identity: Option<SourceStoreIdentity>,
}

impl fmt::Debug for VaultRepository {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VaultRepository")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl VaultRepository {
    fn open(root: impl AsRef<Path>) -> HarnessResult<Self> {
        let source = std::sync::Arc::new(FilesystemContextSource::open(root.as_ref())?);
        Self::with_source(root, source)
    }

    fn bind_policy(&self, id: &str, relative_path: &str) -> HarnessResult<PolicyBinding> {
        validate_relative_path(Path::new(relative_path), "policy path")?;
        let digest = self.digest_file_within_root(relative_path, MAX_POLICY_BYTES)?;
        Ok(PolicyBinding {
            id: id.to_owned(),
            repository_relative_path: relative_path.to_owned(),
            content_digest: digest,
        })
    }

    fn open_scoped_file(&self, relative_path: &Path, max_bytes: u64) -> HarnessResult<File> {
        self.open_source_file(relative_path, max_bytes)
    }

    fn digest_file_within_root(
        &self,
        relative_path: &str,
        max_bytes: u64,
    ) -> HarnessResult<String> {
        let path = self.root.join(relative_path);
        let file = self.open_scoped_file(Path::new(relative_path), max_bytes)?;
        digest_file(file, max_bytes, &path)
    }

    fn learning_sources(
        &self,
        owner: &DataOwner,
        action: HarnessAction,
        intent: HarnessIntent,
        requirements: &[VerificationRequirement],
    ) -> HarnessResult<Vec<LearningSourceBinding>> {
        if owner == &DataOwner::CommonWork {
            return Ok(Vec::new());
        }
        let knowledge_root = curation_root(owner, CurationKind::Knowledge)?;
        if !self.source_metadata(Path::new(&knowledge_root))?.exists() {
            return Ok(Vec::new());
        }
        let mut paths = Vec::new();
        self.collect_markdown_paths(Path::new(&knowledge_root), &mut paths, 0)?;
        paths.sort();
        paths.dedup();
        if paths.len() > MAX_LEARNING_SCAN_FILES {
            return Err(HarnessError::InvalidRepository(format!(
                "learning source scan contains more than {MAX_LEARNING_SCAN_FILES} Markdown files"
            )));
        }
        let planned_units = requirements
            .iter()
            .map(|requirement| requirement.unit)
            .collect::<BTreeSet<_>>();
        let mut scanned_bytes = 0_usize;
        let mut seen_learning_ids = BTreeMap::new();
        let mut sources = Vec::new();
        for relative_path in paths {
            let path = self.root.join(&relative_path);
            let file = self.open_scoped_file(&relative_path, MAX_RETRIEVAL_FILE_BYTES)?;
            let (content, content_digest) = read_text_file(file, MAX_RETRIEVAL_FILE_BYTES, &path)?;
            scanned_bytes = scanned_bytes.checked_add(content.len()).ok_or_else(|| {
                HarnessError::InvalidRepository("learning source scan size overflow".to_owned())
            })?;
            if scanned_bytes > MAX_LEARNING_SCAN_BYTES {
                return Err(HarnessError::InvalidRepository(format!(
                    "learning source scan exceeds the {MAX_LEARNING_SCAN_BYTES} byte limit"
                )));
            }
            let Some(frontmatter) = parse_learning_document(&content, &relative_path)? else {
                continue;
            };
            if content.len() > usize::try_from(MAX_LEARNING_FILE_BYTES).unwrap_or(usize::MAX) {
                return Err(HarnessError::InvalidRepository(format!(
                    "learning document `{}` exceeds the {MAX_LEARNING_FILE_BYTES} byte limit",
                    relative_path.display()
                )));
            }
            let metadata = frontmatter.learning;
            if let Some(previous) =
                seen_learning_ids.insert(metadata.learning_id.clone(), relative_path.clone())
            {
                return Err(HarnessError::InvalidRepository(format!(
                    "learning identifier `{}` is duplicated by `{}` and `{}`",
                    metadata.learning_id,
                    previous.display(),
                    relative_path.display()
                )));
            }
            if metadata.owner != *owner
                || metadata.action != action
                || metadata.intent != intent
                || !planned_units.contains(&metadata.verification_unit)
            {
                continue;
            }
            sources.push(LearningSourceBinding {
                repository_relative_path: portable_path(&relative_path),
                content_digest,
                metadata,
            });
        }
        if sources.len() > MAX_LEARNING_SOURCES {
            return Err(HarnessError::UnsupportedRuntime(format!(
                "matching learning sources exceed the {MAX_LEARNING_SOURCES} item limit"
            )));
        }
        sources.sort_by(|left, right| {
            left.repository_relative_path
                .cmp(&right.repository_relative_path)
        });
        Ok(sources)
    }

    fn ensure_learning_identifier_available(
        &self,
        owner: &DataOwner,
        learning_id: &str,
    ) -> HarnessResult<()> {
        validate_learning_identifier(learning_id)?;
        let knowledge_root = curation_root(owner, CurationKind::Knowledge)?;
        if !self.source_metadata(Path::new(&knowledge_root))?.exists() {
            return Ok(());
        }
        let mut paths = Vec::new();
        self.collect_markdown_paths(Path::new(&knowledge_root), &mut paths, 0)?;
        paths.sort();
        paths.dedup();
        if paths.len() > MAX_LEARNING_SCAN_FILES {
            return Err(HarnessError::InvalidRepository(format!(
                "learning source scan contains more than {MAX_LEARNING_SCAN_FILES} Markdown files"
            )));
        }
        let mut scanned_bytes = 0_usize;
        let mut seen_learning_ids = BTreeMap::new();
        for relative_path in paths {
            let path = self.root.join(&relative_path);
            let file = self.open_scoped_file(&relative_path, MAX_RETRIEVAL_FILE_BYTES)?;
            let (content, _) = read_text_file(file, MAX_RETRIEVAL_FILE_BYTES, &path)?;
            scanned_bytes = scanned_bytes.checked_add(content.len()).ok_or_else(|| {
                HarnessError::InvalidRepository("learning source scan size overflow".to_owned())
            })?;
            if scanned_bytes > MAX_LEARNING_SCAN_BYTES {
                return Err(HarnessError::InvalidRepository(format!(
                    "learning source scan exceeds the {MAX_LEARNING_SCAN_BYTES} byte limit"
                )));
            }
            let Some(frontmatter) = parse_learning_document(&content, &relative_path)? else {
                continue;
            };
            let identifier = frontmatter.learning.learning_id;
            if let Some(previous) =
                seen_learning_ids.insert(identifier.clone(), relative_path.clone())
            {
                return Err(HarnessError::InvalidRepository(format!(
                    "learning identifier `{identifier}` is duplicated by `{}` and `{}`",
                    previous.display(),
                    relative_path.display()
                )));
            }
            if identifier == learning_id {
                return Err(HarnessError::PlanDrift {
                    expected: format!("available learning identifier `{learning_id}`"),
                    actual: format!(
                        "learning identifier already exists at `{}`",
                        relative_path.display()
                    ),
                });
            }
        }
        Ok(())
    }

    #[allow(
        clippy::too_many_lines,
        reason = "bounded retrieval keeps scan limits, deterministic section ranking, exact-byte selection, and provenance construction in one auditable boundary"
    )]
    fn retrieve_context_documents(
        &self,
        roots: &[ContextRoot],
        query: &str,
        max_bytes: usize,
    ) -> HarnessResult<Vec<HarnessContextDocument>> {
        let mut terms = query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>();
        terms.sort();
        terms.dedup();
        if terms.is_empty() {
            return Err(HarnessError::InvalidRequest(
                "retrieval query must contain at least one term".to_owned(),
            ));
        }
        let mut paths = Vec::new();
        for root in roots {
            self.collect_markdown_paths(Path::new(&root.repository_relative_path), &mut paths, 0)?;
        }
        paths.sort();
        paths.dedup();
        if paths.len() > MAX_RETRIEVAL_FILES {
            return Err(HarnessError::InvalidRepository(format!(
                "retrieval roots contain more than {MAX_RETRIEVAL_FILES} Markdown files"
            )));
        }

        let mut scanned_bytes = 0_usize;
        let mut scanned_sections = 0_usize;
        let mut matches = Vec::new();
        for relative in paths {
            let path = self.root.join(&relative);
            let file = self.open_scoped_file(&relative, MAX_RETRIEVAL_FILE_BYTES)?;
            let (content, content_digest) = read_text_file(file, MAX_RETRIEVAL_FILE_BYTES, &path)?;
            scanned_bytes = scanned_bytes.checked_add(content.len()).ok_or_else(|| {
                HarnessError::InvalidRepository("retrieval scan size overflow".to_owned())
            })?;
            if scanned_bytes > MAX_RETRIEVAL_SCAN_BYTES {
                return Err(HarnessError::InvalidRepository(format!(
                    "retrieval scan exceeds the {MAX_RETRIEVAL_SCAN_BYTES} byte limit"
                )));
            }
            let title = repository::markdown_title(&content, &relative);
            let relative_path = portable_path(&relative);
            let remaining_sections = MAX_RETRIEVAL_SECTIONS
                .checked_sub(scanned_sections)
                .ok_or_else(|| {
                    HarnessError::InvalidRepository("retrieval section count overflow".to_owned())
                })?;
            let sections = markdown_sections(&content, remaining_sections)?;
            scanned_sections = scanned_sections
                .checked_add(sections.len())
                .ok_or_else(|| {
                    HarnessError::InvalidRepository("retrieval section count overflow".to_owned())
                })?;
            for section in sections {
                let searchable = section.content.to_lowercase();
                let matched_terms = terms
                    .iter()
                    .filter(|term| searchable.contains(term.as_str()))
                    .cloned()
                    .collect::<Vec<_>>();
                if matched_terms.is_empty() {
                    continue;
                }
                matches.push(HarnessContextDocument {
                    repository_relative_path: relative_path.clone(),
                    content_digest: content_digest.clone(),
                    title: title.clone(),
                    content: section.content,
                    start_line: section.start_line,
                    end_line: section.end_line,
                    matched_terms,
                });
            }
        }
        matches.sort_by(|left, right| {
            right
                .matched_terms
                .len()
                .cmp(&left.matched_terms.len())
                .then_with(|| {
                    left.repository_relative_path
                        .cmp(&right.repository_relative_path)
                })
                .then_with(|| left.start_line.cmp(&right.start_line))
        });
        if matches.is_empty() {
            return Err(HarnessError::InvalidRequest(
                "retrieval query matched no Markdown sections".to_owned(),
            ));
        }
        let mut selected = Vec::new();
        let mut total = 0_usize;
        for document in matches {
            let next = total.checked_add(document.content.len()).ok_or_else(|| {
                HarnessError::InvalidRepository("retrieval result size overflow".to_owned())
            })?;
            if next > max_bytes {
                continue;
            }
            total = next;
            selected.push(document);
        }
        if selected.is_empty() {
            return Err(HarnessError::InvalidRequest(
                "retrieval query matched Markdown sections, but none fit the byte budget"
                    .to_owned(),
            ));
        }
        Ok(selected)
    }

    fn collect_markdown_paths(
        &self,
        relative: &Path,
        paths: &mut Vec<PathBuf>,
        depth: usize,
    ) -> HarnessResult<()> {
        if depth > 128 {
            return Err(HarnessError::InvalidRepository(
                "retrieval directory nesting exceeds the limit".to_owned(),
            ));
        }
        let path = self.root.join(relative);
        let metadata = self.source_metadata(relative)?;
        if metadata.kind == SourcePathKind::Missing {
            return Err(HarnessError::FileRead {
                path,
                message: io::Error::from(io::ErrorKind::NotFound).to_string(),
            });
        }
        if metadata.kind == SourcePathKind::Symlink {
            if depth == 0 {
                return Err(HarnessError::PathEscapesRoot(path));
            }
            return Ok(());
        }
        if contains_raw_conversation_path(relative)
            || relative
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(is_skipped_directory_name)
        {
            return Ok(());
        }
        if metadata.is_file() {
            if path.extension().is_some_and(|extension| extension == "md") {
                paths.push(relative.to_path_buf());
            }
            return Ok(());
        }
        if metadata.kind != SourcePathKind::Directory {
            return Ok(());
        }
        let entries = self.source_children(relative)?;
        for entry in entries {
            let child = relative.join(entry);
            self.collect_markdown_paths(&child, paths, depth + 1)?;
            if paths.len() > MAX_RETRIEVAL_FILES {
                return Err(HarnessError::InvalidRepository(format!(
                    "retrieval roots contain more than {MAX_RETRIEVAL_FILES} Markdown files"
                )));
            }
        }
        Ok(())
    }

    fn require_context_path(&self, relative_path: &str) -> HarnessResult<ContextRoot> {
        validate_relative_path(Path::new(relative_path), "context root")?;
        let path = self.root.join(relative_path);
        match self.source_metadata(Path::new(relative_path))?.kind {
            SourcePathKind::Missing => {
                return Err(HarnessError::FileRead {
                    path,
                    message: io::Error::from(io::ErrorKind::NotFound).to_string(),
                });
            }
            SourcePathKind::Symlink => return Err(HarnessError::PathEscapesRoot(path)),
            _ => {}
        }
        Ok(ContextRoot {
            repository_relative_path: relative_path.to_owned(),
        })
    }

    fn bind_curation_sources(
        &self,
        request: &HarnessRequest,
        allowed: &[ContextRoot],
        denied: &[ContextRoot],
    ) -> HarnessResult<Vec<SourceBinding>> {
        if request.action != HarnessAction::VaultCuration {
            return Ok(Vec::new());
        }
        request
            .curation_sources
            .iter()
            .map(|source| {
                if request.curation_kind == Some(CurationKind::Ontology) {
                    let ontology_root = curation_root(&request.owner, CurationKind::Ontology)?;
                    if path_is_within(source, &ontology_root)
                        || request.targets.contains(source)
                    {
                        return Err(HarnessError::InvalidRequest(format!(
                            "ontology source `{source}` must be a canonical source outside the ontology target set"
                        )));
                    }
                }
                if !allowed
                    .iter()
                    .any(|root| path_is_within(source, &root.repository_relative_path))
                    || denied
                        .iter()
                        .any(|root| path_is_within(source, &root.repository_relative_path))
                {
                    return Err(HarnessError::InvalidRequest(format!(
                        "curation source `{source}` is outside the allowed context paths"
                    )));
                }
                Ok(SourceBinding {
                    repository_relative_path: source.clone(),
                    content_digest: self.digest_file_within_root(source, MAX_POLICY_BYTES)?,
                })
            })
            .collect()
    }

    fn allowed_context_roots(&self, request: &HarnessRequest) -> HarnessResult<Vec<ContextRoot>> {
        let mut roots = match (&request.owner, request.curation_kind) {
            (DataOwner::Personal, Some(CurationKind::Journal))
                if matches!(
                    request.action,
                    HarnessAction::VaultCuration | HarnessAction::VaultRead
                ) =>
            {
                vec![
                    "vault/profile".to_owned(),
                    "vault/personal/index.md".to_owned(),
                    "vault/personal/journal".to_owned(),
                ]
            }
            (DataOwner::Profile, _) => vec!["vault/profile".to_owned()],
            (DataOwner::CommonWork, _) => {
                vec!["vault/profile".to_owned(), "vault/work/common".to_owned()]
            }
            (DataOwner::PersonalBusiness, _) => vec![
                "vault/profile".to_owned(),
                "vault/personal/index.md".to_owned(),
                "vault/personal/business".to_owned(),
            ],
            (DataOwner::Personal, _) => vec![
                "vault/profile".to_owned(),
                "vault/personal/index.md".to_owned(),
                "vault/personal/profile.md".to_owned(),
                "vault/personal/decisions".to_owned(),
                "vault/personal/facts".to_owned(),
                "vault/personal/knowledge".to_owned(),
                "vault/personal/learning".to_owned(),
                "vault/personal/ontology".to_owned(),
                "vault/personal/projects/ideas".to_owned(),
                "vault/personal/writing".to_owned(),
            ],
            (DataOwner::PersonalProject { project }, _) => {
                let mut roots = vec![
                    "vault/profile".to_owned(),
                    "vault/personal/index.md".to_owned(),
                ];
                roots.extend(self.project_context_roots("vault/personal/projects", project)?);
                roots
            }
            (DataOwner::Company { company }, _) => {
                self.require_registered_company(company)?;
                let company_root = format!("vault/work/{company}");
                let company_index = format!("{company_root}/index.md");
                self.digest_file_within_root(&company_index, MAX_POLICY_BYTES)?;
                vec![
                    "vault/profile".to_owned(),
                    "vault/work/common".to_owned(),
                    company_root,
                ]
            }
            (DataOwner::CompanyProject { company, project }, _) => {
                self.require_registered_company(company)?;
                let company_root = format!("vault/work/{company}");
                let company_index = format!("{company_root}/index.md");
                self.digest_file_within_root(&company_index, MAX_POLICY_BYTES)?;
                let mut roots = vec![
                    "vault/profile".to_owned(),
                    "vault/work/common".to_owned(),
                    company_index,
                    format!("{company_root}/rules"),
                ];
                roots.extend(
                    self.project_context_roots(&format!("{company_root}/projects"), project)?,
                );
                let overview = format!("{company_root}/overview/{project}.md");
                if self.source_metadata(Path::new(&overview))?.is_file() {
                    roots.push(overview);
                }
                roots
            }
        };
        if request.action == HarnessAction::VaultCuration
            && request.curation_kind == Some(CurationKind::Ontology)
            && request.owner == DataOwner::Personal
        {
            roots.extend(
                request
                    .curation_sources
                    .iter()
                    .filter(|source| {
                        path_is_within(source, "vault/personal")
                            && !path_is_within(source, "vault/personal/business")
                            && !path_is_within(source, "vault/personal/journal")
                            && !path_is_within(source, "vault/personal/ontology")
                    })
                    .cloned(),
            );
        }
        roots.sort();
        roots.dedup();
        roots
            .iter()
            .map(|root| self.require_context_path(root))
            .collect()
    }

    fn retrieval_roots(&self, request: &HarnessRequest) -> HarnessResult<Vec<ContextRoot>> {
        if request.action != HarnessAction::VaultRead {
            return Ok(Vec::new());
        }
        let kind = request.curation_kind.ok_or_else(|| {
            HarnessError::InvalidRequest("Vault read requires a content kind".to_owned())
        })?;
        let category_root = curation_root(&request.owner, kind)?;
        if self.source_metadata(Path::new(&category_root))?.exists() {
            return vec![category_root]
                .into_iter()
                .map(|candidate| self.require_context_path(&candidate))
                .collect();
        }

        let candidates = match (&request.owner, kind) {
            (
                DataOwner::PersonalProject { project },
                CurationKind::Fact | CurationKind::Knowledge,
            ) => {
                return self
                    .project_context_roots("vault/personal/projects", project)?
                    .iter()
                    .map(|candidate| self.require_context_path(candidate))
                    .collect();
            }
            (DataOwner::PersonalBusiness, CurationKind::Fact | CurationKind::Knowledge) => {
                vec!["vault/personal/business".to_owned()]
            }
            (DataOwner::Company { company }, CurationKind::Fact) => vec![
                format!("vault/work/{company}/index.md"),
                format!("vault/work/{company}/experience"),
                format!("vault/work/{company}/projects"),
                format!("vault/work/{company}/overview"),
            ],
            (DataOwner::Company { company }, CurationKind::Knowledge) => vec![
                format!("vault/work/{company}/index.md"),
                format!("vault/work/{company}/rules"),
                format!("vault/work/{company}/overview"),
            ],
            (
                DataOwner::CompanyProject { company, project },
                CurationKind::Fact | CurationKind::Knowledge,
            ) => {
                return self
                    .project_context_roots(&format!("vault/work/{company}/projects"), project)?
                    .iter()
                    .map(|candidate| self.require_context_path(candidate))
                    .collect();
            }
            _ => Vec::new(),
        };
        self.existing_context_roots(candidates)
    }

    fn context_grant_roots(&self, grant: &ContextGrant) -> HarnessResult<Vec<ContextRoot>> {
        let candidates = match &grant.owner {
            DataOwner::PersonalProject { project } => {
                self.project_context_roots("vault/personal/projects", project)?
            }
            DataOwner::Company { company } => {
                self.require_registered_company(company)?;
                vec![
                    format!("vault/work/{company}/index.md"),
                    format!("vault/work/{company}/experience"),
                    format!("vault/work/{company}/projects"),
                    format!("vault/work/{company}/overview"),
                ]
            }
            DataOwner::CompanyProject { company, project } => {
                self.require_registered_company(company)?;
                let mut roots = vec![format!("vault/work/{company}/index.md")];
                roots.extend(
                    self.project_context_roots(&format!("vault/work/{company}/projects"), project)?,
                );
                let overview = format!("vault/work/{company}/overview/{project}.md");
                if self.source_metadata(Path::new(&overview))?.is_file() {
                    roots.push(overview);
                }
                roots
            }
            _ => {
                return Err(HarnessError::InvalidRequest(
                    "context grant owner must be a personal project, company, or company project"
                        .to_owned(),
                ));
            }
        };
        self.existing_context_roots(candidates)
    }

    fn bind_evidence_sources(
        &self,
        grant: &ContextGrant,
        source_paths: &[String],
    ) -> HarnessResult<Vec<SourceBinding>> {
        validate_evidence_source_paths(source_paths)?;
        let grant_roots = self.context_grant_roots(grant)?;
        let mut bindings = source_paths
            .iter()
            .map(|source_path| {
                if !grant_roots.iter().any(|root| {
                    path_is_within(source_path, &root.repository_relative_path)
                }) {
                    return Err(HarnessError::InvalidRequest(format!(
                        "career evidence source `{source_path}` is outside the selected evidence owner"
                    )));
                }
                let relative = Path::new(source_path);
                let path = self.root.join(relative);
                let file = self.open_scoped_file(relative, MAX_POLICY_BYTES)?;
                Ok(SourceBinding {
                    repository_relative_path: source_path.clone(),
                    content_digest: digest_file(file, MAX_POLICY_BYTES, &path)?,
                })
            })
            .collect::<HarnessResult<Vec<_>>>()?;
        bindings.sort_by(|left, right| {
            left.repository_relative_path
                .cmp(&right.repository_relative_path)
        });
        Ok(bindings)
    }

    fn denied_context_roots(
        request: &HarnessRequest,
        context_grants: &[ContextGrant],
    ) -> Vec<ContextRoot> {
        let journal_is_selected = matches!(
            request.action,
            HarnessAction::VaultCuration | HarnessAction::VaultRead
        ) && request.curation_kind == Some(CurationKind::Journal);
        let roots = match &request.owner {
            DataOwner::Profile => vec!["vault/personal".to_owned(), "vault/work".to_owned()],
            DataOwner::CommonWork => vec!["vault/personal".to_owned()],
            DataOwner::PersonalBusiness => {
                vec!["vault/personal/journal".to_owned(), "vault/work".to_owned()]
            }
            DataOwner::Personal if journal_is_selected => vec!["vault/work".to_owned()],
            DataOwner::Personal => {
                let mut denied = vec![
                    "vault/personal/business".to_owned(),
                    "vault/personal/journal".to_owned(),
                ];
                let has_company_grant = context_grants.iter().any(|grant| {
                    matches!(
                        grant.owner,
                        DataOwner::Company { .. } | DataOwner::CompanyProject { .. }
                    )
                });
                if !has_company_grant {
                    denied.push("vault/work".to_owned());
                }
                denied
            }
            DataOwner::PersonalProject { .. } => vec![
                "vault/personal/business".to_owned(),
                "vault/personal/journal".to_owned(),
                "vault/work".to_owned(),
            ],
            DataOwner::Company { .. } | DataOwner::CompanyProject { .. } => {
                vec!["vault/personal".to_owned()]
            }
        };
        roots
            .into_iter()
            .map(|repository_relative_path| ContextRoot {
                repository_relative_path,
            })
            .collect()
    }

    fn required_policies(
        &self,
        request: &HarnessRequest,
        intent: HarnessIntent,
        context_grants: &[ContextGrant],
        roles: &[HarnessRole],
    ) -> HarnessResult<(
        Vec<PolicyBinding>,
        Vec<PolicyBinding>,
        Vec<RolePolicyBinding>,
    )> {
        let mut orchestrator_requirements = BTreeMap::new();
        for capability in [
            PolicyCapability::AgentHarness,
            PolicyCapability::AgentOperatingPreferences,
            PolicyCapability::ContextScopeRouting,
            PolicyCapability::MandatoryPreflight,
        ] {
            add_policy_requirement(
                &mut orchestrator_requirements,
                capability,
                PolicyRoleMask::ALL,
            );
        }
        close_policy_requirements(&mut orchestrator_requirements)?;
        let orchestrator_bindings = orchestrator_requirements
            .keys()
            .map(|capability| self.bind_policy_capability(capability))
            .collect::<HarnessResult<BTreeMap<_, _>>>()?;
        let orchestrator_policies = orchestrator_bindings.values().cloned().collect::<Vec<_>>();

        let mut requirements = BTreeMap::new();

        self.append_grant_policy_requirements(&mut requirements, context_grants)?;
        Self::append_action_policy_requirements(&mut requirements, request);
        add_policy_requirement(
            &mut requirements,
            PolicyCapability::CommonReviewQuality,
            PolicyRoleMask::REVIEW,
        );
        if uses_solo_mvp_ideation_contract(request, intent) {
            add_policy_requirement(
                &mut requirements,
                PolicyCapability::SoloMvpIdeaDiscovery,
                PolicyRoleMask::ALL,
            );
        }
        if matches!(
            request.action,
            HarnessAction::DocumentWrite
                | HarnessAction::DocumentReview
                | HarnessAction::VaultCuration
        ) {
            add_policy_requirement(
                &mut requirements,
                PolicyCapability::CommonDocumentQuality,
                PolicyRoleMask::ALL,
            );
        }

        self.append_owner_policy_requirements(&mut requirements, request)?;
        Self::append_intent_policy_requirements(&mut requirements, request.action, intent);

        if matches!(
            request.action,
            HarnessAction::VaultCuration | HarnessAction::VaultRead
        ) || request
            .targets
            .iter()
            .any(|target| target.starts_with("vault/"))
        {
            add_policy_requirement(
                &mut requirements,
                PolicyCapability::ContextVaultOperatingModel,
                PolicyRoleMask::ALL,
            );
            add_policy_requirement(
                &mut requirements,
                PolicyCapability::ContextDocumentStability,
                PolicyRoleMask::ALL,
            );
        }

        close_policy_requirements(&mut requirements)?;
        let planned_roles = PolicyRoleMask::for_roles(roles);
        requirements.retain(|_, mask| {
            *mask = mask.intersect(planned_roles);
            !mask.is_empty()
        });
        let bindings = requirements
            .keys()
            .map(|capability| self.bind_policy_capability(capability))
            .collect::<HarnessResult<BTreeMap<_, _>>>()?;
        let mut required_policies = orchestrator_policies.clone();
        required_policies.extend(bindings.values().cloned());
        required_policies.sort();
        let role_policy_bindings = roles
            .iter()
            .map(|role| RolePolicyBinding {
                role: *role,
                policies: requirements
                    .iter()
                    .filter(|(_, mask)| mask.contains(*role))
                    .filter_map(|(capability, _)| bindings.get(capability).cloned())
                    .collect(),
            })
            .collect();
        Ok((
            required_policies,
            orchestrator_policies,
            role_policy_bindings,
        ))
    }

    fn append_action_policy_requirements(
        requirements: &mut BTreeMap<PolicyCapability, PolicyRoleMask>,
        request: &HarnessRequest,
    ) {
        let is_code = matches!(
            request.action,
            HarnessAction::CodeWrite | HarnessAction::CodeReview
        );
        if is_code || request.action == HarnessAction::Design {
            add_policy_requirement(
                requirements,
                PolicyCapability::CommonCodeQuality,
                PolicyRoleMask::ALL,
            );
        }
        if request.action == HarnessAction::CodeReview {
            add_policy_requirement(
                requirements,
                PolicyCapability::CodeReview,
                PolicyRoleMask::REVIEW,
            );
        }
        if is_code && request.targets.iter().any(|target| is_rust_target(target)) {
            add_policy_requirement(
                requirements,
                PolicyCapability::RustCodeStyle,
                PolicyRoleMask::WRITE_REVIEW,
            );
        }
        if is_code
            && request
                .targets
                .iter()
                .any(|target| is_dependency_target(target))
        {
            add_policy_requirement(
                requirements,
                PolicyCapability::Dependency,
                PolicyRoleMask::WRITE_REVIEW,
            );
        }
    }

    fn append_grant_policy_requirements(
        &self,
        requirements: &mut BTreeMap<PolicyCapability, PolicyRoleMask>,
        context_grants: &[ContextGrant],
    ) -> HarnessResult<()> {
        for grant in context_grants {
            let company = match &grant.owner {
                DataOwner::Company { company } | DataOwner::CompanyProject { company, .. } => {
                    company
                }
                DataOwner::PersonalProject { project } => {
                    add_policy_requirement(
                        requirements,
                        PolicyCapability::GrantedPersonalProjectSource(
                            self.personal_project_entrypoint(project)?,
                        ),
                        PolicyRoleMask::ALL,
                    );
                    continue;
                }
                _ => continue,
            };
            self.require_registered_company(company)?;
            add_policy_requirement(
                requirements,
                PolicyCapability::GrantedCompanyIndex(company.clone()),
                PolicyRoleMask::ALL,
            );
        }
        Ok(())
    }

    fn append_intent_policy_requirements(
        requirements: &mut BTreeMap<PolicyCapability, PolicyRoleMask>,
        action: HarnessAction,
        intent: HarnessIntent,
    ) {
        if matches!(
            intent,
            HarnessIntent::PolicyMaintenance | HarnessIntent::OutputAdapter { .. }
        ) {
            add_policy_requirement(
                requirements,
                PolicyCapability::ContextVaultOperatingModel,
                PolicyRoleMask::ALL,
            );
            add_policy_requirement(
                requirements,
                PolicyCapability::ContextDocumentStability,
                stability_policy_roles(action),
            );
        }
        let HarnessIntent::CareerArtifact { surface } = intent else {
            return;
        };
        for (capability, mask) in [
            (PolicyCapability::CareerResumeSourceMap, PolicyRoleMask::ALL),
            (
                PolicyCapability::CareerClaimTokenOutputSystem,
                PolicyRoleMask::ALL,
            ),
            (PolicyCapability::CareerOutputAssembly, PolicyRoleMask::ALL),
            (
                PolicyCapability::CareerContributionAudit,
                PolicyRoleMask::AUTHOR_AND_REVIEWER,
            ),
            (
                PolicyCapability::CareerPerspectiveAndPublicSafety,
                PolicyRoleMask::WRITE_REVIEW,
            ),
            (
                PolicyCapability::CareerPortfolioCasebook,
                PolicyRoleMask::AUTHOR_AND_REVIEWER,
            ),
        ] {
            add_policy_requirement(requirements, capability, mask);
        }
        match surface {
            CareerOutputSurface::Resume => {
                add_policy_requirement(
                    requirements,
                    PolicyCapability::CareerResumeCaseView,
                    PolicyRoleMask::WRITE_REVIEW,
                );
                add_policy_requirement(
                    requirements,
                    PolicyCapability::CareerResumePdfBaseline,
                    PolicyRoleMask::WRITE_REVIEW,
                );
            }
            CareerOutputSurface::CareerDescription => add_policy_requirement(
                requirements,
                PolicyCapability::CareerCaseDocumentContract,
                PolicyRoleMask::WRITE_REVIEW,
            ),
            CareerOutputSurface::Portfolio | CareerOutputSurface::ProfessionalProfile => {}
            CareerOutputSurface::General => {
                add_policy_requirement(
                    requirements,
                    PolicyCapability::CareerResumeCaseView,
                    PolicyRoleMask::WRITE_REVIEW,
                );
                add_policy_requirement(
                    requirements,
                    PolicyCapability::CareerCaseDocumentContract,
                    PolicyRoleMask::WRITE_REVIEW,
                );
            }
        }
    }

    fn append_owner_policy_requirements(
        &self,
        requirements: &mut BTreeMap<PolicyCapability, PolicyRoleMask>,
        request: &HarnessRequest,
    ) -> HarnessResult<()> {
        match &request.owner {
            DataOwner::Profile => {
                add_policy_requirement(
                    requirements,
                    PolicyCapability::ProfileIndex,
                    PolicyRoleMask::ALL,
                );
                add_policy_requirement(
                    requirements,
                    PolicyCapability::ContextDocumentStability,
                    stability_policy_roles(request.action),
                );
            }
            DataOwner::CommonWork => {
                add_policy_requirement(
                    requirements,
                    PolicyCapability::WorkAgentGuide,
                    PolicyRoleMask::ALL,
                );
                add_policy_requirement(
                    requirements,
                    PolicyCapability::WorkAgentOperatingPreferences,
                    PolicyRoleMask::ALL,
                );
                add_policy_requirement(
                    requirements,
                    PolicyCapability::WorkCompanyRegistry,
                    PolicyRoleMask::ALL,
                );
                add_policy_requirement(
                    requirements,
                    PolicyCapability::ContextDocumentStability,
                    stability_policy_roles(request.action),
                );
            }
            DataOwner::PersonalBusiness => {
                add_policy_requirement(
                    requirements,
                    PolicyCapability::PersonalIndex,
                    PolicyRoleMask::ALL,
                );
                add_policy_requirement(
                    requirements,
                    PolicyCapability::PersonalBusinessIndex,
                    PolicyRoleMask::ALL,
                );
            }
            DataOwner::Personal | DataOwner::PersonalProject { .. } => {
                add_policy_requirement(
                    requirements,
                    PolicyCapability::PersonalIndex,
                    PolicyRoleMask::ALL,
                );
            }
            DataOwner::Company { company } | DataOwner::CompanyProject { company, .. } => {
                add_policy_requirement(
                    requirements,
                    PolicyCapability::CompanyIndex(company.clone()),
                    PolicyRoleMask::ALL,
                );
                self.append_company_domain_policy_requirements(requirements, company, request)?;
            }
        }
        if matches!(
            &request.owner,
            DataOwner::PersonalBusiness | DataOwner::Personal | DataOwner::PersonalProject { .. }
        ) {
            add_policy_requirement(
                requirements,
                PolicyCapability::AiCollaborationValues,
                PolicyRoleMask::ALL,
            );
        }
        Ok(())
    }

    fn append_company_domain_policy_requirements(
        &self,
        requirements: &mut BTreeMap<PolicyCapability, PolicyRoleMask>,
        company: &str,
        request: &HarnessRequest,
    ) -> HarnessResult<()> {
        let routing_path = company_routing_path(company);
        let path = self.root.join(&routing_path);
        if !self.source_metadata(Path::new(&routing_path))?.exists() {
            return Ok(());
        }
        add_policy_requirement(
            requirements,
            PolicyCapability::CompanyRouting(company.to_owned()),
            PolicyRoleMask::ALL,
        );
        let file = self.open_scoped_file(Path::new(&routing_path), MAX_POLICY_BYTES)?;
        let (content, _) = read_text_file(file, MAX_POLICY_BYTES, &path)?;
        let searchable = company_route_searchable(request, company);
        let routes = parse_company_domain_routes(&content, company)?;
        for policy in routes
            .iter()
            .flat_map(|route| route.policies.iter())
            .collect::<BTreeSet<_>>()
        {
            self.validate_source_file(Path::new(policy))?;
        }
        for route in routes {
            if route.target_kind.matches(request)
                && route.actions.contains(&request.action)
                && route
                    .signals
                    .iter()
                    .any(|signal| contains_company_route_signal(&searchable, signal))
            {
                for policy in route.policies {
                    add_policy_requirement(
                        requirements,
                        company_domain_policy_capability(company, policy),
                        PolicyRoleMask::ALL,
                    );
                }
            }
        }
        Ok(())
    }

    fn bind_policy_capability(
        &self,
        capability: &PolicyCapability,
    ) -> HarnessResult<(PolicyCapability, PolicyBinding)> {
        let (id, path) = capability.binding_location();
        Ok((capability.clone(), self.bind_policy(id, &path)?))
    }

    fn personal_project_entrypoint(&self, project: &str) -> HarnessResult<String> {
        let source = format!("vault/personal/projects/{project}.md");
        if self.source_metadata(Path::new(&source))?.is_file() {
            return Ok(source);
        }
        let index = format!("vault/personal/projects/{project}/index.md");
        if self.source_metadata(Path::new(&index))?.is_file() {
            return Ok(index);
        }
        Err(HarnessError::InvalidRequest(format!(
            "personal project `{project}` does not have a canonical Markdown entrypoint"
        )))
    }

    fn project_context_roots(&self, base: &str, project: &str) -> HarnessResult<Vec<String>> {
        let candidates = [format!("{base}/{project}.md"), format!("{base}/{project}")];
        let roots = self
            .existing_context_roots(candidates)?
            .into_iter()
            .map(|root| root.repository_relative_path)
            .collect::<Vec<_>>();
        if roots.is_empty() {
            return Err(HarnessError::InvalidRequest(format!(
                "project `{project}` has no canonical Vault context under `{base}`"
            )));
        }
        Ok(roots)
    }

    fn require_registered_company(&self, company: &str) -> HarnessResult<()> {
        let registry = "vault/work/common/router/company-registry.md";
        let path = self.root.join(registry);
        let file = self.open_scoped_file(Path::new(registry), MAX_POLICY_BYTES)?;
        let (content, _) = read_text_file(file, MAX_POLICY_BYTES, &path)?;
        if parse_registered_company_slugs(&content)?.contains(company) {
            Ok(())
        } else {
            Err(HarnessError::InvalidRequest(format!(
                "unknown company `{company}`; add it to the canonical company registry first"
            )))
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one boundary check preserves both native relocated Core and filesystem Vault ownership"
    )]
    fn validate_vault_targets(
        &self,
        request: &HarnessRequest,
        context_grants: &[ContextGrant],
        workspace_root: &Path,
        intent: HarnessIntent,
    ) -> HarnessResult<()> {
        if request.owner == DataOwner::CommonWork {
            if self.store_identity.is_none()
                || workspace_root != self.root
                || !context_grants.is_empty()
            {
                return Err(HarnessError::InvalidRequest(
                    "common work maintenance requires the native source workspace without evidence grants"
                        .to_owned(),
                ));
            }
            for target in &request.targets {
                let relative = Path::new(target);
                if !path_is_within(target, "vault/work/common")
                    || !relative
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
                    || !self.source_metadata(relative)?.is_file()
                {
                    return Err(HarnessError::InvalidRequest(format!(
                        "common work target `{target}` must be an existing Markdown original"
                    )));
                }
                let file = self.open_scoped_file(relative, MAX_POLICY_BYTES)?;
                read_text_file(file, MAX_POLICY_BYTES, &self.root.join(relative))?;
            }
        }
        if workspace_root.starts_with(&self.root) && workspace_root != self.root {
            return Err(HarnessError::InvalidRequest(
                "the Vault repository must use its repository root as the workspace root"
                    .to_owned(),
            ));
        }
        if workspace_root != self.root {
            for target in &request.targets {
                let candidate = workspace_root.join(target);
                if path_resolves_within(&candidate, &self.root)? {
                    return Err(HarnessError::InvalidRequest(
                        "Vault repository targets require the Vault repository root as the workspace root"
                            .to_owned(),
                    ));
                }
                if path_is_within(target, "crates/context-core")
                    || path_resolves_within(
                        &candidate,
                        &workspace_root.join("crates/context-core"),
                    )?
                {
                    if !path_is_within(target, "crates/context-core")
                        || request.owner != DataOwner::Profile
                        || intent != HarnessIntent::PolicyMaintenance
                        || self.store_identity.is_none()
                        || !context_grants.is_empty()
                    {
                        return Err(HarnessError::InvalidRequest(
                            "relocated Core infrastructure requires profile policy-maintenance with a distinct native policy source".to_owned(),
                        ));
                    }
                    let target_kind =
                        profile_infrastructure_target_kind(target).ok_or_else(|| {
                            HarnessError::InvalidRequest(
                                "relocated Core target is outside profile infrastructure"
                                    .to_owned(),
                            )
                        })?;
                    if !profile_action_accepts_target(request.action, target_kind)
                        || target
                            .split('/')
                            .any(|part| matches!(part, "" | "." | ".."))
                    {
                        return Err(HarnessError::InvalidRequest(
                            "relocated Core target must be normalized and compatible with its action".to_owned(),
                        ));
                    }
                    verified_repository_file(workspace_root, Path::new(target))?;
                }
            }
            return Ok(());
        }

        let write_roots = primary_vault_write_roots(&request.owner);
        let grant_roots = context_grants
            .iter()
            .map(|grant| self.context_grant_roots(grant))
            .collect::<HarnessResult<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let denied_roots = Self::denied_context_roots(request, context_grants);

        for target in &request.targets {
            if !target.starts_with("vault/") {
                if request.owner != DataOwner::Profile {
                    return Err(HarnessError::InvalidRequest(format!(
                        "Vault repository infrastructure target `{target}` requires the profile owner"
                    )));
                }
                let Some(target_kind) = profile_infrastructure_target_kind(target) else {
                    return Err(HarnessError::InvalidRequest(format!(
                        "Vault repository target `{target}` is outside the profile infrastructure allowlist"
                    )));
                };
                if !profile_action_accepts_target(request.action, target_kind) {
                    return Err(HarnessError::InvalidRequest(format!(
                        "Vault repository target `{target}` is incompatible with the `{}` action",
                        request.action.as_str()
                    )));
                }
                if self.source_metadata(Path::new(target))?.exists() {
                    self.validate_source_file(Path::new(target))?;
                }
                continue;
            }
            if grant_roots
                .iter()
                .any(|root| path_is_within(target, &root.repository_relative_path))
            {
                return Err(HarnessError::InvalidRequest(format!(
                    "Vault target `{target}` overlaps a read-only context grant"
                )));
            }
            if !write_roots.iter().any(|root| path_is_within(target, root))
                || denied_roots
                    .iter()
                    .any(|root| path_is_within(target, &root.repository_relative_path))
            {
                return Err(HarnessError::InvalidRequest(format!(
                    "Vault target `{target}` is outside the primary owner's write scope"
                )));
            }
            if self.source_metadata(Path::new(target))?.exists() {
                self.validate_source_file(Path::new(target))?;
            }
        }
        Ok(())
    }
}

fn primary_vault_write_roots(owner: &DataOwner) -> Vec<String> {
    match owner {
        DataOwner::Profile => vec!["vault/profile".to_owned()],
        DataOwner::CommonWork => vec!["vault/work/common".to_owned()],
        DataOwner::PersonalBusiness => vec!["vault/personal/business".to_owned()],
        DataOwner::Personal => vec!["vault/personal".to_owned()],
        DataOwner::PersonalProject { project } => vec![
            format!("vault/personal/projects/{project}.md"),
            format!("vault/personal/projects/{project}"),
        ],
        DataOwner::Company { company } => vec![format!("vault/work/{company}")],
        DataOwner::CompanyProject { company, project } => vec![
            format!("vault/work/{company}/projects/{project}.md"),
            format!("vault/work/{company}/projects/{project}"),
        ],
    }
}

pub(crate) fn parse_registered_company_slugs(content: &str) -> HarnessResult<BTreeSet<String>> {
    let (_, body) = split_markdown_frontmatter(content);
    let mut companies = BTreeSet::new();
    let mut in_registry = false;
    let mut registry_section_count = 0;
    let mut fence = None;

    for raw_line in body.lines() {
        let Some(line) = markdown_block_line(raw_line.trim_end()) else {
            continue;
        };
        if let Some(open) = fence {
            if is_markdown_closing_fence(line, open) {
                fence = None;
            }
            continue;
        }
        if let Some(open) = markdown_opening_fence(line) {
            fence = Some(open);
            continue;
        }
        if line == "## Company Registry" {
            registry_section_count += 1;
            in_registry = true;
            continue;
        }
        let heading_level = line.chars().take_while(|value| *value == '#').count();
        if in_registry && heading_level == 2 && is_markdown_atx_heading(line) {
            in_registry = false;
        }
        if !in_registry || !line.starts_with('|') {
            continue;
        }

        for cell in line.split('|').map(str::trim) {
            let Some(candidate) = cell
                .strip_prefix('`')
                .and_then(|value| value.strip_suffix('`'))
            else {
                continue;
            };
            let parts = candidate.split('/').collect::<Vec<_>>();
            let ["vault", "work", company, "index.md"] = parts.as_slice() else {
                continue;
            };
            if *company == "common"
                || company.is_empty()
                || company.starts_with('-')
                || company.ends_with('-')
                || !company
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            {
                return Err(HarnessError::InvalidRepository(format!(
                    "company registry has an invalid company index path: {candidate}"
                )));
            }
            if !companies.insert((*company).to_owned()) {
                return Err(HarnessError::InvalidRepository(format!(
                    "company registry repeats a company index: {candidate}"
                )));
            }
        }
    }
    if registry_section_count != 1 {
        return Err(HarnessError::InvalidRepository(format!(
            "company registry must contain exactly one `## Company Registry` section, found {registry_section_count}"
        )));
    }
    if companies.is_empty() {
        return Err(HarnessError::InvalidRepository(
            "company registry must declare at least one company index".to_owned(),
        ));
    }
    Ok(companies)
}

fn company_routing_path(company: &str) -> String {
    format!("vault/work/{company}/preferences/{company}-routing.md")
}

const SHARED_COMPANY_DOMAIN_POLICIES: &[(&str, PolicyCapability)] = &[
    (
        "vault/profile/rules/common-code-quality.md",
        PolicyCapability::CommonCodeQuality,
    ),
    (
        "vault/work/common/rules/code-review.md",
        PolicyCapability::CodeReview,
    ),
    (
        "vault/work/common/rules/rust-code-style.md",
        PolicyCapability::RustCodeStyle,
    ),
    (
        "vault/work/common/rules/dependency.md",
        PolicyCapability::Dependency,
    ),
    ("vault/work/common/rules/issue.md", PolicyCapability::Issue),
];

fn company_domain_policy_capability(company: &str, path: String) -> PolicyCapability {
    SHARED_COMPANY_DOMAIN_POLICIES
        .iter()
        .find_map(|(known_path, capability)| (path == *known_path).then(|| capability.clone()))
        .unwrap_or_else(|| PolicyCapability::CompanyDomainRule {
            company: company.to_owned(),
            id: format!(
                "company-domain-rule-{}",
                Path::new(&path)
                    .file_stem()
                    .and_then(OsStr::to_str)
                    .expect("validated company rule paths have a UTF-8 file stem")
            ),
            path,
        })
}

fn company_route_searchable(request: &HarnessRequest, company: &str) -> String {
    let company_vault_root = format!("vault/work/{company}/");
    let project = match &request.owner {
        DataOwner::CompanyProject { project, .. } => Some(project.as_str()),
        _ => None,
    };
    std::iter::once(request.objective.as_str())
        .chain(project)
        .chain(request.targets.iter().map(|target| {
            target
                .strip_prefix(&company_vault_root)
                .unwrap_or(target.as_str())
        }))
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase()
}

pub(crate) fn parse_company_domain_routes(
    content: &str,
    company: &str,
) -> HarnessResult<Vec<CompanyDomainRoute>> {
    let frontmatter = ParsedFrontmatter::from_markdown(content)
        .map_err(|error| {
            HarnessError::InvalidRepository(format!(
                "company routing frontmatter is invalid: {error}"
            ))
        })?
        .ok_or_else(|| {
            HarnessError::InvalidRepository(
                "company routing document must have frontmatter".to_owned(),
            )
        })?;
    let metadata = frontmatter
        .deserialize::<CompanyRoutingFrontmatter>()
        .map_err(|error| {
            HarnessError::InvalidRepository(format!("company routing contract is invalid: {error}"))
        })?;
    if metadata.domain_routes.is_empty() {
        return Err(HarnessError::InvalidRepository(
            "company routing contract must declare at least one domain route".to_owned(),
        ));
    }

    let company_rule_root = format!("vault/work/{company}/rules");
    let mut normalized_signals = BTreeSet::new();
    let mut policy_ids = BTreeMap::<String, String>::new();
    for route in &metadata.domain_routes {
        if route.signals.is_empty() {
            return Err(HarnessError::InvalidRepository(
                "company domain route must declare at least one signal".to_owned(),
            ));
        }
        if route.actions.is_empty() {
            return Err(HarnessError::InvalidRepository(
                "company domain route must declare at least one action".to_owned(),
            ));
        }
        if route.policies.is_empty() {
            return Err(HarnessError::InvalidRepository(
                "company domain route must declare at least one policy".to_owned(),
            ));
        }
        let mut route_actions = BTreeSet::new();
        for action in &route.actions {
            if !route_actions.insert(action.as_str()) {
                return Err(HarnessError::InvalidRepository(format!(
                    "company domain route repeats an action: {}",
                    action.as_str()
                )));
            }
        }
        let mut route_policies = BTreeSet::new();
        for policy in &route.policies {
            validate_single_line("company domain route policy", policy, MAX_TARGET_LENGTH)?;
            validate_relative_path(Path::new(policy), "company domain route policy")?;
            let is_company_rule = Path::new(policy).extension().and_then(OsStr::to_str)
                == Some("md")
                && path_is_within(policy, &company_rule_root);
            let is_shared_policy = SHARED_COMPANY_DOMAIN_POLICIES
                .iter()
                .any(|(known_path, _)| policy == known_path);
            if !is_company_rule && !is_shared_policy {
                return Err(HarnessError::InvalidRepository(format!(
                    "company domain route policy `{policy}` must be a company leaf rule or an explicitly supported shared policy"
                )));
            }
            if !route_policies.insert(policy.as_str()) {
                return Err(HarnessError::InvalidRepository(format!(
                    "company domain route repeats a policy: {policy}"
                )));
            }
            let capability = company_domain_policy_capability(company, policy.clone());
            let (id, _) = capability.binding_location();
            if let Some(existing) = policy_ids.insert(id.to_owned(), policy.clone())
                && existing != *policy
            {
                return Err(HarnessError::InvalidRepository(format!(
                    "company domain route policies produce the same policy ID: {existing}, {policy}"
                )));
            }
        }
        for signal in &route.signals {
            validate_single_line("company domain route signal", signal, MAX_COMPANY_LENGTH)?;
            if signal.trim() != signal || signal.is_empty() {
                return Err(HarnessError::InvalidRepository(
                    "company domain route signals must be non-empty and trimmed".to_owned(),
                ));
            }
            if !normalized_signals.insert(signal.to_lowercase()) {
                return Err(HarnessError::InvalidRepository(format!(
                    "company domain route signal is duplicated: {signal}"
                )));
            }
        }
    }
    Ok(metadata.domain_routes)
}

fn contains_company_route_signal(searchable: &str, signal: &str) -> bool {
    let normalized = signal.to_lowercase();
    if !normalized.is_ascii() {
        return searchable.contains(&normalized);
    }
    searchable
        .match_indices(&normalized)
        .any(|(start, matched)| {
            let before = searchable[..start].chars().next_back();
            let after = searchable[start + matched.len()..].chars().next();
            before.is_none_or(|value| !value.is_ascii_alphanumeric())
                && after.is_none_or(|value| !value.is_ascii_alphanumeric())
        })
}

#[derive(Clone, Debug)]
pub struct HarnessEngine {
    router: HarnessRouter,
    vault: VaultRepository,
    workspace_root: PathBuf,
}

fn invalid_missing_context_candidate(error: HarnessError) -> HarnessError {
    let message = error.to_string();
    drop(error);
    HarnessError::InvalidSubmission(format!("missing context candidate is invalid: {message}"))
}

impl HarnessEngine {
    fn workspace_policy(&self) -> HarnessResult<repository::WorkspacePolicy> {
        if let Some(identity) = &self.vault.store_identity {
            identity.validate()?;
            // Both canonical roots are bound when the engine is constructed and
            // revalidated by the durable apply/recovery contract before mutation.
            let source = canonical_directory(&self.vault.root, "stored source root")?;
            let workspace = canonical_directory(&self.workspace_root, "workspace root")?;
            if source == workspace {
                return Ok(repository::WorkspacePolicy::NativePrivate);
            }
        }
        Ok(repository::WorkspacePolicy::External)
    }

    pub(super) fn with_workspace_finalization_lock<T>(
        &self,
        operation: impl FnOnce() -> HarnessResult<T>,
    ) -> HarnessResult<T> {
        with_reported_workspace_lock(
            self.workspace_policy()?,
            &self.workspace_root,
            "current-finalization",
            operation,
        )
    }

    pub fn open(
        vault_repository_root: impl AsRef<Path>,
        workspace_root: impl AsRef<Path>,
    ) -> HarnessResult<Self> {
        #[cfg(unix)]
        {
            Ok(Self {
                router: HarnessRouter::default(),
                vault: VaultRepository::open(vault_repository_root)?,
                workspace_root: canonical_directory(workspace_root.as_ref(), "workspace root")?,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = (vault_repository_root, workspace_root);
            Err(HarnessError::InvalidRepository(
                "the Harness file-safety implementation currently supports Unix platforms only"
                    .to_owned(),
            ))
        }
    }

    pub fn with_router(
        vault_repository_root: impl AsRef<Path>,
        workspace_root: impl AsRef<Path>,
        router: HarnessRouter,
    ) -> HarnessResult<Self> {
        #[cfg(unix)]
        {
            Ok(Self {
                router,
                vault: VaultRepository::open(vault_repository_root)?,
                workspace_root: canonical_directory(workspace_root.as_ref(), "workspace root")?,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = (vault_repository_root, workspace_root, router);
            Err(HarnessError::InvalidRepository(
                "the Harness file-safety implementation currently supports Unix platforms only"
                    .to_owned(),
            ))
        }
    }

    #[cfg(test)]
    pub fn resolve(&self, request: &HarnessRequest) -> HarnessResult<ResolvedHarnessRequest> {
        self.resolve_with_intent_and_career_composition(
            request,
            HarnessIntent::General,
            &[],
            &[],
            None,
        )
    }

    #[cfg(test)]
    pub fn resolve_with_intent(
        &self,
        request: &HarnessRequest,
        intent: HarnessIntent,
    ) -> HarnessResult<ResolvedHarnessRequest> {
        self.resolve_with_intent_and_career_composition(request, intent, &[], &[], None)
    }

    #[cfg(test)]
    pub fn resolve_with_context_grants(
        &self,
        request: &HarnessRequest,
        context_grants: &[ContextGrant],
    ) -> HarnessResult<ResolvedHarnessRequest> {
        let intent = if context_grants.is_empty() {
            HarnessIntent::General
        } else {
            HarnessIntent::CareerArtifact {
                surface: CareerOutputSurface::General,
            }
        };
        self.resolve_with_intent_and_career_composition(request, intent, context_grants, &[], None)
    }

    #[cfg(test)]
    pub fn resolve_with_career_surface(
        &self,
        request: &HarnessRequest,
        career_output_surface: CareerOutputSurface,
    ) -> HarnessResult<ResolvedHarnessRequest> {
        self.resolve_with_intent_and_career_composition(
            request,
            HarnessIntent::CareerArtifact {
                surface: career_output_surface,
            },
            &[],
            &[],
            None,
        )
    }

    #[cfg(test)]
    pub fn resolve_with_context_grants_and_career_surface(
        &self,
        request: &HarnessRequest,
        context_grants: &[ContextGrant],
        career_output_surface: Option<CareerOutputSurface>,
    ) -> HarnessResult<ResolvedHarnessRequest> {
        let intent = career_output_surface.map_or_else(
            || {
                if context_grants.is_empty() {
                    HarnessIntent::General
                } else {
                    HarnessIntent::CareerArtifact {
                        surface: CareerOutputSurface::General,
                    }
                }
            },
            |surface| HarnessIntent::CareerArtifact { surface },
        );
        self.resolve_with_intent_and_career_composition(request, intent, context_grants, &[], None)
    }

    #[cfg(test)]
    pub fn resolve_with_career_composition(
        &self,
        request: &HarnessRequest,
        context_grants: &[ContextGrant],
        career_output_surface: Option<CareerOutputSurface>,
        evidence_source_paths: &[String],
        career_composition_manifest: Option<&CareerCompositionManifest>,
    ) -> HarnessResult<ResolvedHarnessRequest> {
        let intent = career_output_surface
            .map(|surface| HarnessIntent::CareerArtifact { surface })
            .or_else(|| {
                (!context_grants.is_empty()).then_some(HarnessIntent::CareerArtifact {
                    surface: CareerOutputSurface::General,
                })
            })
            .unwrap_or(HarnessIntent::General);
        self.resolve_with_intent_and_career_composition(
            request,
            intent,
            context_grants,
            evidence_source_paths,
            career_composition_manifest,
        )
    }

    #[cfg(test)]
    pub fn resolve_with_intent_and_career_composition(
        &self,
        request: &HarnessRequest,
        intent: HarnessIntent,
        context_grants: &[ContextGrant],
        evidence_source_paths: &[String],
        career_composition_manifest: Option<&CareerCompositionManifest>,
    ) -> HarnessResult<ResolvedHarnessRequest> {
        let mut request = request.clone();
        let mut context_grants = context_grants.to_vec();
        let mut evidence_source_paths = evidence_source_paths.to_vec();
        request.targets.sort();
        request.delete_targets.sort();
        request.curation_sources.sort();
        context_grants.sort();
        evidence_source_paths.sort();
        let envelope = RequestEnvelope::from_structured_request(
            request,
            intent,
            context_grants,
            evidence_source_paths,
            self.router.execution_profile,
            "non-executable structured Harness API",
        )?;
        self.resolve_test_envelope(&envelope, career_composition_manifest)
    }

    #[cfg(test)]
    fn resolve_test_envelope(
        &self,
        envelope: &RequestEnvelope,
        career_composition_manifest: Option<&CareerCompositionManifest>,
    ) -> HarnessResult<ResolvedHarnessRequest> {
        let (contract, decision_trace, request_provenance) = envelope.resolve_test_fixture()?;
        self.resolve_contract(
            contract,
            decision_trace,
            request_provenance,
            career_composition_manifest,
        )
    }

    pub fn resolve_envelope(
        &self,
        envelope: &RequestEnvelope,
        career_composition_manifest: Option<&CareerCompositionManifest>,
    ) -> HarnessResult<ResolvedHarnessRequest> {
        let (contract, decision_trace, request_provenance) = envelope.resolve()?;
        self.resolve_contract(
            contract,
            decision_trace,
            request_provenance,
            career_composition_manifest,
        )
    }

    fn resolve_contract(
        &self,
        contract: ResolvedTaskContract,
        decision_trace: DecisionTrace,
        request_provenance: RequestProvenanceBinding,
        career_composition_manifest: Option<&CareerCompositionManifest>,
    ) -> HarnessResult<ResolvedHarnessRequest> {
        if contract.execution_profile() != self.router.execution_profile {
            return Err(HarnessError::InvalidRequest(
                "resolved task contract execution profile does not match the Harness router"
                    .to_owned(),
            ));
        }
        let mut request = contract.to_harness_request();
        let intent = contract.intent();
        let mut context_grants = contract.context_grants().to_vec();
        let mut evidence_source_paths = contract.evidence_source_paths().to_vec();
        let career_composition_manifest =
            career_composition_manifest.map(CareerCompositionManifest::normalized);
        let career_output_surface = intent.career_surface();
        request.targets.sort();
        request.delete_targets.sort();
        request.curation_sources.sort();
        context_grants.sort();
        evidence_source_paths.sort();
        request.validate()?;
        validate_context_grants(&request, &context_grants)?;
        validate_intent(&request, intent, &context_grants)?;
        if career_composition_manifest.is_some() && !intent.is_career_artifact() {
            return Err(HarnessError::InvalidRequest(
                "career composition manifests require career-artifact intent".to_owned(),
            ));
        }
        self.validate_profile_source_intent(&request, intent)?;
        if let Some(manifest) = career_composition_manifest.as_ref() {
            manifest.validate(&request, career_output_surface)?;
            match context_grants.as_slice() {
                [grant]
                    if manifest.evidence_owners.contains(&grant.owner)
                        && !evidence_source_paths.is_empty()
                        && manifest.evidence_source_paths_for(&grant.owner)
                            == evidence_source_paths => {}
                [] if evidence_source_paths.is_empty() => {}
                _ => {
                    return Err(HarnessError::InvalidRequest(
                        "career review evidence sources must exactly match the manifest lineage for its selected owner"
                            .to_owned(),
                    ));
                }
            }
        }
        if request.action == HarnessAction::VaultCuration {
            validate_curation_targets(
                &request.owner,
                request.curation_kind,
                &request.targets,
                &self.workspace_root,
                &self.vault.root,
            )?;
        }
        self.vault.validate_vault_targets(
            &request,
            &context_grants,
            &self.workspace_root,
            intent,
        )?;
        let targets = request
            .targets
            .iter()
            .map(|target| {
                if self.workspace_root == self.vault.root && target.starts_with("vault/") {
                    let relative = Path::new(target);
                    let metadata =
                        if request.action.is_write() && self.vault.store_identity.is_some() {
                            self.vault.prepare_file_target(relative)?
                        } else {
                            self.vault.source_metadata(relative)?
                        };
                    if metadata.exists() {
                        self.vault.open_scoped_file(relative, MAX_TARGET_BYTES)?;
                    }
                }
                bind_target(
                    &self.workspace_root,
                    target,
                    request.action,
                    request.delete_targets.binary_search(target).is_ok(),
                )
            })
            .collect::<HarnessResult<Vec<_>>>()?;
        let workspace_policies = self.bind_workspace_policies(&request.targets)?;
        let mut plan = self.router.build_plan(HarnessPlanInputs {
            request: &request,
            contract: &contract,
            decision_trace: &decision_trace,
            request_provenance: &request_provenance,
            intent,
            context_grants: &context_grants,
            career_composition_manifest: career_composition_manifest.as_ref(),
            evidence_source_paths: &evidence_source_paths,
            targets,
            workspace_policies,
            vault: &self.vault,
        })?;
        plan.source_versions = self.plan_source_versions(&plan)?;
        plan.validate()?;
        decision_trace.validate_policy_bindings(&plan.required_policies)?;
        let resolved_plan_digest = serialized_digest(&plan)?;
        Ok(ResolvedHarnessRequest {
            request,
            contract,
            decision_trace,
            request_provenance,
            intent,
            context_grants,
            career_composition_manifest,
            evidence_source_paths,
            resolved_plan_digest,
            plan,
        })
    }

    fn validate_profile_source_intent(
        &self,
        request: &HarnessRequest,
        intent: HarnessIntent,
    ) -> HarnessResult<()> {
        let vault_workspace = self.workspace_root == self.vault.root;
        if intent.is_output_adapter() && vault_workspace {
            return Err(HarnessError::InvalidRequest(
                "output-adapter intent requires an external workspace".to_owned(),
            ));
        }
        if request.owner != DataOwner::Profile || !is_profile_source_action(request.action) {
            return Ok(());
        }
        if !vault_workspace {
            return if intent.is_output_adapter()
                || (intent == HarnessIntent::PolicyMaintenance
                    && self.vault.store_identity.is_some()
                    && !request.targets.is_empty()
                    && request
                        .targets
                        .iter()
                        .all(|target| path_is_within(target, "crates/context-core")))
            {
                Ok(())
            } else {
                Err(HarnessError::InvalidRequest(
                    "external profile source work requires explicit output-adapter intent"
                        .to_owned(),
                ))
            };
        }
        let profile_source_targets = request
            .targets
            .iter()
            .filter(|target| path_is_within(target, "vault/profile"))
            .collect::<Vec<_>>();
        if profile_source_targets.is_empty() {
            return Ok(());
        }
        if intent != HarnessIntent::PolicyMaintenance {
            return Err(HarnessError::InvalidRequest(
                "canonical profile source work requires policy-maintenance intent".to_owned(),
            ));
        }
        if profile_source_targets
            .iter()
            .any(|target| !is_profile_policy_source_target(target))
        {
            return Err(HarnessError::InvalidRequest(
                "policy-maintenance targets must stay in canonical profile policy sources"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    fn revalidate_resolved(
        &self,
        resolved: &ResolvedHarnessRequest,
    ) -> HarnessResult<ResolvedHarnessRequest> {
        resolved.plan.require_executable_version()?;
        let supplied_plan_digest = serialized_digest(&resolved.plan)?;
        if supplied_plan_digest != resolved.resolved_plan_digest {
            return Err(HarnessError::InvalidPlan(
                "resolved plan content does not match its resolved-plan digest".to_owned(),
            ));
        }
        let current = self.resolve_contract(
            resolved.contract.clone(),
            resolved.decision_trace.clone(),
            resolved.request_provenance.clone(),
            resolved.career_composition_manifest.as_ref(),
        )?;
        if &current != resolved {
            return Err(HarnessError::PlanDrift {
                expected: resolved.resolved_plan_digest.clone(),
                actual: current.resolved_plan_digest,
            });
        }
        Ok(current)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "runtime preparation orchestration and bundle digest ordering must remain auditable together"
    )]
    pub fn prepare(
        &self,
        resolved: &ResolvedHarnessRequest,
        capabilities: &RoleRuntimeCapabilities,
        context_bundle: Option<&HarnessContextBundle>,
    ) -> HarnessResult<PreparedRoleRun> {
        self.revalidate_resolved(resolved)?;
        validate_role_runtime_capabilities(&resolved.plan, capabilities)?;
        let context_bundle = self.validate_prepared_context(resolved, context_bundle)?;
        let context_bundle_digest = context_bundle
            .as_ref()
            .map(|bundle| bundle.bundle_digest.clone());
        let career_claim_lineage = career_claim_lineage_for_role(resolved);
        let role_bundles = self.build_role_bundles(
            resolved,
            context_bundle.as_ref(),
            capabilities.max_role_bundle_bytes,
            &career_claim_lineage,
        )?;
        let independent_context_required = resolved.plan.role_count() > 1;
        let role_metadata = resolved
            .plan
            .workflow
            .iter()
            .enumerate()
            .map(|(execution_order, node)| {
                let role = node.role;
                let role_bundle = role_bundles.get(execution_order).ok_or_else(|| {
                    HarnessError::InvalidPlan("prepared role bundle order is incomplete".to_owned())
                })?;
                let required_policies = resolved
                    .plan
                    .role_policy_bindings
                    .iter()
                    .find(|binding| binding.role == role)
                    .map(|binding| binding.policies.clone())
                    .ok_or_else(|| {
                        HarnessError::InvalidPlan(format!(
                            "role {role:?} has no prepared policy binding"
                        ))
                    })?;
                let scope = RoleTaskScope {
                    action: resolved.plan.action,
                    owner: resolved.plan.owner.clone(),
                    intent: resolved.plan.intent,
                    context_grants: resolved.plan.context_grants.clone(),
                    career_manifest_digest: resolved.plan.career_manifest_digest.clone(),
                    career_claim_lineage: career_claim_lineage.clone(),
                    evidence_sources: resolved.plan.evidence_sources.clone(),
                    learning_sources: if role == resolved.plan.primary_producer_role {
                        resolved.plan.learning_sources.clone()
                    } else {
                        Vec::new()
                    },
                    task_statement: resolved.request.objective.clone(),
                    allowed_context_roots: resolved.plan.allowed_context_roots.clone(),
                    denied_context_roots: resolved.plan.denied_context_roots.clone(),
                    retrieval_roots: resolved.plan.retrieval_roots.clone(),
                    context_bundle_digest: context_bundle_digest.clone(),
                    promotion_handoff: resolved.plan.promotion_handoff.clone(),
                };
                let task = role_task_contract(&resolved.plan, role);
                Ok(PreparedRoleMetadata {
                    resolved_plan_digest: resolved.resolved_plan_digest.clone(),
                    contract_digest: resolved.plan.contract_digest.clone(),
                    decision_trace_digest: resolved.plan.decision_trace_digest.clone(),
                    request_provenance: resolved.plan.request_provenance.clone(),
                    execution_order,
                    role,
                    independent_context_required,
                    role_bundle_digest: role_bundle.bundle_digest.clone(),
                    required_policies,
                    workspace_policies: resolved.plan.workspace_policies.clone(),
                    scope,
                    task,
                })
            })
            .collect::<HarnessResult<Vec<_>>>()?;
        let source_versions =
            self.prepared_source_versions(resolved, context_bundle.as_ref(), &role_bundles)?;
        let runtime_capabilities_digest = serialized_digest(capabilities)?;
        let prepared_role_run_digest = serialized_digest(&(
            &resolved.resolved_plan_digest,
            &runtime_capabilities_digest,
            &source_versions,
            &context_bundle,
            &role_bundles,
            &role_metadata,
        ))?;
        Ok(PreparedRoleRun {
            source_versions,
            resolved_plan_digest: resolved.resolved_plan_digest.clone(),
            runtime_capabilities_digest,
            prepared_role_run_digest,
            assurance: resolved.plan.minimum_assurance,
            runtime_capabilities: capabilities.clone(),
            context_bundle,
            role_bundles,
            role_metadata,
        })
    }

    fn build_role_bundles(
        &self,
        resolved: &ResolvedHarnessRequest,
        context_bundle: Option<&HarnessContextBundle>,
        max_role_bundle_bytes: usize,
        career_claim_lineage: &[CareerClaimLineage],
    ) -> HarnessResult<Vec<HarnessRoleBundle>> {
        resolved
            .plan
            .workflow
            .iter()
            .map(|node| {
                self.build_role_bundle(
                    resolved,
                    context_bundle,
                    max_role_bundle_bytes,
                    career_claim_lineage,
                    node.role,
                )
            })
            .collect()
    }

    fn build_role_bundle(
        &self,
        resolved: &ResolvedHarnessRequest,
        context_bundle: Option<&HarnessContextBundle>,
        max_role_bundle_bytes: usize,
        career_claim_lineage: &[CareerClaimLineage],
        role: HarnessRole,
    ) -> HarnessResult<HarnessRoleBundle> {
        let role_policies = resolved
            .plan
            .role_policy_bindings
            .iter()
            .find(|binding| binding.role == role)
            .ok_or_else(|| {
                HarnessError::InvalidPlan(format!("role {role:?} has no policy bundle"))
            })?;
        let task = role_task_contract(&resolved.plan, role);
        let (control_head, control_tail) =
            role_control_segments(resolved, career_claim_lineage, role, &task)?;
        let mut documents = self.collect_mandatory_role_documents(
            resolved,
            role_policies,
            max_role_bundle_bytes,
            role,
        )?;
        documents.sort_by(|left, right| {
            left.source
                .cmp(&right.source)
                .then_with(|| left.relative_path.cmp(&right.relative_path))
        });
        finalize_role_bundle(
            &resolved.resolved_plan_digest,
            role,
            control_head.clone(),
            documents.clone(),
            control_tail.clone(),
            max_role_bundle_bytes,
        )?;
        if let Some(bundle) = context_bundle {
            let mut selected_retrieval = 0_usize;
            let mut covered_retrieval = 0_usize;
            for document in &bundle.documents {
                let bound = HarnessBoundDocument {
                    source: HarnessBoundDocumentSource::Retrieval,
                    relative_path: document.repository_relative_path.clone(),
                    content_digest: document.content_digest.clone(),
                    content: document.content.clone(),
                    start_line: document.start_line,
                    end_line: document.end_line,
                    matched_terms: document.matched_terms.clone(),
                };
                if contains_bound_document(
                    &documents,
                    bound.source,
                    &bound.relative_path,
                    &bound.content_digest,
                    bound.start_line,
                    bound.end_line,
                ) {
                    covered_retrieval += 1;
                    continue;
                }
                let mut candidate = documents.clone();
                candidate.push(bound);
                match finalize_role_bundle(
                    &resolved.resolved_plan_digest,
                    role,
                    control_head.clone(),
                    candidate.clone(),
                    control_tail.clone(),
                    max_role_bundle_bytes,
                ) {
                    Ok(_) => {
                        documents = candidate;
                        selected_retrieval += 1;
                    }
                    Err(HarnessError::UnsupportedRuntime(_)) => {}
                    Err(error) => return Err(error),
                }
            }
            if selected_retrieval == 0 && covered_retrieval == 0 {
                return Err(HarnessError::UnsupportedRuntime(format!(
                    "role {role:?} has matched retrieval sections, but none fit the remaining role bundle budget"
                )));
            }
        }
        finalize_role_bundle(
            &resolved.resolved_plan_digest,
            role,
            control_head,
            documents,
            control_tail,
            max_role_bundle_bytes,
        )
    }

    fn collect_mandatory_role_documents(
        &self,
        resolved: &ResolvedHarnessRequest,
        role_policies: &RolePolicyBinding,
        max_content_bytes: usize,
        role: HarnessRole,
    ) -> HarnessResult<Vec<HarnessBoundDocument>> {
        let mut documents = Vec::new();
        let mut remaining_content_bytes = max_content_bytes;
        for target in &resolved.plan.targets {
            if let TargetState::Existing { content_digest } = &target.state {
                self.push_role_document(
                    &mut documents,
                    &mut remaining_content_bytes,
                    &target.workspace_relative_path,
                    content_digest,
                    MAX_TARGET_BYTES,
                    HarnessBoundDocumentSource::Target,
                )?;
            }
        }
        for policy in &role_policies.policies {
            self.push_role_document(
                &mut documents,
                &mut remaining_content_bytes,
                &policy.repository_relative_path,
                &policy.content_digest,
                MAX_POLICY_BYTES,
                HarnessBoundDocumentSource::Policy,
            )?;
        }
        for policy in &resolved.plan.workspace_policies {
            self.push_role_document(
                &mut documents,
                &mut remaining_content_bytes,
                &policy.workspace_relative_path,
                &policy.content_digest,
                MAX_POLICY_BYTES,
                HarnessBoundDocumentSource::WorkspacePolicy,
            )?;
        }
        if role == resolved.plan.primary_producer_role {
            for source in &resolved.plan.learning_sources {
                self.push_learning_document(&mut documents, &mut remaining_content_bytes, source)?;
            }
        }
        for source in &resolved.plan.evidence_sources {
            self.push_role_document(
                &mut documents,
                &mut remaining_content_bytes,
                &source.repository_relative_path,
                &source.content_digest,
                MAX_POLICY_BYTES,
                HarnessBoundDocumentSource::Evidence,
            )?;
        }
        for source in &resolved.plan.curation_sources {
            self.push_role_document(
                &mut documents,
                &mut remaining_content_bytes,
                &source.repository_relative_path,
                &source.content_digest,
                MAX_POLICY_BYTES,
                HarnessBoundDocumentSource::Curation,
            )?;
        }
        Ok(documents)
    }

    fn push_learning_document(
        &self,
        documents: &mut Vec<HarnessBoundDocument>,
        remaining_content_bytes: &mut usize,
        source: &LearningSourceBinding,
    ) -> HarnessResult<()> {
        let relative = Path::new(&source.repository_relative_path);
        let path = self.vault.root.join(relative);
        let file = self
            .vault
            .open_scoped_file(relative, MAX_LEARNING_FILE_BYTES)?;
        let document = read_bound_document_from_file(
            file,
            &path,
            &source.repository_relative_path,
            &source.content_digest,
            MAX_LEARNING_FILE_BYTES,
            *remaining_content_bytes,
            HarnessBoundDocumentSource::Learning,
        )?;
        let frontmatter =
            parse_learning_document(&document.content, relative)?.ok_or_else(|| {
                HarnessError::PlanDrift {
                    expected: source.content_digest.clone(),
                    actual: "learning-metadata-removed".to_owned(),
                }
            })?;
        if frontmatter.learning != source.metadata {
            return Err(HarnessError::PlanDrift {
                expected: source.content_digest.clone(),
                actual: document.content_digest,
            });
        }
        *remaining_content_bytes = remaining_content_bytes
            .checked_sub(document.content.len())
            .ok_or_else(|| {
                HarnessError::UnsupportedRuntime(
                    "mandatory role bundle content size overflow".to_owned(),
                )
            })?;
        documents.push(document);
        Ok(())
    }

    fn validate_prepared_run(
        &self,
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
    ) -> HarnessResult<()> {
        let expected = self.prepare(
            resolved,
            &prepared.runtime_capabilities,
            prepared.context_bundle.as_ref(),
        )?;
        if &expected != prepared {
            return Err(HarnessError::InvalidSubmission(
                "prepared execution receipt is invalid".to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_prepared_context(
        &self,
        resolved: &ResolvedHarnessRequest,
        context_bundle: Option<&HarnessContextBundle>,
    ) -> HarnessResult<Option<HarnessContextBundle>> {
        match (resolved.plan.action, context_bundle) {
            (HarnessAction::VaultRead, Some(bundle)) => {
                if bundle.resolved_plan_digest != resolved.resolved_plan_digest {
                    return Err(HarnessError::InvalidRequest(
                        "context bundle resolved-plan digest does not match the resolved plan"
                            .to_owned(),
                    ));
                }
                let expected = self.retrieve_context(resolved, &bundle.query, bundle.max_bytes)?;
                if &expected != bundle {
                    return Err(HarnessError::InvalidRequest(
                        "context bundle does not match the bounded Vault retrieval".to_owned(),
                    ));
                }
                Ok(Some(bundle.clone()))
            }
            (HarnessAction::VaultRead, None) => Err(HarnessError::InvalidRequest(
                "Vault read preparation requires a bounded context bundle".to_owned(),
            )),
            (_, Some(_)) => Err(HarnessError::InvalidRequest(
                "context bundles are only valid for Vault read preparation".to_owned(),
            )),
            (_, None) => Ok(None),
        }
    }

    fn bind_workspace_policies(
        &self,
        targets: &[String],
    ) -> HarnessResult<Vec<WorkspacePolicyBinding>> {
        let mut candidates = BTreeSet::from([PathBuf::from("AGENTS.md")]);
        for target in targets {
            let mut parent = Path::new(target).parent();
            while let Some(directory) = parent {
                if directory.as_os_str().is_empty() {
                    break;
                }
                candidates.insert(directory.join("AGENTS.md"));
                parent = directory.parent();
            }
        }
        let mut policies = Vec::new();
        for relative in candidates {
            let path = self.workspace_root.join(&relative);
            let relative_path = portable_path(&relative);
            let source_document = self
                .uses_source_document(HarnessBoundDocumentSource::WorkspacePolicy, &relative_path);
            let exists = if source_document {
                self.vault.source_metadata(&relative)?.is_file()
            } else {
                path.is_file()
            };
            if !exists {
                continue;
            }
            let file = if source_document {
                self.vault.open_scoped_file(&relative, MAX_POLICY_BYTES)?
            } else {
                let file = open_verified_file(&self.workspace_root, &relative)?;
                ensure_single_hard_link(&file, &path)?;
                file
            };
            policies.push(WorkspacePolicyBinding {
                workspace_relative_path: relative_path,
                content_digest: digest_file(file, MAX_POLICY_BYTES, &path)?,
            });
        }
        Ok(policies)
    }

    pub fn retrieve_context(
        &self,
        resolved: &ResolvedHarnessRequest,
        query: &str,
        max_bytes: usize,
    ) -> HarnessResult<HarnessContextBundle> {
        resolved.plan.require_executable_version()?;
        if resolved.plan.action != HarnessAction::VaultRead {
            return Err(HarnessError::InvalidRequest(
                "context retrieval requires the vault-read action".to_owned(),
            ));
        }
        validate_single_line("retrieval query", query, MAX_RETRIEVAL_QUERY_LENGTH)?;
        if max_bytes == 0 || max_bytes > MAX_RETRIEVAL_BYTES {
            return Err(HarnessError::InvalidRequest(format!(
                "retrieval byte budget must be between 1 and {MAX_RETRIEVAL_BYTES}"
            )));
        }
        self.revalidate_resolved(resolved)?;
        let documents = self.vault.retrieve_context_documents(
            &resolved.plan.retrieval_roots,
            query,
            max_bytes,
        )?;
        let total_content_bytes = documents.iter().try_fold(0_usize, |total, document| {
            total.checked_add(document.content.len()).ok_or_else(|| {
                HarnessError::InvalidRepository("retrieval result size overflow".to_owned())
            })
        })?;
        let bundle_digest = serialized_digest(&(
            &resolved.resolved_plan_digest,
            query,
            max_bytes,
            &documents,
            total_content_bytes,
        ))?;
        Ok(HarnessContextBundle {
            resolved_plan_digest: resolved.resolved_plan_digest.clone(),
            query: query.to_owned(),
            max_bytes,
            documents,
            total_content_bytes,
            bundle_digest,
        })
    }

    #[cfg(test)]
    pub fn validate_submission(
        &self,
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
        submission: &HarnessSubmission,
    ) -> HarnessResult<HarnessEvaluationResult> {
        self.validate_submission_with_history(resolved, prepared, submission, &[])
    }

    pub fn begin_execution(
        &self,
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
        revision_history: &[HarnessTaskEvaluation],
    ) -> HarnessResult<HarnessExecutionStep> {
        self.revalidate_resolved(resolved)?;
        self.validate_prepared_run(resolved, prepared)?;
        begin_execution_record(&resolved.plan, prepared, revision_history)
    }

    pub fn advance_execution(
        &self,
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
        revision_history: &[HarnessTaskEvaluation],
        record: &RoleExecutionRecord,
        event: HarnessExecutionEvent,
    ) -> HarnessResult<HarnessExecutionStep> {
        self.revalidate_resolved(resolved)?;
        self.validate_prepared_run(resolved, prepared)?;
        let missing_context = match &event {
            HarnessExecutionEvent::RoleResult { result } => match &result.outcome {
                RoleExecutionOutcome::MissingContext { request } => Some(request.clone()),
                RoleExecutionOutcome::Completed { .. }
                | RoleExecutionOutcome::Failed { .. }
                | RoleExecutionOutcome::Cancelled
                | RoleExecutionOutcome::TimedOut
                | RoleExecutionOutcome::Unsupported { .. } => None,
            },
            HarnessExecutionEvent::ToolEvidence { .. } => None,
        };
        let step =
            advance_execution_record(&resolved.plan, prepared, revision_history, record, event)?;
        if let Some(request) = &missing_context {
            self.validate_missing_context_candidate(resolved, prepared, request)?;
        }
        Ok(step)
    }

    fn validate_missing_context_candidate(
        &self,
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
        request: &MissingContextRequest,
    ) -> HarnessResult<()> {
        let prevalidation = match &request.candidate {
            MissingContextCandidate::AdditionalWorkspaceTarget {
                workspace_relative_path,
            } => self.prevalidate_additional_workspace_target(resolved, workspace_relative_path),
            MissingContextCandidate::VaultEvidence {
                repository_relative_path,
            } => self.prevalidate_additional_vault_evidence(resolved, repository_relative_path),
        };
        prevalidation.map_err(invalid_missing_context_candidate)?;
        let (candidate_file, initial_metadata) = self
            .open_missing_context_candidate(&request.candidate)
            .map_err(invalid_missing_context_candidate)?;
        if self
            .missing_context_candidate_is_bound(prepared, &initial_metadata)
            .map_err(invalid_missing_context_candidate)?
        {
            return Err(HarnessError::InvalidSubmission(
                "missing context candidate is already bound to the current run".to_owned(),
            ));
        }
        self.validate_open_missing_context_candidate(&request.candidate, candidate_file)
            .map_err(invalid_missing_context_candidate)?;
        let final_metadata = self
            .open_missing_context_candidate(&request.candidate)
            .map(|(_, metadata)| metadata)
            .map_err(invalid_missing_context_candidate)?;
        if !repository::same_file(&initial_metadata, &final_metadata) {
            return Err(HarnessError::InvalidSubmission(
                "missing context candidate path changed during validation".to_owned(),
            ));
        }
        if self
            .missing_context_candidate_is_bound(prepared, &final_metadata)
            .map_err(invalid_missing_context_candidate)?
        {
            return Err(HarnessError::InvalidSubmission(
                "missing context candidate is already bound to the current run".to_owned(),
            ));
        }
        Ok(())
    }

    fn open_missing_context_candidate(
        &self,
        candidate: &MissingContextCandidate,
    ) -> HarnessResult<(File, Metadata)> {
        let (candidate_root, candidate_file) = match candidate {
            MissingContextCandidate::AdditionalWorkspaceTarget { .. } => {
                let file = if self.workspace_root == self.vault.root {
                    self.vault
                        .open_scoped_file(Path::new(candidate.relative_path()), MAX_TARGET_BYTES)?
                } else {
                    let file = open_verified_file(
                        &self.workspace_root,
                        Path::new(candidate.relative_path()),
                    )?;
                    ensure_single_hard_link(
                        &file,
                        &self.workspace_root.join(candidate.relative_path()),
                    )?;
                    file
                };
                (&self.workspace_root, file)
            }
            MissingContextCandidate::VaultEvidence { .. } => (
                &self.vault.root,
                self.vault
                    .open_scoped_file(Path::new(candidate.relative_path()), MAX_POLICY_BYTES)?,
            ),
        };
        let metadata = candidate_file
            .metadata()
            .map_err(|source| HarnessError::FileRead {
                path: candidate_root.join(candidate.relative_path()),
                message: source.to_string(),
            })?;
        Ok((candidate_file, metadata))
    }

    fn validate_open_missing_context_candidate(
        &self,
        candidate: &MissingContextCandidate,
        candidate_file: File,
    ) -> HarnessResult<()> {
        let (candidate_root, max_bytes) = match candidate {
            MissingContextCandidate::AdditionalWorkspaceTarget { .. } => {
                (&self.workspace_root, MAX_TARGET_BYTES)
            }
            MissingContextCandidate::VaultEvidence { .. } => (&self.vault.root, MAX_POLICY_BYTES),
        };
        let path = candidate_root.join(candidate.relative_path());
        digest_file(candidate_file, max_bytes, &path)?;
        Ok(())
    }

    fn missing_context_candidate_is_bound(
        &self,
        prepared: &PreparedRoleRun,
        candidate_metadata: &Metadata,
    ) -> HarnessResult<bool> {
        for bundle in &prepared.role_bundles {
            for document in bundle.bound_documents() {
                let document_root = match document.source {
                    HarnessBoundDocumentSource::Target
                    | HarnessBoundDocumentSource::WorkspacePolicy => &self.workspace_root,
                    HarnessBoundDocumentSource::Policy
                    | HarnessBoundDocumentSource::Learning
                    | HarnessBoundDocumentSource::Evidence
                    | HarnessBoundDocumentSource::Curation
                    | HarnessBoundDocumentSource::Retrieval => &self.vault.root,
                };
                let document_file = if self
                    .uses_source_document(document.source, &document.relative_path)
                {
                    self.vault
                        .open_scoped_file(Path::new(&document.relative_path), MAX_TARGET_BYTES)?
                } else {
                    open_verified_file(document_root, Path::new(&document.relative_path))?
                };
                let document_metadata =
                    document_file
                        .metadata()
                        .map_err(|source| HarnessError::FileRead {
                            path: document_root.join(&document.relative_path),
                            message: source.to_string(),
                        })?;
                if repository::same_file(candidate_metadata, &document_metadata) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn prevalidate_additional_workspace_target(
        &self,
        resolved: &ResolvedHarnessRequest,
        workspace_relative_path: &str,
    ) -> HarnessResult<()> {
        if matches!(
            resolved.plan.action,
            HarnessAction::VaultRead | HarnessAction::VaultCuration
        ) {
            return Err(HarnessError::InvalidRequest(
                "this action cannot add a workspace target on rerun".to_owned(),
            ));
        }
        repository::validate_target_path_is_not_reserved(workspace_relative_path)?;
        let mut rerun_request = resolved.request.clone();
        rerun_request
            .targets
            .push(workspace_relative_path.to_owned());
        rerun_request.targets.sort();
        rerun_request.validate()?;
        self.validate_profile_source_intent(&rerun_request, resolved.intent)?;
        self.vault.validate_vault_targets(
            &rerun_request,
            &resolved.plan.context_grants,
            &self.workspace_root,
            resolved.intent,
        )?;
        self.bind_workspace_policies(&rerun_request.targets)?;
        let roles = resolved.plan.roles().collect::<Vec<_>>();
        self.vault.required_policies(
            &rerun_request,
            resolved.intent,
            &resolved.plan.context_grants,
            &roles,
        )?;
        Ok(())
    }

    fn prevalidate_additional_vault_evidence(
        &self,
        resolved: &ResolvedHarnessRequest,
        repository_relative_path: &str,
    ) -> HarnessResult<()> {
        let denied =
            resolved.plan.denied_context_roots.iter().any(|root| {
                path_is_within(repository_relative_path, &root.repository_relative_path)
            });
        if denied {
            return Err(HarnessError::InvalidRequest(
                "Vault evidence path is denied by the current owner and grant scope".to_owned(),
            ));
        }
        let grant_roots = resolved
            .plan
            .context_grants
            .iter()
            .map(|grant| self.vault.context_grant_roots(grant))
            .collect::<HarnessResult<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let allowed =
            resolved.plan.allowed_context_roots.iter().any(|root| {
                path_is_within(repository_relative_path, &root.repository_relative_path)
            }) || grant_roots.iter().any(|root| {
                path_is_within(repository_relative_path, &root.repository_relative_path)
            });
        if !allowed {
            return Err(HarnessError::InvalidRequest(
                "Vault evidence path is outside the current owner and grant scope".to_owned(),
            ));
        }
        if resolved.plan.action == HarnessAction::VaultCuration {
            let mut rerun_request = resolved.request.clone();
            rerun_request
                .curation_sources
                .push(repository_relative_path.to_owned());
            rerun_request.curation_sources.sort();
            if rerun_request.curation_kind == Some(CurationKind::Ontology) {
                let ontology_root = curation_root(&rerun_request.owner, CurationKind::Ontology)?;
                if path_is_within(repository_relative_path, &ontology_root)
                    || rerun_request
                        .targets
                        .iter()
                        .any(|target| target == repository_relative_path)
                {
                    return Err(HarnessError::InvalidRequest(format!(
                        "ontology source `{repository_relative_path}` must be a canonical source outside the ontology target set"
                    )));
                }
            }
            rerun_request.validate()
        } else {
            let [grant] = resolved.plan.context_grants.as_slice() else {
                return Err(HarnessError::InvalidRequest(
                    "a Vault evidence candidate requires exactly one existing evidence grant so a fresh request can bind the exact file"
                        .to_owned(),
                ));
            };
            if !self.vault.context_grant_roots(grant)?.iter().any(|root| {
                path_is_within(repository_relative_path, &root.repository_relative_path)
            }) {
                return Err(HarnessError::InvalidRequest(
                    "Vault evidence candidate is outside the selected evidence owner".to_owned(),
                ));
            }
            let mut rerun_evidence_sources = resolved.evidence_source_paths.clone();
            rerun_evidence_sources.push(repository_relative_path.to_owned());
            rerun_evidence_sources.sort();
            validate_evidence_source_paths(&rerun_evidence_sources)
        }
    }

    pub fn evaluate_execution(
        &self,
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
        revision_history: &[HarnessTaskEvaluation],
        record: &RoleExecutionRecord,
    ) -> HarnessResult<HarnessTaskEvaluation> {
        self.revalidate_resolved(resolved)?;
        self.validate_prepared_run(resolved, prepared)?;
        let evaluation =
            evaluate_execution_record(&resolved.plan, prepared, revision_history, record)?;
        if let Some(request) = &evaluation.missing_context {
            self.validate_missing_context_candidate(resolved, prepared, request)?;
        }
        Ok(evaluation)
    }

    pub fn validate_execution(
        &self,
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
        revision_history: &[HarnessTaskEvaluation],
        record: &RoleExecutionRecord,
    ) -> HarnessResult<HarnessTaskEvaluation> {
        require_accepted_execution_evaluation(self.evaluate_execution(
            resolved,
            prepared,
            revision_history,
            record,
        )?)
    }

    pub fn apply_validated_execution(
        &self,
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
        revision_history: &[HarnessTaskEvaluation],
        record: &RoleExecutionRecord,
    ) -> HarnessResult<AppliedHarnessBatch> {
        self.apply_validated_execution_internal(
            resolved,
            prepared,
            revision_history,
            record,
            None,
            None,
        )
    }

    pub fn apply_validated_execution_for_attempt(
        &self,
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
        revision_history: &[HarnessTaskEvaluation],
        record: &RoleExecutionRecord,
        attempt_identifier: &str,
    ) -> HarnessResult<AppliedHarnessBatch> {
        self.apply_validated_execution_internal(
            resolved,
            prepared,
            revision_history,
            record,
            Some(attempt_identifier),
            None,
        )
    }

    fn apply_validated_execution_internal(
        &self,
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
        revision_history: &[HarnessTaskEvaluation],
        record: &RoleExecutionRecord,
        attempt_identifier: Option<&str>,
        native_permit: Option<&ContextApplyContract>,
    ) -> HarnessResult<AppliedHarnessBatch> {
        self.require_native_permit(resolved, native_permit)?;
        resolved.plan.require_executable_version()?;
        if !resolved.plan.source_write_allowed {
            return Err(HarnessError::InvalidRequest(
                "the resolved action does not permit source changes".to_owned(),
            ));
        }
        let evaluation = self.validate_execution(resolved, prepared, revision_history, record)?;
        let candidate_digest = evaluation.candidate_digest.clone().ok_or_else(|| {
            HarnessError::InvalidSubmission(
                "validated write execution omitted its candidate digest".to_owned(),
            )
        })?;
        let mut changes = evaluated_execution_changes(&evaluation)?;
        changes.sort_by(|left, right| left.path().cmp(right.path()));
        if changes.len() != resolved.plan.targets.len() {
            return Err(HarnessError::InvalidSubmission(
                "validated change count does not match the planned target count".to_owned(),
            ));
        }
        let inputs = resolved
            .plan
            .targets
            .iter()
            .zip(&changes)
            .map(|(target, change)| {
                if target.workspace_relative_path != change.path() {
                    return Err(HarnessError::InvalidSubmission(
                        "validated change order does not match the planned targets".to_owned(),
                    ));
                }
                Ok(repository::BatchApplyInput {
                    change,
                    planned_parent_directories: &target.parent_directories_to_create,
                    validate_vault_markdown: requires_vault_markdown_validation(
                        self.workspace_root == self.vault.root,
                        &target.workspace_relative_path,
                    ),
                })
            })
            .collect::<HarnessResult<Vec<_>>>()?;
        let lifecycle_receipt = self.apply_change_batch_with_lifecycle(
            resolved,
            &inputs,
            &candidate_digest,
            attempt_identifier,
        )?;
        self.confirm_execution_applied_batch(resolved, evaluation, lifecycle_receipt, true)
    }

    pub fn recover_validated_execution_apply(
        &self,
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
        revision_history: &[HarnessTaskEvaluation],
        record: &RoleExecutionRecord,
        journal_relative_path: &str,
    ) -> HarnessResult<AppliedHarnessBatch> {
        self.require_native_permit(resolved, None)?;
        self.recover_validated_execution_apply_internal(
            resolved,
            prepared,
            revision_history,
            record,
            journal_relative_path,
            false,
        )
    }

    fn recover_validated_execution_apply_internal(
        &self,
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
        revision_history: &[HarnessTaskEvaluation],
        record: &RoleExecutionRecord,
        journal_relative_path: &str,
        inspect_only: bool,
    ) -> HarnessResult<AppliedHarnessBatch> {
        resolved.plan.require_executable_version()?;
        if !resolved.plan.source_write_allowed {
            return Err(HarnessError::InvalidRequest(
                "the resolved action does not permit source changes".to_owned(),
            ));
        }
        let evaluation = require_accepted_execution_evaluation(evaluate_execution_record(
            &resolved.plan,
            prepared,
            revision_history,
            record,
        )?)?;
        let lifecycle_receipt =
            self.recover_apply_batch_internal(journal_relative_path, inspect_only, false)?;
        if lifecycle_receipt.journal_relative_path != journal_relative_path {
            return Err(HarnessError::BatchApply {
                message: "recovered batch receipt does not match the requested journal path"
                    .to_owned(),
                receipt: Box::new(lifecycle_receipt),
            });
        }
        self.confirm_execution_applied_batch(resolved, evaluation, lifecycle_receipt, false)
    }

    fn inspect_validated_execution_apply(
        &self,
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
        revision_history: &[HarnessTaskEvaluation],
        record: &RoleExecutionRecord,
        journal_relative_path: &str,
    ) -> HarnessResult<AppliedHarnessBatch> {
        self.recover_validated_execution_apply_internal(
            resolved,
            prepared,
            revision_history,
            record,
            journal_relative_path,
            true,
        )
    }

    fn require_native_permit(
        &self,
        resolved: &ResolvedHarnessRequest,
        permit: Option<&ContextApplyContract>,
    ) -> HarnessResult<()> {
        if self.workspace_root == self.vault.root
            && self.vault.store_identity.is_some()
            && resolved
                .plan
                .targets
                .iter()
                .any(|target| target.workspace_relative_path.starts_with("vault/"))
        {
            let contract = permit.ok_or_else(|| {
                HarnessError::InvalidSubmission(
                    "native context writes require the durable Core effect boundary".to_owned(),
                )
            })?;
            if !contract.requires_commit()
                || contract.store_identity() != self.vault.store_identity.as_ref()
                || contract.attempt().resolved_plan_digest != resolved.resolved_plan_digest
            {
                return Err(HarnessError::InvalidSubmission(
                    "native mutation permit does not match this engine and plan".to_owned(),
                ));
            }
        }
        Ok(())
    }

    pub fn recover_apply_batch(
        &self,
        journal_relative_path: &str,
    ) -> HarnessResult<HarnessBatchApplyLifecycleReceipt> {
        if self.vault.store_identity.is_some() {
            return Err(HarnessError::InvalidSubmission(
                "stored-source recovery requires the source-free durable recovery handle"
                    .to_owned(),
            ));
        }
        self.recover_apply_batch_internal(journal_relative_path, false, false)
    }

    fn recover_apply_batch_internal(
        &self,
        journal_relative_path: &str,
        inspect_only: bool,
        rolled_back: bool,
    ) -> HarnessResult<HarnessBatchApplyLifecycleReceipt> {
        let policy = self.workspace_policy()?;
        let acquisition = repository::WorkspaceMutationLock::acquire_reported_with_policy(
            self.workspace_policy()?,
            &self.workspace_root,
        )?;
        let Some(mutation_lock) = acquisition.lock else {
            let receipt = HarnessBatchApplyLifecycleReceipt {
                batch_id: String::new(),
                resolved_plan_digest: String::new(),
                candidate_digest: String::new(),
                journal_relative_path: journal_relative_path.to_owned(),
                completion_receipt_relative_path: None,
                journal_retained: true,
                outcome: BatchApplyOutcomeReceipt::RecoveryRequired,
                lock: lock_lifecycle_receipt(&acquisition.report),
                targets: Vec::new(),
                failure_history: Vec::new(),
                orchestration_failures: vec![LifecycleFailureReceipt {
                    stage: "workspace-lock-acquisition".to_owned(),
                    message: "reported lock acquisition did not produce a usable lock".to_owned(),
                }],
            };
            return Err(HarnessError::BatchApply {
                message: "batch recovery could not acquire the workspace mutation lock".to_owned(),
                receipt: Box::new(receipt),
            });
        };
        let report = if inspect_only {
            repository::inspect_completed_batch_reported(
                &self.workspace_root,
                journal_relative_path,
                rolled_back,
            )
        } else {
            repository::recover_change_batch_reported_with_policy(
                self.workspace_policy()?,
                &self.workspace_root,
                journal_relative_path,
            )
        };
        let lock = lock_lifecycle_receipt(&mutation_lock.release());
        let report = report.map(|report| {
            if !inspect_only
                && lock.is_fully_confirmed()
                && report.outcome != repository::BatchApplyOutcome::RecoveryRequired
            {
                repository::complete_batch_cleanup_after_lock_release_with_policy(
                    policy,
                    &self.workspace_root,
                    report,
                )
            } else {
                report
            }
        });
        let mut receipt = match report {
            Ok(report) => batch_apply_lifecycle_receipt(report, lock),
            Err(error) => HarnessBatchApplyLifecycleReceipt {
                batch_id: String::new(),
                resolved_plan_digest: String::new(),
                candidate_digest: String::new(),
                journal_relative_path: journal_relative_path.to_owned(),
                completion_receipt_relative_path: None,
                journal_retained: true,
                outcome: BatchApplyOutcomeReceipt::RecoveryRequired,
                lock,
                targets: Vec::new(),
                failure_history: Vec::new(),
                orchestration_failures: vec![LifecycleFailureReceipt {
                    stage: "batch-recovery".to_owned(),
                    message: error.to_string(),
                }],
            },
        };
        if !receipt.lock.is_fully_confirmed() {
            receipt.outcome = BatchApplyOutcomeReceipt::RecoveryRequired;
            receipt
                .orchestration_failures
                .push(LifecycleFailureReceipt {
                    stage: "workspace-lock-release".to_owned(),
                    message: "workspace mutation lock release was not fully confirmed".to_owned(),
                });
        }
        if receipt.outcome == BatchApplyOutcomeReceipt::RecoveryRequired
            || !receipt.orchestration_failures.is_empty()
        {
            return Err(HarnessError::BatchApply {
                message: "batch recovery still requires manual inspection".to_owned(),
                receipt: Box::new(receipt),
            });
        }
        Ok(receipt)
    }

    #[cfg(test)]
    pub fn attest_career_review(
        &self,
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
        submission: &HarnessSubmission,
    ) -> HarnessResult<CareerReviewReceipt> {
        resolved.plan.require_executable_version()?;
        with_reported_workspace_lock(
            self.workspace_policy()?,
            &self.workspace_root,
            "career-review-attestation",
            || self.attest_career_review_unlocked(resolved, prepared, submission),
        )
    }

    pub fn attest_career_execution_review(
        &self,
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
        record: &RoleExecutionRecord,
    ) -> HarnessResult<CareerExecutionReviewReceipt> {
        resolved.plan.require_executable_version()?;
        with_reported_workspace_lock(
            self.workspace_policy()?,
            &self.workspace_root,
            "career-execution-attestation",
            || self.attest_career_execution_review_unlocked(resolved, prepared, record),
        )
    }

    #[allow(
        clippy::too_many_lines,
        reason = "career review orchestration and attestation digest ordering must remain auditable together"
    )]
    fn attest_career_execution_review_unlocked(
        &self,
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
        record: &RoleExecutionRecord,
    ) -> HarnessResult<CareerExecutionReviewReceipt> {
        if resolved.plan.action != HarnessAction::DocumentReview
            || resolved.plan.owner != DataOwner::Personal
            || !resolved.plan.intent.is_career_artifact()
            || resolved.plan.targets.is_empty()
            || resolved.plan.targets.iter().any(|target| {
                target.operation != TargetOperation::Inspect
                    || !matches!(target.state, TargetState::Existing { .. })
            })
        {
            return Err(HarnessError::InvalidRequest(
                "career review attestation requires a personal career document-review plan over existing artifacts"
                    .to_owned(),
            ));
        }
        let manifest = resolved
            .career_composition_manifest
            .as_ref()
            .ok_or_else(|| {
                HarnessError::InvalidRequest(
                    "career review attestation requires a predeclared composition manifest"
                        .to_owned(),
                )
            })?;
        manifest.validate(&resolved.request, resolved.intent.career_surface())?;
        let manifest_digest = serialized_digest(manifest)?;
        if resolved.plan.career_manifest_digest.as_deref() != Some(manifest_digest.as_str()) {
            return Err(HarnessError::InvalidPlan(
                "career composition manifest is not bound to the review plan".to_owned(),
            ));
        }
        match resolved.context_grants.as_slice() {
            [] if resolved.plan.evidence_sources.is_empty()
                && resolved.evidence_source_paths.is_empty() => {}
            [grant]
                if !resolved.plan.evidence_sources.is_empty()
                    && manifest.evidence_owners.contains(&grant.owner)
                    && manifest.evidence_source_paths_for(&grant.owner)
                        == resolved.evidence_source_paths => {}
            [] => {
                return Err(HarnessError::InvalidPlan(
                    "holistic career review must not bind evidence sources".to_owned(),
                ));
            }
            [_] => {
                return Err(HarnessError::InvalidPlan(
                    "owner career review requires the manifest's exact evidence source bundle"
                        .to_owned(),
                ));
            }
            _ => {
                return Err(HarnessError::InvalidPlan(
                    "career review attestation supports at most one evidence owner".to_owned(),
                ));
            }
        }
        let evaluation = self.validate_execution(resolved, prepared, &[], record)?;
        if evaluation.subject_status != SubjectStatus::Accepted {
            return Err(HarnessError::InvalidSubmission(
                "career review target was not accepted".to_owned(),
            ));
        }
        let reviewer_result_digest = record
            .role_results
            .iter()
            .find(|result| result.role == HarnessRole::Reviewer)
            .map(|result| result.result_digest.clone())
            .ok_or_else(|| {
                HarnessError::InvalidSubmission(
                    "career review is missing its Reviewer result".to_owned(),
                )
            })?;
        let verification_bindings = career_verification_bindings(&resolved.plan, record)?;
        let artifact_set_digest = serialized_digest(&resolved.plan.targets)?;
        let evidence_bundle_digest = (!resolved.plan.evidence_sources.is_empty())
            .then(|| serialized_digest(&resolved.plan.evidence_sources))
            .transpose()?;
        let version = HARNESS_SCHEMA_VERSION;
        let receipt_digest = serialized_digest(&(
            version,
            resolved,
            prepared,
            record,
            &evaluation,
            &reviewer_result_digest,
            &verification_bindings,
            &artifact_set_digest,
            &evidence_bundle_digest,
        ))?;
        Ok(CareerExecutionReviewReceipt {
            version,
            resolved: resolved.clone(),
            prepared: prepared.clone(),
            evaluation,
            reviewer_result_digest,
            verification_bindings,
            artifact_set_digest,
            evidence_bundle_digest,
            receipt_digest,
        })
    }

    #[cfg(test)]
    fn attest_career_review_unlocked(
        &self,
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
        submission: &HarnessSubmission,
    ) -> HarnessResult<CareerReviewReceipt> {
        if resolved.plan.action != HarnessAction::DocumentReview
            || resolved.plan.owner != DataOwner::Personal
            || !resolved.plan.intent.is_career_artifact()
            || resolved.plan.targets.is_empty()
            || resolved.plan.targets.iter().any(|target| {
                target.operation != TargetOperation::Inspect
                    || !matches!(target.state, TargetState::Existing { .. })
            })
        {
            return Err(HarnessError::InvalidRequest(
                "career review attestation requires a personal career document-review plan over existing artifacts"
                    .to_owned(),
            ));
        }
        let manifest = resolved
            .career_composition_manifest
            .as_ref()
            .ok_or_else(|| {
                HarnessError::InvalidRequest(
                    "career review attestation requires a predeclared composition manifest"
                        .to_owned(),
                )
            })?;
        manifest.validate(&resolved.request, resolved.intent.career_surface())?;
        let manifest_digest = serialized_digest(manifest)?;
        if resolved.plan.career_manifest_digest.as_deref() != Some(manifest_digest.as_str()) {
            return Err(HarnessError::InvalidPlan(
                "career composition manifest is not bound to the review plan".to_owned(),
            ));
        }
        match resolved.context_grants.as_slice() {
            [] if resolved.plan.evidence_sources.is_empty()
                && resolved.evidence_source_paths.is_empty() => {}
            [grant]
                if !resolved.plan.evidence_sources.is_empty()
                    && manifest.evidence_owners.contains(&grant.owner)
                    && manifest.evidence_source_paths_for(&grant.owner)
                        == resolved.evidence_source_paths => {}
            [] => {
                return Err(HarnessError::InvalidPlan(
                    "holistic career review must not bind evidence sources".to_owned(),
                ));
            }
            [_] => {
                return Err(HarnessError::InvalidPlan(
                    "owner career review requires the manifest's exact evidence source bundle"
                        .to_owned(),
                ));
            }
            _ => {
                return Err(HarnessError::InvalidPlan(
                    "career review attestation supports at most one evidence owner".to_owned(),
                ));
            }
        }
        let evaluation = self.validate_submission(resolved, prepared, submission)?;
        let artifact_set_digest = serialized_digest(&resolved.plan.targets)?;
        let evidence_bundle_digest = (!resolved.plan.evidence_sources.is_empty())
            .then(|| serialized_digest(&resolved.plan.evidence_sources))
            .transpose()?;
        let version = HARNESS_SCHEMA_VERSION;
        let receipt_digest = serialized_digest(&(
            version,
            resolved,
            prepared,
            submission,
            &evaluation,
            &artifact_set_digest,
            &evidence_bundle_digest,
        ))?;
        Ok(CareerReviewReceipt {
            version,
            resolved: resolved.clone(),
            prepared: prepared.clone(),
            submission: submission.clone(),
            evaluation,
            artifact_set_digest,
            evidence_bundle_digest,
            receipt_digest,
        })
    }

    #[cfg(test)]
    pub fn compose_career_reviews(
        &self,
        manifest: &CareerCompositionManifest,
        holistic_review: &CareerReviewReceipt,
        evidence_reviews: &[CareerReviewReceipt],
    ) -> HarnessResult<CareerCompositionReceipt> {
        holistic_review.resolved.plan.require_executable_version()?;
        for review in evidence_reviews {
            review.resolved.plan.require_executable_version()?;
        }
        with_reported_workspace_lock(
            self.workspace_policy()?,
            &self.workspace_root,
            "career-review-composition",
            || self.compose_career_reviews_unlocked(manifest, holistic_review, evidence_reviews),
        )
    }

    #[cfg(test)]
    fn compose_career_reviews_unlocked(
        &self,
        manifest: &CareerCompositionManifest,
        holistic_review: &CareerReviewReceipt,
        evidence_reviews: &[CareerReviewReceipt],
    ) -> HarnessResult<CareerCompositionReceipt> {
        let manifest = manifest.normalized();
        let holistic_current = self.attest_career_review_unlocked(
            &holistic_review.resolved,
            &holistic_review.prepared,
            &holistic_review.submission,
        )?;
        if &holistic_current != holistic_review {
            return Err(HarnessError::InvalidSubmission(
                "holistic career review receipt is invalid or stale".to_owned(),
            ));
        }
        if !holistic_review.resolved.context_grants.is_empty() {
            return Err(HarnessError::InvalidSubmission(
                "holistic career review receipt must not use an evidence grant".to_owned(),
            ));
        }
        let mut current_evidence = evidence_reviews
            .iter()
            .map(|receipt| {
                let current = self.attest_career_review_unlocked(
                    &receipt.resolved,
                    &receipt.prepared,
                    &receipt.submission,
                )?;
                if &current != receipt {
                    return Err(HarnessError::InvalidSubmission(
                        "owner career review receipt is invalid or stale".to_owned(),
                    ));
                }
                let [grant] = receipt.resolved.context_grants.as_slice() else {
                    return Err(HarnessError::InvalidSubmission(
                        "each owner career review receipt must bind exactly one evidence owner"
                            .to_owned(),
                    ));
                };
                Ok((grant.owner.clone(), receipt.clone()))
            })
            .collect::<HarnessResult<Vec<_>>>()?;
        current_evidence.sort_by(|left, right| left.0.cmp(&right.0));

        let manifest_digest = serialized_digest(&manifest)?;
        let artifact_targets =
            validate_career_receipt_set(&manifest, holistic_review, &current_evidence)?;

        let holistic_review = career_review_binding(None, holistic_review);
        let evidence_reviews = current_evidence
            .iter()
            .map(|(owner, receipt)| career_review_binding(Some(owner.clone()), receipt))
            .collect::<Vec<_>>();
        let declared_evidence_owners = manifest.evidence_owners.clone();
        let artifact_set_digest = holistic_review.artifact_set_digest.clone();
        let version = HARNESS_SCHEMA_VERSION;
        let coverage_is_caller_attested = true;
        let assurance = ExecutionAssurance::Advisory;
        let composition_digest = serialized_digest(&(
            version,
            &manifest_digest,
            manifest.career_output_surface,
            manifest.coverage,
            &declared_evidence_owners,
            &artifact_targets,
            &artifact_set_digest,
            &holistic_review,
            &evidence_reviews,
            coverage_is_caller_attested,
            assurance,
        ))?;
        Ok(CareerCompositionReceipt {
            version,
            manifest_digest,
            career_output_surface: manifest.career_output_surface,
            coverage: manifest.coverage,
            declared_evidence_owners,
            artifact_targets,
            artifact_set_digest,
            holistic_review,
            evidence_reviews,
            coverage_is_caller_attested,
            assurance,
            composition_digest,
        })
    }

    pub fn compose_career_execution_reviews(
        &self,
        manifest: &CareerCompositionManifest,
        holistic_review: &CareerExecutionReviewReceipt,
        evidence_reviews: &[CareerExecutionReviewReceipt],
    ) -> HarnessResult<CareerExecutionCompositionReceipt> {
        holistic_review.resolved.plan.require_executable_version()?;
        for review in evidence_reviews {
            review.resolved.plan.require_executable_version()?;
        }
        with_reported_workspace_lock(
            self.workspace_policy()?,
            &self.workspace_root,
            "career-execution-composition",
            || {
                self.compose_career_execution_reviews_unlocked(
                    manifest,
                    holistic_review,
                    evidence_reviews,
                )
            },
        )
    }

    fn compose_career_execution_reviews_unlocked(
        &self,
        manifest: &CareerCompositionManifest,
        holistic_review: &CareerExecutionReviewReceipt,
        evidence_reviews: &[CareerExecutionReviewReceipt],
    ) -> HarnessResult<CareerExecutionCompositionReceipt> {
        let manifest = manifest.normalized();
        let holistic_current = self.attest_career_execution_review_unlocked(
            &holistic_review.resolved,
            &holistic_review.prepared,
            &holistic_review.evaluation.execution_record,
        )?;
        if &holistic_current != holistic_review {
            return Err(HarnessError::InvalidSubmission(
                "holistic career execution receipt is invalid or stale".to_owned(),
            ));
        }
        if !holistic_review.resolved.context_grants.is_empty() {
            return Err(HarnessError::InvalidSubmission(
                "holistic career review receipt must not use an evidence grant".to_owned(),
            ));
        }
        let mut current_evidence = evidence_reviews
            .iter()
            .map(|receipt| {
                let current = self.attest_career_execution_review_unlocked(
                    &receipt.resolved,
                    &receipt.prepared,
                    &receipt.evaluation.execution_record,
                )?;
                if &current != receipt {
                    return Err(HarnessError::InvalidSubmission(
                        "owner career execution receipt is invalid or stale".to_owned(),
                    ));
                }
                let [grant] = receipt.resolved.context_grants.as_slice() else {
                    return Err(HarnessError::InvalidSubmission(
                        "each owner career review receipt must bind exactly one evidence owner"
                            .to_owned(),
                    ));
                };
                Ok((grant.owner.clone(), receipt.clone()))
            })
            .collect::<HarnessResult<Vec<_>>>()?;
        current_evidence.sort_by(|left, right| left.0.cmp(&right.0));
        let manifest_digest = serialized_digest(&manifest)?;
        let artifact_targets =
            validate_career_execution_receipt_set(&manifest, holistic_review, &current_evidence)?;
        let holistic_review = career_execution_review_binding(None, holistic_review);
        let evidence_reviews = current_evidence
            .iter()
            .map(|(owner, receipt)| career_execution_review_binding(Some(owner.clone()), receipt))
            .collect::<Vec<_>>();
        let declared_evidence_owners = manifest.evidence_owners.clone();
        let artifact_set_digest = holistic_review.artifact_set_digest.clone();
        let version = HARNESS_SCHEMA_VERSION;
        let coverage_is_caller_attested = true;
        let assurance = ExecutionAssurance::Advisory;
        let composition_digest = serialized_digest(&(
            version,
            &manifest_digest,
            manifest.career_output_surface,
            manifest.coverage,
            &declared_evidence_owners,
            &artifact_targets,
            &artifact_set_digest,
            &holistic_review,
            &evidence_reviews,
            coverage_is_caller_attested,
            assurance,
        ))?;
        Ok(CareerExecutionCompositionReceipt {
            version,
            manifest_digest,
            career_output_surface: manifest.career_output_surface,
            coverage: manifest.coverage,
            declared_evidence_owners,
            artifact_targets,
            artifact_set_digest,
            holistic_review,
            evidence_reviews,
            coverage_is_caller_attested,
            assurance,
            composition_digest,
        })
    }

    #[cfg(test)]
    pub fn apply_validated_submission(
        &self,
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
        submission: &HarnessSubmission,
        revision_history: &[HarnessEvaluationResult],
    ) -> HarnessResult<AppliedHarnessChange> {
        self.require_native_permit(resolved, None)?;
        resolved.plan.require_executable_version()?;
        if !resolved.plan.source_write_allowed {
            return Err(HarnessError::InvalidRequest(
                "the resolved action does not permit source changes".to_owned(),
            ));
        }
        self.revalidate_resolved(resolved)?;
        let evaluated = self.validate_submission_with_history(
            resolved,
            prepared,
            submission,
            revision_history,
        )?;
        let change = evaluated_single_change(&evaluated)?;
        let [target] = resolved.plan.targets.as_slice() else {
            return Err(HarnessError::InvalidPlan(
                "single-file apply requires exactly one planned target".to_owned(),
            ));
        };
        let validate_vault_markdown = requires_vault_markdown_validation(
            self.workspace_root == self.vault.root,
            &target.workspace_relative_path,
        );
        let lifecycle_receipt =
            self.apply_change_with_lifecycle(resolved, &change, target, validate_vault_markdown)?;
        self.confirm_applied_change(resolved, evaluated, lifecycle_receipt)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "durable journal and rollback ordering must remain explicit in one lifecycle"
    )]
    fn apply_change_batch_with_lifecycle(
        &self,
        resolved: &ResolvedHarnessRequest,
        inputs: &[repository::BatchApplyInput<'_>],
        candidate_digest: &str,
        attempt_identifier: Option<&str>,
    ) -> HarnessResult<HarnessBatchApplyLifecycleReceipt> {
        let policy = self.workspace_policy()?;
        let acquisition = repository::WorkspaceMutationLock::acquire_reported_with_policy(
            self.workspace_policy()?,
            &self.workspace_root,
        )?;
        let Some(mutation_lock) = acquisition.lock else {
            let receipt = HarnessBatchApplyLifecycleReceipt {
                batch_id: String::new(),
                resolved_plan_digest: resolved.resolved_plan_digest.clone(),
                candidate_digest: candidate_digest.to_owned(),
                journal_relative_path: String::new(),
                completion_receipt_relative_path: None,
                journal_retained: false,
                outcome: BatchApplyOutcomeReceipt::RecoveryRequired,
                lock: lock_lifecycle_receipt(&acquisition.report),
                targets: Vec::new(),
                failure_history: Vec::new(),
                orchestration_failures: vec![LifecycleFailureReceipt {
                    stage: "workspace-lock-acquisition".to_owned(),
                    message: "reported lock acquisition did not produce a usable lock".to_owned(),
                }],
            };
            return Err(HarnessError::BatchApply {
                message: "workspace mutation lock acquisition was not fully confirmed".to_owned(),
                receipt: Box::new(receipt),
            });
        };
        let report = repository::ensure_no_pending_batch_recovery(&self.workspace_root)
            .and_then(|()| self.revalidate_resolved(resolved))
            .and_then(|_revalidated| self.revalidate_learning_promotion_identifier(resolved))
            .and_then(|_revalidated| match attempt_identifier {
                Some(attempt_identifier) => {
                    repository::apply_change_batch_reported_for_attempt_with_policy(
                        self.workspace_policy()?,
                        &self.workspace_root,
                        inputs,
                        &resolved.resolved_plan_digest,
                        candidate_digest,
                        attempt_identifier,
                    )
                }
                None => repository::apply_change_batch_reported_with_policy(
                    self.workspace_policy()?,
                    &self.workspace_root,
                    inputs,
                    &resolved.resolved_plan_digest,
                    candidate_digest,
                ),
            });
        let lock = lock_lifecycle_receipt(&mutation_lock.release());
        let report = report.map(|report| {
            if lock.is_fully_confirmed()
                && report.outcome != repository::BatchApplyOutcome::RecoveryRequired
            {
                repository::complete_batch_cleanup_after_lock_release_with_policy(
                    policy,
                    &self.workspace_root,
                    report,
                )
            } else {
                report
            }
        });
        let mut receipt = match report {
            Ok(report) => batch_apply_lifecycle_receipt(report, lock),
            Err(error) => {
                let (journal_relative_path, journal_retained, outcome, stage) = match &error {
                    HarnessError::PendingBatchRecovery { artifacts } => (
                        artifacts.first().cloned().unwrap_or_default(),
                        true,
                        BatchApplyOutcomeReceipt::RecoveryRequired,
                        "pending-batch-recovery",
                    ),
                    _ => (
                        String::new(),
                        false,
                        BatchApplyOutcomeReceipt::RolledBack,
                        "batch-preflight",
                    ),
                };
                HarnessBatchApplyLifecycleReceipt {
                    batch_id: String::new(),
                    resolved_plan_digest: resolved.resolved_plan_digest.clone(),
                    candidate_digest: candidate_digest.to_owned(),
                    journal_relative_path,
                    completion_receipt_relative_path: None,
                    journal_retained,
                    outcome,
                    lock,
                    targets: Vec::new(),
                    failure_history: Vec::new(),
                    orchestration_failures: vec![LifecycleFailureReceipt {
                        stage: stage.to_owned(),
                        message: error.to_string(),
                    }],
                }
            }
        };
        if !receipt.lock.is_fully_confirmed() {
            receipt.outcome = BatchApplyOutcomeReceipt::RecoveryRequired;
            receipt
                .orchestration_failures
                .push(LifecycleFailureReceipt {
                    stage: "workspace-lock-release".to_owned(),
                    message: "workspace mutation lock release was not fully confirmed".to_owned(),
                });
        }
        if receipt.outcome != BatchApplyOutcomeReceipt::Applied
            || !receipt.orchestration_failures.is_empty()
        {
            let message = match receipt.outcome {
                BatchApplyOutcomeReceipt::Applied => {
                    "batch targets were applied but lifecycle confirmation failed"
                }
                BatchApplyOutcomeReceipt::RolledBack => {
                    "batch apply failed and all source targets were rolled back"
                }
                BatchApplyOutcomeReceipt::RecoveryRequired => {
                    "batch apply requires recovery before another source mutation"
                }
            };
            return Err(HarnessError::BatchApply {
                message: message.to_owned(),
                receipt: Box::new(receipt),
            });
        }
        Ok(receipt)
    }

    #[cfg(test)]
    fn apply_change_with_lifecycle(
        &self,
        resolved: &ResolvedHarnessRequest,
        change: &FileChange,
        target: &TargetBinding,
        validate_vault_markdown: bool,
    ) -> HarnessResult<HarnessApplyLifecycleReceipt> {
        let acquisition = repository::WorkspaceMutationLock::acquire_reported_with_policy(
            self.workspace_policy()?,
            &self.workspace_root,
        )?;
        let Some(mutation_lock) = acquisition.lock else {
            let lock = lock_lifecycle_receipt(&acquisition.report);
            let mut orchestration_failures = vec![LifecycleFailureReceipt {
                stage: "workspace-lock-acquisition".to_owned(),
                message: "reported lock acquisition did not produce a usable lock".to_owned(),
            }];
            if !lock.release_cleanup_confirmed() {
                orchestration_failures.push(LifecycleFailureReceipt {
                    stage: "workspace-lock-release".to_owned(),
                    message: "failed lock acquisition cleanup was not fully confirmed".to_owned(),
                });
            }
            return Err(HarnessError::ApplyLifecycle {
                message: "workspace mutation lock acquisition was not fully confirmed".to_owned(),
                receipt: Box::new(HarnessApplyLifecycleReceipt {
                    lock,
                    target_apply: None,
                    orchestration_failures,
                }),
            });
        };
        let mut orchestration_failures = Vec::new();
        if let Err(error) = self.revalidate_resolved(resolved) {
            orchestration_failures.push(LifecycleFailureReceipt {
                stage: "plan-revalidation".to_owned(),
                message: error.to_string(),
            });
        }
        let target_apply = if orchestration_failures.is_empty() {
            match apply_single_change_reported(
                &self.workspace_root,
                change,
                &resolved.resolved_plan_digest,
                &target.parent_directories_to_create,
                validate_vault_markdown,
            ) {
                Ok(report) => Some(repository_apply_receipt(&report)),
                Err(error) => {
                    orchestration_failures.push(LifecycleFailureReceipt {
                        stage: "repository-apply".to_owned(),
                        message: error.to_string(),
                    });
                    None
                }
            }
        } else {
            None
        };
        let lock = lock_lifecycle_receipt(&mutation_lock.release());
        if !lock.is_fully_confirmed() {
            orchestration_failures.push(LifecycleFailureReceipt {
                stage: "workspace-lock-release".to_owned(),
                message: "workspace mutation lock release was not fully confirmed".to_owned(),
            });
        }
        let lifecycle_receipt = HarnessApplyLifecycleReceipt {
            lock,
            target_apply,
            orchestration_failures,
        };
        if !lifecycle_receipt.is_fully_confirmed() {
            return Err(HarnessError::ApplyLifecycle {
                message: "lock, target mutation, and release were not all fully confirmed"
                    .to_owned(),
                receipt: Box::new(lifecycle_receipt),
            });
        }
        Ok(lifecycle_receipt)
    }

    #[cfg(test)]
    fn confirm_applied_change(
        &self,
        resolved: &ResolvedHarnessRequest,
        evaluated: HarnessEvaluationResult,
        lifecycle_receipt: HarnessApplyLifecycleReceipt,
    ) -> HarnessResult<AppliedHarnessChange> {
        let Some(target_apply) = lifecycle_receipt.target_apply.clone() else {
            return Err(HarnessError::ApplyLifecycle {
                message: "fully confirmed apply receipt omitted the target report".to_owned(),
                receipt: Box::new(lifecycle_receipt),
            });
        };
        let resulting_content_digest = match &target_apply.verified_target_state {
            VerifiedTargetStateReceipt::Present { content_digest } => Some(content_digest.clone()),
            VerifiedTargetStateReceipt::Absent => None,
            VerifiedTargetStateReceipt::Unverified => {
                return Err(HarnessError::ApplyLifecycle {
                    message: "fully confirmed apply receipt has an unverified target state"
                        .to_owned(),
                    receipt: Box::new(lifecycle_receipt),
                });
            }
        };
        let workspace_relative_path = target_apply.workspace_relative_path;
        let operation = target_apply.operation;
        let created_parent_directories = target_apply.created_parent_directories;
        let completion_state = HarnessCompletionState::Applied;
        Ok(AppliedHarnessChange {
            resolved_plan_digest: resolved.resolved_plan_digest.clone(),
            candidate_digest: evaluated.candidate_digest,
            validation_receipt_digest: evaluated.validation_receipt_digest,
            workspace_relative_path,
            operation,
            resulting_content_digest,
            created_parent_directories,
            completion_state,
            lifecycle_receipt,
        })
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Harness engine apply confirmation audits the complete journal, target and repository evidence contract together"
    )]
    fn confirm_execution_applied_batch(
        &self,
        resolved: &ResolvedHarnessRequest,
        evaluation: HarnessTaskEvaluation,
        lifecycle_receipt: HarnessBatchApplyLifecycleReceipt,
        require_repository_evidence: bool,
    ) -> HarnessResult<AppliedHarnessBatch> {
        let candidate_digest = evaluation.candidate_digest.clone().ok_or_else(|| {
            HarnessError::InvalidSubmission(
                "validated write execution omitted its candidate digest".to_owned(),
            )
        })?;
        if lifecycle_receipt.outcome != BatchApplyOutcomeReceipt::Applied
            || lifecycle_receipt.journal_retained
            || !lifecycle_receipt.orchestration_failures.is_empty()
            || lifecycle_receipt.resolved_plan_digest != resolved.resolved_plan_digest
            || lifecycle_receipt.candidate_digest != candidate_digest
            || lifecycle_receipt
                .targets
                .iter()
                .any(|target| target.state != BatchTargetApplyStateReceipt::Applied)
        {
            return Err(HarnessError::BatchApply {
                message: "applied batch receipt is not fully confirmed".to_owned(),
                receipt: Box::new(lifecycle_receipt),
            });
        }
        let mut changes = evaluated_execution_changes(&evaluation)?;
        changes.sort_by(|left, right| left.path().cmp(right.path()));
        let mut targets = lifecycle_receipt.targets.iter().collect::<Vec<_>>();
        targets.sort_by(|left, right| {
            left.workspace_relative_path
                .cmp(&right.workspace_relative_path)
        });
        if changes.len() != targets.len() {
            return Err(HarnessError::BatchApply {
                message: "applied batch target count does not match the evaluated candidate"
                    .to_owned(),
                receipt: Box::new(lifecycle_receipt),
            });
        }
        for (change, target) in changes.iter().zip(targets) {
            let (operation, original_content_digest, intended_content_digest) = match change {
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
            let repository_evidence_valid = match &target.repository_apply {
                Some(repository) => {
                    repository.workspace_relative_path == change.path()
                        && repository.operation == operation
                        && match (&repository.verified_target_state, &intended_content_digest) {
                            (
                                VerifiedTargetStateReceipt::Present { content_digest },
                                Some(expected),
                            ) => content_digest == expected,
                            (VerifiedTargetStateReceipt::Absent, None) => true,
                            _ => false,
                        }
                }
                None => !require_repository_evidence,
            };
            if target.workspace_relative_path != change.path()
                || target.operation != operation
                || target.original_content_digest != original_content_digest
                || target.intended_content_digest != intended_content_digest
                || !repository_evidence_valid
            {
                return Err(HarnessError::BatchApply {
                    message: "applied batch targets do not match the exact evaluated candidate"
                        .to_owned(),
                    receipt: Box::new(lifecycle_receipt),
                });
            }
        }
        let completion_state = HarnessCompletionState::Applied;
        Ok(AppliedHarnessBatch {
            resolved_plan_digest: resolved.resolved_plan_digest.clone(),
            candidate_digest,
            task_evaluation_receipt_digest: evaluation.task_evaluation_receipt_digest,
            batch_id: lifecycle_receipt.batch_id.clone(),
            targets: lifecycle_receipt.targets.clone(),
            completion_state,
            lifecycle_receipt,
        })
    }

    #[cfg(test)]
    pub fn validate_submission_with_history(
        &self,
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
        submission: &HarnessSubmission,
        revision_history: &[HarnessEvaluationResult],
    ) -> HarnessResult<HarnessEvaluationResult> {
        let result = self.evaluate_submission_with_history(
            resolved,
            prepared,
            submission,
            revision_history,
        )?;
        if !result.accepted {
            if result.findings.is_empty() {
                return Err(HarnessError::InvalidSubmission(
                    "one or more required verification checks failed".to_owned(),
                ));
            }
            return Err(HarnessError::ReviewFindings(result.findings));
        }
        Ok(result)
    }

    #[cfg(test)]
    pub fn evaluate_submission_with_history(
        &self,
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
        submission: &HarnessSubmission,
        revision_history: &[HarnessEvaluationResult],
    ) -> HarnessResult<HarnessEvaluationResult> {
        self.validate_submission_envelope(resolved, prepared, submission, revision_history)?;
        validate_reported_context_ids(
            &resolved.plan,
            &submission.reported_producer_context_id,
            &submission.reported_reviewer_context_id,
            &submission.reported_role_contexts,
        )?;
        validate_reported_role_lifecycles(
            &resolved.plan,
            &prepared.runtime_capabilities,
            &submission.reported_role_contexts,
            &submission.reported_role_lifecycles,
        )?;
        validate_submission_artifact(&resolved.plan, &submission.artifact)?;
        validate_submission_context_binding(prepared, &submission.artifact)?;
        let candidate_digest = serialized_digest(&(
            &resolved.resolved_plan_digest,
            &submission.prepared_role_run_digest,
            submission.revision,
            &submission.previous_candidate_digest,
            &submission.revision_feedback,
            &submission.reported_producer_context_id,
            &submission.artifact,
        ))?;
        validate_checks(
            &resolved.plan.verification_requirements,
            &submission.verification_checks,
        )?;
        validate_bounded_findings("review findings", &submission.findings)?;
        if submission.assurance != ExecutionAssurance::Advisory {
            return Err(HarnessError::InvalidSubmission(
                "the current common adapter contract only validates advisory execution".to_owned(),
            ));
        }
        let accepted =
            evaluation_is_accepted(&submission.verification_checks, &submission.findings);
        let validation_receipt_digest = serialized_digest(&(
            &candidate_digest,
            &submission.reported_reviewer_context_id,
            &submission.reported_role_contexts,
            &submission.reported_role_lifecycles,
            &submission.verification_checks,
            &submission.findings,
            submission.assurance,
            accepted,
        ))?;

        Ok(HarnessEvaluationResult {
            resolved_plan_digest: resolved.resolved_plan_digest.clone(),
            prepared_role_run_digest: prepared.prepared_role_run_digest.clone(),
            revision: submission.revision,
            previous_candidate_digest: submission.previous_candidate_digest.clone(),
            revision_feedback: submission.revision_feedback.clone(),
            candidate_digest,
            validation_receipt_digest,
            artifact: submission.artifact.clone(),
            reported_producer_context_id: submission.reported_producer_context_id.clone(),
            reported_reviewer_context_id: submission.reported_reviewer_context_id.clone(),
            reported_role_contexts: submission.reported_role_contexts.clone(),
            reported_role_lifecycles: submission.reported_role_lifecycles.clone(),
            verification_checks: submission.verification_checks.clone(),
            findings: submission.findings.clone(),
            accepted,
            assurance: ExecutionAssurance::Advisory,
        })
    }

    #[cfg(test)]
    fn validate_submission_envelope(
        &self,
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
        submission: &HarnessSubmission,
        revision_history: &[HarnessEvaluationResult],
    ) -> HarnessResult<()> {
        self.revalidate_resolved(resolved)?;
        if submission.resolved_plan_digest != resolved.resolved_plan_digest {
            return Err(HarnessError::InvalidSubmission(
                "submission resolved-plan digest does not match the resolved plan".to_owned(),
            ));
        }
        self.validate_prepared_run(resolved, prepared)?;
        if submission.prepared_role_run_digest != prepared.prepared_role_run_digest {
            return Err(HarnessError::InvalidSubmission(
                "submission prepared-role-run digest does not match the prepared role execution"
                    .to_owned(),
            ));
        }
        if !resolved.plan.action.is_write()
            && (submission.revision != 0 || !revision_history.is_empty())
        {
            return Err(HarnessError::InvalidSubmission(
                "non-write actions do not accept revision history or revised candidates".to_owned(),
            ));
        }
        if submission.revision > resolved.plan.max_revisions {
            return Err(HarnessError::InvalidSubmission(format!(
                "submission revision {} exceeds the maximum {}",
                submission.revision, resolved.plan.max_revisions
            )));
        }
        validate_revision_history(
            resolved,
            &prepared.prepared_role_run_digest,
            &prepared.runtime_capabilities,
            revision_history,
        )?;
        validate_submission_context_freshness(submission, revision_history)?;
        revision_history.iter().try_for_each(|result| {
            validate_submission_context_binding(prepared, &result.artifact)
        })?;
        if submission.revision as usize != revision_history.len() {
            return Err(HarnessError::InvalidSubmission(
                "submission revision must equal the validated revision history length".to_owned(),
            ));
        }
        validate_revision_chain(submission, revision_history.last())
    }

    #[cfg(test)]
    pub fn submission_template(
        resolved: &ResolvedHarnessRequest,
        prepared: &PreparedRoleRun,
    ) -> HarnessResult<HarnessSubmission> {
        resolved.plan.require_executable_version()?;
        if prepared.resolved_plan_digest != resolved.resolved_plan_digest {
            return Err(HarnessError::InvalidSubmission(
                "prepared execution does not match the resolved plan".to_owned(),
            ));
        }
        Ok(HarnessSubmission {
            resolved_plan_digest: resolved.resolved_plan_digest.clone(),
            prepared_role_run_digest: prepared.prepared_role_run_digest.clone(),
            revision: 0,
            previous_candidate_digest: None,
            revision_feedback: Vec::new(),
            artifact: submission_artifact_template(
                &resolved.plan,
                prepared
                    .context_bundle
                    .as_ref()
                    .map(|bundle| bundle.bundle_digest.clone()),
            )?,
            reported_producer_context_id: String::new(),
            reported_reviewer_context_id: String::new(),
            reported_role_contexts: resolved
                .plan
                .roles()
                .map(|role| ReportedRoleContext {
                    role,
                    context_id: String::new(),
                })
                .collect(),
            reported_role_lifecycles: resolved
                .plan
                .roles()
                .map(|role| ReportedRoleLifecycle {
                    role,
                    context_id: String::new(),
                    started_at_millis: 0,
                    context_ready_at_millis: 0,
                    first_output_at_millis: None,
                    interrupt_requested_at_millis: None,
                    grace_deadline_at_millis: None,
                    terminal_at_millis: 0,
                    closed_at_millis: 0,
                    terminal_state: RoleTerminalState::Completed,
                })
                .collect(),
            verification_checks: resolved
                .plan
                .verification_requirements
                .iter()
                .filter(|requirement| requirement.owner != VerificationOwner::Deterministic)
                .map(|requirement| VerificationCheck {
                    id: requirement.unit.as_str().to_owned(),
                    passed: false,
                    detail: String::new(),
                })
                .collect(),
            findings: Vec::new(),
            assurance: ExecutionAssurance::Advisory,
        })
    }

    fn revalidate_learning_promotion_identifier(
        &self,
        resolved: &ResolvedHarnessRequest,
    ) -> HarnessResult<()> {
        let Some(handoff) = &resolved.plan.promotion_handoff else {
            return Ok(());
        };
        let PromotionProposalOrigin::ReviewerLearning { learning_id, .. } =
            &handoff.proposal.origin
        else {
            return Ok(());
        };
        self.vault
            .ensure_learning_identifier_available(&handoff.proposal.owner, learning_id)
    }
}

mod source;
pub use source::*;

mod request;
pub use request::*;

mod execution;
pub use execution::*;

mod requirements;
pub use requirements::*;

mod tool_plan;
pub use tool_plan::*;

mod finalization;
pub use finalization::*;

mod persistence;
pub use persistence::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HarnessError {
    InvalidRequest(String),
    InvalidPlan(String),
    InvalidRepository(String),
    InvalidSubmission(String),
    FileRead {
        path: PathBuf,
        message: String,
    },
    FileWrite {
        path: PathBuf,
        message: String,
    },
    PathEscapesRoot(PathBuf),
    ClarificationRequired(Vec<ClarificationRequest>),
    UnsupportedPlanVersion {
        found: u32,
        current: u32,
    },
    PlanDrift {
        expected: String,
        actual: String,
    },
    PendingBatchRecovery {
        artifacts: Vec<String>,
    },
    ReviewFindings(Vec<Finding>),
    ApplyLifecycle {
        message: String,
        receipt: Box<HarnessApplyLifecycleReceipt>,
    },
    BatchApply {
        message: String,
        receipt: Box<HarnessBatchApplyLifecycleReceipt>,
    },
    Finalization {
        message: String,
        applied: Box<AppliedHarnessBatch>,
    },
    UnsupportedRuntime(String),
}

impl Display for HarnessError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(message) => {
                write!(formatter, "invalid harness request: {message}")
            }
            Self::InvalidPlan(message) => write!(formatter, "invalid harness plan: {message}"),
            Self::InvalidRepository(message) => write!(formatter, "invalid repository: {message}"),
            Self::InvalidSubmission(message) => {
                write!(formatter, "invalid harness submission: {message}")
            }
            Self::FileRead { path, message } => {
                write!(formatter, "failed to read `{}`: {message}", path.display())
            }
            Self::FileWrite { path, message } => {
                write!(formatter, "failed to write `{}`: {message}", path.display())
            }
            Self::PathEscapesRoot(path) => {
                write!(
                    formatter,
                    "path escapes its allowed root: `{}`",
                    path.display()
                )
            }
            Self::ClarificationRequired(questions) => {
                write!(formatter, "request requires clarification")?;
                for question in questions {
                    write!(formatter, "; {}: {}", question.field, question.question)?;
                }
                Ok(())
            }
            Self::UnsupportedPlanVersion { found, current } => write!(
                formatter,
                "Harness plan version {found} is inspection-only; rerun harness resolve to create version {current}"
            ),
            Self::PlanDrift { expected, actual } => write!(
                formatter,
                "resolved plan changed before validation: expected {expected}, got {actual}"
            ),
            Self::PendingBatchRecovery { artifacts } => write!(
                formatter,
                "pending Harness batch recovery artifacts block new applies: {}; finish recovery before applying another batch",
                artifacts.join(", ")
            ),
            Self::ReviewFindings(findings) => {
                write!(formatter, "review contains {} finding(s)", findings.len())
            }
            Self::ApplyLifecycle { message, .. } => {
                write!(formatter, "harness apply lifecycle failed: {message}")
            }
            Self::BatchApply { message, .. } => {
                write!(formatter, "harness batch apply failed: {message}")
            }
            Self::Finalization { message, .. } => {
                write!(
                    formatter,
                    "harness current post-apply finalization failed: {message}"
                )
            }
            Self::UnsupportedRuntime(message) => {
                write!(formatter, "unsupported runtime: {message}")
            }
        }
    }
}

impl std::error::Error for HarnessError {}

pub type HarnessResult<T> = std::result::Result<T, HarnessError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProfileInfrastructureTargetKind {
    Code,
    Document,
}

fn is_profile_policy_source_target(target: &str) -> bool {
    Path::new(target)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        && (target == "vault/profile/index.md"
            || path_is_within(target, "vault/profile/preferences")
            || path_is_within(target, "vault/profile/rules")
            || path_is_within(target, "vault/profile/working-style"))
}

const fn is_profile_source_action(action: HarnessAction) -> bool {
    matches!(
        action,
        HarnessAction::CodeWrite
            | HarnessAction::CodeReview
            | HarnessAction::DocumentWrite
            | HarnessAction::DocumentReview
            | HarnessAction::Investigation
            | HarnessAction::Design
    )
}

fn profile_infrastructure_target_kind(target: &str) -> Option<ProfileInfrastructureTargetKind> {
    let allowed = matches!(
        target,
        ".gitignore"
            | ".gitmessage"
            | ".markdownlint-cli2.jsonc"
            | ".nvmrc"
            | ".github/dependabot.yml"
            | "AGENTS.md"
            | "Cargo.lock"
            | "Cargo.toml"
            | "README.md"
            | "package.json"
            | "pnpm-lock.yaml"
            | "rust-toolchain.toml"
            | "scripts/verify-latest-rust.sh"
    ) || path_is_within(target, "crates/context-core")
        || path_is_within(target, ".github/workflows")
        || path_is_within(target, "docs");
    if !allowed {
        return None;
    }
    Some(if is_document_target(target) {
        ProfileInfrastructureTargetKind::Document
    } else {
        ProfileInfrastructureTargetKind::Code
    })
}

fn profile_action_accepts_target(
    action: HarnessAction,
    target_kind: ProfileInfrastructureTargetKind,
) -> bool {
    match action {
        HarnessAction::CodeWrite | HarnessAction::CodeReview => {
            target_kind == ProfileInfrastructureTargetKind::Code
        }
        HarnessAction::DocumentWrite | HarnessAction::DocumentReview => {
            target_kind == ProfileInfrastructureTargetKind::Document
        }
        HarnessAction::VaultCuration | HarnessAction::VaultRead => false,
        HarnessAction::Investigation | HarnessAction::Design | HarnessAction::Ideation => true,
    }
}

fn is_document_target(target: &str) -> bool {
    if is_dependency_target(target) {
        return false;
    }
    let path = Path::new(target);
    if path.extension().is_some_and(|extension| {
        ["adoc", "md", "mdx", "rst", "txt"]
            .iter()
            .any(|candidate| extension.eq_ignore_ascii_case(candidate))
    }) {
        return true;
    }
    let Some(file_name) = path.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    if file_name == ".gitmessage" {
        return true;
    }
    let upper = file_name.to_ascii_uppercase();
    let extensionless_document_names = [
        "AUTHORS",
        "CHANGELOG",
        "CONTRIBUTING",
        "LICENSE",
        "NOTICE",
        "README",
        "SECURITY",
    ];
    extensionless_document_names.contains(&upper.as_str())
        || path.extension().is_none() && upper.starts_with("LICENSE-")
}

fn is_rust_target(target: &str) -> bool {
    let path = Path::new(target);
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("rs"))
        || path.file_name().is_some_and(|file_name| {
            file_name.eq_ignore_ascii_case("Cargo.toml")
                || file_name.eq_ignore_ascii_case("Cargo.lock")
        })
}

fn is_dependency_target(target: &str) -> bool {
    if matches!(
        target,
        ".github/dependabot.yml" | "scripts/verify-latest-rust.sh"
    ) {
        return true;
    }
    if path_is_within(target, ".github/workflows") {
        return Path::new(target).extension().is_some_and(|extension| {
            extension.eq_ignore_ascii_case("yaml") || extension.eq_ignore_ascii_case("yml")
        });
    }
    let Some(file_name) = Path::new(target).file_name().and_then(OsStr::to_str) else {
        return false;
    };
    let lower = file_name.to_ascii_lowercase();
    DEPENDENCY_FILE_NAMES.contains(&lower.as_str())
        || is_python_dependency_file(&lower)
        || ["csproj", "fsproj", "vbproj"].iter().any(|extension| {
            Path::new(&lower)
                .extension()
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(extension))
        })
}

fn is_python_dependency_file(file_name: &str) -> bool {
    (file_name.starts_with("constraints") || file_name.starts_with("requirements"))
        && Path::new(file_name).extension().is_none_or(|extension| {
            extension.eq_ignore_ascii_case("in") || extension.eq_ignore_ascii_case("txt")
        })
}

fn path_resolves_within(candidate: &Path, root: &Path) -> HarnessResult<bool> {
    let mut existing = candidate;
    loop {
        match existing.canonicalize() {
            Ok(canonical) => return Ok(canonical.starts_with(root)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let Some(parent) = existing.parent() else {
                    return Ok(false);
                };
                existing = parent;
            }
            Err(error) => {
                return Err(HarnessError::FileRead {
                    path: existing.to_path_buf(),
                    message: error.to_string(),
                });
            }
        }
    }
}

fn requires_vault_markdown_validation(is_vault_workspace: bool, target: &str) -> bool {
    is_vault_workspace
        && target.starts_with("vault/")
        && Path::new(target)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

fn roles_for(
    request: &HarnessRequest,
    execution_profile: HarnessExecutionProfile,
) -> Vec<HarnessRole> {
    let mut roles = ExecutionProfileSpec::for_profile(execution_profile)
        .roles_for(request.action)
        .to_vec();
    if is_personal_idea_curation(request.action, &request.owner, request.curation_kind)
        && !roles.contains(&HarnessRole::Verifier)
    {
        let reviewer_position = roles
            .iter()
            .position(|role| *role == HarnessRole::Reviewer)
            .expect("validated Vault curation profiles always contain a Reviewer");
        roles.insert(reviewer_position, HarnessRole::Verifier);
    }
    roles
}

fn workflow_for(
    action: HarnessAction,
    roles: &[HarnessRole],
    primary_producer_role: HarnessRole,
) -> Vec<WorkflowRoleNode> {
    roles
        .iter()
        .map(|role| {
            if action.is_review() {
                return WorkflowRoleNode {
                    role: *role,
                    dependencies: Vec::new(),
                    subject_source: WorkflowSubjectSource::FrozenTargets,
                };
            }
            if *role == primary_producer_role {
                return WorkflowRoleNode {
                    role: *role,
                    dependencies: Vec::new(),
                    subject_source: WorkflowSubjectSource::TaskContract,
                };
            }
            WorkflowRoleNode {
                role: *role,
                dependencies: vec![primary_producer_role],
                subject_source: WorkflowSubjectSource::PrimaryProducer,
            }
        })
        .collect()
}

struct ExecutionProfileSpec {
    write: &'static [HarnessRole],
    review: &'static [HarnessRole],
    specialist_review: &'static [HarnessRole],
    vault_read: &'static [HarnessRole],
}

impl ExecutionProfileSpec {
    const STANDARD: Self = Self {
        write: &[HarnessRole::Writer, HarnessRole::Reviewer],
        review: &[HarnessRole::Verifier, HarnessRole::Reviewer],
        specialist_review: &[HarnessRole::Specialist, HarnessRole::Reviewer],
        vault_read: &[HarnessRole::Specialist],
    };

    const STRICT: Self = Self {
        write: &[
            HarnessRole::Writer,
            HarnessRole::Verifier,
            HarnessRole::Reviewer,
        ],
        review: &[HarnessRole::Verifier, HarnessRole::Reviewer],
        specialist_review: &[
            HarnessRole::Specialist,
            HarnessRole::Verifier,
            HarnessRole::Reviewer,
        ],
        vault_read: &[HarnessRole::Specialist, HarnessRole::Reviewer],
    };

    fn for_profile(execution_profile: HarnessExecutionProfile) -> &'static Self {
        match execution_profile {
            HarnessExecutionProfile::Standard => &Self::STANDARD,
            HarnessExecutionProfile::Strict => &Self::STRICT,
        }
    }

    fn roles_for(&self, action: HarnessAction) -> &'static [HarnessRole] {
        match action {
            HarnessAction::CodeWrite
            | HarnessAction::DocumentWrite
            | HarnessAction::VaultCuration => self.write,
            HarnessAction::CodeReview | HarnessAction::DocumentReview => self.review,
            HarnessAction::Investigation | HarnessAction::Design | HarnessAction::Ideation => {
                self.specialist_review
            }
            HarnessAction::VaultRead => self.vault_read,
        }
    }
}

fn required_concurrent_roles_for(_action: HarnessAction, _roles: &[HarnessRole]) -> usize {
    // Concurrency is an execution optimization; independence is enforced by separate contexts.
    1
}

fn action_requires_tool_execution(action: HarnessAction) -> bool {
    matches!(
        action,
        HarnessAction::CodeWrite | HarnessAction::CodeReview | HarnessAction::VaultCuration
    )
}

#[cfg(test)]
fn validate_career_receipt_set(
    manifest: &CareerCompositionManifest,
    holistic_review: &CareerReviewReceipt,
    evidence_reviews: &[(DataOwner, CareerReviewReceipt)],
) -> HarnessResult<Vec<TargetBinding>> {
    let expected_manifest = Some(manifest.clone());
    if holistic_review.resolved.career_composition_manifest != expected_manifest
        || evidence_reviews.iter().any(|(_, receipt)| {
            receipt.resolved.career_composition_manifest.as_ref() != Some(manifest)
        })
    {
        return Err(HarnessError::InvalidSubmission(
            "all career review receipts must bind the exact supplied manifest".to_owned(),
        ));
    }
    manifest.validate(
        &holistic_review.resolved.request,
        holistic_review.resolved.intent.career_surface(),
    )?;
    let expected_owners = manifest
        .evidence_owners
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let actual_owners = evidence_reviews
        .iter()
        .map(|(owner, _)| owner.clone())
        .collect::<BTreeSet<_>>();
    if actual_owners.len() != evidence_reviews.len() || actual_owners != expected_owners {
        return Err(HarnessError::InvalidSubmission(
            "owner review receipts must exactly cover the manifest evidence owners".to_owned(),
        ));
    }
    let artifact_targets = holistic_review.resolved.plan.targets.clone();
    if evidence_reviews.iter().any(|(_, receipt)| {
        receipt.resolved.plan.targets != artifact_targets
            || receipt.artifact_set_digest != holistic_review.artifact_set_digest
    }) {
        return Err(HarnessError::InvalidSubmission(
            "all career review receipts must bind the same final artifact set".to_owned(),
        ));
    }
    let mut context_ids = BTreeSet::new();
    for receipt in
        std::iter::once(holistic_review).chain(evidence_reviews.iter().map(|(_, receipt)| receipt))
    {
        for role_context in &receipt.evaluation.reported_role_contexts {
            if !context_ids.insert(role_context.context_id.clone()) {
                return Err(HarnessError::InvalidSubmission(
                    "career review role context IDs must be globally disjoint".to_owned(),
                ));
            }
        }
    }
    Ok(artifact_targets)
}

fn validate_career_execution_receipt_set(
    manifest: &CareerCompositionManifest,
    holistic_review: &CareerExecutionReviewReceipt,
    evidence_reviews: &[(DataOwner, CareerExecutionReviewReceipt)],
) -> HarnessResult<Vec<TargetBinding>> {
    let expected_manifest = Some(manifest.clone());
    if holistic_review.resolved.career_composition_manifest != expected_manifest
        || evidence_reviews.iter().any(|(_, receipt)| {
            receipt.resolved.career_composition_manifest.as_ref() != Some(manifest)
        })
    {
        return Err(HarnessError::InvalidSubmission(
            "all career execution receipts must bind the exact supplied manifest".to_owned(),
        ));
    }
    manifest.validate(
        &holistic_review.resolved.request,
        holistic_review.resolved.intent.career_surface(),
    )?;
    let expected_owners = manifest
        .evidence_owners
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let actual_owners = evidence_reviews
        .iter()
        .map(|(owner, _)| owner.clone())
        .collect::<BTreeSet<_>>();
    if actual_owners.len() != evidence_reviews.len() || actual_owners != expected_owners {
        return Err(HarnessError::InvalidSubmission(
            "owner execution receipts must exactly cover the manifest evidence owners".to_owned(),
        ));
    }
    let artifact_targets = holistic_review.resolved.plan.targets.clone();
    if evidence_reviews.iter().any(|(_, receipt)| {
        receipt.resolved.plan.targets != artifact_targets
            || receipt.artifact_set_digest != holistic_review.artifact_set_digest
    }) {
        return Err(HarnessError::InvalidSubmission(
            "all career execution receipts must bind the same final artifact set".to_owned(),
        ));
    }
    let mut context_ids = BTreeSet::new();
    for receipt in
        std::iter::once(holistic_review).chain(evidence_reviews.iter().map(|(_, receipt)| receipt))
    {
        for role_execution in &receipt.evaluation.role_executions {
            if !context_ids.insert(role_execution.context_id.clone()) {
                return Err(HarnessError::InvalidSubmission(
                    "career execution role context IDs must be globally disjoint".to_owned(),
                ));
            }
        }
    }
    Ok(artifact_targets)
}

#[cfg(test)]
fn career_review_binding(
    evidence_owner: Option<DataOwner>,
    receipt: &CareerReviewReceipt,
) -> CareerReviewBinding {
    CareerReviewBinding {
        evidence_owner,
        resolved_plan_digest: receipt.resolved.resolved_plan_digest.clone(),
        prepared_role_run_digest: receipt.prepared.prepared_role_run_digest.clone(),
        candidate_digest: receipt.evaluation.candidate_digest.clone(),
        validation_receipt_digest: receipt.evaluation.validation_receipt_digest.clone(),
        artifact_set_digest: receipt.artifact_set_digest.clone(),
        evidence_bundle_digest: receipt.evidence_bundle_digest.clone(),
        receipt_digest: receipt.receipt_digest.clone(),
        reported_producer_context_id: receipt.evaluation.reported_producer_context_id.clone(),
        reported_reviewer_context_id: receipt.evaluation.reported_reviewer_context_id.clone(),
        reported_role_contexts: receipt.evaluation.reported_role_contexts.clone(),
        reported_role_lifecycles: receipt.evaluation.reported_role_lifecycles.clone(),
    }
}

const fn bound_document_repository(source: HarnessBoundDocumentSource) -> u8 {
    match source {
        HarnessBoundDocumentSource::Target | HarnessBoundDocumentSource::WorkspacePolicy => 0,
        HarnessBoundDocumentSource::Policy
        | HarnessBoundDocumentSource::Learning
        | HarnessBoundDocumentSource::Evidence
        | HarnessBoundDocumentSource::Curation
        | HarnessBoundDocumentSource::Retrieval => 1,
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "verification orchestration ordering must remain explicit for deterministic plans"
)]
fn verification_requirements_for(
    request: &HarnessRequest,
    intent: HarnessIntent,
    roles: &[HarnessRole],
) -> Vec<VerificationRequirement> {
    let action = request.action;
    let produced_subject = match action {
        HarnessAction::CodeReview | HarnessAction::DocumentReview => {
            EvaluationSubjectKind::FrozenTargets
        }
        _ => EvaluationSubjectKind::ProducedArtifact,
    };
    let review_owner = roles
        .iter()
        .copied()
        .find(|role| *role == HarnessRole::Reviewer)
        .unwrap_or_else(|| {
            primary_producer_role_for_action(action, roles)
                .expect("validated role profiles always contain one producer")
        });
    let verification_owner = roles
        .iter()
        .copied()
        .find(|role| *role == HarnessRole::Verifier)
        .unwrap_or(review_owner);
    let role_requirement = |unit, role| VerificationRequirement {
        unit,
        owner: VerificationOwner::Role { role },
        subject: produced_subject,
    };
    let mut requirements = vec![
        role_requirement(VerificationUnit::CompletionContract, review_owner),
        role_requirement(VerificationUnit::ScopeCompliance, review_owner),
        role_requirement(VerificationUnit::TerminologyAndReadability, review_owner),
    ];
    match action {
        HarnessAction::CodeWrite | HarnessAction::CodeReview => {
            requirements.push(role_requirement(
                VerificationUnit::CodeCorrectness,
                verification_owner,
            ));
            requirements.push(VerificationRequirement {
                unit: VerificationUnit::TestsAndStaticAnalysis,
                owner: VerificationOwner::Tool,
                subject: produced_subject,
            });
        }
        HarnessAction::DocumentWrite | HarnessAction::DocumentReview => {
            requirements.push(role_requirement(
                VerificationUnit::SourceOwnership,
                review_owner,
            ));
            requirements.push(role_requirement(
                VerificationUnit::FactAndClaimBoundary,
                review_owner,
            ));
        }
        HarnessAction::Investigation | HarnessAction::Design => {
            requirements.push(role_requirement(
                VerificationUnit::EvidenceAndUncertainty,
                verification_owner,
            ));
        }
        HarnessAction::Ideation => {
            requirements.push(role_requirement(
                VerificationUnit::IdeaNotPromotedToDecision,
                review_owner,
            ));
            if uses_solo_mvp_ideation_contract(request, intent) {
                requirements.push(role_requirement(
                    VerificationUnit::EvidenceAndUncertainty,
                    verification_owner,
                ));
            }
        }
        HarnessAction::VaultRead => {
            requirements.push(VerificationRequirement {
                unit: VerificationUnit::RetrievalScopeAndDigest,
                owner: VerificationOwner::Deterministic,
                subject: EvaluationSubjectKind::TaskContract,
            });
        }
        HarnessAction::VaultCuration => {
            requirements.push(role_requirement(
                VerificationUnit::CurationProvenance,
                review_owner,
            ));
            requirements.push(role_requirement(
                VerificationUnit::SourceAndOntologyBoundary,
                review_owner,
            ));
            if uses_solo_mvp_ideation_contract(request, intent) {
                requirements.push(role_requirement(
                    VerificationUnit::IdeaNotPromotedToDecision,
                    review_owner,
                ));
                requirements.push(role_requirement(
                    VerificationUnit::EvidenceAndUncertainty,
                    verification_owner,
                ));
            }
        }
    }
    if uses_solo_mvp_ideation_contract(request, intent) {
        requirements.push(role_requirement(
            VerificationUnit::SoloMvpIdeationContract,
            review_owner,
        ));
    }
    if let HarnessIntent::CareerArtifact { surface } = intent {
        requirements.extend([
            role_requirement(VerificationUnit::CareerEvidenceLineage, review_owner),
            role_requirement(VerificationUnit::CareerSurfaceContract, review_owner),
            role_requirement(VerificationUnit::CareerPerspectiveRouting, review_owner),
            role_requirement(VerificationUnit::CareerPublicSafety, review_owner),
        ]);
        requirements.push(role_requirement(
            match surface {
                CareerOutputSurface::General => VerificationUnit::CareerOutputSurfaceSelection,
                CareerOutputSurface::Resume => VerificationUnit::ResumeFirstScreenAndArtifact,
                CareerOutputSurface::CareerDescription => {
                    VerificationUnit::CareerDescriptionCaseStructure
                }
                CareerOutputSurface::Portfolio => {
                    VerificationUnit::PortfolioLocalBuildAndPublicCopy
                }
                CareerOutputSurface::ProfessionalProfile => {
                    VerificationUnit::ProfessionalProfileArtifact
                }
            },
            review_owner,
        ));
    } else if let HarnessIntent::OutputAdapter { surface } = intent {
        requirements.push(role_requirement(
            VerificationUnit::OutputAdapterContract,
            review_owner,
        ));
        requirements.push(role_requirement(
            match surface {
                CareerOutputSurface::General => VerificationUnit::OutputAdapterSurfaceSelection,
                CareerOutputSurface::Resume => VerificationUnit::ResumeOutputAdapterArtifact,
                CareerOutputSurface::CareerDescription => {
                    VerificationUnit::CareerDescriptionOutputAdapterArtifact
                }
                CareerOutputSurface::Portfolio => {
                    VerificationUnit::PortfolioLocalBuildAndPublicCopy
                }
                CareerOutputSurface::ProfessionalProfile => {
                    VerificationUnit::ProfessionalProfileOutputAdapterArtifact
                }
            },
            review_owner,
        ));
    }
    requirements
}

#[cfg(test)]
fn evaluated_single_change(evaluated: &HarnessEvaluationResult) -> HarnessResult<FileChange> {
    let changes = match &evaluated.artifact {
        SubmissionArtifact::Changes { changes } => changes.clone(),
        SubmissionArtifact::Curation { entries, .. } => {
            entries.iter().map(|entry| entry.change.clone()).collect()
        }
        _ => {
            return Err(HarnessError::InvalidSubmission(
                "only file-change submissions can be applied".to_owned(),
            ));
        }
    };
    let [change] = changes.as_slice() else {
        return Err(HarnessError::InvalidSubmission(
            "atomic apply currently requires exactly one file change".to_owned(),
        ));
    };
    Ok(change.clone())
}

fn role_requirements(
    plan: &ResolvedHarnessPlan,
    role: HarnessRole,
) -> Vec<VerificationRequirement> {
    plan.verification_requirements
        .iter()
        .copied()
        .filter(|requirement| requirement.owner == VerificationOwner::Role { role })
        .collect()
}

fn career_claim_lineage_for_role(resolved: &ResolvedHarnessRequest) -> Vec<CareerClaimLineage> {
    resolved
        .career_composition_manifest
        .as_ref()
        .and_then(|manifest| {
            resolved
                .context_grants
                .first()
                .map(|grant| (manifest, &grant.owner))
        })
        .map_or_else(Vec::new, |(manifest, owner)| {
            manifest
                .claim_lineage
                .iter()
                .filter(|lineage| &lineage.evidence_owner == owner)
                .cloned()
                .collect()
        })
}

fn role_task_contract(plan: &ResolvedHarnessPlan, role: HarnessRole) -> RoleTaskContract {
    let requirements = role_requirements(plan, role);
    match role {
        HarnessRole::Writer => RoleTaskContract::Writer {
            targets: plan.targets.clone(),
            curation_kind: plan.curation_kind,
        },
        HarnessRole::Specialist => RoleTaskContract::Specialist {
            source_targets: plan.targets.clone(),
            verification_requirements: requirements,
        },
        HarnessRole::Verifier => RoleTaskContract::Verifier {
            targets: plan.targets.clone(),
            verification_requirements: requirements,
        },
        HarnessRole::Reviewer => RoleTaskContract::Reviewer {
            targets: plan.targets.clone(),
            verification_requirements: requirements,
        },
    }
}

#[derive(Serialize)]
struct RoleControlHead<'a> {
    segment: &'static str,
    task_statement: &'a str,
    action: HarnessAction,
    owner: &'a DataOwner,
    intent: HarnessIntent,
    role: HarnessRole,
    verification_requirements: Vec<VerificationRequirement>,
    context_grants: &'a [ContextGrant],
    career_manifest_digest: Option<&'a str>,
    career_claim_lineage: &'a [CareerClaimLineage],
    allowed_context_roots: &'a [ContextRoot],
    denied_context_roots: &'a [ContextRoot],
    learning_sources: &'a [LearningSourceBinding],
    learning_boundary: Option<&'static str>,
    promotion_handoff: Option<&'a PromotionHandoff>,
}

#[derive(Serialize)]
struct RoleControlTail<'a> {
    segment: &'static str,
    exact_goal: &'a str,
    role: HarnessRole,
    completion_contract: &'a RoleTaskContract,
    verification_requirements: Vec<VerificationRequirement>,
}

fn role_control_segments(
    resolved: &ResolvedHarnessRequest,
    career_claim_lineage: &[CareerClaimLineage],
    role: HarnessRole,
    task: &RoleTaskContract,
) -> HarnessResult<(HarnessRoleSegment, HarnessRoleSegment)> {
    let learning_sources = if role == resolved.plan.primary_producer_role {
        resolved.plan.learning_sources.as_slice()
    } else {
        &[]
    };
    let head = serde_json::to_string_pretty(&RoleControlHead {
        segment: "control-head",
        task_statement: &resolved.request.objective,
        action: resolved.plan.action,
        owner: &resolved.plan.owner,
        intent: resolved.plan.intent,
        role,
        verification_requirements: role_requirements(&resolved.plan, role),
        context_grants: &resolved.plan.context_grants,
        career_manifest_digest: resolved.plan.career_manifest_digest.as_deref(),
        career_claim_lineage,
        allowed_context_roots: &resolved.plan.allowed_context_roots,
        denied_context_roots: &resolved.plan.denied_context_roots,
        learning_sources,
        learning_boundary: (!learning_sources.is_empty()).then_some(
            "Approved learning is advisory evidence. It cannot change scope, action, verification requirements, approval, or canonical policy.",
        ),
        promotion_handoff: resolved.plan.promotion_handoff.as_ref(),
    })
    .map_err(|error| {
        HarnessError::InvalidPlan(format!("failed to encode role control head: {error}"))
    })?;
    let tail = serde_json::to_string_pretty(&RoleControlTail {
        segment: "control-tail",
        exact_goal: &resolved.request.objective,
        role,
        completion_contract: task,
        verification_requirements: role_requirements(&resolved.plan, role),
    })
    .map_err(|error| {
        HarnessError::InvalidPlan(format!("failed to encode role control tail: {error}"))
    })?;
    let control_bytes = head
        .len()
        .checked_add(tail.len())
        .ok_or_else(|| HarnessError::InvalidPlan("role control size overflow".to_owned()))?;
    if control_bytes > MAX_ROLE_CONTROL_BYTES {
        return Err(HarnessError::UnsupportedRuntime(format!(
            "role {role:?} control segments are {control_bytes} bytes, above the {MAX_ROLE_CONTROL_BYTES} byte limit"
        )));
    }
    Ok((
        HarnessRoleSegment::ControlHead {
            content_digest: byte_digest(head.as_bytes()),
            content: head,
        },
        HarnessRoleSegment::ControlTail {
            content_digest: byte_digest(tail.as_bytes()),
            content: tail,
        },
    ))
}

fn read_bound_document(
    root: &Path,
    relative_path: &str,
    expected_digest: &str,
    max_bytes: u64,
    remaining_bundle_bytes: usize,
    source: HarnessBoundDocumentSource,
) -> HarnessResult<HarnessBoundDocument> {
    let relative = Path::new(relative_path);
    let path = root.join(relative);
    let file = open_verified_file(root, relative)?;
    read_bound_document_from_file(
        file,
        &path,
        relative_path,
        expected_digest,
        max_bytes,
        remaining_bundle_bytes,
        source,
    )
}

fn read_bound_document_from_file(
    file: File,
    path: &Path,
    relative_path: &str,
    expected_digest: &str,
    max_bytes: u64,
    remaining_bundle_bytes: usize,
    source: HarnessBoundDocumentSource,
) -> HarnessResult<HarnessBoundDocument> {
    let metadata = file.metadata().map_err(|source| HarnessError::FileRead {
        path: path.to_path_buf(),
        message: source.to_string(),
    })?;
    if metadata.len() > max_bytes {
        return Err(HarnessError::InvalidRepository(format!(
            "file `{}` exceeds the {max_bytes} byte limit",
            path.display()
        )));
    }
    let remaining_bundle_bytes = u64::try_from(remaining_bundle_bytes).map_err(|_| {
        HarnessError::UnsupportedRuntime("remaining role bundle size overflow".to_owned())
    })?;
    if metadata.len() > remaining_bundle_bytes {
        return Err(HarnessError::UnsupportedRuntime(format!(
            "document `{relative_path}` exceeds the remaining role bundle content budget of {remaining_bundle_bytes} bytes"
        )));
    }
    let read_limit = max_bytes.min(remaining_bundle_bytes);
    let (content, content_digest) = read_text_file(file, read_limit, path).map_err(|error| {
        if read_limit < max_bytes
            && matches!(&error, HarnessError::InvalidRepository(message) if message.contains("byte limit"))
        {
            HarnessError::UnsupportedRuntime(format!(
                "document `{relative_path}` exceeds the remaining role bundle content budget of {remaining_bundle_bytes} bytes"
            ))
        } else {
            error
        }
    })?;
    if content_digest != expected_digest {
        return Err(HarnessError::PlanDrift {
            expected: expected_digest.to_owned(),
            actual: content_digest,
        });
    }
    Ok(HarnessBoundDocument {
        source,
        relative_path: relative_path.to_owned(),
        content_digest: expected_digest.to_owned(),
        start_line: 1,
        end_line: content.split_inclusive('\n').count().max(1),
        matched_terms: Vec::new(),
        content,
    })
}

fn push_bound_document(
    documents: &mut Vec<HarnessBoundDocument>,
    remaining_bundle_bytes: &mut usize,
    root: &Path,
    relative_path: &str,
    expected_digest: &str,
    max_bytes: u64,
    source: HarnessBoundDocumentSource,
) -> HarnessResult<()> {
    if contains_bound_document(
        documents,
        source,
        relative_path,
        expected_digest,
        1,
        usize::MAX,
    ) {
        return Ok(());
    }
    documents.push(read_bound_document(
        root,
        relative_path,
        expected_digest,
        max_bytes,
        *remaining_bundle_bytes,
        source,
    )?);
    *remaining_bundle_bytes = remaining_bundle_bytes
        .checked_sub(
            documents
                .last()
                .map_or(0, |document| document.content.len()),
        )
        .ok_or_else(|| {
            HarnessError::UnsupportedRuntime(
                "mandatory role bundle content size overflow".to_owned(),
            )
        })?;
    Ok(())
}

fn contains_bound_document(
    documents: &[HarnessBoundDocument],
    source: HarnessBoundDocumentSource,
    relative_path: &str,
    content_digest: &str,
    start_line: usize,
    end_line: usize,
) -> bool {
    let repository = bound_document_repository(source);
    documents.iter().any(|document| {
        bound_document_repository(document.source) == repository
            && document.relative_path == relative_path
            && document.content_digest == content_digest
            && (document.matched_terms.is_empty()
                || end_line == usize::MAX
                || (document.start_line == start_line && document.end_line == end_line))
    })
}

fn finalize_role_bundle(
    resolved_plan_digest: &str,
    role: HarnessRole,
    control_head: HarnessRoleSegment,
    documents: Vec<HarnessBoundDocument>,
    control_tail: HarnessRoleSegment,
    max_role_bundle_bytes: usize,
) -> HarnessResult<HarnessRoleBundle> {
    let mut segments = Vec::with_capacity(documents.len() + 2);
    segments.push(control_head);
    segments.extend(
        documents
            .into_iter()
            .map(|document| HarnessRoleSegment::BoundDocument { document }),
    );
    segments.push(control_tail);
    let total_content_bytes = segments.iter().try_fold(0_usize, |total, segment| {
        let content_bytes = match segment {
            HarnessRoleSegment::ControlHead { content, .. }
            | HarnessRoleSegment::ControlTail { content, .. } => content.len(),
            HarnessRoleSegment::BoundDocument { document } => document.content.len(),
        };
        total.checked_add(content_bytes).ok_or_else(|| {
            HarnessError::UnsupportedRuntime("role bundle content size overflow".to_owned())
        })
    })?;
    let digest_payload =
        serde_json::to_vec(&(resolved_plan_digest, role, &segments, total_content_bytes)).map_err(
            |error| {
                HarnessError::InvalidPlan(format!("failed to encode role bundle payload: {error}"))
            },
        )?;
    let mut bundle = HarnessRoleBundle {
        resolved_plan_digest: resolved_plan_digest.to_owned(),
        role,
        segments,
        total_content_bytes,
        serialized_bytes: 0,
        bundle_digest: byte_digest(&digest_payload),
    };
    for _ in 0..8 {
        let serialized_bytes = serde_json::to_vec(&bundle)
            .map_err(|error| {
                HarnessError::InvalidPlan(format!("failed to encode role bundle: {error}"))
            })?
            .len();
        if serialized_bytes == bundle.serialized_bytes {
            break;
        }
        bundle.serialized_bytes = serialized_bytes;
    }
    let verified_serialized_bytes = serde_json::to_vec(&bundle)
        .map_err(|error| {
            HarnessError::InvalidPlan(format!("failed to verify role bundle size: {error}"))
        })?
        .len();
    if verified_serialized_bytes != bundle.serialized_bytes {
        return Err(HarnessError::InvalidPlan(
            "role bundle serialized size did not stabilize".to_owned(),
        ));
    }
    if bundle.serialized_bytes > max_role_bundle_bytes {
        return Err(HarnessError::UnsupportedRuntime(format!(
            "role {role:?} bundle is {} serialized bytes, but the runtime limit is {max_role_bundle_bytes}",
            bundle.serialized_bytes
        )));
    }
    Ok(bundle)
}

mod repository;
#[cfg(test)]
use repository::apply_single_change_reported;
use repository::{
    bind_target, canonical_directory, curation_root, digest_file, ensure_no_symlink_components,
    open_verified_file, read_text_file, validate_curation_targets,
};

fn lock_confirmation_receipt(
    confirmation: repository::LockLifecycleConfirmation,
) -> LifecycleConfirmationReceipt {
    match confirmation {
        repository::LockLifecycleConfirmation::Confirmed => LifecycleConfirmationReceipt::Confirmed,
        repository::LockLifecycleConfirmation::Unconfirmed => {
            LifecycleConfirmationReceipt::Unconfirmed
        }
    }
}

fn apply_confirmation_receipt(
    confirmation: repository::ApplyConfirmation,
) -> LifecycleConfirmationReceipt {
    match confirmation {
        repository::ApplyConfirmation::NotRequired => LifecycleConfirmationReceipt::NotRequired,
        repository::ApplyConfirmation::Confirmed => LifecycleConfirmationReceipt::Confirmed,
        repository::ApplyConfirmation::Unconfirmed => LifecycleConfirmationReceipt::Unconfirmed,
    }
}

fn lock_lifecycle_receipt(
    report: &repository::WorkspaceMutationLockReport,
) -> WorkspaceMutationLockReceipt {
    WorkspaceMutationLockReceipt {
        lock_file_creation_committed: report.lock_file_creation_committed,
        lock_content_durability: lock_confirmation_receipt(report.lock_content_durability),
        creation_parent_durability: lock_confirmation_receipt(report.creation_parent_durability),
        explicit_unlink_committed: report.explicit_unlink_committed,
        verified_final_absence: lock_confirmation_receipt(report.verified_final_absence),
        unlink_parent_durability: lock_confirmation_receipt(report.unlink_parent_durability),
        failures: report
            .failures
            .iter()
            .map(|failure| LifecycleFailureReceipt {
                stage: failure.stage.clone(),
                message: failure.message.clone(),
            })
            .collect(),
    }
}

fn repository_apply_receipt(report: &repository::RepositoryApplyReport) -> RepositoryApplyReceipt {
    let verified_target_state = match &report.verified_target_state {
        repository::VerifiedTargetState::Unverified => VerifiedTargetStateReceipt::Unverified,
        repository::VerifiedTargetState::Present { content_digest } => {
            VerifiedTargetStateReceipt::Present {
                content_digest: content_digest.clone(),
            }
        }
        repository::VerifiedTargetState::Absent => VerifiedTargetStateReceipt::Absent,
    };
    let created_parent_directory_state = match report.created_parent_directory_state {
        repository::CreatedParentDirectoryState::NotCreated => {
            CreatedParentDirectoryStateReceipt::NotCreated
        }
        repository::CreatedParentDirectoryState::Retained => {
            CreatedParentDirectoryStateReceipt::Retained
        }
        repository::CreatedParentDirectoryState::RolledBack => {
            CreatedParentDirectoryStateReceipt::RolledBack
        }
        repository::CreatedParentDirectoryState::Unconfirmed => {
            CreatedParentDirectoryStateReceipt::Unconfirmed
        }
    };
    RepositoryApplyReceipt {
        workspace_relative_path: report.workspace_relative_path.clone(),
        operation: report.operation,
        filesystem_mutation_occurred: report.filesystem_mutation_occurred,
        target_mutation_committed: report.target_mutation_committed,
        verified_target_state,
        file_durability: apply_confirmation_receipt(report.file_durability),
        parent_directory_durability: apply_confirmation_receipt(report.parent_directory_durability),
        temporary_cleanup: apply_confirmation_receipt(report.temporary_cleanup),
        created_parent_directories: report.created_parent_directories.clone(),
        created_parent_directory_state,
        failures: report
            .failures
            .iter()
            .map(|failure| LifecycleFailureReceipt {
                stage: failure.stage.clone(),
                message: failure.message.clone(),
            })
            .collect(),
    }
}

fn batch_apply_lifecycle_receipt(
    report: repository::BatchApplyReport,
    lock: WorkspaceMutationLockReceipt,
) -> HarnessBatchApplyLifecycleReceipt {
    let outcome = match report.outcome {
        repository::BatchApplyOutcome::Applied => BatchApplyOutcomeReceipt::Applied,
        repository::BatchApplyOutcome::RolledBack => BatchApplyOutcomeReceipt::RolledBack,
        repository::BatchApplyOutcome::RecoveryRequired => {
            BatchApplyOutcomeReceipt::RecoveryRequired
        }
    };
    let targets = report
        .targets
        .into_iter()
        .map(|target| BatchTargetApplyReceipt {
            workspace_relative_path: target.workspace_relative_path,
            operation: target.operation,
            original_content_digest: target.original_content_digest,
            intended_content_digest: target.intended_content_digest,
            staged_relative_path: target.staged_relative_path,
            backup_relative_path: target.backup_relative_path,
            state: match target.state {
                repository::BatchTargetApplyState::Pending => BatchTargetApplyStateReceipt::Pending,
                repository::BatchTargetApplyState::Staged => BatchTargetApplyStateReceipt::Staged,
                repository::BatchTargetApplyState::BackedUp => {
                    BatchTargetApplyStateReceipt::BackedUp
                }
                repository::BatchTargetApplyState::Applied => BatchTargetApplyStateReceipt::Applied,
                repository::BatchTargetApplyState::RolledBack => {
                    BatchTargetApplyStateReceipt::RolledBack
                }
                repository::BatchTargetApplyState::RecoveryRequired => {
                    BatchTargetApplyStateReceipt::RecoveryRequired
                }
            },
            repository_apply: target
                .repository_apply
                .as_ref()
                .map(repository_apply_receipt),
            failures: target
                .failures
                .into_iter()
                .map(|failure| LifecycleFailureReceipt {
                    stage: failure.stage,
                    message: failure.message,
                })
                .collect(),
        })
        .collect();
    let failures = report
        .failures
        .into_iter()
        .map(|failure| LifecycleFailureReceipt {
            stage: failure.stage,
            message: failure.message,
        })
        .collect::<Vec<_>>();
    let (failure_history, orchestration_failures) =
        if outcome == BatchApplyOutcomeReceipt::RecoveryRequired {
            (Vec::new(), failures)
        } else {
            (failures, Vec::new())
        };
    HarnessBatchApplyLifecycleReceipt {
        batch_id: report.batch_id,
        resolved_plan_digest: report.resolved_plan_digest,
        candidate_digest: report.candidate_digest,
        journal_relative_path: report.journal_relative_path,
        completion_receipt_relative_path: report.completion_receipt_relative_path,
        journal_retained: report.journal_retained,
        outcome,
        lock,
        targets,
        failure_history,
        orchestration_failures,
    }
}

fn with_reported_workspace_lock<T>(
    policy: repository::WorkspacePolicy,
    workspace_root: &Path,
    operation_stage: &str,
    operation: impl FnOnce() -> HarnessResult<T>,
) -> HarnessResult<T> {
    let acquisition =
        repository::WorkspaceMutationLock::acquire_reported_with_policy(policy, workspace_root)?;
    let Some(mutation_lock) = acquisition.lock else {
        let lock = lock_lifecycle_receipt(&acquisition.report);
        let mut orchestration_failures = vec![LifecycleFailureReceipt {
            stage: "workspace-lock-acquisition".to_owned(),
            message: "reported lock acquisition did not produce a usable lock".to_owned(),
        }];
        if !lock.release_cleanup_confirmed() {
            orchestration_failures.push(LifecycleFailureReceipt {
                stage: "workspace-lock-release".to_owned(),
                message: "failed lock acquisition cleanup was not fully confirmed".to_owned(),
            });
        }
        return Err(HarnessError::ApplyLifecycle {
            message: "workspace mutation lock acquisition was not fully confirmed".to_owned(),
            receipt: Box::new(HarnessApplyLifecycleReceipt {
                lock,
                target_apply: None,
                orchestration_failures,
            }),
        });
    };

    let operation_result = operation();
    let lock_receipt = lock_lifecycle_receipt(&mutation_lock.release());
    let mut orchestration_failures = Vec::new();
    let operation_value = match operation_result {
        Ok(value) => Some(value),
        Err(error) => {
            orchestration_failures.push(LifecycleFailureReceipt {
                stage: operation_stage.to_owned(),
                message: error.to_string(),
            });
            None
        }
    };
    if !lock_receipt.is_fully_confirmed() {
        orchestration_failures.push(LifecycleFailureReceipt {
            stage: "workspace-lock-release".to_owned(),
            message: "workspace mutation lock release was not fully confirmed".to_owned(),
        });
    }
    if let Some(value) = operation_value
        && orchestration_failures.is_empty()
    {
        return Ok(value);
    }
    Err(HarnessError::ApplyLifecycle {
        message:
            "reported workspace-lock operation did not complete with a fully confirmed lifecycle"
                .to_owned(),
        receipt: Box::new(HarnessApplyLifecycleReceipt {
            lock: lock_receipt,
            target_apply: None,
            orchestration_failures,
        }),
    })
}

fn validate_company_id(company: &str) -> HarnessResult<()> {
    validate_partition_id("company", company)
}

fn validate_partition_id(field: &str, value: &str) -> HarnessResult<()> {
    validate_single_line(field, value, MAX_COMPANY_LENGTH)?;
    if value.starts_with('-')
        || value.ends_with('-')
        || !value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
    {
        return Err(HarnessError::InvalidRequest(format!(
            "{field} must be a lowercase kebab-case identifier"
        )));
    }
    Ok(())
}

fn validate_relative_path(path: &Path, field: &str) -> HarnessResult<()> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(HarnessError::InvalidRequest(format!(
            "{field} must be a non-empty relative path"
        )));
    }
    for component in path.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(HarnessError::InvalidRequest(format!(
                "{field} must not contain `.`, `..`, root, or prefix components"
            )));
        }
    }
    Ok(())
}

fn validate_single_line(field: &str, value: &str, max_length: usize) -> HarnessResult<()> {
    validate_required_text(field, value, max_length)?;
    if value.chars().any(char::is_control) {
        return Err(HarnessError::InvalidRequest(format!(
            "{field} must not contain control characters"
        )));
    }
    Ok(())
}

fn validate_multiline(field: &str, value: &str, max_length: usize) -> HarnessResult<()> {
    validate_required_text(field, value, max_length)?;
    if value
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        return Err(HarnessError::InvalidRequest(format!(
            "{field} contains an unsupported control character"
        )));
    }
    Ok(())
}

fn validate_required_text(field: &str, value: &str, max_length: usize) -> HarnessResult<()> {
    if value.trim().is_empty() {
        return Err(HarnessError::InvalidRequest(format!(
            "{field} must not be empty"
        )));
    }
    if value.chars().count() > max_length {
        return Err(HarnessError::InvalidRequest(format!(
            "{field} must not exceed {max_length} characters"
        )));
    }
    Ok(())
}

fn validate_runtime_text(field: &str, value: &str) -> HarnessResult<()> {
    if value.trim().is_empty()
        || value.len() > MAX_SUBMISSION_TEXT_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(HarnessError::InvalidSubmission(format!(
            "{field} must be a non-empty single line no larger than {MAX_SUBMISSION_TEXT_BYTES} bytes"
        )));
    }
    Ok(())
}

#[cfg(test)]
fn validate_reported_context_ids(
    plan: &ResolvedHarnessPlan,
    producer_context_id: &str,
    reviewer_context_id: &str,
    role_contexts: &[ReportedRoleContext],
) -> HarnessResult<()> {
    validate_runtime_text("reported producer context ID", producer_context_id)?;
    validate_reported_role_contexts(plan, role_contexts)?;
    let producer_role = plan.primary_producer_role;
    let producer_role_context_id = reported_role_context_id(role_contexts, producer_role)?;
    if producer_context_id != producer_role_context_id {
        return Err(HarnessError::InvalidSubmission(
            "reported producer context ID must match the primary producer role context ID"
                .to_owned(),
        ));
    }
    if plan.contains_role(HarnessRole::Reviewer) {
        validate_runtime_text("reported reviewer context ID", reviewer_context_id)?;
        let reviewer_role_context_id =
            reported_role_context_id(role_contexts, HarnessRole::Reviewer)?;
        if reviewer_context_id != reviewer_role_context_id {
            return Err(HarnessError::InvalidSubmission(
                "reported reviewer context ID must match the reviewer role context ID".to_owned(),
            ));
        }
        if producer_role != HarnessRole::Reviewer && producer_context_id == reviewer_context_id {
            return Err(HarnessError::InvalidSubmission(
                "producer and reviewer context IDs must be different".to_owned(),
            ));
        }
    } else if !reviewer_context_id.is_empty() {
        return Err(HarnessError::InvalidSubmission(
            "plans without a reviewer role require an empty reviewer context ID".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
fn validate_reported_role_contexts(
    plan: &ResolvedHarnessPlan,
    role_contexts: &[ReportedRoleContext],
) -> HarnessResult<()> {
    if role_contexts.len() != plan.role_count() {
        return Err(HarnessError::InvalidSubmission(
            "reported role context IDs must include every planned role".to_owned(),
        ));
    }
    let mut observed_context_ids = BTreeSet::new();
    for (expected_role, reported) in plan.roles().zip(role_contexts) {
        if reported.role != expected_role {
            return Err(HarnessError::InvalidSubmission(
                "reported role context IDs must match the planned role order".to_owned(),
            ));
        }
        validate_runtime_text(
            &format!("reported {expected_role:?} context ID"),
            &reported.context_id,
        )?;
        if !observed_context_ids.insert(reported.context_id.as_str()) {
            return Err(HarnessError::InvalidSubmission(
                "reported role context IDs must be unique".to_owned(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
fn validate_reported_role_lifecycles(
    plan: &ResolvedHarnessPlan,
    capabilities: &RoleRuntimeCapabilities,
    role_contexts: &[ReportedRoleContext],
    lifecycles: &[ReportedRoleLifecycle],
) -> HarnessResult<()> {
    if lifecycles.len() != plan.role_count() {
        return Err(HarnessError::InvalidSubmission(
            "role lifecycle receipts must include every planned role".to_owned(),
        ));
    }
    for ((expected_role, role_context), lifecycle) in
        plan.roles().zip(role_contexts).zip(lifecycles)
    {
        if lifecycle.role != expected_role || lifecycle.context_id != role_context.context_id {
            return Err(HarnessError::InvalidSubmission(
                "role lifecycle receipts must match role order and context IDs".to_owned(),
            ));
        }
        let Some(first_output) = lifecycle.first_output_at_millis else {
            return Err(HarnessError::UnsupportedRuntime(format!(
                "role {expected_role:?} produced no usable output before termination"
            )));
        };
        if lifecycle.started_at_millis == 0
            || lifecycle.started_at_millis > lifecycle.context_ready_at_millis
            || lifecycle.context_ready_at_millis > first_output
            || first_output > lifecycle.terminal_at_millis
            || lifecycle.terminal_at_millis > lifecycle.closed_at_millis
        {
            return Err(HarnessError::InvalidSubmission(format!(
                "role {expected_role:?} lifecycle timestamps are not monotonic"
            )));
        }
        match (
            lifecycle.interrupt_requested_at_millis,
            lifecycle.grace_deadline_at_millis,
        ) {
            (None, None) => {
                let execution_duration = lifecycle
                    .terminal_at_millis
                    .checked_sub(lifecycle.started_at_millis)
                    .ok_or_else(|| {
                        HarnessError::InvalidSubmission(format!(
                            "role {expected_role:?} execution timestamps are invalid"
                        ))
                    })?;
                if execution_duration > capabilities.max_role_execution_millis {
                    return Err(HarnessError::UnsupportedRuntime(format!(
                        "role {expected_role:?} exceeded the runtime execution limit of {} milliseconds",
                        capabilities.max_role_execution_millis
                    )));
                }
            }
            (Some(interrupt), Some(grace_deadline))
                if lifecycle.context_ready_at_millis <= interrupt
                    && interrupt <= grace_deadline
                    && interrupt <= lifecycle.terminal_at_millis
                    && lifecycle.terminal_at_millis <= grace_deadline =>
            {
                let execution_duration = interrupt
                    .checked_sub(lifecycle.started_at_millis)
                    .ok_or_else(|| {
                        HarnessError::InvalidSubmission(format!(
                            "role {expected_role:?} execution timestamps are invalid"
                        ))
                    })?;
                if execution_duration > capabilities.max_role_execution_millis {
                    return Err(HarnessError::UnsupportedRuntime(format!(
                        "role {expected_role:?} exceeded the runtime execution limit of {} milliseconds",
                        capabilities.max_role_execution_millis
                    )));
                }
                let grace_duration = grace_deadline.checked_sub(interrupt).ok_or_else(|| {
                    HarnessError::InvalidSubmission(format!(
                        "role {expected_role:?} grace timestamps are invalid"
                    ))
                })?;
                if grace_duration > capabilities.max_role_grace_millis {
                    return Err(HarnessError::UnsupportedRuntime(format!(
                        "role {expected_role:?} exceeded the runtime grace limit of {} milliseconds",
                        capabilities.max_role_grace_millis
                    )));
                }
            }
            _ => {
                return Err(HarnessError::InvalidSubmission(format!(
                    "role {expected_role:?} interrupt and grace timestamps are invalid"
                )));
            }
        }
        if lifecycle.terminal_state != RoleTerminalState::Completed {
            return Err(HarnessError::UnsupportedRuntime(format!(
                "role {expected_role:?} ended as {:?}",
                lifecycle.terminal_state
            )));
        }
    }
    Ok(())
}

fn primary_producer_role_for_action(
    action: HarnessAction,
    roles: &[HarnessRole],
) -> HarnessResult<HarnessRole> {
    let expected = match action {
        HarnessAction::CodeWrite | HarnessAction::DocumentWrite | HarnessAction::VaultCuration => {
            HarnessRole::Writer
        }
        HarnessAction::Investigation
        | HarnessAction::Design
        | HarnessAction::Ideation
        | HarnessAction::VaultRead => HarnessRole::Specialist,
        HarnessAction::CodeReview | HarnessAction::DocumentReview => HarnessRole::Reviewer,
    };
    if roles.contains(&expected) {
        Ok(expected)
    } else {
        Err(HarnessError::InvalidPlan(format!(
            "planned roles are missing the required {expected:?} producer"
        )))
    }
}

#[cfg(test)]
fn reported_role_context_id(
    role_contexts: &[ReportedRoleContext],
    role: HarnessRole,
) -> HarnessResult<&str> {
    role_contexts
        .iter()
        .find(|role_context| role_context.role == role)
        .map(|role_context| role_context.context_id.as_str())
        .ok_or_else(|| {
            HarnessError::InvalidSubmission(format!(
                "reported role context ID is missing for {role:?}"
            ))
        })
}

#[cfg(test)]
fn validate_checks(
    required: &[VerificationRequirement],
    checks: &[VerificationCheck],
) -> HarnessResult<()> {
    let required = required
        .iter()
        .filter(|requirement| requirement.owner != VerificationOwner::Deterministic)
        .collect::<Vec<_>>();
    if checks.len() > MAX_SUBMISSION_LIST_ITEMS {
        return Err(HarnessError::InvalidSubmission(format!(
            "verification checks exceed the {MAX_SUBMISSION_LIST_ITEMS} item limit"
        )));
    }
    if checks.len() != required.len() {
        return Err(HarnessError::InvalidSubmission(
            "verification checks must exactly match the planned check list".to_owned(),
        ));
    }
    let mut observed = BTreeSet::new();
    let mut total = 0_usize;
    for (requirement, check) in required.iter().zip(checks) {
        let required_id = requirement.unit.as_str();
        if check.id.trim().is_empty() || check.detail.trim().is_empty() {
            return Err(HarnessError::InvalidSubmission(
                "verification checks require a non-empty ID and detail".to_owned(),
            ));
        }
        if check.id != required_id {
            return Err(HarnessError::InvalidSubmission(format!(
                "verification check `{}` does not match planned check `{required_id}` in this position",
                check.id
            )));
        }
        if !observed.insert(check.id.as_str()) {
            return Err(HarnessError::InvalidSubmission(format!(
                "duplicate verification check `{}`",
                check.id
            )));
        }
        total = total
            .checked_add(check.id.len())
            .and_then(|value| value.checked_add(check.detail.len()))
            .ok_or_else(|| {
                HarnessError::InvalidSubmission("verification check size overflow".to_owned())
            })?;
        if total > MAX_SUBMISSION_TEXT_BYTES {
            return Err(HarnessError::InvalidSubmission(format!(
                "verification checks exceed the {MAX_SUBMISSION_TEXT_BYTES} byte limit"
            )));
        }
    }
    Ok(())
}

fn ensure_unique_paths(label: &str, roots: &[ContextRoot]) -> HarnessResult<()> {
    ensure_unique_strings(
        label,
        &roots
            .iter()
            .map(|root| root.repository_relative_path.clone())
            .collect::<Vec<_>>(),
    )
}

fn ensure_disjoint_context_roots(
    allowed: &[ContextRoot],
    denied: &[ContextRoot],
) -> HarnessResult<()> {
    for allowed_root in allowed {
        for denied_root in denied {
            if allowed_root.repository_relative_path == denied_root.repository_relative_path {
                return Err(HarnessError::InvalidPlan(format!(
                    "context root `{}` is both allowed and denied",
                    allowed_root.repository_relative_path
                )));
            }
        }
    }
    Ok(())
}

fn ensure_unique_strings(label: &str, values: &[String]) -> HarnessResult<()> {
    let mut seen = BTreeSet::new();
    for value in values {
        if value.trim().is_empty() || !seen.insert(value.as_str()) {
            return Err(HarnessError::InvalidPlan(format!(
                "{label} values must be non-empty and unique"
            )));
        }
    }
    Ok(())
}

fn ensure_unique_verification_requirements(
    requirements: &[VerificationRequirement],
) -> HarnessResult<()> {
    let mut units = BTreeSet::new();
    for requirement in requirements {
        if !units.insert(requirement.unit) {
            return Err(HarnessError::InvalidPlan(
                "verification requirement units must be unique".to_owned(),
            ));
        }
    }
    Ok(())
}

fn ensure_unique_policy_ids(policies: &[PolicyBinding]) -> HarnessResult<()> {
    let ids = policies
        .iter()
        .map(|policy| policy.id.clone())
        .collect::<Vec<_>>();
    ensure_unique_strings("policy ID", &ids)?;
    let paths = policies
        .iter()
        .map(|policy| policy.repository_relative_path.clone())
        .collect::<Vec<_>>();
    ensure_unique_strings("policy path", &paths)
}

fn ensure_unique_workspace_policy_paths(policies: &[WorkspacePolicyBinding]) -> HarnessResult<()> {
    let paths = policies
        .iter()
        .map(|policy| policy.workspace_relative_path.clone())
        .collect::<Vec<_>>();
    ensure_unique_strings("workspace policy path", &paths)
}

fn path_is_within(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn portable_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

pub(super) fn byte_digest(content: &[u8]) -> String {
    let digest = Sha256::digest(content);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

/// Decodes an externally supplied Harness artifact without accepting a wider JSON shape.
///
/// Serde normally ignores unknown fields on nested structs. Comparing the parsed input value
/// with the value serialized from the decoded artifact makes that behavior fail closed across
/// the full object tree, including duplicate keys, optional fields and nested records.
pub fn decode_current_json<T>(bytes: &[u8], label: &str) -> HarnessResult<T>
where
    T: DeserializeOwned + Serialize,
{
    let input: serde_json::Value = serde_json::from_slice(bytes).map_err(|error| {
        HarnessError::InvalidSubmission(format!("invalid {label} JSON: {error}"))
    })?;
    for object_keys in JsonObjectKeyScanner::scan(bytes).map_err(|message| {
        HarnessError::InvalidSubmission(format!("invalid {label} JSON: {message}"))
    })? {
        let mut unique_keys = BTreeSet::new();
        for (start, end) in object_keys {
            let key: String = serde_json::from_slice(&bytes[start..end]).map_err(|error| {
                HarnessError::InvalidSubmission(format!("invalid {label} JSON key: {error}"))
            })?;
            if !unique_keys.insert(key.clone()) {
                return Err(HarnessError::InvalidSubmission(format!(
                    "invalid {label} JSON: duplicate object key `{key}`"
                )));
            }
        }
    }
    let decoded: T = serde_json::from_value(input.clone()).map_err(|error| {
        HarnessError::InvalidSubmission(format!("invalid current {label}: {error}"))
    })?;
    let canonical = serde_json::to_value(&decoded).map_err(|error| {
        HarnessError::InvalidSubmission(format!(
            "failed to encode decoded current {label}: {error}"
        ))
    })?;
    if input != canonical {
        return Err(HarnessError::InvalidSubmission(format!(
            "{label} contains fields or representations outside the exact current schema"
        )));
    }
    Ok(decoded)
}

struct JsonObjectKeyScanner<'a> {
    bytes: &'a [u8],
    cursor: usize,
    objects: Vec<Vec<(usize, usize)>>,
}

impl<'a> JsonObjectKeyScanner<'a> {
    fn scan(bytes: &'a [u8]) -> Result<Vec<Vec<(usize, usize)>>, &'static str> {
        let mut scanner = Self {
            bytes,
            cursor: 0,
            objects: Vec::new(),
        };
        scanner.scan_value()?;
        scanner.skip_whitespace();
        if scanner.cursor != bytes.len() {
            return Err("content follows the top-level value");
        }
        Ok(scanner.objects)
    }

    fn scan_value(&mut self) -> Result<(), &'static str> {
        self.skip_whitespace();
        match self.peek() {
            Some(b'{') => self.scan_object(),
            Some(b'[') => self.scan_array(),
            Some(b'"') => self.scan_string(),
            Some(_) => self.scan_primitive(),
            None => Err("JSON value is missing"),
        }
    }

    fn scan_object(&mut self) -> Result<(), &'static str> {
        self.consume(b'{')?;
        self.skip_whitespace();
        let mut keys = Vec::new();
        if self.peek() == Some(b'}') {
            self.cursor += 1;
            self.objects.push(keys);
            return Ok(());
        }
        loop {
            self.skip_whitespace();
            let start = self.cursor;
            self.scan_string()?;
            keys.push((start, self.cursor));
            self.skip_whitespace();
            self.consume(b':')?;
            self.scan_value()?;
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.cursor += 1,
                Some(b'}') => {
                    self.cursor += 1;
                    self.objects.push(keys);
                    return Ok(());
                }
                _ => return Err("object entry has no valid delimiter"),
            }
        }
    }

    fn scan_array(&mut self) -> Result<(), &'static str> {
        self.consume(b'[')?;
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.cursor += 1;
            return Ok(());
        }
        loop {
            self.scan_value()?;
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.cursor += 1,
                Some(b']') => {
                    self.cursor += 1;
                    return Ok(());
                }
                _ => return Err("array value has no valid delimiter"),
            }
        }
    }

    fn scan_string(&mut self) -> Result<(), &'static str> {
        self.consume(b'"')?;
        while let Some(byte) = self.peek() {
            match byte {
                b'"' => {
                    self.cursor += 1;
                    return Ok(());
                }
                b'\\' => {
                    self.cursor = self
                        .cursor
                        .checked_add(2)
                        .filter(|cursor| *cursor <= self.bytes.len())
                        .ok_or("string escape exceeds the input")?;
                }
                _ => self.cursor += 1,
            }
        }
        Err("string is not terminated")
    }

    fn scan_primitive(&mut self) -> Result<(), &'static str> {
        let start = self.cursor;
        while !matches!(
            self.peek(),
            None | Some(b' ' | b'\n' | b'\r' | b'\t' | b',' | b']' | b'}')
        ) {
            self.cursor += 1;
        }
        if self.cursor == start {
            return Err("primitive value is missing");
        }
        Ok(())
    }

    fn consume(&mut self, expected: u8) -> Result<(), &'static str> {
        if self.peek() != Some(expected) {
            return Err("JSON structure is invalid");
        }
        self.cursor += 1;
        Ok(())
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.cursor += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.cursor).copied()
    }
}

pub(super) fn serialized_digest(value: &impl Serialize) -> HarnessResult<String> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        HarnessError::InvalidPlan(format!("failed to encode digest input: {error}"))
    })?;
    Ok(byte_digest(&bytes))
}

pub(super) fn validate_serialized_size(
    label: &str,
    value: &impl Serialize,
    maximum_bytes: usize,
) -> HarnessResult<()> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        HarnessError::InvalidSubmission(format!("{label} cannot serialize: {error}"))
    })?;
    if bytes.len() > maximum_bytes {
        return Err(HarnessError::InvalidSubmission(format!(
            "{label} exceeds the {maximum_bytes} byte aggregate limit"
        )));
    }
    Ok(())
}

pub(super) fn validate_digest(label: &str, digest: &str) -> HarnessResult<()> {
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(HarnessError::InvalidPlan(format!(
            "{label} must be a 64-character hexadecimal SHA-256 digest"
        )));
    }
    Ok(())
}

pub(super) fn validate_identifier(label: &str, value: &str) -> HarnessResult<()> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(HarnessError::InvalidPlan(format!(
            "{label} is not a portable opaque identifier"
        )));
    }
    Ok(())
}

pub(super) fn require_version(label: &str, found: u32, expected: u32) -> HarnessResult<()> {
    if found != expected {
        return Err(HarnessError::InvalidPlan(format!(
            "{label} version must be {expected}, found {found}"
        )));
    }
    Ok(())
}

#[cfg(test)]
#[path = "harness/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "harness/source_tests.rs"]
mod source_tests;
