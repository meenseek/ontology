use std::sync::{Arc, Mutex};

use super::*;
fn policy_configuration() -> PolicyConfiguration {
    PolicyConfiguration::from_bytes(include_bytes!("../../tests/fixtures/policy-settings.json"))
        .unwrap()
}

#[test]
fn composed_request_cannot_cross_configuration_before_source_access() {
    let fixture = SourceFixture::new();
    let mut request = SourceFixture::request(DataOwner::Personal);
    request.action = HarnessAction::DocumentWrite;
    let mut envelope = RequestEnvelope::from_structured_request(
        request,
        HarnessIntent::General,
        vec![],
        vec![],
        HarnessExecutionProfile::Standard,
        "fictional configuration-binding test",
    )
    .unwrap();
    let statement = "statement-0000000000000001".to_owned();
    envelope.source = RequestSource::UserLanguage {
        statements: vec![UserStatement {
            identifier: statement.clone(),
            text: "Write the fictional README candidate.".into(),
        }],
    };
    for record in &mut envelope.decision_trace.records {
        if let DecisionRecord::StructuredCaller {
            identifier,
            value_digest,
            ..
        } = record
        {
            *record = DecisionRecord::UserStatement {
                identifier: identifier.clone(),
                value_digest: value_digest.clone(),
                statement_identifiers: vec![statement.clone()],
            };
        }
    }
    let DraftTaskRequest::Write(ref mut draft) = envelope.draft else {
        unreachable!()
    };
    let mut defaults = Vec::new();
    if let DraftValue::Resolved {
        decision_identifier,
        ..
    } = &mut draft.common.intent
    {
        defaults.push(std::mem::take(decision_identifier));
    }
    if let DraftValue::Resolved {
        decision_identifier,
        ..
    } = &mut draft.common.execution_profile
    {
        defaults.push(std::mem::take(decision_identifier));
    }
    envelope.decision_trace.records.retain(|record| {
        !matches!(record,
        DecisionRecord::UserStatement { identifier, .. } if defaults.contains(identifier))
    });
    let pending = envelope.compose_decisions(&policy_configuration()).unwrap();
    let mut engine = fixture.engine();
    let mut changed = include_bytes!("../../tests/fixtures/policy-settings.json").to_vec();
    changed.push(b' ');
    engine.vault.policy_configuration = PolicyConfiguration::from_bytes(&changed).unwrap();
    fixture.source.reset_calls();
    assert!(
        engine
            .resolve_composed(pending, None)
            .unwrap_err()
            .to_string()
            .contains("different caller policy configuration")
    );
    let calls = fixture.source.calls.lock().unwrap();
    assert!(
        calls.opens.is_empty()
            && calls.metadata.is_empty()
            && calls.validations.is_empty()
            && calls.children.is_empty()
    );
}

#[test]
fn fictional_rule_axes_select_only_matching_source_and_role_bindings() {
    for axis in ["intent", "surface", "curation", "dependency", "native"] {
        for matched in [true, false] {
            let fixture = SourceFixture::new();
            let marker = "vault/profile/rules/marker.md";
            fixture.source.write(marker, "# Independent marker\n", 1);
            let mut settings: serde_json::Value =
                serde_json::from_slice(include_bytes!("../../tests/fixtures/policy-settings.json"))
                    .unwrap();
            settings["documents"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({
                    "key": "marker", "id": "marker", "path": marker,
                    "dependencies": ["foundation"], "project_entrypoint": false
                }));
            let mut selector = serde_json::json!({
                "actions": [], "owners": [], "intents": [], "surfaces": [],
                "curation_kinds": [], "target_kind": "any"
            });
            let mut request = SourceFixture::request(DataOwner::Personal);
            let mut intent = HarnessIntent::General;
            match axis {
                "intent" => {
                    selector["intents"] = serde_json::json!(["career-artifact"]);
                    if matched {
                        intent = HarnessIntent::CareerArtifact {
                            surface: CareerOutputSurface::General,
                        };
                    }
                }
                "surface" => {
                    selector["surfaces"] = serde_json::json!(["general"]);
                    request.owner = DataOwner::Profile;
                    intent = HarnessIntent::OutputAdapter {
                        surface: if matched {
                            CareerOutputSurface::General
                        } else {
                            CareerOutputSurface::Portfolio
                        },
                    };
                }
                "curation" => {
                    selector["curation_kinds"] = serde_json::json!(["knowledge"]);
                    request.action = HarnessAction::VaultRead;
                    request.curation_kind = Some(if matched {
                        CurationKind::Knowledge
                    } else {
                        CurationKind::Fact
                    });
                    let category = if matched { "knowledge" } else { "facts" };
                    request.targets.clear();
                    fixture.source.write(
                        &format!("vault/personal/{category}/selection.md"),
                        "# Fictional selection\n",
                        1,
                    );
                }
                "dependency" => {
                    selector["target_kind"] = "dependency".into();
                    request.action = HarnessAction::CodeReview;
                    request.owner = DataOwner::PersonalProject {
                        project: "sample".into(),
                    };
                    request.targets = vec![
                        if matched {
                            "package.json"
                        } else {
                            "src/main.rs"
                        }
                        .into(),
                    ];
                    fs::write(fixture.workspace.0.join("package.json"), "{}\n").unwrap();
                    fs::create_dir_all(fixture.workspace.0.join("src")).unwrap();
                    fs::write(fixture.workspace.0.join("src/main.rs"), "fn main() {}\n").unwrap();
                }
                "native" => {
                    selector["target_kind"] = "native".into();
                    if matched {
                        request.targets = vec!["vault/personal/knowledge/selection.md".into()];
                    }
                    fixture.source.write(
                        "vault/personal/knowledge/selection.md",
                        "# Fictional selection\n",
                        1,
                    );
                    fs::write(
                        fixture.source.view_root().join("README.md"),
                        "# External target\n",
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
            settings["rules"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({
                    "selector": selector, "owner_source": "primary",
                    "documents": ["marker"], "roles": ["writer", "verifier", "reviewer", "specialist"]
                }));
            let mut engine = if axis == "curation" || (axis == "native" && matched) {
                fixture.context_engine()
            } else {
                fixture.engine()
            };
            engine.vault.policy_configuration =
                PolicyConfiguration::from_bytes(&serde_json::to_vec(&settings).unwrap()).unwrap();
            fixture.source.reset_calls();
            let result = if axis == "curation" {
                let envelope = RequestEnvelope::from_structured_vault_read(
                    request,
                    HarnessExecutionProfile::Standard,
                    "selection".to_owned(),
                    4096,
                    "independent bounded read fixture",
                )
                .unwrap();
                engine.resolve_test_envelope(&envelope, None)
            } else {
                engine.resolve_with_intent(&request, intent)
            };
            let resolved =
                result.unwrap_or_else(|error| panic!("{axis} matched={matched}: {error}"));
            assert_eq!(
                resolved
                    .plan
                    .required_policies
                    .iter()
                    .any(|binding| binding.id == "marker"),
                matched,
                "{axis}"
            );
            assert_eq!(
                resolved
                    .plan
                    .role_policy_bindings
                    .iter()
                    .any(|binding| binding.policies.iter().any(|policy| policy.id == "marker")),
                matched,
                "{axis}"
            );
            assert_eq!(
                fixture.source.opened_paths().contains(marker),
                matched,
                "{axis}"
            );
        }
    }
}

#[test]
fn configuration_preflight_rejects_foreign_routes_projects_and_uncovered_roles_without_source_reads()
 {
    let fixture = SourceFixture::new();
    let settings = || {
        serde_json::from_slice::<serde_json::Value>(include_bytes!(
            "../../tests/fixtures/policy-settings.json"
        ))
        .unwrap()
    };
    let mut cases = Vec::new();
    let mut value = settings();
    value["documents"][5]["path"] = "vault/work/example-company/preferences/routes.md".into();
    cases.push((
        value,
        DataOwner::Company {
            company: "acme".into(),
        },
        HarnessIntent::General,
        Vec::new(),
        HarnessAction::DocumentReview,
    ));
    let mut value = settings();
    value["documents"][6]["path"] = "vault/personal/projects/foreign.md".into();
    value["documents"][6]["project_entrypoint"] = false.into();
    cases.push((
        value,
        DataOwner::Personal,
        HarnessIntent::CareerArtifact {
            surface: CareerOutputSurface::General,
        },
        vec![ContextGrant {
            owner: DataOwner::PersonalProject {
                project: "sample".into(),
            },
            purpose: ContextGrantPurpose::CareerWritingEvidence,
            access: ContextAccess::ReadOnly,
        }],
        HarnessAction::DocumentReview,
    ));
    let mut value = settings();
    value["documents"][7]["path"] = "vault/work/{company}/projects/foreign.md".into();
    cases.push((
        value,
        DataOwner::Personal,
        HarnessIntent::CareerArtifact {
            surface: CareerOutputSurface::General,
        },
        vec![ContextGrant {
            owner: DataOwner::CompanyProject {
                company: "acme".into(),
                project: "sample".into(),
            },
            purpose: ContextGrantPurpose::CareerWritingEvidence,
            access: ContextAccess::ReadOnly,
        }],
        HarnessAction::DocumentReview,
    ));
    let mut value = settings();
    value["documents"][8]["path"] = "vault/personal/projects/foreign.md".into();
    cases.push((
        value,
        DataOwner::PersonalProject {
            project: "sample".into(),
        },
        HarnessIntent::General,
        Vec::new(),
        HarnessAction::CodeReview,
    ));
    let mut value = settings();
    value["rules"][0]["roles"] = serde_json::json!(["writer"]);
    cases.push((
        value,
        DataOwner::PersonalProject {
            project: "sample".into(),
        },
        HarnessIntent::General,
        Vec::new(),
        HarnessAction::Investigation,
    ));
    for owner in [
        DataOwner::Company {
            company: "acme".into(),
        },
        DataOwner::CompanyProject {
            company: "acme".into(),
            project: "sample".into(),
        },
    ] {
        let mut value = settings();
        value["company"] = serde_json::Value::Null;
        cases.push((
            value,
            DataOwner::Personal,
            HarnessIntent::CareerArtifact {
                surface: CareerOutputSurface::General,
            },
            vec![ContextGrant {
                owner,
                purpose: ContextGrantPurpose::CareerWritingEvidence,
                access: ContextAccess::ReadOnly,
            }],
            HarnessAction::DocumentReview,
        ));
    }
    for (value, owner, intent, grants, action) in cases {
        let mut engine = fixture.engine();
        engine.vault.policy_configuration =
            PolicyConfiguration::from_bytes(&serde_json::to_vec(&value).unwrap()).unwrap();
        let mut request = SourceFixture::request(owner);
        request.action = action;
        if action == HarnessAction::CodeReview {
            request.targets = vec!["src/example.rs".into()];
        }
        fixture.source.reset_calls();
        assert!(
            engine
                .resolve_with_intent_and_career_composition(&request, intent, &grants, &[], None)
                .is_err()
        );
        let calls = fixture.source.calls.lock().unwrap();
        assert!(
            calls.opens.is_empty()
                && calls.metadata.is_empty()
                && calls.validations.is_empty()
                && calls.children.is_empty()
        );
    }
}

#[test]
fn conflicting_configured_bindings_fail_without_source_access() {
    let fixture = SourceFixture::new();
    let settings = || {
        serde_json::from_slice::<serde_json::Value>(include_bytes!(
            "../../tests/fixtures/policy-settings.json"
        ))
        .unwrap()
    };
    let mut cases = Vec::new();
    let mut direct = settings();
    direct["rules"][0]["documents"] = serde_json::json!(["control"]);
    cases.push((direct, Vec::new(), "orchestrator policies"));
    let mut dependency = settings();
    dependency["documents"][2]["dependencies"] = serde_json::json!(["control"]);
    cases.push((dependency, Vec::new(), "orchestrator policies"));
    let mut alias = settings();
    alias["documents"][2]["path"] = alias["documents"][3]["path"].clone();
    cases.push((alias, Vec::new(), "conflicting identifiers or paths"));
    for entrypoint in ["sample.md", "sample/index.md"] {
        let mut value = settings();
        value["documents"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "key": "project-note", "id": "project-note",
                "path": format!("vault/personal/projects/{entrypoint}"),
                "dependencies": [], "project_entrypoint": false
            }));
        value["rules"][0]["documents"]
            .as_array_mut()
            .unwrap()
            .push("project-note".into());
        cases.push((
            value,
            vec![ContextGrant {
                owner: DataOwner::PersonalProject {
                    project: "sample".into(),
                },
                purpose: ContextGrantPurpose::CareerWritingEvidence,
                access: ContextAccess::ReadOnly,
            }],
            "conflicting identifiers or paths",
        ));
    }
    for (settings, grants, expected) in cases {
        let mut engine = fixture.engine();
        engine.vault.policy_configuration =
            PolicyConfiguration::from_bytes(&serde_json::to_vec(&settings).unwrap()).unwrap();
        let mut request = SourceFixture::request(DataOwner::Personal);
        request.action = HarnessAction::DocumentReview;
        fixture.source.reset_calls();
        let error = engine
            .resolve_with_intent_and_career_composition(
                &request,
                HarnessIntent::CareerArtifact {
                    surface: CareerOutputSurface::General,
                },
                &grants,
                &[],
                None,
            )
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
        let calls = fixture.source.calls.lock().unwrap();
        assert!(
            calls.opens.is_empty()
                && calls.metadata.is_empty()
                && calls.validations.is_empty()
                && calls.children.is_empty()
        );
    }
    for collection in ["leaf_dependencies", "shared_documents"] {
        let mut value = settings();
        value["company"][collection] = serde_json::json!(["control"]);
        let mut engine = fixture.engine();
        engine.vault.policy_configuration =
            PolicyConfiguration::from_bytes(&serde_json::to_vec(&value).unwrap()).unwrap();
        let mut request = SourceFixture::request(DataOwner::Company {
            company: "cedar".into(),
        });
        request.action = HarnessAction::DocumentReview;
        fixture.source.reset_calls();
        let error = engine
            .resolve_with_intent(&request, HarnessIntent::General)
            .unwrap_err();
        assert!(
            error.to_string().contains("orchestrator policies"),
            "{error}"
        );
        let calls = fixture.source.calls.lock().unwrap();
        assert!(
            calls.opens.is_empty()
                && calls.metadata.is_empty()
                && calls.validations.is_empty()
                && calls.children.is_empty()
        );
    }
}

#[test]
fn exact_registry_constraint_and_changed_config_are_revalidated_by_the_engine() {
    let fixture = SourceFixture::new();
    let engine = fixture.engine();
    let mut request = SourceFixture::request(DataOwner::Personal);
    request.action = HarnessAction::Ideation;
    request.targets = vec!["vault/personal/projects/ideas/wrong.md".into()];
    fixture.source.reset_calls();
    assert!(
        engine
            .resolve_with_intent(&request, HarnessIntent::SoloMvpIdeation)
            .is_err()
    );
    assert!(fixture.source.opened_paths().is_empty());
    let (mut engine, resolved) = resolve(&fixture);
    let mut changed = include_bytes!("../../tests/fixtures/policy-settings.json").to_vec();
    changed.push(b' ');
    engine.vault.policy_configuration = PolicyConfiguration::from_bytes(&changed).unwrap();
    fixture.source.reset_calls();
    assert!(matches!(
        engine.revalidate_resolved(&resolved),
        Err(HarnessError::PlanDrift { .. })
    ));
    let calls = fixture.source.calls.lock().unwrap();
    assert!(
        calls.opens.is_empty()
            && calls.metadata.is_empty()
            && calls.validations.is_empty()
            && calls.children.is_empty()
    );
}

struct SyntheticDirectory(PathBuf);
impl SyntheticDirectory {
    fn new() -> Self {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("test clock follows the epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "context-source-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("unique synthetic directory must be created");
        Self(
            path.canonicalize()
                .expect("new synthetic directory must resolve"),
        )
    }
}
impl Drop for SyntheticDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("owned synthetic directory must be removed");
    }
}

#[derive(Default)]
struct SourceCalls {
    opens: Vec<(String, u64)>,
    metadata: Vec<String>,
    validations: Vec<String>,
    children: Vec<String>,
}

struct RecordingSource {
    filesystem: FilesystemContextSource,
    identity: Mutex<SourceStoreIdentity>,
    versions: Mutex<BTreeMap<String, StoredSourceState>>,
    overrides: Mutex<BTreeMap<String, SourceMetadata>>,
    calls: Mutex<SourceCalls>,
}
impl RecordingSource {
    fn new(root: &Path) -> Self {
        fs::create_dir(root.join("vault")).expect("synthetic view must contain vault");
        Self {
            filesystem: FilesystemContextSource::open(root)
                .expect("synthetic source root must open"),
            identity: Mutex::new(SourceStoreIdentity {
                store_id: "synthetic-store".to_owned(),
            }),
            versions: Mutex::new(BTreeMap::new()),
            overrides: Mutex::new(BTreeMap::new()),
            calls: Mutex::new(SourceCalls::default()),
        }
    }
    fn write(&self, path: &str, content: &str, revision: u64) {
        let target = self.view_root().join(path);
        fs::create_dir_all(target.parent().expect("synthetic file has parent"))
            .expect("synthetic parent must be created");
        fs::write(target, content).expect("synthetic file must be written");
        self.versions
            .lock()
            .expect("test version lock is not poisoned")
            .insert(
                path.to_owned(),
                StoredSourceState::Live {
                    material_id: byte_digest(path.as_bytes()),
                    revision,
                    content_digest: byte_digest(content.as_bytes()),
                },
            );
    }
    fn bump(&self, path: &str) {
        let mut versions = self
            .versions
            .lock()
            .expect("test version lock is not poisoned");
        match versions
            .get_mut(path)
            .expect("synthetic version must exist")
        {
            StoredSourceState::Live { revision, .. }
            | StoredSourceState::Deleted { revision, .. } => {
                *revision = revision
                    .checked_add(1)
                    .expect("test revisions stay bounded")
            }
            StoredSourceState::Missing => panic!("bump requires a material revision"),
        }
    }
    fn tombstone(&self, path: &str, revision: u64) {
        let target = self.view_root().join(path);
        if target.exists() {
            fs::remove_file(target).expect("synthetic target must be removable");
        }
        self.versions
            .lock()
            .expect("test version lock is not poisoned")
            .insert(
                path.to_owned(),
                StoredSourceState::Deleted {
                    material_id: byte_digest(path.as_bytes()),
                    revision,
                },
            );
    }
    fn reset_calls(&self) {
        *self.calls.lock().expect("test calls lock is not poisoned") = SourceCalls::default();
    }
    fn opened_paths(&self) -> BTreeSet<String> {
        self.calls
            .lock()
            .expect("test calls lock is not poisoned")
            .opens
            .iter()
            .map(|(path, _)| path.clone())
            .collect()
    }
    fn metadata_value(&self, relative: &Path) -> HarnessResult<SourceMetadata> {
        let path = portable_path(relative);
        if let Some(metadata) = self
            .overrides
            .lock()
            .expect("test override lock is not poisoned")
            .get(&path)
        {
            return Ok(metadata.clone());
        }
        let mut metadata = self.filesystem.metadata(relative)?;
        if metadata.kind != SourcePathKind::Directory {
            metadata.stored_version = Some(SourceVersion {
                logical_path: path.clone(),
                state: self
                    .versions
                    .lock()
                    .expect("test version lock is not poisoned")
                    .get(&path)
                    .cloned()
                    .unwrap_or(StoredSourceState::Missing),
            });
        }
        Ok(metadata)
    }
}
impl ContextSource for RecordingSource {
    fn view_root(&self) -> &Path {
        self.filesystem.view_root()
    }
    fn store_identity(&self) -> HarnessResult<Option<SourceStoreIdentity>> {
        Ok(Some(
            self.identity
                .lock()
                .expect("test identity lock is not poisoned")
                .clone(),
        ))
    }
    fn metadata(&self, path: &Path) -> HarnessResult<SourceMetadata> {
        self.calls
            .lock()
            .expect("test calls lock is not poisoned")
            .metadata
            .push(portable_path(path));
        self.metadata_value(path)
    }
    fn children(&self, path: &Path, limit: usize) -> HarnessResult<Vec<OsString>> {
        self.calls
            .lock()
            .expect("test calls lock is not poisoned")
            .children
            .push(portable_path(path));
        self.filesystem.children(path, limit)
    }
    fn open_file(&self, path: &Path, limit: u64) -> HarnessResult<File> {
        self.calls
            .lock()
            .expect("test calls lock is not poisoned")
            .opens
            .push((portable_path(path), limit));
        self.filesystem.open_file(path, limit)
    }
    fn validate_regular_file(&self, path: &Path) -> HarnessResult<SourceMetadata> {
        self.calls
            .lock()
            .expect("test calls lock is not poisoned")
            .validations
            .push(portable_path(path));
        self.filesystem.validate_regular_file(path)?;
        self.metadata_value(path)
    }
}

struct SourceFixture {
    source: Arc<RecordingSource>,
    workspace: SyntheticDirectory,
    _repository: SyntheticDirectory,
}
impl SourceFixture {
    fn new() -> Self {
        let repository = SyntheticDirectory::new();
        let source = Arc::new(RecordingSource::new(&repository.0));
        for path in [
            "vault/profile/rules/control.md",
            "vault/profile/rules/foundation.md",
            "vault/profile/rules/authoring.md",
            "vault/profile/rules/checks.md",
            "vault/profile/rules/language.md",
            "vault/personal/index.md",
        ] {
            source.write(path, "# Fictional policy\n\nFixture requirements.\n", 1);
        }
        source.write("vault/work/common/router/directory.md", "# Synthetic registry\n\n## Company Registry\n\n| Company | Source |\n| --- | --- |\n| acme | `vault/work/acme/index.md` |\n", 1);
        source.write("vault/work/acme/index.md", "# Acme fixture\n", 1);
        source.write(
            "vault/work/acme/projects/sample.md",
            "# Sample fixture\n",
            1,
        );
        source.write(
            "vault/personal/projects/sample.md",
            "# Sample personal project\n",
            1,
        );
        fs::create_dir_all(source.view_root().join("vault/work/acme/rules"))
            .expect("synthetic company rules must exist");
        for directory in [
            "facts",
            "knowledge",
            "learning",
            "ontology",
            "writing",
            "projects/ideas",
            "decisions",
        ] {
            fs::create_dir_all(
                source
                    .view_root()
                    .join(format!("vault/personal/{directory}")),
            )
            .expect("personal fixture context directory must exist");
        }
        source.write("vault/personal/profile.md", "# Synthetic profile\n", 1);
        let workspace = SyntheticDirectory::new();
        fs::write(
            workspace.0.join("README.md"),
            "# Synthetic external target\n",
        )
        .expect("external target must exist");
        Self {
            source,
            workspace,
            _repository: repository,
        }
    }
    fn engine(&self) -> HarnessEngine {
        HarnessEngine::with_source(
            self.source.view_root(),
            &self.workspace.0,
            self.source.clone(),
            policy_configuration(),
        )
        .expect("recording source must construct the engine")
    }
    fn context_engine(&self) -> HarnessEngine {
        HarnessEngine::with_source(
            self.source.view_root(),
            self.source.view_root(),
            self.source.clone(),
            policy_configuration(),
        )
        .expect("recording source must own the workspace")
    }
    fn request(owner: DataOwner) -> HarnessRequest {
        HarnessRequest {
            action: HarnessAction::DocumentReview,
            owner,
            targets: vec!["README.md".to_owned()],
            objective: "review the exact fixture".to_owned(),
            curation_kind: None,
            explicit_user_confirmation_reported: false,
            curation_sources: Vec::new(),
            delete_targets: Vec::new(),
        }
    }
}
fn capabilities() -> RoleRuntimeCapabilities {
    RoleRuntimeCapabilities {
        available_roles: vec![
            HarnessRole::Writer,
            HarnessRole::Verifier,
            HarnessRole::Reviewer,
            HarnessRole::Specialist,
        ],
        max_concurrent_roles: 5,
        separate_contexts: true,
        file_reading: true,
        tool_execution: true,
        max_role_bundle_bytes: MAX_ROLE_BUNDLE_BYTES,
        max_role_invocation_bytes: MAX_ROLE_INVOCATION_BYTES,
        max_role_execution_millis: MAX_ROLE_EXECUTION_MILLIS,
        max_role_grace_millis: MAX_ROLE_GRACE_MILLIS,
    }
}
fn resolve(fixture: &SourceFixture) -> (HarnessEngine, ResolvedHarnessRequest) {
    let engine = fixture.engine();
    let resolved = engine
        .resolve_with_intent(
            &SourceFixture::request(DataOwner::Profile),
            HarnessIntent::OutputAdapter {
                surface: CareerOutputSurface::General,
            },
        )
        .expect("profile synthetic request must resolve");
    (engine, resolved)
}
fn context_request(path: &str) -> HarnessRequest {
    HarnessRequest {
        action: HarnessAction::DocumentWrite,
        owner: DataOwner::PersonalProject {
            project: "sample".to_owned(),
        },
        targets: vec![path.to_owned()],
        objective: "update the sample project source".to_owned(),
        curation_kind: None,
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    }
}

#[test]
fn common_work_maintenance_only_updates_existing_native_markdown() {
    let fixture = SourceFixture::new();
    let path = "vault/work/common/router/directory.md";
    fixture.source.write(path, "# Fictional directory\n", 1);
    let mut request = SourceFixture::request(DataOwner::CommonWork);
    request.action = HarnessAction::DocumentWrite;
    request.targets = vec![path.to_owned()];
    let engine = fixture.context_engine();
    let resolved = engine
        .resolve_with_intent(&request, HarnessIntent::PolicyMaintenance)
        .expect("existing common work source must resolve");
    assert_eq!(resolved.plan.owner, DataOwner::CommonWork);
    assert!(resolved.plan.learning_sources.is_empty());
    assert!(resolved.plan.source_write_allowed);
    assert!(!resolved.plan.external_write_allowed);
    engine
        .prepare(&resolved, &capabilities(), None)
        .expect("common work maintenance must prepare independent roles");
    assert!(
        engine
            .resolve_with_intent(&request, HarnessIntent::General)
            .is_err()
    );

    for denied in [
        "vault/work/acme/index.md",
        "vault/personal/profile.md",
        "vault/work/common/router/missing.md",
        "vault/work/common/router/directory.txt",
    ] {
        request.targets = vec![denied.to_owned()];
        assert!(
            engine
                .resolve_with_intent(&request, HarnessIntent::PolicyMaintenance)
                .is_err(),
            "unexpected common work target: {denied}"
        );
    }
    request.targets = vec![path.to_owned()];
    assert!(
        fixture
            .engine()
            .resolve_with_intent(&request, HarnessIntent::PolicyMaintenance)
            .is_err()
    );
    request.delete_targets = vec![path.to_owned()];
    assert!(
        engine
            .resolve_with_intent(&request, HarnessIntent::PolicyMaintenance)
            .is_err()
    );
    request.delete_targets.clear();
    request.action = HarnessAction::DocumentReview;
    assert!(
        engine
            .resolve_with_intent(&request, HarnessIntent::PolicyMaintenance)
            .is_err()
    );
}

#[test]
fn source_profile_resolution_and_preparation_acquire_only_required_bodies_with_bounded_calls() {
    for unrelated_count in [1, 8] {
        let fixture = SourceFixture::new();
        for index in 0..unrelated_count {
            fixture.source.write(
                &format!("vault/personal/projects/unrelated-{index}.md"),
                "# Private fixture\n",
                1,
            );
        }
        fixture.source.reset_calls();
        let (engine, resolved) = resolve(&fixture);
        let prepared = engine
            .prepare(&resolved, &capabilities(), None)
            .expect("profile role must prepare");
        let expected = resolved
            .plan
            .required_policies
            .iter()
            .map(|policy| policy.repository_relative_path.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(fixture.source.opened_paths(), expected);
        assert_eq!(
            resolved
                .plan
                .source_versions
                .versions
                .iter()
                .map(|version| version.logical_path.clone())
                .collect::<BTreeSet<_>>(),
            expected
        );
        assert_eq!(
            prepared.bound_source_versions(),
            resolved.plan.bound_source_versions()
        );
        assert!(
            !prepared
                .source_versions
                .versions
                .iter()
                .any(|version| version.logical_path == "README.md")
        );
        let calls = fixture
            .source
            .calls
            .lock()
            .expect("test calls lock is not poisoned");
        assert!(
            calls.opens.len() <= 4 * expected.len(),
            "body calls are bounded by selected policies, independently of unrelated input size"
        );
        assert!(
            calls.metadata.len() <= 12 * expected.len() + 8,
            "metadata calls remain linear in the exact selection"
        );
        assert!(
            calls
                .opens
                .iter()
                .all(|(_, limit)| *limit == MAX_POLICY_BYTES)
        );
        assert!(calls.children.is_empty());
    }
}

#[test]
fn source_company_project_routing_validates_unselected_policies_without_bodies() {
    for unselected_count in [1, 5] {
        let fixture = SourceFixture::new();
        let mut routes = "---\ndomain_routes:\n  - signals: [selected]\n    actions: [document-review]\n    target_kind: any\n    policies: [vault/work/acme/rules/selected.md]\n".to_owned();
        fixture
            .source
            .write("vault/work/acme/rules/selected.md", "# Selected\n", 1);
        for index in 0..unselected_count {
            routes.push_str(&format!("  - signals: [unused{index}]\n    actions: [document-review]\n    target_kind: any\n    policies: [vault/work/acme/rules/unused-{index}.md]\n"));
            fixture.source.write(
                &format!("vault/work/acme/rules/unused-{index}.md"),
                "# Unselected\n",
                1,
            );
        }
        routes.push_str("---\n# Synthetic routing\n");
        fixture
            .source
            .write("vault/work/acme/preferences/routes.md", &routes, 1);
        let engine = fixture.engine();
        let mut request = SourceFixture::request(DataOwner::CompanyProject {
            company: "acme".to_owned(),
            project: "sample".to_owned(),
        });
        request.objective = "selected domain review".to_owned();
        fixture.source.reset_calls();
        let resolved = engine
            .resolve(&request)
            .expect("registered company project must resolve");
        engine
            .prepare(&resolved, &capabilities(), None)
            .expect("company project must prepare");
        assert!(
            resolved
                .plan
                .allowed_context_roots
                .iter()
                .any(|root| root.repository_relative_path == "vault/work/acme/projects/sample.md")
        );
        assert!(
            resolved.plan.required_policies.iter().any(
                |policy| policy.repository_relative_path == "vault/work/acme/rules/selected.md"
            )
        );
        let opened = fixture.source.opened_paths();
        for index in 0..unselected_count {
            assert!(!opened.contains(&format!("vault/work/acme/rules/unused-{index}.md")));
        }
        assert!(
            !opened
                .iter()
                .any(|path| path.starts_with("vault/personal/"))
        );
        let calls = fixture
            .source
            .calls
            .lock()
            .expect("test calls lock is not poisoned");
        assert_eq!(calls.validations.len(), 2 * (unselected_count + 1));
        assert!(
            calls.metadata.len()
                <= 14 * resolved.plan.required_policies.len() + 4 * unselected_count + 30
        );
    }
}

#[test]
fn source_context_targets_bind_revisions_and_reject_same_bytes_changes() {
    let fixture = SourceFixture::new();
    let engine = fixture.context_engine();
    let target = "vault/personal/projects/sample.md";
    let resolved = engine
        .resolve(&context_request(target))
        .expect("canonical target must resolve");
    let frozen = HarnessPlan::from_resolved(
        resolved.clone(),
        fixture.source.view_root(),
        RoleLifecycleLimits {
            max_role_execution_millis: 100,
            max_role_grace_millis: 10,
            max_role_close_millis: 10,
            max_total_role_millis: 120,
        },
    )
    .expect("context target must freeze");
    assert_eq!(
        frozen.bound_source_versions(),
        resolved.plan.bound_source_versions()
    );
    assert!(
        frozen
            .bound_source_versions()
            .versions
            .iter()
            .any(|version| version.logical_path == target
                && matches!(version.state, StoredSourceState::Live { revision: 1, .. }))
    );
    let prepared = engine
        .prepare(&resolved, &capabilities(), None)
        .expect("canonical target must prepare");
    fixture.source.bump(target);
    assert!(engine.begin_execution(&resolved, &prepared, &[]).is_err());
    assert!(engine.prepare(&resolved, &capabilities(), None).is_err());
}

#[test]
fn source_absent_target_tombstone_and_recreation_do_not_collapse_to_byte_identity() {
    let fixture = SourceFixture::new();
    let engine = fixture.context_engine();
    let target = "vault/personal/projects/sample/new.md";
    let request = context_request(target);
    let missing = engine
        .resolve(&request)
        .expect("absent canonical create target must resolve");
    assert!(
        missing
            .plan
            .source_versions
            .versions
            .iter()
            .any(|version| version.logical_path == target
                && version.state == StoredSourceState::Missing)
    );
    fixture.source.write(target, "# Created and deleted\n", 1);
    fixture.source.tombstone(target, 2);
    assert!(engine.prepare(&missing, &capabilities(), None).is_err());
    let deleted = engine
        .resolve(&request)
        .expect("deleted create target must retain a tombstone binding");
    assert!(
        deleted
            .plan
            .source_versions
            .versions
            .iter()
            .any(|version| version.logical_path == target
                && matches!(
                    version.state,
                    StoredSourceState::Deleted { revision: 2, .. }
                ))
    );
    fixture.source.write(target, "# Created and deleted\n", 3);
    assert!(engine.prepare(&deleted, &capabilities(), None).is_err());
    fixture.source.tombstone(target, 4);
    assert!(engine.prepare(&deleted, &capabilities(), None).is_err());
}

#[test]
fn source_policy_revision_drift_and_forged_identity_fail_before_role_execution() {
    let fixture = SourceFixture::new();
    let (engine, resolved) = resolve(&fixture);
    let prepared = engine
        .prepare(&resolved, &capabilities(), None)
        .expect("profile must prepare");
    fixture
        .source
        .bump(&resolved.plan.required_policies[0].repository_relative_path);
    assert!(engine.begin_execution(&resolved, &prepared, &[]).is_err());
    let resolved = engine
        .resolve_with_intent(
            &SourceFixture::request(DataOwner::Profile),
            HarnessIntent::OutputAdapter {
                surface: CareerOutputSurface::General,
            },
        )
        .expect("fresh revision can resolve");
    *fixture
        .source
        .identity
        .lock()
        .expect("test identity lock is not poisoned") = SourceStoreIdentity {
        store_id: "forged-store".to_owned(),
    };
    assert!(engine.prepare(&resolved, &capabilities(), None).is_err());
    let other_engine = fixture.engine();
    assert!(
        other_engine
            .prepare(&resolved, &capabilities(), None)
            .is_err(),
        "fresh provider construction cannot erase a frozen store identity"
    );
}

#[test]
fn source_metadata_digest_kind_and_path_forgery_fail_in_actual_resolution() {
    let path = "vault/profile/rules/control.md";
    for forged in [
        SourceVersion {
            logical_path: path.to_owned(),
            state: StoredSourceState::Live {
                material_id: "material".to_owned(),
                revision: 1,
                content_digest: "0".repeat(64),
            },
        },
        SourceVersion {
            logical_path: path.to_owned(),
            state: StoredSourceState::Live {
                material_id: "material".to_owned(),
                revision: 0,
                content_digest: "0".repeat(64),
            },
        },
        SourceVersion {
            logical_path: path.to_owned(),
            state: StoredSourceState::Deleted {
                material_id: "material".to_owned(),
                revision: 1,
            },
        },
        SourceVersion {
            logical_path: "vault/profile/../forged.md".to_owned(),
            state: StoredSourceState::Missing,
        },
    ] {
        let fixture = SourceFixture::new();
        fixture
            .source
            .overrides
            .lock()
            .expect("test override lock is not poisoned")
            .insert(
                path.to_owned(),
                SourceMetadata {
                    kind: SourcePathKind::RegularFile,
                    stored_version: Some(forged),
                },
            );
        let engine = fixture.engine();
        fixture.source.reset_calls();
        assert!(
            engine
                .resolve_with_intent(
                    &SourceFixture::request(DataOwner::Profile),
                    HarnessIntent::OutputAdapter {
                        surface: CareerOutputSurface::General
                    }
                )
                .is_err()
        );
        assert!(
            fixture
                .source
                .calls
                .lock()
                .expect("test calls lock is not poisoned")
                .opens
                .len()
                <= 1,
            "failure does not retry or expand body acquisition"
        );
    }
}

#[test]
fn source_version_bindings_reject_extra_missing_duplicates_reordering_and_old_schema() {
    let fixture = SourceFixture::new();
    let (engine, resolved) = resolve(&fixture);
    let prepared = engine
        .prepare(&resolved, &capabilities(), None)
        .expect("profile must prepare");
    for mutation in 0..5 {
        let mut forged = resolved.clone();
        match mutation {
            0 => {
                forged.plan.source_versions.versions.push(SourceVersion {
                    logical_path: "vault/work/extra.md".to_owned(),
                    state: StoredSourceState::Missing,
                });
            }
            1 => {
                forged.plan.source_versions.versions.pop();
            }
            2 => {
                forged
                    .plan
                    .source_versions
                    .versions
                    .push(forged.plan.source_versions.versions[0].clone());
            }
            3 => {
                forged.plan.source_versions.versions.reverse();
            }
            _ => {
                forged.plan.version = 6;
            }
        }
        forged.resolved_plan_digest =
            serialized_digest(&forged.plan).expect("forged plan must serialize");
        assert!(engine.prepare(&forged, &capabilities(), None).is_err());
    }
    for mutation in 0..4 {
        let mut forged = prepared.clone();
        match mutation {
            0 => {
                forged.source_versions.versions.push(SourceVersion {
                    logical_path: "vault/work/extra.md".to_owned(),
                    state: StoredSourceState::Missing,
                });
            }
            1 => {
                forged.source_versions.versions.pop();
            }
            2 => {
                forged
                    .source_versions
                    .versions
                    .push(forged.source_versions.versions[0].clone());
            }
            _ => {
                forged.source_versions.versions.reverse();
            }
        }
        forged.prepared_role_run_digest = serialized_digest(&(
            &forged.resolved_plan_digest,
            &forged.runtime_capabilities_digest,
            &forged.source_versions,
            &forged.context_bundle,
            &forged.role_bundles,
            &forged.role_metadata,
        ))
        .expect("forged prepared role must serialize");
        assert!(engine.begin_execution(&resolved, &forged, &[]).is_err());
    }
    let mut json = serde_json::to_value(&resolved.plan).expect("plan must serialize");
    json.as_object_mut()
        .expect("plan is an object")
        .remove("source_versions");
    assert!(
        decode_current_json::<ResolvedHarnessPlan>(
            &serde_json::to_vec(&json).expect("omitted binding JSON serializes"),
            "resolved plan"
        )
        .is_err()
    );
}

#[test]
fn source_all_target_scope_checks_precede_any_target_body_acquisition() {
    let fixture = SourceFixture::new();
    let engine = fixture.context_engine();
    fixture.source.write(
        "vault/personal/projects/sample/a.md",
        "# Allowed target\n",
        1,
    );
    let mut request = context_request("vault/personal/projects/sample/a.md");
    request.targets.push("vault/work/acme/index.md".to_owned());
    fixture.source.reset_calls();
    assert!(engine.resolve(&request).is_err());
    assert!(
        !fixture
            .source
            .opened_paths()
            .contains("vault/personal/projects/sample/a.md")
    );
    assert!(
        !fixture
            .source
            .opened_paths()
            .contains("vault/work/acme/index.md")
    );
}

#[test]
fn source_retrieval_binds_only_selected_documents_and_preserves_raw_and_byte_limits() {
    let fixture = SourceFixture::new();
    let root = "vault/personal/projects/sample/knowledge";
    fixture.source.write(
        &format!("{root}/selected.md"),
        "# Match\n\nneedle selected evidence\n",
        1,
    );
    fixture.source.write(
        &format!("{root}/unmatched.md"),
        "# Different\n\nother words\n",
        1,
    );
    fixture.source.write(
        &format!("{root}/conversations/raw/denied.md"),
        "# needle denied raw\n",
        1,
    );
    fixture.source.write(
        "vault/personal/journal/denied.md",
        "# needle denied journal\n",
        1,
    );
    let engine = fixture.engine();
    let request = HarnessRequest {
        action: HarnessAction::VaultRead,
        owner: DataOwner::PersonalProject {
            project: "sample".to_owned(),
        },
        targets: Vec::new(),
        objective: "retrieve selected knowledge".to_owned(),
        curation_kind: Some(CurationKind::Knowledge),
        explicit_user_confirmation_reported: false,
        curation_sources: Vec::new(),
        delete_targets: Vec::new(),
    };
    let envelope = RequestEnvelope::from_structured_vault_read(
        request,
        HarnessExecutionProfile::Standard,
        "needle".to_owned(),
        4096,
        "synthetic explicit retrieval",
    )
    .expect("synthetic retrieval envelope must build");
    let resolved = engine
        .resolve_test_envelope(&envelope, None)
        .expect("project knowledge read must resolve");
    fixture.source.reset_calls();
    let context = engine
        .retrieve_context(&resolved, "needle", 4096)
        .expect("matching context must be retrieved");
    assert_eq!(context.documents.len(), 1);
    assert_eq!(
        context.documents[0].repository_relative_path,
        format!("{root}/selected.md")
    );
    let prepared = engine
        .prepare(&resolved, &capabilities(), Some(&context))
        .expect("retrieval must prepare with exact context");
    assert!(
        prepared
            .source_versions
            .versions
            .iter()
            .any(|version| version.logical_path == format!("{root}/selected.md"))
    );
    assert!(
        !prepared
            .source_versions
            .versions
            .iter()
            .any(|version| version.logical_path == format!("{root}/unmatched.md"))
    );
    assert!(
        !fixture
            .source
            .opened_paths()
            .iter()
            .any(|path| path.contains("conversations/raw") || path.contains("/journal/"))
    );
    assert!(
        engine
            .retrieve_context(&resolved, "needle", MAX_RETRIEVAL_BYTES + 1)
            .is_err()
    );
    fixture.source.bump(&format!("{root}/selected.md"));
    assert!(engine.begin_execution(&resolved, &prepared, &[]).is_err());
    fixture.source.write(
        &format!("{root}/oversized.md"),
        &"x".repeat(usize::try_from(MAX_RETRIEVAL_FILE_BYTES).expect("test limit fits usize") + 1),
        1,
    );
    assert!(engine.retrieve_context(&resolved, "needle", 4096).is_err());
}

#[test]
fn source_granted_evidence_keeps_selection_and_binds_non_target_revision() {
    let fixture = SourceFixture::new();
    let engine = fixture.engine();
    let evidence = "vault/work/acme/projects/sample.md";
    let request = SourceFixture::request(DataOwner::Personal);
    let grant = ContextGrant {
        owner: DataOwner::CompanyProject {
            company: "acme".to_owned(),
            project: "sample".to_owned(),
        },
        purpose: ContextGrantPurpose::CareerWritingEvidence,
        access: ContextAccess::ReadOnly,
    };
    let resolved = engine
        .resolve_with_intent_and_career_composition(
            &request,
            HarnessIntent::CareerArtifact {
                surface: CareerOutputSurface::General,
            },
            &[grant],
            &[evidence.to_owned()],
            None,
        )
        .expect("exact company evidence grant must resolve");
    assert_eq!(resolved.plan.evidence_sources.len(), 1);
    assert_eq!(
        resolved.plan.evidence_sources[0].repository_relative_path,
        evidence
    );
    let prepared = engine
        .prepare(&resolved, &capabilities(), None)
        .expect("granted evidence must prepare");
    assert!(
        prepared
            .role_bundles
            .iter()
            .flat_map(HarnessRoleBundle::bound_documents)
            .any(
                |document| document.source == HarnessBoundDocumentSource::Evidence
                    && document.relative_path == evidence
            )
    );
    fixture.source.bump(evidence);
    assert!(engine.begin_execution(&resolved, &prepared, &[]).is_err());
}

#[test]
fn source_learning_discovery_retains_owner_and_primary_role_selection() {
    let fixture = SourceFixture::new();
    let path = "vault/personal/projects/sample/knowledge/learning.md";
    let frontmatter = LearningDocumentFrontmatter {
        title: "Synthetic learning".to_owned(),
        scope: "personal".to_owned(),
        export: false,
        learning: LearningMetadata {
            learning_id: "learning-0000000000000001".to_owned(),
            owner: DataOwner::PersonalProject {
                project: "sample".to_owned(),
            },
            action: HarnessAction::DocumentReview,
            intent: HarnessIntent::General,
            verification_unit: VerificationUnit::CompletionContract,
            source_task_evaluation_receipt_digest: "1".repeat(64),
            promotion_handoff_digest: "2".repeat(64),
        },
    };
    let text = format!(
        "---\n{}---\n# Synthetic learning\n\nCheck the completion contract.\n",
        serde_yaml_ng::to_string(&frontmatter)
            .expect("synthetic learning frontmatter must serialize")
    );
    fixture.source.write(path, &text, 1);
    fixture
        .source
        .write("vault/work/acme/knowledge/unrelated.md", &text, 1);
    let engine = fixture.engine();
    let resolved = engine
        .resolve(&SourceFixture::request(DataOwner::PersonalProject {
            project: "sample".to_owned(),
        }))
        .expect("synthetic learning request must resolve");
    assert_eq!(resolved.plan.learning_sources.len(), 1);
    assert_eq!(
        resolved.plan.learning_sources[0].repository_relative_path,
        path
    );
    let prepared = engine
        .prepare(&resolved, &capabilities(), None)
        .expect("learning context must prepare");
    for bundle in &prepared.role_bundles {
        let learning = bundle
            .bound_documents()
            .filter(|document| document.source == HarnessBoundDocumentSource::Learning)
            .count();
        assert_eq!(
            learning,
            usize::from(bundle.role == resolved.plan.primary_producer_role)
        );
    }
    assert!(
        !fixture
            .source
            .opened_paths()
            .contains("vault/work/acme/knowledge/unrelated.md")
    );
    fixture.source.bump(path);
    assert!(engine.begin_execution(&resolved, &prepared, &[]).is_err());
}
