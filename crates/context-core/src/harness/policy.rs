use super::*;

/// Explicit caller-owned selection data. Native source identity and bytes remain in ContextSource.
#[derive(Clone, Debug)]
pub struct PolicyConfiguration {
    pub(super) data: PolicySettings,
    pub(super) digest: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PolicySettings {
    documents: Vec<PolicyDocument>,
    orchestrator: Vec<String>,
    rules: Vec<PolicyRule>,
    defaults: Vec<DefaultAuthority>,
    target_constraints: Vec<TargetConstraint>,
    pub(super) context_exclusions: Vec<ContextExclusion>,
    pub(super) company: Option<CompanySettings>,
    career: Option<CareerSourceRoots>,
}

/// Pointers to existing caller-owned routers; never another experience registry.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CareerSourceRoots {
    pub experience_routes: String,
    pub source_map: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PolicyDocument {
    key: String,
    id: String,
    pub(super) path: String,
    project_entrypoint: bool,
    dependencies: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PolicyRule {
    selector: Selector,
    owner_source: OwnerSource,
    documents: Vec<String>,
    roles: Vec<HarnessRole>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
enum OwnerSource {
    Primary,
    Grant,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Selector {
    actions: Vec<HarnessAction>,
    owners: Vec<OwnerKind>,
    intents: Vec<IntentKind>,
    surfaces: Vec<CareerOutputSurface>,
    curation_kinds: Vec<CurationKind>,
    target_kind: TargetKind,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum OwnerKind {
    Profile,
    CommonWork,
    PersonalBusiness,
    Personal,
    PersonalProject,
    Company,
    CompanyProject,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum IntentKind {
    General,
    CareerArtifact,
    PolicyMaintenance,
    OutputAdapter,
    SoloMvpIdeation,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum TargetKind {
    Any,
    Rust,
    Dependency,
    Native,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DefaultAuthority {
    rule: PolicyDefaultRule,
    document: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TargetConstraint {
    selector: Selector,
    pub(super) path: String,
    allow_curation_source: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ContextExclusion {
    pub(super) intent: IntentKind,
    pub(super) paths: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CompanySettings {
    pub(super) registry: String,
    routing: String,
    leaf_dependencies: Vec<String>,
    leaf_id_prefix: String,
    shared_documents: Vec<String>,
}

type Requirements = BTreeMap<(String, DataOwner, OwnerSource), PolicyRoleMask>;

impl OwnerKind {
    fn of(owner: &DataOwner) -> Self {
        match owner {
            DataOwner::Profile => Self::Profile,
            DataOwner::CommonWork => Self::CommonWork,
            DataOwner::PersonalBusiness => Self::PersonalBusiness,
            DataOwner::Personal => Self::Personal,
            DataOwner::PersonalProject { .. } => Self::PersonalProject,
            DataOwner::Company { .. } => Self::Company,
            DataOwner::CompanyProject { .. } => Self::CompanyProject,
        }
    }
}

impl IntentKind {
    pub(super) fn of(intent: HarnessIntent) -> Self {
        match intent {
            HarnessIntent::General => Self::General,
            HarnessIntent::CareerArtifact { .. } => Self::CareerArtifact,
            HarnessIntent::PolicyMaintenance => Self::PolicyMaintenance,
            HarnessIntent::OutputAdapter { .. } => Self::OutputAdapter,
            HarnessIntent::SoloMvpIdeation => Self::SoloMvpIdeation,
        }
    }
}

impl Selector {
    fn validate(&self) -> HarnessResult<()> {
        if self.actions.len() > 9
            || self.owners.len() > 7
            || self.intents.len() > 5
            || self.surfaces.len() > 7
            || self.curation_kinds.len() > 6
        {
            return Err(settings_error("selector exceeds the supported axes"));
        }
        Ok(())
    }

    fn matches(&self, request: &HarnessRequest, intent: HarnessIntent, owner: &DataOwner) -> bool {
        (self.actions.is_empty() || self.actions.contains(&request.action))
            && (self.owners.is_empty() || self.owners.contains(&OwnerKind::of(owner)))
            && (self.intents.is_empty() || self.intents.contains(&IntentKind::of(intent)))
            && (self.surfaces.is_empty()
                || intent
                    .output_surface()
                    .is_some_and(|surface| self.surfaces.contains(&surface)))
            && (self.curation_kinds.is_empty()
                || request
                    .curation_kind
                    .is_some_and(|kind| self.curation_kinds.contains(&kind)))
            && match self.target_kind {
                TargetKind::Any => true,
                TargetKind::Rust => request.targets.iter().any(|path| is_rust_target(path)),
                TargetKind::Dependency => request
                    .targets
                    .iter()
                    .any(|path| is_dependency_target(path)),
                TargetKind::Native => request
                    .targets
                    .iter()
                    .any(|path| path.starts_with("vault/")),
            }
    }
}

pub(super) fn settings_error(message: impl Into<String>) -> HarnessError {
    HarnessError::InvalidRequest(format!("policy configuration: {}", message.into()))
}

fn checked_template(path: &str) -> HarnessResult<()> {
    validate_single_line("policy path", path, MAX_TARGET_LENGTH)?;
    let normalized = path
        .replace("{company}", "example-company")
        .replace("{project}", "example-project");
    if normalized.contains(['{', '}']) {
        return Err(settings_error("path contains an unsupported template"));
    }
    validate_relative_path(Path::new(&normalized), "policy path")?;
    if !normalized.starts_with("vault/") || contains_raw_conversation_path(Path::new(&normalized)) {
        return Err(settings_error("path must name a scoped native source"));
    }
    Ok(())
}

fn expand_path(path: &str, owner: &DataOwner) -> HarnessResult<String> {
    owner.validate()?;
    let mut expanded = path.to_owned();
    match owner {
        DataOwner::Company { company } | DataOwner::CompanyProject { company, .. } => {
            expanded = expanded.replace("{company}", company);
        }
        _ => {}
    }
    match owner {
        DataOwner::PersonalProject { project } | DataOwner::CompanyProject { project, .. } => {
            expanded = expanded.replace("{project}", project);
        }
        _ => {}
    }
    if expanded.contains(['{', '}']) {
        return Err(settings_error(
            "selected document cannot be located for this owner",
        ));
    }
    checked_template(&expanded)?;
    Ok(expanded)
}

impl PolicyConfiguration {
    /// Canonical exact router paths selected by this caller configuration.
    pub fn career_source_roots(&self) -> HarnessResult<Vec<String>> {
        let selected = self
            .data
            .career
            .as_ref()
            .ok_or_else(|| settings_error("career source roots are not configured"))?;
        if selected.experience_routes == selected.source_map {
            return Err(settings_error("career routers must have distinct owners"));
        }
        [&selected.experience_routes, &selected.source_map]
            .into_iter()
            .map(|key| {
                let document = self.document(key)?;
                if document.project_entrypoint
                    || !document.path.starts_with("vault/personal/writing/")
                    || document.path.contains(['{', '}'])
                {
                    return Err(settings_error(
                        "career roots must be exact personal writing routers",
                    ));
                }
                Ok(document.path.clone())
            })
            .collect()
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    /// Read a bounded regular file without following a file link or accepting multiple links.
    pub fn read(path: impl AsRef<Path>) -> HarnessResult<Self> {
        let path = path.as_ref();
        let parent = canonical_directory(
            path.parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
            "policy configuration parent",
        )?;
        let name = path
            .file_name()
            .ok_or_else(|| settings_error("file name is missing"))?;
        let file = verified_repository_file(&parent, Path::new(name))?;
        let (text, _) = read_text_file(file, MAX_POLICY_BYTES, &parent.join(name))?;
        Self::from_bytes(text.as_bytes())
    }

    pub fn from_bytes(bytes: &[u8]) -> HarnessResult<Self> {
        if bytes.len() > usize::try_from(MAX_POLICY_BYTES).expect("policy limit fits usize") {
            return Err(settings_error("input exceeds the bounded limit"));
        }
        let data: PolicySettings = decode_current_json(bytes, "policy configuration")?;
        let configuration = Self {
            data,
            digest: byte_digest(bytes),
        };
        configuration.validate()?;
        Ok(configuration)
    }

    /// Validate selection and owner/grant boundaries without accessing a ContextSource.
    pub fn validate_contract(&self, contract: &ResolvedTaskContract) -> HarnessResult<()> {
        let request = contract.to_harness_request();
        let grants = contract.context_grants();
        let intent = contract.intent();
        request.validate()?;
        validate_context_grants(&request, grants)?;
        validate_intent(&request, intent, grants)?;
        self.validate_request(
            &request,
            intent,
            grants,
            &roles_for(&request, contract.execution_profile()),
        )
    }

    /// Check an existing resolution's current configuration before any source lookup.
    pub fn validate_resolved(&self, resolved: &ResolvedHarnessRequest) -> HarnessResult<()> {
        resolved.plan.require_executable_version()?;
        if serialized_digest(&resolved.plan)? != resolved.resolved_plan_digest {
            return Err(HarnessError::InvalidPlan(
                "resolved plan content does not match its resolved-plan digest".to_owned(),
            ));
        }
        self.validate_contract(&resolved.contract)?;
        self.validate_defaults(&resolved.decision_trace, &resolved.plan.required_policies)?;
        let contract_digest = serialized_digest(&resolved.contract)?;
        let trace_digest = serialized_digest(&resolved.decision_trace)?;
        let manifest_digest = resolved
            .career_composition_manifest
            .as_ref()
            .map(serialized_digest)
            .transpose()?;
        let current = serialized_digest(&(
            &contract_digest,
            &trace_digest,
            &resolved.request_provenance,
            &manifest_digest,
            &self.digest,
        ))?;
        if current != resolved.plan.request_digest {
            return Err(HarnessError::PlanDrift {
                expected: resolved.plan.request_digest.clone(),
                actual: current,
            });
        }
        Ok(())
    }

    pub(super) fn document(&self, key: &str) -> HarnessResult<&PolicyDocument> {
        self.data
            .documents
            .iter()
            .find(|document| document.key == key)
            .ok_or_else(|| settings_error("selected document is undeclared"))
    }

    fn validate(&self) -> HarnessResult<()> {
        if self.data.career.is_some() {
            self.career_source_roots()?;
        }
        if self.data.documents.is_empty()
            || self.data.documents.len() > 128
            || self.data.rules.is_empty()
            || self.data.rules.len() > 512
            || self.data.orchestrator.is_empty()
            || self.data.orchestrator.len() > 128
            || self.data.target_constraints.len() > 128
            || self.data.context_exclusions.len() > 128
        {
            return Err(settings_error(
                "document, rule or constraint collection is empty or too large",
            ));
        }
        let mut keys = BTreeSet::new();
        let mut ids = BTreeSet::new();
        for document in &self.data.documents {
            for value in [&document.key, &document.id] {
                validate_single_line("policy identifier", value, MAX_COMPANY_LENGTH)?;
                if value.is_empty()
                    || value.trim() != value
                    || value.chars().any(char::is_whitespace)
                {
                    return Err(settings_error(
                        "identifier must be nonempty without whitespace",
                    ));
                }
            }
            if !keys.insert(document.key.as_str()) || !ids.insert(document.id.as_str()) {
                return Err(settings_error(
                    "document keys and binding identifiers must be unique",
                ));
            }
            checked_template(&document.path)?;
            if !document.project_entrypoint && !document.path.ends_with(".md") {
                return Err(settings_error("policy document must be Markdown"));
            }
            if document.project_entrypoint && document.path != "vault/personal/projects/{project}" {
                return Err(settings_error(
                    "project entrypoint must stay inside the selected personal project",
                ));
            }
            ensure_unique_strings("policy dependency", &document.dependencies)?;
        }
        let check_keys = |selected: &[String]| -> HarnessResult<()> {
            if selected.len() > 128 {
                return Err(settings_error("too many selected documents"));
            }
            ensure_unique_strings("selected policy", selected)?;
            for key in selected {
                self.document(key)?;
            }
            Ok(())
        };
        for document in &self.data.documents {
            check_keys(&document.dependencies)?;
        }
        check_keys(&self.data.orchestrator)?;
        let orchestrator = self.orchestrator_requirements()?;
        for (key, owner, source) in orchestrator.keys() {
            self.validate_document_owner(self.document(key)?, owner, *source)?;
        }
        self.validate_binding_union(&orchestrator, &BTreeMap::new())?;
        for rule in &self.data.rules {
            rule.selector.validate()?;
            check_keys(&rule.documents)?;
            if rule.documents.is_empty()
                || rule.roles.is_empty()
                || rule.roles.len() > MAX_PLANNED_ROLES
            {
                return Err(settings_error("rule must select documents and roles"));
            }
            let unique = rule.roles.iter().collect::<BTreeSet<_>>();
            if unique.len() != rule.roles.len() {
                return Err(settings_error("rule repeats a role"));
            }
        }
        for document in &self.data.documents {
            let mut requirements = BTreeMap::new();
            self.close(
                &document.key,
                &DataOwner::Profile,
                OwnerSource::Primary,
                PolicyRoleMask::ALL,
                &mut requirements,
                &mut BTreeSet::new(),
            )?;
        }
        let expected = [
            PolicyDefaultRule::GeneralIntent,
            PolicyDefaultRule::StandardExecutionProfile,
            PolicyDefaultRule::NonJournalReadConfirmationNotRequired,
        ];
        if self.data.defaults.len() != expected.len()
            || expected.iter().any(|rule| {
                self.data
                    .defaults
                    .iter()
                    .filter(|entry| entry.rule == *rule)
                    .count()
                    != 1
            })
        {
            return Err(settings_error(
                "every default rule requires one exact authority",
            ));
        }
        for authority in &self.data.defaults {
            self.document(&authority.document)?;
            let mut requirements = BTreeMap::new();
            for key in &self.data.orchestrator {
                self.close(
                    key,
                    &DataOwner::Profile,
                    OwnerSource::Primary,
                    PolicyRoleMask::ALL,
                    &mut requirements,
                    &mut BTreeSet::new(),
                )?;
            }
            if !requirements
                .keys()
                .any(|(key, _, _)| key == &authority.document)
            {
                return Err(settings_error(
                    "default authority must be an orchestrator dependency",
                ));
            }
        }
        for constraint in &self.data.target_constraints {
            constraint.selector.validate()?;
            checked_template(&constraint.path)?;
        }
        for exclusion in &self.data.context_exclusions {
            for path in &exclusion.paths {
                checked_template(path)?;
            }
        }
        if let Some(company) = &self.data.company {
            let registry = self.document(&company.registry)?;
            if registry.project_entrypoint
                || !path_is_within(&registry.path, "vault/work/common")
                || registry.path.contains(['{', '}'])
            {
                return Err(settings_error(
                    "company registry must be a shared work original",
                ));
            }
            let routing = self.document(&company.routing)?;
            if routing.project_entrypoint
                || !path_is_within(
                    &expand_path(
                        &routing.path,
                        &DataOwner::Company {
                            company: "example-company".to_owned(),
                        },
                    )?,
                    "vault/work/example-company",
                )
            {
                return Err(settings_error(
                    "company routing must stay inside its company",
                ));
            }
            check_keys(&company.leaf_dependencies)?;
            check_keys(&company.shared_documents)?;
            let mut shared = BTreeMap::new();
            for key in &company.shared_documents {
                self.close(
                    key,
                    &DataOwner::Profile,
                    OwnerSource::Primary,
                    PolicyRoleMask::ALL,
                    &mut shared,
                    &mut BTreeSet::new(),
                )?;
            }
            for (key, owner, source) in shared.keys() {
                self.validate_document_owner(self.document(key)?, owner, *source)?;
            }
            validate_single_line(
                "company rule prefix",
                &company.leaf_id_prefix,
                MAX_COMPANY_LENGTH,
            )?;
            if company.leaf_id_prefix.is_empty()
                || company.leaf_id_prefix.chars().any(char::is_whitespace)
            {
                return Err(settings_error(
                    "company rule prefix must be nonempty without whitespace",
                ));
            }
        }
        Ok(())
    }

    fn close(
        &self,
        key: &str,
        owner: &DataOwner,
        owner_source: OwnerSource,
        mask: PolicyRoleMask,
        requirements: &mut Requirements,
        visiting: &mut BTreeSet<String>,
    ) -> HarnessResult<()> {
        if visiting.contains(key) {
            return Err(settings_error("policy dependencies contain a cycle"));
        }
        let selection = (key.to_owned(), owner.clone(), owner_source);
        if requirements
            .get(&selection)
            .is_some_and(|existing| existing.union(mask) == *existing)
        {
            return Ok(());
        }
        requirements
            .entry(selection)
            .and_modify(|value| *value = value.union(mask))
            .or_insert(mask);
        visiting.insert(key.to_owned());
        for dependency in &self.document(key)?.dependencies {
            self.close(
                dependency,
                owner,
                owner_source,
                mask,
                requirements,
                visiting,
            )?;
        }
        visiting.remove(key);
        Ok(())
    }

    fn orchestrator_requirements(&self) -> HarnessResult<Requirements> {
        let mut requirements = BTreeMap::new();
        for key in &self.data.orchestrator {
            self.close(
                key,
                &DataOwner::Profile,
                OwnerSource::Primary,
                PolicyRoleMask::ALL,
                &mut requirements,
                &mut BTreeSet::new(),
            )?;
        }
        Ok(requirements)
    }

    fn validate_binding_union(
        &self,
        orchestrator: &Requirements,
        roles: &Requirements,
    ) -> HarnessResult<()> {
        let mut ids = BTreeMap::new();
        let mut paths = BTreeMap::new();
        let mut control_ids = BTreeSet::new();
        let mut control_paths = BTreeSet::new();
        for (requirements, is_control) in [(orchestrator, true), (roles, false)] {
            for (key, owner, _) in requirements.keys() {
                let document = self.document(key)?;
                let path = expand_path(&document.path, owner)?;
                // The source chooses between these entrypoints later. Both must stay
                // disjoint from other configured bindings before opening any source.
                let candidates = if document.project_entrypoint {
                    vec![format!("{path}.md"), format!("{path}/index.md")]
                } else {
                    vec![path]
                };
                if !is_control
                    && (control_ids.contains(&document.id)
                        || candidates.iter().any(|path| control_paths.contains(path)))
                {
                    return Err(settings_error(
                        "orchestrator policies must not appear in role requirements",
                    ));
                }
                if ids
                    .get(&document.id)
                    .is_some_and(|existing| existing != &candidates)
                    || candidates.iter().any(|path| {
                        paths
                            .get(path)
                            .is_some_and(|existing| existing != &document.id)
                    })
                {
                    return Err(settings_error(
                        "configured policy bindings have conflicting identifiers or paths",
                    ));
                }
                ids.insert(document.id.clone(), candidates.clone());
                for path in candidates {
                    paths.insert(path.clone(), document.id.clone());
                    if is_control {
                        control_paths.insert(path);
                    }
                }
                if is_control {
                    control_ids.insert(document.id.clone());
                }
            }
        }
        Ok(())
    }

    fn static_requirements(
        &self,
        request: &HarnessRequest,
        intent: HarnessIntent,
        grants: &[ContextGrant],
        roles: &[HarnessRole],
    ) -> HarnessResult<Requirements> {
        let mut requirements = BTreeMap::new();
        for rule in &self.data.rules {
            let owners = match rule.owner_source {
                OwnerSource::Primary => vec![&request.owner],
                OwnerSource::Grant => grants.iter().map(|grant| &grant.owner).collect(),
            };
            for owner in owners {
                if rule.selector.matches(request, intent, owner) {
                    for key in &rule.documents {
                        self.close(
                            key,
                            owner,
                            rule.owner_source,
                            PolicyRoleMask::for_roles(&rule.roles),
                            &mut requirements,
                            &mut BTreeSet::new(),
                        )?;
                    }
                }
            }
        }
        let planned = PolicyRoleMask::for_roles(roles);
        requirements.retain(|_, mask| {
            *mask = mask.intersect(planned);
            !mask.is_empty()
        });
        for role in roles {
            if !requirements.values().any(|mask| mask.contains(*role)) {
                return Err(settings_error("settings do not cover every planned role"));
            }
        }
        Ok(requirements)
    }

    pub(super) fn validate_request(
        &self,
        request: &HarnessRequest,
        intent: HarnessIntent,
        grants: &[ContextGrant],
        roles: &[HarnessRole],
    ) -> HarnessResult<()> {
        let requirements = self.static_requirements(request, intent, grants, roles)?;
        for (key, owner, source) in requirements.keys() {
            self.validate_document_owner(self.document(key)?, owner, *source)?;
        }
        let orchestrator = self.orchestrator_requirements()?;
        self.validate_binding_union(&orchestrator, &requirements)?;
        let company_owner = |owner: &DataOwner| {
            matches!(
                owner,
                DataOwner::Company { .. } | DataOwner::CompanyProject { .. }
            )
        };
        if (company_owner(&request.owner) || grants.iter().any(|grant| company_owner(&grant.owner)))
            && self.data.company.is_none()
        {
            return Err(settings_error("company routing is not configured"));
        }
        if company_owner(&request.owner) {
            let company = self
                .data
                .company
                .as_ref()
                .ok_or_else(|| settings_error("company routing is not configured"))?;
            let mut routing = BTreeMap::new();
            self.close(
                &company.routing,
                &request.owner,
                OwnerSource::Primary,
                PolicyRoleMask::ALL,
                &mut routing,
                &mut BTreeSet::new(),
            )?;
            for key in &company.leaf_dependencies {
                self.close(
                    key,
                    &request.owner,
                    OwnerSource::Primary,
                    PolicyRoleMask::ALL,
                    &mut routing,
                    &mut BTreeSet::new(),
                )?;
            }
            for key in &company.shared_documents {
                self.close(
                    key,
                    &request.owner,
                    OwnerSource::Primary,
                    PolicyRoleMask::ALL,
                    &mut routing,
                    &mut BTreeSet::new(),
                )?;
            }
            for (key, owner, source) in routing.keys() {
                self.validate_document_owner(self.document(key)?, owner, *source)?;
            }
            for (selection, mask) in &requirements {
                routing.entry(selection.clone()).or_insert(*mask);
            }
            self.validate_binding_union(&orchestrator, &routing)?;
        }
        let mut constrained = false;
        for constraint in &self.data.target_constraints {
            if constraint.selector.matches(request, intent, &request.owner) {
                if intent == HarnessIntent::SoloMvpIdeation && constraint.allow_curation_source {
                    return Err(settings_error(
                        "ideation registry constraint must require an exact target",
                    ));
                }
                constrained = true;
                let path = expand_path(&constraint.path, &request.owner)?;
                if !request.targets.contains(&path)
                    && !(constraint.allow_curation_source
                        && request.curation_sources.contains(&path))
                {
                    return Err(settings_error(
                        "request does not bind the exact constrained target or source",
                    ));
                }
            }
        }
        if uses_solo_mvp_ideation_contract(request, intent) && !constrained {
            return Err(settings_error(
                "idea operations require an exact configured registry constraint",
            ));
        }
        Ok(())
    }

    fn validate_document_owner(
        &self,
        document: &PolicyDocument,
        owner: &DataOwner,
        source: OwnerSource,
    ) -> HarnessResult<()> {
        let path = expand_path(&document.path, owner)?;
        let shared =
            path_is_within(&path, "vault/profile") || path_is_within(&path, "vault/work/common");
        let personal_project_path = |project: &str| {
            path == format!("vault/personal/projects/{project}.md")
                || path_is_within(&path, &format!("vault/personal/projects/{project}"))
        };
        let company_project_path = |company: &str, project: &str| {
            path == format!("vault/work/{company}/projects/{project}.md")
                || path_is_within(&path, &format!("vault/work/{company}/projects/{project}"))
                || path == format!("vault/work/{company}/overview/{project}.md")
        };
        let permitted = shared
            || match (source, owner) {
                (_, DataOwner::Profile | DataOwner::CommonWork) => false,
                (OwnerSource::Primary, DataOwner::PersonalBusiness) => {
                    path_is_within(&path, "vault/personal")
                        && !path_is_within(&path, "vault/personal/journal")
                }
                (OwnerSource::Primary, DataOwner::Personal) => {
                    path_is_within(&path, "vault/personal")
                        && !path_is_within(&path, "vault/personal/journal")
                        && !path_is_within(&path, "vault/personal/business")
                }
                (OwnerSource::Primary, DataOwner::PersonalProject { project }) => {
                    path == "vault/personal/index.md"
                        || path_is_within(&path, "vault/personal/decisions")
                        || personal_project_path(project)
                }
                (OwnerSource::Grant, DataOwner::PersonalProject { project }) => {
                    personal_project_path(project)
                }
                (OwnerSource::Primary, DataOwner::Company { company }) => {
                    path_is_within(&path, &format!("vault/work/{company}"))
                }
                (OwnerSource::Primary, DataOwner::CompanyProject { company, project }) => {
                    path == format!("vault/work/{company}/index.md")
                        || path_is_within(&path, &format!("vault/work/{company}/rules"))
                        || path_is_within(&path, &format!("vault/work/{company}/preferences"))
                        || company_project_path(company, project)
                }
                (OwnerSource::Grant, DataOwner::Company { company }) => {
                    path == format!("vault/work/{company}/index.md")
                        || ["experience", "projects", "overview"]
                            .iter()
                            .any(|category| {
                                path_is_within(&path, &format!("vault/work/{company}/{category}"))
                            })
                }
                (OwnerSource::Grant, DataOwner::CompanyProject { company, project }) => {
                    path == format!("vault/work/{company}/index.md")
                        || company_project_path(company, project)
                }
                (OwnerSource::Grant, DataOwner::Personal | DataOwner::PersonalBusiness) => false,
            };
        if !permitted {
            return Err(settings_error(
                "policy lies outside the selected owner and grant boundary",
            ));
        }
        Ok(())
    }

    fn path(
        &self,
        document: &PolicyDocument,
        owner: &DataOwner,
        source: OwnerSource,
        vault: &VaultRepository,
    ) -> HarnessResult<String> {
        self.validate_document_owner(document, owner, source)?;
        if document.project_entrypoint {
            let DataOwner::PersonalProject { project } = owner else {
                return Err(settings_error(
                    "project entrypoint requires a personal project",
                ));
            };
            vault.personal_project_entrypoint(project)
        } else {
            expand_path(&document.path, owner)
        }
    }

    fn bind_requirements(
        &self,
        vault: &VaultRepository,
        requirements: &Requirements,
    ) -> HarnessResult<BTreeMap<PolicyBinding, PolicyRoleMask>> {
        let mut bindings = BTreeMap::new();
        for ((key, owner, source), mask) in requirements {
            let document = self.document(key)?;
            let binding =
                vault.bind_policy(&document.id, &self.path(document, owner, *source, vault)?)?;
            bindings
                .entry(binding)
                .and_modify(|value: &mut PolicyRoleMask| *value = value.union(*mask))
                .or_insert(*mask);
        }
        Ok(bindings)
    }

    fn bind(
        &self,
        vault: &VaultRepository,
        request: &HarnessRequest,
        intent: HarnessIntent,
        grants: &[ContextGrant],
        roles: &[HarnessRole],
    ) -> HarnessResult<(
        Vec<PolicyBinding>,
        Vec<PolicyBinding>,
        Vec<RolePolicyBinding>,
    )> {
        self.validate_request(request, intent, grants, roles)?;
        let orchestrator = self.orchestrator_requirements()?;
        let orchestrator_policies = self
            .bind_requirements(vault, &orchestrator)?
            .into_keys()
            .collect::<Vec<_>>();
        let mut requirements = self.static_requirements(request, intent, grants, roles)?;
        let leaves = self.company_requirements(vault, request, &mut requirements)?;
        let planned = PolicyRoleMask::for_roles(roles);
        let mut bindings = self.bind_requirements(vault, &requirements)?;
        for (id, path) in leaves {
            bindings.insert(vault.bind_policy(&id, &path)?, planned);
        }
        let mut required = orchestrator_policies.clone();
        required.extend(bindings.keys().cloned());
        required.sort();
        required.dedup();
        let role_bindings = roles
            .iter()
            .map(|role| RolePolicyBinding {
                role: *role,
                policies: bindings
                    .iter()
                    .filter(|(_, mask)| mask.contains(*role))
                    .map(|(binding, _)| binding.clone())
                    .collect(),
            })
            .collect();
        Ok((required, orchestrator_policies, role_bindings))
    }

    fn company_requirements(
        &self,
        vault: &VaultRepository,
        request: &HarnessRequest,
        requirements: &mut Requirements,
    ) -> HarnessResult<Vec<(String, String)>> {
        let (DataOwner::Company { company } | DataOwner::CompanyProject { company, .. }) =
            &request.owner
        else {
            return Ok(Vec::new());
        };
        let settings = self
            .data
            .company
            .as_ref()
            .ok_or_else(|| settings_error("company routing is not configured"))?;
        let routing_path = expand_path(&self.document(&settings.routing)?.path, &request.owner)?;
        if !vault.source_metadata(Path::new(&routing_path))?.exists() {
            return Ok(Vec::new());
        }
        self.close(
            &settings.routing,
            &request.owner,
            OwnerSource::Primary,
            PolicyRoleMask::ALL,
            requirements,
            &mut BTreeSet::new(),
        )?;
        let file = vault.open_scoped_file(Path::new(&routing_path), MAX_POLICY_BYTES)?;
        let (content, _) = read_text_file(file, MAX_POLICY_BYTES, &vault.root.join(&routing_path))?;
        let shared = settings
            .shared_documents
            .iter()
            .map(|key| {
                let document = self.document(key)?;
                Ok((expand_path(&document.path, &request.owner)?, key.clone()))
            })
            .collect::<HarnessResult<BTreeMap<_, _>>>()?;
        let shared_ids = shared
            .iter()
            .map(|(path, key)| Ok((path.clone(), self.document(key)?.id.clone())))
            .collect::<HarnessResult<BTreeMap<_, _>>>()?;
        let reserved_ids = self
            .data
            .documents
            .iter()
            .map(|document| document.id.clone())
            .collect::<BTreeSet<_>>();
        let routes = parse_company_domain_routes(
            &content,
            company,
            &shared_ids,
            &settings.leaf_id_prefix,
            &reserved_ids,
        )?;
        for path in routes
            .iter()
            .flat_map(|route| &route.policies)
            .collect::<BTreeSet<_>>()
        {
            vault.validate_source_file(Path::new(path))?;
        }
        let searchable = company_route_searchable(request, company);
        let mut leaves = BTreeSet::new();
        for route in routes {
            if route.target_kind.matches(request)
                && route.actions.contains(&request.action)
                && route
                    .signals
                    .iter()
                    .any(|signal| contains_company_route_signal(&searchable, signal))
            {
                for path in route.policies {
                    if let Some(key) = shared.get(&path) {
                        self.close(
                            key,
                            &request.owner,
                            OwnerSource::Primary,
                            PolicyRoleMask::ALL,
                            requirements,
                            &mut BTreeSet::new(),
                        )?;
                    } else {
                        for key in &settings.leaf_dependencies {
                            self.close(
                                key,
                                &request.owner,
                                OwnerSource::Primary,
                                PolicyRoleMask::ALL,
                                requirements,
                                &mut BTreeSet::new(),
                            )?;
                        }
                        let stem = Path::new(&path)
                            .file_stem()
                            .and_then(OsStr::to_str)
                            .ok_or_else(|| settings_error("company rule has no UTF-8 stem"))?;
                        leaves.insert((format!("{}{stem}", settings.leaf_id_prefix), path));
                    }
                }
            }
        }
        Ok(leaves.into_iter().collect())
    }

    pub(super) fn default_policy(&self, rule: PolicyDefaultRule) -> HarnessResult<(&str, &str)> {
        let authority = self
            .data
            .defaults
            .iter()
            .find(|entry| entry.rule == rule)
            .ok_or_else(|| settings_error("default rule has no authority"))?;
        let document = self.document(&authority.document)?;
        Ok((&document.id, &document.path))
    }

    /// Check configured authority identifiers without reading their source bytes.
    pub fn validate_decision_defaults(&self, trace: &DecisionTrace) -> HarnessResult<()> {
        for record in &trace.records {
            if let DecisionRecord::PolicyDefault {
                rule,
                policy_identifier,
                ..
            } = record
            {
                let authority = self
                    .data
                    .defaults
                    .iter()
                    .find(|entry| entry.rule == *rule)
                    .ok_or_else(|| settings_error("default rule has no authority"))?;
                if self.document(&authority.document)?.id != *policy_identifier {
                    return Err(settings_error(
                        "default rule does not use its exact configured authority",
                    ));
                }
            }
        }
        Ok(())
    }

    pub(super) fn validate_defaults(
        &self,
        trace: &DecisionTrace,
        required: &[PolicyBinding],
    ) -> HarnessResult<()> {
        self.validate_decision_defaults(trace)?;
        trace.validate_policy_bindings(required)?;
        for record in &trace.records {
            if let DecisionRecord::PolicyDefault {
                rule,
                policy_identifier,
                policy_content_digest,
                ..
            } = record
            {
                let authority = self
                    .data
                    .defaults
                    .iter()
                    .find(|entry| entry.rule == *rule)
                    .ok_or_else(|| settings_error("default rule has no authority"))?;
                let document = self.document(&authority.document)?;
                if document.id != *policy_identifier
                    || !required.iter().any(|binding| {
                        binding.id == document.id
                            && binding.content_digest == *policy_content_digest
                    })
                {
                    return Err(settings_error(
                        "default rule does not use its exact configured authority and digest",
                    ));
                }
            }
        }
        Ok(())
    }
}

impl VaultRepository {
    pub(super) fn required_policies(
        &self,
        request: &HarnessRequest,
        intent: HarnessIntent,
        grants: &[ContextGrant],
        roles: &[HarnessRole],
    ) -> HarnessResult<(
        Vec<PolicyBinding>,
        Vec<PolicyBinding>,
        Vec<RolePolicyBinding>,
    )> {
        self.policy_configuration
            .bind(self, request, intent, grants, roles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDirectory;

    fn fixture() -> serde_json::Value {
        serde_json::from_slice(include_bytes!("../../tests/fixtures/policy-settings.json")).unwrap()
    }

    fn configured(value: &serde_json::Value) -> HarnessResult<PolicyConfiguration> {
        PolicyConfiguration::from_bytes(&serde_json::to_vec(value).unwrap())
    }

    #[test]
    fn configured_binding_ids_stay_unique_across_expanded_owners() {
        let config = configured(&fixture()).unwrap();
        let mut roles = BTreeMap::new();
        for company in ["cedar", "lumen"] {
            config
                .close(
                    "company-evidence",
                    &DataOwner::Company {
                        company: company.into(),
                    },
                    OwnerSource::Grant,
                    PolicyRoleMask::ALL,
                    &mut roles,
                    &mut BTreeSet::new(),
                )
                .unwrap();
        }
        let error = config
            .validate_binding_union(&config.orchestrator_requirements().unwrap(), &roles)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("conflicting identifiers or paths")
        );
    }

    #[test]
    fn configuration_rejects_incomplete_ambiguous_and_cyclic_inputs() {
        let mut mutations = Vec::new();
        let mut value = fixture();
        value["unexpected"] = true.into();
        mutations.push(value);
        let mut value = fixture();
        value.as_object_mut().unwrap().remove("defaults");
        mutations.push(value);
        let mut value = fixture();
        value["defaults"].as_array_mut().unwrap().pop();
        mutations.push(value);
        let mut value = fixture();
        value["documents"][1]["dependencies"] = serde_json::json!(["checks"]);
        mutations.push(value);
        let mut value = fixture();
        value["rules"][0]["documents"] = serde_json::json!(["undeclared"]);
        mutations.push(value);
        let mut value = fixture();
        value["documents"][0]["path"] = "vault/profile/../../outside.md".into();
        mutations.push(value);
        let mut value = fixture();
        value["documents"][0]["path"] = "vault/profile/{unknown}.md".into();
        mutations.push(value);
        for value in mutations {
            assert!(configured(&value).is_err());
        }
        let duplicate = format!(
            "{{\"documents\":[],{}",
            String::from_utf8(serde_json::to_vec(&fixture()).unwrap())
                .unwrap()
                .trim_start_matches('{')
        );
        assert!(PolicyConfiguration::from_bytes(duplicate.as_bytes()).is_err());
        assert!(PolicyConfiguration::from_bytes(b"not JSON").is_err());
    }

    #[test]
    fn shared_policy_dependencies_cannot_cross_owner_boundaries() {
        for path in [
            "vault/personal/profile.md",
            "vault/work/foreign/rules/hidden.md",
        ] {
            let mut value = fixture();
            value["documents"][1]["path"] = path.into();
            assert!(configured(&value).is_err());
        }
    }

    #[test]
    fn dependency_dag_closure_reuses_completed_nodes() {
        let mut value = fixture();
        for index in 0..80 {
            let dependencies = ((index + 1)..=(index + 2))
                .filter(|next| *next < 80)
                .map(|next| format!("node-{next}"))
                .collect::<Vec<_>>();
            value["documents"].as_array_mut().unwrap().push(serde_json::json!({"key":format!("node-{index}"),"id":format!("node-{index}"),"path":format!("vault/profile/rules/node-{index}.md"),"project_entrypoint":false,"dependencies":dependencies}));
        }
        let started = std::time::Instant::now();
        let configuration = configured(&value).unwrap();
        let mut requirements = BTreeMap::new();
        configuration
            .close(
                "node-0",
                &DataOwner::Profile,
                OwnerSource::Primary,
                PolicyRoleMask::ALL,
                &mut requirements,
                &mut BTreeSet::new(),
            )
            .unwrap();
        assert_eq!(requirements.len(), 80);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn every_default_requires_its_configured_authority_and_exact_digest() {
        let configuration = configured(&fixture()).unwrap();
        let digest = "a".repeat(64);
        let bindings = [
            PolicyBinding {
                id: "control".into(),
                repository_relative_path: "vault/profile/rules/control.md".into(),
                content_digest: digest.clone(),
            },
            PolicyBinding {
                id: "foundation".into(),
                repository_relative_path: "vault/profile/rules/foundation.md".into(),
                content_digest: digest.clone(),
            },
        ];
        for rule in [
            PolicyDefaultRule::GeneralIntent,
            PolicyDefaultRule::StandardExecutionProfile,
            PolicyDefaultRule::NonJournalReadConfirmationNotRequired,
        ] {
            let trace = |identifier: &str, policy_digest: String| DecisionTrace {
                records: vec![DecisionRecord::PolicyDefault {
                    identifier: "decision-0000000000000001".into(),
                    value_digest: "b".repeat(64),
                    policy_identifier: identifier.into(),
                    policy_content_digest: policy_digest,
                    rule,
                }],
            };
            configuration
                .validate_defaults(&trace("control", digest.clone()), &bindings)
                .unwrap();
            assert!(
                configuration
                    .validate_defaults(&trace("foundation", digest.clone()), &bindings)
                    .is_err()
            );
            assert!(
                configuration
                    .validate_defaults(&trace("control", "c".repeat(64)), &bindings)
                    .is_err()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn input_file_rejects_links_missing_files_and_oversized_data() {
        let temporary = TempDirectory::new("policy-input");
        let regular = temporary.path().join("settings.json");
        fs::write(&regular, serde_json::to_vec(&fixture()).unwrap()).unwrap();
        PolicyConfiguration::read(&regular).unwrap();
        let symbolic = temporary.path().join("symbolic.json");
        std::os::unix::fs::symlink(&regular, &symbolic).unwrap();
        assert!(PolicyConfiguration::read(&symbolic).is_err());
        let hard = temporary.path().join("hard.json");
        fs::hard_link(&regular, &hard).unwrap();
        assert!(PolicyConfiguration::read(&hard).is_err());
        assert!(PolicyConfiguration::read(&regular).is_err());
        assert!(PolicyConfiguration::read(temporary.path().join("missing.json")).is_err());
        fs::remove_file(hard).unwrap();
        fs::write(
            &regular,
            vec![b' '; usize::try_from(MAX_POLICY_BYTES).unwrap() + 1],
        )
        .unwrap();
        assert!(PolicyConfiguration::read(&regular).is_err());
    }

    #[test]
    fn company_collision_check_uses_final_ids_and_reserves_configured_ids() {
        let routing = "---\ndomain_routes:\n  - signals: [widget]\n    actions: [investigation]\n    target_kind: any\n    policies: [vault/profile/rules/authoring.md, vault/work/acme/rules/authoring.md]\n---\n# Routing\n";
        let shared = BTreeMap::from([(
            "vault/profile/rules/authoring.md".into(),
            "authoring".into(),
        )]);
        parse_company_domain_routes(
            routing,
            "acme",
            &shared,
            "leaf-",
            &BTreeSet::from(["authoring".into()]),
        )
        .unwrap();
        assert!(
            parse_company_domain_routes(
                routing,
                "acme",
                &shared,
                "",
                &BTreeSet::from(["authoring".into()])
            )
            .is_err()
        );
        assert!(
            parse_company_domain_routes(
                routing,
                "acme",
                &shared,
                "leaf-",
                &BTreeSet::from(["leaf-authoring".into()])
            )
            .is_err()
        );
    }
}
