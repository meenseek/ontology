use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{
    ContextGrant, CurationKind, DataOwner, HARNESS_SCHEMA_VERSION, HarnessAction, HarnessError,
    HarnessExecutionProfile, HarnessIntent, HarnessRequest, HarnessResult, MAX_RETRIEVAL_BYTES,
    MAX_RETRIEVAL_QUERY_LENGTH, PolicyBinding, PromotionHandoff, serialized_digest,
    validate_single_line,
};

const REQUEST_TEXT_LIMIT: usize = 16 * 1024;
const REQUEST_LIST_LIMIT: usize = 128;
const OPAQUE_IDENTIFIER_HEX_LENGTH: usize = 16;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequestEnvelope {
    pub version: u32,
    pub source: RequestSource,
    pub draft: DraftTaskRequest,
    pub decision_trace: DecisionTrace,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum RequestSource {
    UserLanguage { statements: Vec<UserStatement> },
    StructuredCaller { description: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UserStatement {
    pub identifier: String,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionTrace {
    pub records: Vec<DecisionRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum DecisionRecord {
    UserStatement {
        identifier: String,
        value_digest: String,
        statement_identifiers: Vec<String>,
    },
    UserConfirmation {
        identifier: String,
        value_digest: String,
        confirms_record_identifier: String,
        statement_identifiers: Vec<String>,
    },
    PolicyDefault {
        identifier: String,
        value_digest: String,
        policy_identifier: String,
        policy_content_digest: String,
        rule: PolicyDefaultRule,
    },
    ObservedFact {
        identifier: String,
        value_digest: String,
        evidence: DecisionEvidence,
    },
    ModelInterpretation {
        identifier: String,
        value_digest: String,
        based_on_record_identifiers: Vec<String>,
    },
    ModelProposal {
        identifier: String,
        value_digest: String,
        based_on_record_identifiers: Vec<String>,
    },
    StructuredCaller {
        identifier: String,
        value_digest: String,
        description: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionEvidence {
    pub kind: DecisionEvidenceKind,
    pub content_digest: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DecisionEvidenceKind {
    RepositoryState,
    FileContent,
    RuntimeObservation,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PolicyDefaultRule {
    GeneralIntent,
    StandardExecutionProfile,
    NonJournalReadConfirmationNotRequired,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UserConfirmationStatus {
    NotRequired,
    Confirmed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub enum DraftValue<T> {
    Resolved {
        value: T,
        decision_identifier: String,
    },
    Unresolved {
        question: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum DraftTaskRequest {
    Write(DraftWriteContract),
    Review(DraftReviewContract),
    Analysis(DraftAnalysisContract),
    VaultRead(DraftVaultReadContract),
    Curation(Box<DraftCurationContract>),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WriteKind {
    Code,
    Document,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewKind {
    Code,
    Document,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnalysisKind {
    Investigation,
    Design,
    Ideation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DraftTaskCommon {
    pub owner: DraftValue<DataOwner>,
    /// User-grounded Harness intent for write, review, and analysis requests.
    pub intent: DraftValue<HarnessIntent>,
    pub task_statement: DraftValue<String>,
    pub execution_profile: DraftValue<HarnessExecutionProfile>,
    pub context_grants: Option<DraftValue<Vec<ContextGrant>>>,
    pub evidence_source_paths: Option<DraftValue<Vec<String>>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DraftOwnedTaskCommon {
    pub owner: DraftValue<DataOwner>,
    pub task_statement: DraftValue<String>,
    pub execution_profile: DraftValue<HarnessExecutionProfile>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DraftWriteContract {
    pub common: DraftTaskCommon,
    pub write_kind: DraftValue<WriteKind>,
    pub targets: DraftValue<Vec<String>>,
    pub delete_targets: Option<DraftValue<Vec<String>>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DraftReviewContract {
    pub common: DraftTaskCommon,
    pub review_kind: DraftValue<ReviewKind>,
    pub targets: DraftValue<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DraftAnalysisContract {
    pub common: DraftTaskCommon,
    pub analysis_kind: DraftValue<AnalysisKind>,
    pub targets: Option<DraftValue<Vec<String>>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DraftVaultReadContract {
    pub owner: DraftValue<DataOwner>,
    pub task_statement: DraftValue<String>,
    pub execution_profile: DraftValue<HarnessExecutionProfile>,
    pub curation_kind: DraftValue<CurationKind>,
    pub query: DraftValue<String>,
    pub maximum_context_bytes: DraftValue<usize>,
    pub confirmation: DraftValue<UserConfirmationStatus>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DraftCurationContract {
    pub common: DraftOwnedTaskCommon,
    pub curation_kind: DraftValue<CurationKind>,
    pub target: DraftValue<String>,
    pub curation_sources: Option<DraftValue<Vec<String>>>,
    pub delete_target: Option<DraftValue<String>>,
    pub promotion_handoff: Option<DraftValue<PromotionHandoff>>,
    pub confirmation: DraftValue<UserConfirmationStatus>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum ResolvedTaskContract {
    Write(WriteContract),
    Review(ReviewContract),
    Analysis(AnalysisContract),
    VaultRead(VaultReadContract),
    Curation(Box<CurationContract>),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaskContractCommon {
    pub owner: DataOwner,
    /// Resolved Harness intent preserved from the request decision trace.
    pub intent: HarnessIntent,
    pub task_statement: String,
    pub execution_profile: HarnessExecutionProfile,
    pub context_grants: Vec<ContextGrant>,
    pub evidence_source_paths: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OwnedTaskContractCommon {
    pub owner: DataOwner,
    pub task_statement: String,
    pub execution_profile: HarnessExecutionProfile,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WriteContract {
    pub common: TaskContractCommon,
    pub write_kind: WriteKind,
    pub targets: Vec<String>,
    pub delete_targets: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReviewContract {
    pub common: TaskContractCommon,
    pub review_kind: ReviewKind,
    pub targets: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AnalysisContract {
    pub common: TaskContractCommon,
    pub analysis_kind: AnalysisKind,
    pub targets: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VaultReadContract {
    pub owner: DataOwner,
    pub task_statement: String,
    pub execution_profile: HarnessExecutionProfile,
    pub curation_kind: CurationKind,
    pub query: String,
    pub maximum_context_bytes: usize,
    pub confirmation: UserConfirmationStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CurationContract {
    pub common: OwnedTaskContractCommon,
    pub curation_kind: CurationKind,
    pub target: String,
    pub curation_sources: Vec<String>,
    pub delete_target: Option<String>,
    pub promotion_handoff: Option<PromotionHandoff>,
    pub confirmation: UserConfirmationStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequestProvenanceBinding {
    pub compact_request_digest: String,
    pub decision_trace_digest: String,
    pub statement_bindings: Vec<RequestStatementBinding>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequestStatementBinding {
    pub identifier: String,
    pub content_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ClarificationRequest {
    pub field: String,
    pub question: String,
}

impl RequestProvenanceBinding {
    pub(super) fn validate(&self) -> HarnessResult<()> {
        validate_plan_digest("compact request digest", &self.compact_request_digest)?;
        validate_plan_digest("decision trace digest", &self.decision_trace_digest)?;
        if self.statement_bindings.len() > REQUEST_LIST_LIMIT {
            return Err(HarnessError::InvalidPlan(format!(
                "request statement bindings must not exceed {REQUEST_LIST_LIMIT} values"
            )));
        }
        let mut identifiers = BTreeSet::new();
        for binding in &self.statement_bindings {
            validate_plan_opaque_identifier(
                "request statement binding identifier",
                &binding.identifier,
                "statement-",
            )?;
            validate_plan_digest(
                "request statement binding content digest",
                &binding.content_digest,
            )?;
            if !identifiers.insert(binding.identifier.as_str()) {
                return Err(HarnessError::InvalidPlan(format!(
                    "duplicate request statement binding identifier `{}`",
                    binding.identifier
                )));
            }
        }
        Ok(())
    }
}

impl RequestEnvelope {
    #[cfg(test)]
    #[allow(
        clippy::too_many_lines,
        reason = "structured request orchestration and decision digest ordering must remain auditable together"
    )]
    pub fn from_structured_request(
        request: HarnessRequest,
        intent: HarnessIntent,
        context_grants: Vec<ContextGrant>,
        evidence_source_paths: Vec<String>,
        execution_profile: HarnessExecutionProfile,
        caller_description: impl Into<String>,
    ) -> HarnessResult<Self> {
        request.validate()?;
        if request.action == HarnessAction::VaultRead {
            return Err(HarnessError::InvalidRequest(
                "Vault read requires a request envelope with an explicit query and context byte limit"
                    .to_owned(),
                ));
        }
        if request.action == HarnessAction::VaultCuration
            && (!context_grants.is_empty() || !evidence_source_paths.is_empty())
        {
            return Err(HarnessError::InvalidRequest(
                "Vault curation does not accept cross-scope context grants or evidence sources"
                    .to_owned(),
            ));
        }
        let mut decisions = StructuredDecisionBuilder::default();
        if request.action == HarnessAction::VaultCuration {
            let curation_kind = request.curation_kind.ok_or_else(|| {
                HarnessError::InvalidRequest("Vault curation requires a curation kind".to_owned())
            })?;
            let [target] = request.targets.as_slice() else {
                return Err(HarnessError::InvalidRequest(
                    "Vault curation requires exactly one target".to_owned(),
                ));
            };
            let draft = DraftTaskRequest::Curation(Box::new(DraftCurationContract {
                common: DraftOwnedTaskCommon {
                    owner: decisions.value("owner", request.owner)?,
                    task_statement: decisions.value("task-statement", request.objective.clone())?,
                    execution_profile: decisions.value("execution-profile", execution_profile)?,
                },
                curation_kind: decisions.value("curation-kind", curation_kind)?,
                target: decisions.value("target", target.clone())?,
                curation_sources: decisions
                    .optional_value("curation-sources", request.curation_sources)?,
                delete_target: request
                    .delete_targets
                    .into_iter()
                    .next()
                    .map(|target| decisions.value("delete-target", target))
                    .transpose()?,
                promotion_handoff: None,
                confirmation: decisions.value(
                    "confirmation",
                    if request.explicit_user_confirmation_reported {
                        UserConfirmationStatus::Confirmed
                    } else {
                        UserConfirmationStatus::NotRequired
                    },
                )?,
            }));
            return Ok(Self {
                version: HARNESS_SCHEMA_VERSION,
                source: RequestSource::StructuredCaller {
                    description: caller_description.into(),
                },
                draft,
                decision_trace: DecisionTrace {
                    records: decisions.records,
                },
            });
        }
        let common = DraftTaskCommon {
            owner: decisions.value("owner", request.owner.clone())?,
            intent: decisions.value("intent", intent)?,
            task_statement: decisions.value("task-statement", request.objective.clone())?,
            execution_profile: decisions.value("execution-profile", execution_profile)?,
            context_grants: decisions.optional_value("context-grants", context_grants)?,
            evidence_source_paths: decisions
                .optional_value("evidence-source-paths", evidence_source_paths)?,
        };
        let draft = match request.action {
            HarnessAction::CodeWrite | HarnessAction::DocumentWrite => {
                let write_kind = match request.action {
                    HarnessAction::CodeWrite => WriteKind::Code,
                    HarnessAction::DocumentWrite => WriteKind::Document,
                    _ => unreachable!(),
                };
                DraftTaskRequest::Write(DraftWriteContract {
                    common,
                    write_kind: decisions.value("write-kind", write_kind)?,
                    targets: decisions.value("targets", request.targets)?,
                    delete_targets: decisions
                        .optional_value("delete-targets", request.delete_targets)?,
                })
            }
            HarnessAction::CodeReview | HarnessAction::DocumentReview => {
                let review_kind = match request.action {
                    HarnessAction::CodeReview => ReviewKind::Code,
                    HarnessAction::DocumentReview => ReviewKind::Document,
                    _ => unreachable!(),
                };
                DraftTaskRequest::Review(DraftReviewContract {
                    common,
                    review_kind: decisions.value("review-kind", review_kind)?,
                    targets: decisions.value("targets", request.targets)?,
                })
            }
            HarnessAction::Investigation | HarnessAction::Design | HarnessAction::Ideation => {
                let analysis_kind = match request.action {
                    HarnessAction::Investigation => AnalysisKind::Investigation,
                    HarnessAction::Design => AnalysisKind::Design,
                    HarnessAction::Ideation => AnalysisKind::Ideation,
                    _ => unreachable!(),
                };
                DraftTaskRequest::Analysis(DraftAnalysisContract {
                    common,
                    analysis_kind: decisions.value("analysis-kind", analysis_kind)?,
                    targets: decisions.optional_value("targets", request.targets)?,
                })
            }
            HarnessAction::VaultRead | HarnessAction::VaultCuration => unreachable!(),
        };
        Ok(Self {
            version: HARNESS_SCHEMA_VERSION,
            source: RequestSource::StructuredCaller {
                description: caller_description.into(),
            },
            draft,
            decision_trace: DecisionTrace {
                records: decisions.records,
            },
        })
    }

    #[cfg(test)]
    pub fn from_structured_vault_read(
        request: HarnessRequest,
        execution_profile: HarnessExecutionProfile,
        query: String,
        maximum_context_bytes: usize,
        caller_description: impl Into<String>,
    ) -> HarnessResult<Self> {
        request.validate()?;
        if request.action != HarnessAction::VaultRead {
            return Err(HarnessError::InvalidRequest(
                "structured Vault read conversion requires the Vault read action".to_owned(),
            ));
        }
        let curation_kind = request.curation_kind.ok_or_else(|| {
            HarnessError::InvalidRequest("Vault read requires a content kind".to_owned())
        })?;
        let mut decisions = StructuredDecisionBuilder::default();
        let draft = DraftTaskRequest::VaultRead(DraftVaultReadContract {
            owner: decisions.value("owner", request.owner)?,
            task_statement: decisions.value("task-statement", request.objective)?,
            execution_profile: decisions.value("execution-profile", execution_profile)?,
            curation_kind: decisions.value("curation-kind", curation_kind)?,
            query: decisions.value("query", query)?,
            maximum_context_bytes: decisions
                .value("maximum-context-bytes", maximum_context_bytes)?,
            confirmation: decisions.value(
                "confirmation",
                if request.explicit_user_confirmation_reported {
                    UserConfirmationStatus::Confirmed
                } else {
                    UserConfirmationStatus::NotRequired
                },
            )?,
        });
        Ok(Self {
            version: HARNESS_SCHEMA_VERSION,
            source: RequestSource::StructuredCaller {
                description: caller_description.into(),
            },
            draft,
            decision_trace: DecisionTrace {
                records: decisions.records,
            },
        })
    }

    pub fn resolve(
        &self,
    ) -> HarnessResult<(
        ResolvedTaskContract,
        DecisionTrace,
        RequestProvenanceBinding,
    )> {
        self.resolve_internal(false)
    }

    #[cfg(test)]
    pub(super) fn resolve_test_fixture(
        &self,
    ) -> HarnessResult<(
        ResolvedTaskContract,
        DecisionTrace,
        RequestProvenanceBinding,
    )> {
        self.resolve_internal(true)
    }

    fn resolve_internal(
        &self,
        allow_structured_test_fixture: bool,
    ) -> HarnessResult<(
        ResolvedTaskContract,
        DecisionTrace,
        RequestProvenanceBinding,
    )> {
        if self.version != HARNESS_SCHEMA_VERSION {
            return Err(HarnessError::InvalidRequest(format!(
                "request envelope version must be {HARNESS_SCHEMA_VERSION}"
            )));
        }
        if matches!(self.source, RequestSource::StructuredCaller { .. })
            && !allow_structured_test_fixture
        {
            return Err(HarnessError::InvalidRequest(
                "structured-caller request envelopes are not executable".to_owned(),
            ));
        }
        if matches!(self.source, RequestSource::StructuredCaller { .. })
            && self.draft.has_promotion_handoff()
        {
            return Err(HarnessError::InvalidRequest(
                "promotion handoffs require a user-language request and cannot be supplied by a structured caller"
                    .to_owned(),
            ));
        }
        let statements = self.source.validate()?;
        self.decision_trace.validate(&self.source, statements)?;
        let unresolved = self.draft.unresolved_questions();
        if !unresolved.is_empty() {
            return Err(HarnessError::ClarificationRequired(unresolved));
        }
        if !allow_structured_test_fixture {
            self.draft
                .validate_required_policy_authorities(&self.decision_trace)?;
        }
        self.draft
            .validate_promotion_handoff_authority(&self.decision_trace)?;
        let contract = self.draft.resolve(&self.decision_trace)?;
        contract.to_harness_request().validate()?;
        let decision_trace = self
            .decision_trace
            .compact_for(&self.draft.decision_identifiers())?;
        let decision_trace_digest = serialized_digest(&decision_trace)?;
        let referenced_statement_identifiers = decision_trace.statement_identifiers();
        let statement_bindings = statements
            .iter()
            .filter(|statement| {
                referenced_statement_identifiers.contains(statement.identifier.as_str())
            })
            .map(|statement| {
                Ok(RequestStatementBinding {
                    identifier: statement.identifier.clone(),
                    content_digest: serialized_digest(&statement.text)?,
                })
            })
            .collect::<HarnessResult<Vec<_>>>()?;
        let compact_request_digest = serialized_digest(&(
            self.version,
            &self.draft,
            &decision_trace,
            &statement_bindings,
        ))?;
        Ok((
            contract,
            decision_trace,
            RequestProvenanceBinding {
                compact_request_digest,
                decision_trace_digest,
                statement_bindings,
            },
        ))
    }
}

#[cfg(test)]
#[derive(Default)]
struct StructuredDecisionBuilder {
    records: Vec<DecisionRecord>,
}

#[cfg(test)]
impl StructuredDecisionBuilder {
    fn value<T>(&mut self, name: &str, value: T) -> HarnessResult<DraftValue<T>>
    where
        T: Serialize,
    {
        let decision_identifier = format!("structured-{name}");
        self.records.push(DecisionRecord::StructuredCaller {
            identifier: decision_identifier.clone(),
            value_digest: serialized_digest(&value)?,
            description: format!("structured caller supplied {name}"),
        });
        Ok(DraftValue::Resolved {
            value,
            decision_identifier,
        })
    }

    fn optional_value<T>(&mut self, name: &str, value: T) -> HarnessResult<Option<DraftValue<T>>>
    where
        T: IsEmpty + Serialize,
    {
        if value.is_empty() {
            return Ok(None);
        }
        self.value(name, value).map(Some)
    }
}

#[cfg(test)]
trait IsEmpty {
    fn is_empty(&self) -> bool;
}

#[cfg(test)]
impl<T> IsEmpty for Vec<T> {
    fn is_empty(&self) -> bool {
        Vec::is_empty(self)
    }
}

impl RequestSource {
    fn validate(&self) -> HarnessResult<&[UserStatement]> {
        match self {
            Self::UserLanguage { statements } => {
                if statements.is_empty() || statements.len() > REQUEST_LIST_LIMIT {
                    return Err(HarnessError::InvalidRequest(format!(
                        "user-language requests must contain between one and {REQUEST_LIST_LIMIT} statements"
                    )));
                }
                let mut identifiers = BTreeSet::new();
                for statement in statements {
                    validate_opaque_identifier(
                        "user statement identifier",
                        &statement.identifier,
                        "statement-",
                    )?;
                    validate_text("user statement", &statement.text)?;
                    if !identifiers.insert(statement.identifier.as_str()) {
                        return Err(HarnessError::InvalidRequest(format!(
                            "duplicate user statement identifier `{}`",
                            statement.identifier
                        )));
                    }
                }
                Ok(statements)
            }
            Self::StructuredCaller { description } => {
                validate_text("structured caller description", description)?;
                Ok(&[])
            }
        }
    }
}

impl DecisionTrace {
    fn validate(&self, source: &RequestSource, statements: &[UserStatement]) -> HarnessResult<()> {
        if self.records.is_empty() || self.records.len() > REQUEST_LIST_LIMIT {
            return Err(HarnessError::InvalidRequest(format!(
                "decision trace must contain between one and {REQUEST_LIST_LIMIT} records"
            )));
        }
        let statement_identifiers = statements
            .iter()
            .map(|statement| statement.identifier.as_str())
            .collect::<BTreeSet<_>>();
        let mut records = BTreeMap::new();
        for record in &self.records {
            if matches!(source, RequestSource::UserLanguage { .. }) {
                validate_opaque_identifier(
                    "decision record identifier",
                    record.identifier(),
                    "decision-",
                )?;
            } else {
                validate_text("decision record identifier", record.identifier())?;
            }
            validate_digest("decision value digest", record.value_digest())?;
            if records.insert(record.identifier(), record).is_some() {
                return Err(HarnessError::InvalidRequest(format!(
                    "duplicate decision record identifier `{}`",
                    record.identifier()
                )));
            }
        }
        let mut prior_record_identifiers = BTreeSet::new();
        for record in &self.records {
            match (source, record) {
                (RequestSource::UserLanguage { .. }, DecisionRecord::StructuredCaller { .. }) => {
                    return Err(HarnessError::InvalidRequest(
                        "user-language requests cannot use structured-caller decisions".to_owned(),
                    ));
                }
                (
                    RequestSource::StructuredCaller { .. },
                    DecisionRecord::UserStatement { .. } | DecisionRecord::UserConfirmation { .. },
                ) => {
                    return Err(HarnessError::InvalidRequest(
                        "structured-caller requests cannot use user statement or confirmation decisions"
                            .to_owned(),
                    ));
                }
                _ => {}
            }
            record.validate_references(&records, &statement_identifiers)?;
            record.validate_chronology(&prior_record_identifiers)?;
            prior_record_identifiers.insert(record.identifier());
        }
        Ok(())
    }

    fn authoritative_record(&self, identifier: &str) -> HarnessResult<&DecisionRecord> {
        let record = self
            .records
            .iter()
            .find(|record| record.identifier() == identifier)
            .ok_or_else(|| {
                HarnessError::InvalidRequest(format!(
                    "decision record `{identifier}` does not exist"
                ))
            })?;
        if !record.is_authoritative() {
            return Err(HarnessError::InvalidRequest(format!(
                "decision record `{identifier}` is not authoritative and cannot authorize an execution value"
            )));
        }
        Ok(record)
    }

    pub(super) fn validate_policy_bindings(
        &self,
        required_policies: &[PolicyBinding],
    ) -> HarnessResult<()> {
        for record in &self.records {
            let DecisionRecord::PolicyDefault {
                policy_identifier,
                policy_content_digest,
                ..
            } = record
            else {
                continue;
            };
            if !required_policies.iter().any(|policy| {
                policy.id == *policy_identifier && policy.content_digest == *policy_content_digest
            }) {
                return Err(HarnessError::InvalidPlan(format!(
                    "policy default `{policy_identifier}` is not bound to the exact policy content selected by the plan"
                )));
            }
        }
        Ok(())
    }

    fn compact_for(&self, direct_identifiers: &BTreeSet<String>) -> HarnessResult<Self> {
        let records = self
            .records
            .iter()
            .map(|record| (record.identifier(), record))
            .collect::<BTreeMap<_, _>>();
        let mut required = direct_identifiers.clone();
        let mut pending = direct_identifiers.iter().cloned().collect::<Vec<_>>();
        while let Some(identifier) = pending.pop() {
            let record = records.get(identifier.as_str()).ok_or_else(|| {
                HarnessError::InvalidRequest(format!(
                    "decision record `{identifier}` does not exist"
                ))
            })?;
            for reference in record.decision_references() {
                if required.insert(reference.to_owned()) {
                    pending.push(reference.to_owned());
                }
            }
        }
        Ok(Self {
            records: self
                .records
                .iter()
                .filter(|record| required.contains(record.identifier()))
                .cloned()
                .collect(),
        })
    }

    fn statement_identifiers(&self) -> BTreeSet<&str> {
        self.records
            .iter()
            .flat_map(DecisionRecord::statement_identifiers)
            .collect()
    }
}

impl DecisionRecord {
    fn identifier(&self) -> &str {
        match self {
            Self::UserStatement { identifier, .. }
            | Self::UserConfirmation { identifier, .. }
            | Self::PolicyDefault { identifier, .. }
            | Self::ObservedFact { identifier, .. }
            | Self::ModelInterpretation { identifier, .. }
            | Self::ModelProposal { identifier, .. }
            | Self::StructuredCaller { identifier, .. } => identifier,
        }
    }

    fn value_digest(&self) -> &str {
        match self {
            Self::UserStatement { value_digest, .. }
            | Self::UserConfirmation { value_digest, .. }
            | Self::PolicyDefault { value_digest, .. }
            | Self::ObservedFact { value_digest, .. }
            | Self::ModelInterpretation { value_digest, .. }
            | Self::ModelProposal { value_digest, .. }
            | Self::StructuredCaller { value_digest, .. } => value_digest,
        }
    }

    fn decision_references(&self) -> Vec<&str> {
        match self {
            Self::UserConfirmation {
                confirms_record_identifier,
                ..
            } => vec![confirms_record_identifier],
            Self::ModelInterpretation {
                based_on_record_identifiers,
                ..
            }
            | Self::ModelProposal {
                based_on_record_identifiers,
                ..
            } => based_on_record_identifiers
                .iter()
                .map(String::as_str)
                .collect(),
            _ => Vec::new(),
        }
    }

    fn statement_identifiers(&self) -> Vec<&str> {
        match self {
            Self::UserStatement {
                statement_identifiers,
                ..
            }
            | Self::UserConfirmation {
                statement_identifiers,
                ..
            } => statement_identifiers.iter().map(String::as_str).collect(),
            _ => Vec::new(),
        }
    }

    fn is_authoritative(&self) -> bool {
        matches!(
            self,
            Self::UserStatement { .. }
                | Self::UserConfirmation { .. }
                | Self::PolicyDefault { .. }
                | Self::StructuredCaller { .. }
        )
    }

    fn validate_references<'a>(
        &'a self,
        records: &BTreeMap<&'a str, &'a DecisionRecord>,
        statement_identifiers: &BTreeSet<&str>,
    ) -> HarnessResult<()> {
        match self {
            Self::UserStatement {
                statement_identifiers: references,
                ..
            } => validate_statement_references(references, statement_identifiers),
            Self::UserConfirmation {
                value_digest,
                confirms_record_identifier,
                statement_identifiers: references,
                ..
            } => {
                validate_statement_references(references, statement_identifiers)?;
                let confirmed = records
                    .get(confirms_record_identifier.as_str())
                    .ok_or_else(|| {
                        HarnessError::InvalidRequest(format!(
                            "confirmed decision record `{confirms_record_identifier}` does not exist"
                        ))
                    })?;
                if confirmed.is_authoritative() || confirmed.value_digest() != value_digest {
                    return Err(HarnessError::InvalidRequest(
                        "user confirmation must confirm a non-authoritative observation, interpretation, or proposal with the same value"
                            .to_owned(),
                    ));
                }
                Ok(())
            }
            Self::PolicyDefault {
                policy_identifier,
                policy_content_digest,
                rule,
                ..
            } => {
                validate_text("policy default identifier", policy_identifier)?;
                validate_digest("policy default content digest", policy_content_digest)?;
                if policy_identifier != PolicyDefaultRule::POLICY_IDENTIFIER {
                    return Err(HarnessError::InvalidRequest(format!(
                        "policy default rule `{}` must use policy `{}`",
                        rule.as_str(),
                        PolicyDefaultRule::POLICY_IDENTIFIER
                    )));
                }
                Ok(())
            }
            Self::ObservedFact { evidence, .. } => {
                validate_digest("decision evidence content digest", &evidence.content_digest)
            }
            Self::ModelInterpretation {
                based_on_record_identifiers,
                ..
            }
            | Self::ModelProposal {
                based_on_record_identifiers,
                ..
            } => validate_record_references(based_on_record_identifiers, records),
            Self::StructuredCaller { description, .. } => {
                validate_text("structured caller decision description", description)
            }
        }
    }

    fn validate_chronology(&self, prior_record_identifiers: &BTreeSet<&str>) -> HarnessResult<()> {
        match self {
            Self::UserConfirmation {
                confirms_record_identifier,
                ..
            } => validate_prior_record_reference(
                confirms_record_identifier,
                prior_record_identifiers,
            ),
            Self::ModelInterpretation {
                based_on_record_identifiers,
                ..
            }
            | Self::ModelProposal {
                based_on_record_identifiers,
                ..
            } => {
                for identifier in based_on_record_identifiers {
                    validate_prior_record_reference(identifier, prior_record_identifiers)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn validate_field_authority(&self, field: &str, value_digest: &str) -> HarnessResult<()> {
        let Self::PolicyDefault { rule, .. } = self else {
            return Ok(());
        };
        if rule.field() != field || rule.value_digest()? != value_digest {
            return Err(HarnessError::InvalidRequest(format!(
                "policy default rule `{}` does not authorize field `{field}` with this value",
                rule.as_str()
            )));
        }
        Ok(())
    }
}

impl PolicyDefaultRule {
    const POLICY_IDENTIFIER: &str = "agent-harness";

    const fn as_str(self) -> &'static str {
        match self {
            Self::GeneralIntent => "general-intent",
            Self::StandardExecutionProfile => "standard-execution-profile",
            Self::NonJournalReadConfirmationNotRequired => {
                "non-journal-read-confirmation-not-required"
            }
        }
    }

    const fn field(self) -> &'static str {
        match self {
            Self::GeneralIntent => "intent",
            Self::StandardExecutionProfile => "execution_profile",
            Self::NonJournalReadConfirmationNotRequired => "confirmation",
        }
    }

    fn value_digest(self) -> HarnessResult<String> {
        match self {
            Self::GeneralIntent => serialized_digest(&HarnessIntent::General),
            Self::StandardExecutionProfile => serialized_digest(&HarnessExecutionProfile::Standard),
            Self::NonJournalReadConfirmationNotRequired => {
                serialized_digest(&UserConfirmationStatus::NotRequired)
            }
        }
    }
}

impl DraftTaskRequest {
    fn unresolved_questions(&self) -> Vec<ClarificationRequest> {
        let mut questions = Vec::new();
        match self {
            Self::Write(contract) => {
                contract.common.collect_unresolved(&mut questions);
                collect_unresolved("write_kind", &contract.write_kind, &mut questions);
                collect_unresolved("targets", &contract.targets, &mut questions);
                collect_optional_unresolved(
                    "delete_targets",
                    contract.delete_targets.as_ref(),
                    &mut questions,
                );
            }
            Self::Review(contract) => {
                contract.common.collect_unresolved(&mut questions);
                collect_unresolved("review_kind", &contract.review_kind, &mut questions);
                collect_unresolved("targets", &contract.targets, &mut questions);
            }
            Self::Analysis(contract) => {
                contract.common.collect_unresolved(&mut questions);
                collect_unresolved("analysis_kind", &contract.analysis_kind, &mut questions);
                collect_optional_unresolved("targets", contract.targets.as_ref(), &mut questions);
            }
            Self::VaultRead(contract) => {
                collect_unresolved("owner", &contract.owner, &mut questions);
                collect_unresolved("task_statement", &contract.task_statement, &mut questions);
                collect_unresolved(
                    "execution_profile",
                    &contract.execution_profile,
                    &mut questions,
                );
                collect_unresolved("curation_kind", &contract.curation_kind, &mut questions);
                collect_unresolved("query", &contract.query, &mut questions);
                collect_unresolved(
                    "maximum_context_bytes",
                    &contract.maximum_context_bytes,
                    &mut questions,
                );
                collect_unresolved("confirmation", &contract.confirmation, &mut questions);
            }
            Self::Curation(contract) => {
                contract.common.collect_unresolved(&mut questions);
                collect_unresolved("curation_kind", &contract.curation_kind, &mut questions);
                collect_unresolved("target", &contract.target, &mut questions);
                collect_optional_unresolved(
                    "curation_sources",
                    contract.curation_sources.as_ref(),
                    &mut questions,
                );
                collect_optional_unresolved(
                    "delete_target",
                    contract.delete_target.as_ref(),
                    &mut questions,
                );
                collect_optional_unresolved(
                    "promotion_handoff",
                    contract.promotion_handoff.as_ref(),
                    &mut questions,
                );
                collect_unresolved("confirmation", &contract.confirmation, &mut questions);
            }
        }
        questions
    }

    fn has_promotion_handoff(&self) -> bool {
        matches!(self, Self::Curation(contract) if contract.promotion_handoff.is_some())
    }

    fn validate_promotion_handoff_authority(&self, trace: &DecisionTrace) -> HarnessResult<()> {
        let Self::Curation(contract) = self else {
            return Ok(());
        };
        let Some(DraftValue::Resolved {
            value,
            decision_identifier,
        }) = &contract.promotion_handoff
        else {
            return Ok(());
        };
        let record = trace
            .records
            .iter()
            .find(|record| record.identifier() == decision_identifier)
            .ok_or_else(|| {
                HarnessError::InvalidRequest(format!(
                    "promotion handoff decision record `{decision_identifier}` does not exist"
                ))
            })?;
        if !matches!(record, DecisionRecord::UserConfirmation { .. })
            || record.value_digest() != serialized_digest(value)?
        {
            return Err(HarnessError::InvalidRequest(
                "promotion handoffs require exact-value user confirmation".to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_required_policy_authorities(&self, trace: &DecisionTrace) -> HarnessResult<()> {
        match self {
            Self::Write(contract) => contract.common.validate_required_policy_authorities(trace),
            Self::Review(contract) => contract.common.validate_required_policy_authorities(trace),
            Self::Analysis(contract) => contract.common.validate_required_policy_authorities(trace),
            Self::VaultRead(contract) => {
                validate_required_policy_authority(
                    "execution_profile",
                    &contract.execution_profile,
                    trace,
                )?;
                validate_required_policy_authority("confirmation", &contract.confirmation, trace)
            }
            Self::Curation(contract) => contract.common.validate_required_policy_authorities(trace),
        }
    }

    fn decision_identifiers(&self) -> BTreeSet<String> {
        let mut identifiers = BTreeSet::new();
        match self {
            Self::Write(contract) => {
                contract
                    .common
                    .collect_decision_identifiers(&mut identifiers);
                collect_decision_identifier(&contract.write_kind, &mut identifiers);
                collect_decision_identifier(&contract.targets, &mut identifiers);
                collect_optional_decision_identifier(
                    contract.delete_targets.as_ref(),
                    &mut identifiers,
                );
            }
            Self::Review(contract) => {
                contract
                    .common
                    .collect_decision_identifiers(&mut identifiers);
                collect_decision_identifier(&contract.review_kind, &mut identifiers);
                collect_decision_identifier(&contract.targets, &mut identifiers);
            }
            Self::Analysis(contract) => {
                contract
                    .common
                    .collect_decision_identifiers(&mut identifiers);
                collect_decision_identifier(&contract.analysis_kind, &mut identifiers);
                collect_optional_decision_identifier(contract.targets.as_ref(), &mut identifiers);
            }
            Self::VaultRead(contract) => {
                collect_decision_identifier(&contract.owner, &mut identifiers);
                collect_decision_identifier(&contract.task_statement, &mut identifiers);
                collect_decision_identifier(&contract.execution_profile, &mut identifiers);
                collect_decision_identifier(&contract.curation_kind, &mut identifiers);
                collect_decision_identifier(&contract.query, &mut identifiers);
                collect_decision_identifier(&contract.maximum_context_bytes, &mut identifiers);
                collect_decision_identifier(&contract.confirmation, &mut identifiers);
            }
            Self::Curation(contract) => {
                contract
                    .common
                    .collect_decision_identifiers(&mut identifiers);
                collect_decision_identifier(&contract.curation_kind, &mut identifiers);
                collect_decision_identifier(&contract.target, &mut identifiers);
                collect_optional_decision_identifier(
                    contract.curation_sources.as_ref(),
                    &mut identifiers,
                );
                collect_optional_decision_identifier(
                    contract.delete_target.as_ref(),
                    &mut identifiers,
                );
                collect_optional_decision_identifier(
                    contract.promotion_handoff.as_ref(),
                    &mut identifiers,
                );
                collect_decision_identifier(&contract.confirmation, &mut identifiers);
            }
        }
        identifiers
    }

    fn resolve(&self, trace: &DecisionTrace) -> HarnessResult<ResolvedTaskContract> {
        let resolved = match self {
            Self::Write(contract) => Ok(ResolvedTaskContract::Write(WriteContract {
                common: contract.common.resolve(trace)?,
                write_kind: resolve_value("write_kind", &contract.write_kind, trace)?,
                targets: resolve_value("targets", &contract.targets, trace)?,
                delete_targets: resolve_optional_value(
                    "delete_targets",
                    contract.delete_targets.as_ref(),
                    trace,
                )?
                .unwrap_or_default(),
            })),
            Self::Review(contract) => Ok(ResolvedTaskContract::Review(ReviewContract {
                common: contract.common.resolve(trace)?,
                review_kind: resolve_value("review_kind", &contract.review_kind, trace)?,
                targets: resolve_value("targets", &contract.targets, trace)?,
            })),
            Self::Analysis(contract) => Ok(ResolvedTaskContract::Analysis(AnalysisContract {
                common: contract.common.resolve(trace)?,
                analysis_kind: resolve_value("analysis_kind", &contract.analysis_kind, trace)?,
                targets: resolve_optional_value("targets", contract.targets.as_ref(), trace)?
                    .unwrap_or_default(),
            })),
            Self::VaultRead(contract) => Ok(ResolvedTaskContract::VaultRead(VaultReadContract {
                owner: resolve_value("owner", &contract.owner, trace)?,
                task_statement: resolve_value("task_statement", &contract.task_statement, trace)?,
                execution_profile: resolve_value(
                    "execution_profile",
                    &contract.execution_profile,
                    trace,
                )?,
                curation_kind: resolve_value("curation_kind", &contract.curation_kind, trace)?,
                query: resolve_value("query", &contract.query, trace)?,
                maximum_context_bytes: resolve_value(
                    "maximum_context_bytes",
                    &contract.maximum_context_bytes,
                    trace,
                )?,
                confirmation: resolve_value("confirmation", &contract.confirmation, trace)?,
            })),
            Self::Curation(contract) => {
                Ok(ResolvedTaskContract::Curation(Box::new(CurationContract {
                    common: contract.common.resolve(trace)?,
                    curation_kind: resolve_value("curation_kind", &contract.curation_kind, trace)?,
                    target: resolve_value("target", &contract.target, trace)?,
                    curation_sources: resolve_optional_value(
                        "curation_sources",
                        contract.curation_sources.as_ref(),
                        trace,
                    )?
                    .unwrap_or_default(),
                    delete_target: resolve_optional_value(
                        "delete_target",
                        contract.delete_target.as_ref(),
                        trace,
                    )?,
                    promotion_handoff: resolve_optional_value(
                        "promotion_handoff",
                        contract.promotion_handoff.as_ref(),
                        trace,
                    )?,
                    confirmation: resolve_value("confirmation", &contract.confirmation, trace)?,
                })))
            }
        }?;
        resolved.validate()?;
        Ok(resolved)
    }
}

impl DraftTaskCommon {
    fn collect_unresolved(&self, questions: &mut Vec<ClarificationRequest>) {
        collect_unresolved("owner", &self.owner, questions);
        collect_unresolved("intent", &self.intent, questions);
        collect_unresolved("task_statement", &self.task_statement, questions);
        collect_unresolved("execution_profile", &self.execution_profile, questions);
        collect_optional_unresolved("context_grants", self.context_grants.as_ref(), questions);
        collect_optional_unresolved(
            "evidence_source_paths",
            self.evidence_source_paths.as_ref(),
            questions,
        );
    }

    fn resolve(&self, trace: &DecisionTrace) -> HarnessResult<TaskContractCommon> {
        Ok(TaskContractCommon {
            owner: resolve_value("owner", &self.owner, trace)?,
            intent: resolve_value("intent", &self.intent, trace)?,
            task_statement: resolve_value("task_statement", &self.task_statement, trace)?,
            execution_profile: resolve_value("execution_profile", &self.execution_profile, trace)?,
            context_grants: resolve_optional_value(
                "context_grants",
                self.context_grants.as_ref(),
                trace,
            )?
            .unwrap_or_default(),
            evidence_source_paths: resolve_optional_value(
                "evidence_source_paths",
                self.evidence_source_paths.as_ref(),
                trace,
            )?
            .unwrap_or_default(),
        })
    }

    fn collect_decision_identifiers(&self, identifiers: &mut BTreeSet<String>) {
        collect_decision_identifier(&self.owner, identifiers);
        collect_decision_identifier(&self.intent, identifiers);
        collect_decision_identifier(&self.task_statement, identifiers);
        collect_decision_identifier(&self.execution_profile, identifiers);
        collect_optional_decision_identifier(self.context_grants.as_ref(), identifiers);
        collect_optional_decision_identifier(self.evidence_source_paths.as_ref(), identifiers);
    }

    fn validate_required_policy_authorities(&self, trace: &DecisionTrace) -> HarnessResult<()> {
        validate_required_policy_authority("intent", &self.intent, trace)?;
        validate_required_policy_authority("execution_profile", &self.execution_profile, trace)
    }
}

impl DraftOwnedTaskCommon {
    fn collect_unresolved(&self, questions: &mut Vec<ClarificationRequest>) {
        collect_unresolved("owner", &self.owner, questions);
        collect_unresolved("task_statement", &self.task_statement, questions);
        collect_unresolved("execution_profile", &self.execution_profile, questions);
    }

    fn resolve(&self, trace: &DecisionTrace) -> HarnessResult<OwnedTaskContractCommon> {
        Ok(OwnedTaskContractCommon {
            owner: resolve_value("owner", &self.owner, trace)?,
            task_statement: resolve_value("task_statement", &self.task_statement, trace)?,
            execution_profile: resolve_value("execution_profile", &self.execution_profile, trace)?,
        })
    }

    fn collect_decision_identifiers(&self, identifiers: &mut BTreeSet<String>) {
        collect_decision_identifier(&self.owner, identifiers);
        collect_decision_identifier(&self.task_statement, identifiers);
        collect_decision_identifier(&self.execution_profile, identifiers);
    }

    fn validate_required_policy_authorities(&self, trace: &DecisionTrace) -> HarnessResult<()> {
        validate_required_policy_authority("execution_profile", &self.execution_profile, trace)
    }
}

impl ResolvedTaskContract {
    fn validate(&self) -> HarnessResult<()> {
        match self {
            Self::VaultRead(contract) => {
                validate_single_line(
                    "Vault read query",
                    &contract.query,
                    MAX_RETRIEVAL_QUERY_LENGTH,
                )?;
                if contract.maximum_context_bytes == 0
                    || contract.maximum_context_bytes > MAX_RETRIEVAL_BYTES
                {
                    return Err(HarnessError::InvalidRequest(format!(
                        "Vault read context byte limit must be between 1 and {MAX_RETRIEVAL_BYTES}"
                    )));
                }
                let expected_confirmation = if contract.curation_kind == CurationKind::Journal {
                    UserConfirmationStatus::Confirmed
                } else {
                    UserConfirmationStatus::NotRequired
                };
                if contract.confirmation != expected_confirmation {
                    return Err(HarnessError::InvalidRequest(
                        "Vault read confirmation must be confirmed for journal reads and not-required for other kinds"
                            .to_owned(),
                    ));
                }
            }
            Self::Curation(contract) => {
                if contract.confirmation != UserConfirmationStatus::Confirmed {
                    return Err(HarnessError::InvalidRequest(
                        "Vault curation requires confirmed user authorization".to_owned(),
                    ));
                }
                if let Some(handoff) = &contract.promotion_handoff {
                    handoff.validate()?;
                    if handoff.proposal.owner != contract.common.owner
                        || handoff.proposal.curation_kind != contract.curation_kind
                    {
                        return Err(HarnessError::InvalidRequest(
                            "promotion handoff owner and curation kind must match the curation request"
                                .to_owned(),
                        ));
                    }
                    if contract.delete_target.is_some() {
                        return Err(HarnessError::InvalidRequest(
                            "promotion-backed curation cannot delete a Vault target".to_owned(),
                        ));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    #[must_use]
    pub fn action(&self) -> HarnessAction {
        match self {
            Self::Write(contract) => match contract.write_kind {
                WriteKind::Code => HarnessAction::CodeWrite,
                WriteKind::Document => HarnessAction::DocumentWrite,
            },
            Self::Review(contract) => match contract.review_kind {
                ReviewKind::Code => HarnessAction::CodeReview,
                ReviewKind::Document => HarnessAction::DocumentReview,
            },
            Self::Analysis(contract) => match contract.analysis_kind {
                AnalysisKind::Investigation => HarnessAction::Investigation,
                AnalysisKind::Design => HarnessAction::Design,
                AnalysisKind::Ideation => HarnessAction::Ideation,
            },
            Self::VaultRead(_) => HarnessAction::VaultRead,
            Self::Curation(_) => HarnessAction::VaultCuration,
        }
    }

    #[must_use]
    pub fn owner(&self) -> &DataOwner {
        match self {
            Self::Write(contract) => &contract.common.owner,
            Self::Review(contract) => &contract.common.owner,
            Self::Analysis(contract) => &contract.common.owner,
            Self::VaultRead(contract) => &contract.owner,
            Self::Curation(contract) => &contract.common.owner,
        }
    }

    #[must_use]
    pub fn intent(&self) -> HarnessIntent {
        match self {
            Self::Write(contract) => contract.common.intent,
            Self::Review(contract) => contract.common.intent,
            Self::Analysis(contract) => contract.common.intent,
            Self::VaultRead(_) | Self::Curation(_) => HarnessIntent::General,
        }
    }

    #[must_use]
    pub fn execution_profile(&self) -> HarnessExecutionProfile {
        match self {
            Self::Write(contract) => contract.common.execution_profile,
            Self::Review(contract) => contract.common.execution_profile,
            Self::Analysis(contract) => contract.common.execution_profile,
            Self::VaultRead(contract) => contract.execution_profile,
            Self::Curation(contract) => contract.common.execution_profile,
        }
    }

    #[must_use]
    pub fn context_grants(&self) -> &[ContextGrant] {
        match self {
            Self::Write(contract) => &contract.common.context_grants,
            Self::Review(contract) => &contract.common.context_grants,
            Self::Analysis(contract) => &contract.common.context_grants,
            Self::VaultRead(_) | Self::Curation(_) => &[],
        }
    }

    #[must_use]
    pub fn evidence_source_paths(&self) -> &[String] {
        match self {
            Self::Write(contract) => &contract.common.evidence_source_paths,
            Self::Review(contract) => &contract.common.evidence_source_paths,
            Self::Analysis(contract) => &contract.common.evidence_source_paths,
            Self::VaultRead(_) | Self::Curation(_) => &[],
        }
    }

    pub(super) fn promotion_handoff(&self) -> Option<&PromotionHandoff> {
        match self {
            Self::Curation(contract) => contract.promotion_handoff.as_ref(),
            _ => None,
        }
    }

    #[must_use]
    pub fn vault_read_parameters(&self) -> Option<(&str, usize)> {
        match self {
            Self::VaultRead(contract) => {
                Some((contract.query.as_str(), contract.maximum_context_bytes))
            }
            _ => None,
        }
    }

    #[must_use]
    pub fn to_harness_request(&self) -> HarnessRequest {
        match self {
            Self::Write(contract) => HarnessRequest {
                action: self.action(),
                owner: contract.common.owner.clone(),
                targets: contract.targets.clone(),
                objective: contract.common.task_statement.clone(),
                curation_kind: None,
                explicit_user_confirmation_reported: false,
                curation_sources: Vec::new(),
                delete_targets: contract.delete_targets.clone(),
            },
            Self::Review(contract) => HarnessRequest {
                action: self.action(),
                owner: contract.common.owner.clone(),
                targets: contract.targets.clone(),
                objective: contract.common.task_statement.clone(),
                curation_kind: None,
                explicit_user_confirmation_reported: false,
                curation_sources: Vec::new(),
                delete_targets: Vec::new(),
            },
            Self::Analysis(contract) => HarnessRequest {
                action: self.action(),
                owner: contract.common.owner.clone(),
                targets: contract.targets.clone(),
                objective: contract.common.task_statement.clone(),
                curation_kind: None,
                explicit_user_confirmation_reported: false,
                curation_sources: Vec::new(),
                delete_targets: Vec::new(),
            },
            Self::VaultRead(contract) => HarnessRequest {
                action: HarnessAction::VaultRead,
                owner: contract.owner.clone(),
                targets: Vec::new(),
                objective: contract.task_statement.clone(),
                curation_kind: Some(contract.curation_kind),
                explicit_user_confirmation_reported: contract.confirmation
                    == UserConfirmationStatus::Confirmed,
                curation_sources: Vec::new(),
                delete_targets: Vec::new(),
            },
            Self::Curation(contract) => HarnessRequest {
                action: HarnessAction::VaultCuration,
                owner: contract.common.owner.clone(),
                targets: vec![contract.target.clone()],
                objective: contract.common.task_statement.clone(),
                curation_kind: Some(contract.curation_kind),
                explicit_user_confirmation_reported: contract.confirmation
                    == UserConfirmationStatus::Confirmed,
                curation_sources: contract.curation_sources.clone(),
                delete_targets: contract.delete_target.iter().cloned().collect(),
            },
        }
    }
}

fn collect_unresolved<T>(
    field: &str,
    value: &DraftValue<T>,
    questions: &mut Vec<ClarificationRequest>,
) {
    if let DraftValue::Unresolved { question } = value {
        questions.push(ClarificationRequest {
            field: field.to_owned(),
            question: question.clone(),
        });
    }
}

fn collect_optional_unresolved<T>(
    field: &str,
    value: Option<&DraftValue<T>>,
    questions: &mut Vec<ClarificationRequest>,
) {
    if let Some(value) = value {
        collect_unresolved(field, value, questions);
    }
}

fn collect_decision_identifier<T>(value: &DraftValue<T>, identifiers: &mut BTreeSet<String>) {
    if let DraftValue::Resolved {
        decision_identifier,
        ..
    } = value
    {
        identifiers.insert(decision_identifier.clone());
    }
}

fn collect_optional_decision_identifier<T>(
    value: Option<&DraftValue<T>>,
    identifiers: &mut BTreeSet<String>,
) {
    if let Some(value) = value {
        collect_decision_identifier(value, identifiers);
    }
}

fn validate_required_policy_authority<T>(
    field: &str,
    value: &DraftValue<T>,
    trace: &DecisionTrace,
) -> HarnessResult<()>
where
    T: Serialize,
{
    let DraftValue::Resolved {
        value,
        decision_identifier,
    } = value
    else {
        return Ok(());
    };
    let value_digest = serialized_digest(value)?;
    let Some(required_rule) = required_policy_default_rule(field, &value_digest)? else {
        return Ok(());
    };
    let record = trace.authoritative_record(decision_identifier)?;
    if !matches!(
        record,
        DecisionRecord::PolicyDefault { rule, .. } if *rule == required_rule
    ) {
        return Err(HarnessError::InvalidRequest(format!(
            "field `{field}` with this value requires policy default rule `{}`",
            required_rule.as_str()
        )));
    }
    Ok(())
}

fn required_policy_default_rule(
    field: &str,
    value_digest: &str,
) -> HarnessResult<Option<PolicyDefaultRule>> {
    let candidates = match field {
        "intent" => &[PolicyDefaultRule::GeneralIntent][..],
        "execution_profile" => &[PolicyDefaultRule::StandardExecutionProfile][..],
        "confirmation" => &[PolicyDefaultRule::NonJournalReadConfirmationNotRequired][..],
        _ => &[],
    };
    for rule in candidates {
        if rule.value_digest()? == value_digest {
            return Ok(Some(*rule));
        }
    }
    Ok(None)
}

fn resolve_value<T>(field: &str, value: &DraftValue<T>, trace: &DecisionTrace) -> HarnessResult<T>
where
    T: Clone + Serialize,
{
    let DraftValue::Resolved {
        value,
        decision_identifier,
    } = value
    else {
        return Err(HarnessError::InvalidRequest(format!(
            "field `{field}` is unresolved"
        )));
    };
    let record = trace.authoritative_record(decision_identifier)?;
    let actual_digest = serialized_digest(value)?;
    if record.value_digest() != actual_digest {
        return Err(HarnessError::InvalidRequest(format!(
            "decision record `{decision_identifier}` does not match field `{field}`"
        )));
    }
    record.validate_field_authority(field, &actual_digest)?;
    Ok(value.clone())
}

fn resolve_optional_value<T>(
    field: &str,
    value: Option<&DraftValue<T>>,
    trace: &DecisionTrace,
) -> HarnessResult<Option<T>>
where
    T: Clone + Serialize,
{
    value
        .map(|value| resolve_value(field, value, trace))
        .transpose()
}

fn validate_statement_references(
    references: &[String],
    statements: &BTreeSet<&str>,
) -> HarnessResult<()> {
    if references.is_empty() || references.len() > REQUEST_LIST_LIMIT {
        return Err(HarnessError::InvalidRequest(format!(
            "user decision records must reference between one and {REQUEST_LIST_LIMIT} statements"
        )));
    }
    let mut seen = BTreeSet::new();
    for reference in references {
        if !statements.contains(reference.as_str()) {
            return Err(HarnessError::InvalidRequest(format!(
                "user statement `{reference}` does not exist"
            )));
        }
        if !seen.insert(reference.as_str()) {
            return Err(HarnessError::InvalidRequest(format!(
                "duplicate user statement reference `{reference}`"
            )));
        }
    }
    Ok(())
}

fn validate_record_references<'a>(
    references: &[String],
    records: &BTreeMap<&'a str, &'a DecisionRecord>,
) -> HarnessResult<()> {
    if references.is_empty() || references.len() > REQUEST_LIST_LIMIT {
        return Err(HarnessError::InvalidRequest(format!(
            "model decision records must reference between one and {REQUEST_LIST_LIMIT} prior records"
        )));
    }
    let mut seen = BTreeSet::new();
    for reference in references {
        if !records.contains_key(reference.as_str()) {
            return Err(HarnessError::InvalidRequest(format!(
                "decision record `{reference}` does not exist"
            )));
        }
        if !seen.insert(reference.as_str()) {
            return Err(HarnessError::InvalidRequest(format!(
                "duplicate decision record reference `{reference}`"
            )));
        }
    }
    Ok(())
}

fn validate_prior_record_reference(
    reference: &str,
    prior_record_identifiers: &BTreeSet<&str>,
) -> HarnessResult<()> {
    if !prior_record_identifiers.contains(reference) {
        return Err(HarnessError::InvalidRequest(format!(
            "decision record `{reference}` must appear before a record that depends on it"
        )));
    }
    Ok(())
}

fn validate_text(field: &str, value: &str) -> HarnessResult<()> {
    if value.trim().is_empty() || value.len() > REQUEST_TEXT_LIMIT {
        return Err(HarnessError::InvalidRequest(format!(
            "{field} must contain between one and {REQUEST_TEXT_LIMIT} bytes"
        )));
    }
    Ok(())
}

fn validate_opaque_identifier(field: &str, value: &str, prefix: &str) -> HarnessResult<()> {
    let Some(suffix) = value.strip_prefix(prefix) else {
        return Err(HarnessError::InvalidRequest(format!(
            "{field} must use the opaque `{prefix}<16 lowercase hex characters>` form"
        )));
    };
    if suffix.len() != OPAQUE_IDENTIFIER_HEX_LENGTH
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(HarnessError::InvalidRequest(format!(
            "{field} must use the opaque `{prefix}<16 lowercase hex characters>` form"
        )));
    }
    Ok(())
}

fn validate_digest(field: &str, value: &str) -> HarnessResult<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(HarnessError::InvalidRequest(format!(
            "{field} must be a 64-character hexadecimal digest"
        )));
    }
    Ok(())
}

fn validate_plan_digest(field: &str, value: &str) -> HarnessResult<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(HarnessError::InvalidPlan(format!(
            "{field} must be a 64-character hexadecimal digest"
        )));
    }
    Ok(())
}

fn validate_plan_opaque_identifier(field: &str, value: &str, prefix: &str) -> HarnessResult<()> {
    let Some(suffix) = value.strip_prefix(prefix) else {
        return Err(HarnessError::InvalidPlan(format!(
            "{field} must use the opaque `{prefix}<16 lowercase hex characters>` form"
        )));
    };
    if suffix.len() != OPAQUE_IDENTIFIER_HEX_LENGTH
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(HarnessError::InvalidPlan(format!(
            "{field} must use the opaque `{prefix}<16 lowercase hex characters>` form"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved_value<T>(value: T, decision_identifier: &str) -> DraftValue<T> {
        DraftValue::Resolved {
            value,
            decision_identifier: decision_identifier.to_owned(),
        }
    }

    fn structured_decision<T: Serialize>(
        identifier: &str,
        value: &T,
    ) -> HarnessResult<DecisionRecord> {
        Ok(DecisionRecord::StructuredCaller {
            identifier: identifier.to_owned(),
            value_digest: serialized_digest(value)?,
            description: "structured test input".to_owned(),
        })
    }

    fn statement_identifier(number: u64) -> String {
        format!("statement-{number:016x}")
    }

    fn decision_identifier(number: u64) -> String {
        format!("decision-{number:016x}")
    }

    fn user_decision<T: Serialize>(
        number: u64,
        value: &T,
        statement_identifier: &str,
    ) -> HarnessResult<DecisionRecord> {
        Ok(DecisionRecord::UserStatement {
            identifier: decision_identifier(number),
            value_digest: serialized_digest(value)?,
            statement_identifiers: vec![statement_identifier.to_owned()],
        })
    }

    fn write_envelope() -> HarnessResult<RequestEnvelope> {
        let owner = DataOwner::Profile;
        let intent = HarnessIntent::PolicyMaintenance;
        let task_statement = "implement the version six request contract".to_owned();
        let execution_profile = HarnessExecutionProfile::Strict;
        let write_kind = WriteKind::Code;
        let targets = vec!["crates/llm-context-vault/src/harness.rs".to_owned()];
        let values = [
            structured_decision("owner", &owner)?,
            structured_decision("intent", &intent)?,
            structured_decision("task-statement", &task_statement)?,
            structured_decision("execution-profile", &execution_profile)?,
            structured_decision("write-kind", &write_kind)?,
            structured_decision("targets", &targets)?,
        ];
        Ok(RequestEnvelope {
            version: HARNESS_SCHEMA_VERSION,
            source: RequestSource::StructuredCaller {
                description: "request contract unit test".to_owned(),
            },
            draft: DraftTaskRequest::Write(DraftWriteContract {
                common: DraftTaskCommon {
                    owner: resolved_value(owner, "owner"),
                    intent: resolved_value(intent, "intent"),
                    task_statement: resolved_value(task_statement, "task-statement"),
                    execution_profile: resolved_value(execution_profile, "execution-profile"),
                    context_grants: None,
                    evidence_source_paths: None,
                },
                write_kind: resolved_value(write_kind, "write-kind"),
                targets: resolved_value(targets, "targets"),
                delete_targets: None,
            }),
            decision_trace: DecisionTrace {
                records: values.into_iter().collect(),
            },
        })
    }

    fn user_write_envelope() -> HarnessResult<RequestEnvelope> {
        let owner = DataOwner::Profile;
        let intent = HarnessIntent::PolicyMaintenance;
        let task_statement = "implement the version six request contract".to_owned();
        let execution_profile = HarnessExecutionProfile::Strict;
        let write_kind = WriteKind::Code;
        let targets = vec!["crates/llm-context-vault/src/harness.rs".to_owned()];
        let statement_identifier = statement_identifier(1);
        let values = [
            user_decision(1, &owner, &statement_identifier)?,
            user_decision(2, &intent, &statement_identifier)?,
            user_decision(3, &task_statement, &statement_identifier)?,
            user_decision(4, &execution_profile, &statement_identifier)?,
            user_decision(5, &write_kind, &statement_identifier)?,
            user_decision(6, &targets, &statement_identifier)?,
        ];
        Ok(RequestEnvelope {
            version: HARNESS_SCHEMA_VERSION,
            source: RequestSource::UserLanguage {
                statements: vec![UserStatement {
                    identifier: statement_identifier,
                    text: "하네스 요청 계약을 구현해".to_owned(),
                }],
            },
            draft: DraftTaskRequest::Write(DraftWriteContract {
                common: DraftTaskCommon {
                    owner: resolved_value(owner, &decision_identifier(1)),
                    intent: resolved_value(intent, &decision_identifier(2)),
                    task_statement: resolved_value(task_statement, &decision_identifier(3)),
                    execution_profile: resolved_value(execution_profile, &decision_identifier(4)),
                    context_grants: None,
                    evidence_source_paths: None,
                },
                write_kind: resolved_value(write_kind, &decision_identifier(5)),
                targets: resolved_value(targets, &decision_identifier(6)),
                delete_targets: None,
            }),
            decision_trace: DecisionTrace {
                records: values.into_iter().collect(),
            },
        })
    }

    fn structured_vault_read_envelope(
        curation_kind: CurationKind,
        query: &str,
        maximum_context_bytes: usize,
        confirmed: bool,
    ) -> HarnessResult<RequestEnvelope> {
        RequestEnvelope::from_structured_vault_read(
            HarnessRequest {
                action: HarnessAction::VaultRead,
                owner: DataOwner::Personal,
                targets: Vec::new(),
                objective: "read personal context".to_owned(),
                curation_kind: Some(curation_kind),
                explicit_user_confirmation_reported: confirmed,
                curation_sources: Vec::new(),
                delete_targets: Vec::new(),
            },
            HarnessExecutionProfile::Standard,
            query.to_owned(),
            maximum_context_bytes,
            "Vault read request test",
        )
    }

    #[test]
    fn resolved_contract_rejects_unresolved_fields() {
        let mut envelope = write_envelope().expect("fixture must build");
        let DraftTaskRequest::Write(contract) = &mut envelope.draft else {
            panic!("fixture must remain a write request");
        };
        contract.common.owner = DraftValue::Unresolved {
            question: "Is this profile or project work?".to_owned(),
        };

        let error = envelope
            .resolve_test_fixture()
            .expect_err("unresolved owner must require clarification");
        assert!(matches!(
            &error,
            HarnessError::ClarificationRequired(questions)
                if questions == &vec![ClarificationRequest {
                    field: "owner".to_owned(),
                    question: "Is this profile or project work?".to_owned(),
                }]
        ));
        assert!(
            error
                .to_string()
                .contains("owner: Is this profile or project work?")
        );
    }

    #[test]
    fn model_interpretation_cannot_authorize_an_execution_value() {
        let mut envelope = write_envelope().expect("fixture must build");
        let owner = DataOwner::Profile;
        envelope
            .decision_trace
            .records
            .push(DecisionRecord::ModelInterpretation {
                identifier: "interpreted-owner".to_owned(),
                value_digest: serialized_digest(&owner).expect("owner must serialize"),
                based_on_record_identifiers: vec!["owner".to_owned()],
            });
        let DraftTaskRequest::Write(contract) = &mut envelope.draft else {
            panic!("fixture must remain a write request");
        };
        contract.common.owner = resolved_value(owner, "interpreted-owner");

        assert!(matches!(
            envelope.resolve_test_fixture(),
            Err(HarnessError::InvalidRequest(message))
                if message.contains("cannot authorize an execution value")
        ));
    }

    #[test]
    fn observed_fact_cannot_authorize_an_execution_value_without_confirmation() {
        let mut envelope = user_write_envelope().expect("fixture must build");
        let owner = DataOwner::Profile;
        envelope
            .decision_trace
            .records
            .push(DecisionRecord::ObservedFact {
                identifier: decision_identifier(7),
                value_digest: serialized_digest(&owner).expect("owner must serialize"),
                evidence: DecisionEvidence {
                    kind: DecisionEvidenceKind::RepositoryState,
                    content_digest: "a".repeat(64),
                },
            });
        let DraftTaskRequest::Write(contract) = &mut envelope.draft else {
            panic!("fixture must remain a write request");
        };
        contract.common.owner = resolved_value(owner, &decision_identifier(7));

        assert!(matches!(
            envelope.resolve_test_fixture(),
            Err(HarnessError::InvalidRequest(message))
                if message.contains("cannot authorize an execution value")
        ));
    }

    #[test]
    fn policy_default_requires_the_exact_selected_policy_content() {
        let policy_identifier = "agent-harness".to_owned();
        let policy_content_digest = "a".repeat(64);
        let trace = DecisionTrace {
            records: vec![DecisionRecord::PolicyDefault {
                identifier: "execution-profile-default".to_owned(),
                value_digest: "b".repeat(64),
                policy_identifier: policy_identifier.clone(),
                policy_content_digest: policy_content_digest.clone(),
                rule: PolicyDefaultRule::StandardExecutionProfile,
            }],
        };
        let exact_policy = PolicyBinding {
            id: policy_identifier.clone(),
            repository_relative_path: "vault/profile/rules/agent-harness.md".to_owned(),
            content_digest: policy_content_digest,
        };

        trace
            .validate_policy_bindings(std::slice::from_ref(&exact_policy))
            .expect("exact policy binding must be accepted");
        let changed_policy = PolicyBinding {
            content_digest: "c".repeat(64),
            ..exact_policy
        };
        assert!(matches!(
            trace.validate_policy_bindings(&[changed_policy]),
            Err(HarnessError::InvalidPlan(message))
                if message.contains("not bound to the exact policy content")
        ));
    }

    #[test]
    fn policy_default_rule_rejects_the_wrong_field_or_value() {
        let standard_digest =
            serialized_digest(&HarnessExecutionProfile::Standard).expect("profile must serialize");
        let record = DecisionRecord::PolicyDefault {
            identifier: decision_identifier(7),
            value_digest: standard_digest.clone(),
            policy_identifier: "agent-harness".to_owned(),
            policy_content_digest: "a".repeat(64),
            rule: PolicyDefaultRule::StandardExecutionProfile,
        };

        record
            .validate_field_authority("execution_profile", &standard_digest)
            .expect("matching default rule must authorize its field and value");
        assert!(matches!(
            record.validate_field_authority("owner", &standard_digest),
            Err(HarnessError::InvalidRequest(message))
                if message.contains("does not authorize field `owner`")
        ));
        let strict_digest =
            serialized_digest(&HarnessExecutionProfile::Strict).expect("profile must serialize");
        assert!(matches!(
            record.validate_field_authority("execution_profile", &strict_digest),
            Err(HarnessError::InvalidRequest(message))
                if message.contains("does not authorize field `execution_profile`")
        ));
    }

    #[test]
    fn required_policy_defaults_reject_user_statement_or_confirmation_substitutes() {
        let statement_identifier = statement_identifier(1);
        let standard = HarnessExecutionProfile::Standard;
        let user_statement =
            user_decision(1, &standard, &statement_identifier).expect("user decision must build");
        let user_trace = DecisionTrace {
            records: vec![user_statement],
        };
        let user_value = resolved_value(standard, &decision_identifier(1));
        assert!(matches!(
            validate_required_policy_authority("execution_profile", &user_value, &user_trace),
            Err(HarnessError::InvalidRequest(message))
                if message.contains("requires policy default rule `standard-execution-profile`")
        ));

        let general = HarnessIntent::General;
        let general_trace = DecisionTrace {
            records: vec![
                user_decision(1, &general, &statement_identifier)
                    .expect("user decision must build"),
            ],
        };
        let general_value = resolved_value(general, &decision_identifier(1));
        assert!(matches!(
            validate_required_policy_authority("intent", &general_value, &general_trace),
            Err(HarnessError::InvalidRequest(message))
                if message.contains("requires policy default rule `general-intent`")
        ));

        let not_required = UserConfirmationStatus::NotRequired;
        let value_digest = serialized_digest(&not_required).expect("confirmation must serialize");
        let source = RequestSource::UserLanguage {
            statements: vec![UserStatement {
                identifier: statement_identifier.clone(),
                text: "use that confirmation state".to_owned(),
            }],
        };
        let confirmation_trace = DecisionTrace {
            records: vec![
                user_decision(0, &true, &statement_identifier).expect("basis decision must build"),
                DecisionRecord::ModelProposal {
                    identifier: decision_identifier(1),
                    value_digest: value_digest.clone(),
                    based_on_record_identifiers: vec![decision_identifier(0)],
                },
                DecisionRecord::UserConfirmation {
                    identifier: decision_identifier(2),
                    value_digest,
                    confirms_record_identifier: decision_identifier(1),
                    statement_identifiers: vec![statement_identifier],
                },
            ],
        };
        let RequestSource::UserLanguage { statements } = &source else {
            unreachable!()
        };
        confirmation_trace
            .validate(&source, statements)
            .expect("confirmation trace must be valid");
        let confirmation_value = resolved_value(not_required, &decision_identifier(2));
        assert!(matches!(
            validate_required_policy_authority(
                "confirmation",
                &confirmation_value,
                &confirmation_trace,
            ),
            Err(HarnessError::InvalidRequest(message))
                if message.contains(
                    "requires policy default rule `non-journal-read-confirmation-not-required`"
                )
        ));
    }

    #[test]
    fn user_confirmation_can_authorize_a_matching_model_proposal() {
        let mut envelope = user_write_envelope().expect("fixture must build");
        let target = vec!["crates/llm-context-vault/src/harness/request.rs".to_owned()];
        let proposal_digest = serialized_digest(&target).expect("target must serialize");
        let RequestSource::UserLanguage { statements } = &mut envelope.source else {
            panic!("fixture must use user language");
        };
        statements.push(UserStatement {
            identifier: statement_identifier(2),
            text: "그 대상으로 진행해".to_owned(),
        });
        envelope.decision_trace.records.extend([
            DecisionRecord::ModelProposal {
                identifier: decision_identifier(7),
                value_digest: proposal_digest.clone(),
                based_on_record_identifiers: vec![decision_identifier(6)],
            },
            DecisionRecord::UserConfirmation {
                identifier: decision_identifier(8),
                value_digest: proposal_digest,
                confirms_record_identifier: decision_identifier(7),
                statement_identifiers: vec![statement_identifier(2)],
            },
        ]);
        let DraftTaskRequest::Write(contract) = &mut envelope.draft else {
            panic!("fixture must remain a write request");
        };
        contract.targets = resolved_value(target.clone(), &decision_identifier(8));

        let (resolved, _, _) = envelope.resolve().expect("confirmation must resolve");
        let ResolvedTaskContract::Write(resolved) = resolved else {
            panic!("resolved contract must remain a write request");
        };
        assert_eq!(resolved.targets, target);
    }

    #[test]
    fn resolved_output_does_not_contain_user_statement_text() {
        let mut envelope = user_write_envelope().expect("fixture must build");
        let RequestSource::UserLanguage { statements } = &mut envelope.source else {
            panic!("fixture must use user language");
        };
        statements[0].text = "sensitive sentinel that must remain transient".to_owned();
        let (resolved, trace, provenance) = envelope.resolve().expect("request must resolve");
        let output = serde_json::to_string(&(resolved, trace, provenance))
            .expect("resolved output must serialize");

        assert!(!output.contains("sensitive sentinel"));
    }

    #[test]
    fn resolved_output_omits_unused_user_statements_and_decisions() {
        let baseline = user_write_envelope().expect("fixture must build");
        let mut envelope = baseline.clone();
        let unused_statement_identifier = statement_identifier(2);
        let RequestSource::UserLanguage { statements } = &mut envelope.source else {
            panic!("fixture must use user language");
        };
        statements.push(UserStatement {
            identifier: unused_statement_identifier.clone(),
            text: "unused sensitive sentinel".to_owned(),
        });
        envelope
            .decision_trace
            .records
            .push(DecisionRecord::UserStatement {
                identifier: decision_identifier(7),
                value_digest: serialized_digest(&true).expect("value must serialize"),
                statement_identifiers: vec![unused_statement_identifier.clone()],
            });

        let baseline_resolved = baseline.resolve().expect("baseline request must resolve");
        let resolved = envelope.resolve().expect("request must resolve");
        let output = serde_json::to_string(&resolved).expect("resolved output must serialize");

        assert_eq!(resolved, baseline_resolved);
        assert!(!output.contains("unused sensitive sentinel"));
        assert!(!output.contains(&unused_statement_identifier));
        assert!(!output.contains(&decision_identifier(7)));
    }

    #[test]
    fn structured_caller_envelopes_are_not_executable_in_production() {
        let envelope = write_envelope().expect("fixture must build");

        assert!(matches!(
            envelope.resolve(),
            Err(HarnessError::InvalidRequest(message))
                if message.contains("structured-caller request envelopes are not executable")
        ));
    }

    #[test]
    fn user_language_request_rejects_structured_caller_decisions() {
        let mut envelope = user_write_envelope().expect("fixture must build");
        envelope.decision_trace.records.push(
            structured_decision(&decision_identifier(7), &true).expect("value must serialize"),
        );

        assert!(matches!(
            envelope.resolve(),
            Err(HarnessError::InvalidRequest(message))
                if message == "user-language requests cannot use structured-caller decisions"
        ));
    }

    #[test]
    fn decision_trace_rejects_dependencies_on_later_records() {
        let mut envelope = write_envelope().expect("fixture must build");
        envelope.decision_trace.records.extend([
            DecisionRecord::ModelInterpretation {
                identifier: "cycle-a".to_owned(),
                value_digest: serialized_digest(&true).expect("value must serialize"),
                based_on_record_identifiers: vec!["cycle-b".to_owned()],
            },
            DecisionRecord::ModelProposal {
                identifier: "cycle-b".to_owned(),
                value_digest: serialized_digest(&true).expect("value must serialize"),
                based_on_record_identifiers: vec!["cycle-a".to_owned()],
            },
        ]);

        assert!(matches!(
            envelope.resolve_test_fixture(),
            Err(HarnessError::InvalidRequest(message))
                if message.contains("must appear before")
        ));
    }

    #[test]
    fn vault_read_contract_has_no_file_target_field() {
        let owner = DataOwner::Personal;
        let task_statement = "read personal knowledge".to_owned();
        let execution_profile = HarnessExecutionProfile::Standard;
        let curation_kind = CurationKind::Knowledge;
        let query = "architecture".to_owned();
        let maximum_context_bytes = 64 * 1024;
        let confirmation = UserConfirmationStatus::NotRequired;
        let records = [
            structured_decision("owner", &owner).expect("owner must serialize"),
            structured_decision("task-statement", &task_statement)
                .expect("statement must serialize"),
            structured_decision("execution-profile", &execution_profile)
                .expect("profile must serialize"),
            structured_decision("curation-kind", &curation_kind).expect("kind must serialize"),
            structured_decision("query", &query).expect("query must serialize"),
            structured_decision("maximum-context-bytes", &maximum_context_bytes)
                .expect("maximum must serialize"),
            structured_decision("confirmation", &confirmation)
                .expect("confirmation must serialize"),
        ];
        let envelope = RequestEnvelope {
            version: HARNESS_SCHEMA_VERSION,
            source: RequestSource::StructuredCaller {
                description: "Vault read contract unit test".to_owned(),
            },
            draft: DraftTaskRequest::VaultRead(DraftVaultReadContract {
                owner: resolved_value(owner, "owner"),
                task_statement: resolved_value(task_statement, "task-statement"),
                execution_profile: resolved_value(execution_profile, "execution-profile"),
                curation_kind: resolved_value(curation_kind, "curation-kind"),
                query: resolved_value(query, "query"),
                maximum_context_bytes: resolved_value(
                    maximum_context_bytes,
                    "maximum-context-bytes",
                ),
                confirmation: resolved_value(confirmation, "confirmation"),
            }),
            decision_trace: DecisionTrace {
                records: records.into_iter().collect(),
            },
        };

        let (resolved, _, _) = envelope.resolve_test_fixture().expect("read must resolve");
        let request = resolved.to_harness_request();
        assert!(request.targets.is_empty());
    }

    #[test]
    fn vault_read_contract_rejects_invalid_query_and_context_limit() {
        for (query, maximum_context_bytes) in [
            ("", 64 * 1024),
            ("architecture", 0),
            ("architecture", MAX_RETRIEVAL_BYTES + 1),
        ] {
            let envelope = structured_vault_read_envelope(
                CurationKind::Knowledge,
                query,
                maximum_context_bytes,
                false,
            )
            .expect("Vault read fixture must build");

            assert!(matches!(
                envelope.resolve_test_fixture(),
                Err(HarnessError::InvalidRequest(_))
            ));
        }
    }

    #[test]
    fn vault_read_confirmation_is_explicit_and_kind_specific() {
        let journal_without_confirmation = ResolvedTaskContract::VaultRead(VaultReadContract {
            owner: DataOwner::Personal,
            task_statement: "read journal".to_owned(),
            execution_profile: HarnessExecutionProfile::Standard,
            curation_kind: CurationKind::Journal,
            query: "today".to_owned(),
            maximum_context_bytes: 64 * 1024,
            confirmation: UserConfirmationStatus::NotRequired,
        });
        assert!(matches!(
            journal_without_confirmation.validate(),
            Err(HarnessError::InvalidRequest(message))
                if message.contains("confirmed for journal reads")
        ));

        let non_journal_with_confirmation = ResolvedTaskContract::VaultRead(VaultReadContract {
            owner: DataOwner::Personal,
            task_statement: "read knowledge".to_owned(),
            execution_profile: HarnessExecutionProfile::Standard,
            curation_kind: CurationKind::Knowledge,
            query: "architecture".to_owned(),
            maximum_context_bytes: 64 * 1024,
            confirmation: UserConfirmationStatus::Confirmed,
        });
        assert!(matches!(
            non_journal_with_confirmation.validate(),
            Err(HarnessError::InvalidRequest(message))
                if message.contains("not-required for other kinds")
        ));

        let mut unresolved = structured_vault_read_envelope(
            CurationKind::Knowledge,
            "architecture",
            64 * 1024,
            false,
        )
        .expect("knowledge fixture must build");
        let DraftTaskRequest::VaultRead(contract) = &mut unresolved.draft else {
            panic!("fixture must remain a Vault read");
        };
        contract.confirmation = DraftValue::Unresolved {
            question: "Is confirmation required for this read?".to_owned(),
        };
        assert!(matches!(
            unresolved.resolve_test_fixture(),
            Err(HarnessError::ClarificationRequired(questions))
                if questions.iter().any(|question| question.field == "confirmation")
        ));
    }
}
