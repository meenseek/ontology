//! Finite, caller-driven support-outcome flow. No model executor or Core acceptance authority.
use crate::{
    context::{ContextScope, restricted, validate_path},
    domain::Error,
    store::{Store, digest},
};
use context_core::harness::{
    CareerOutputSurface, ContextCommitReceipt, SourceVersion, StoredSourceState,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgConnection, Row};
use std::collections::BTreeSet;

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CareerCommand {
    List {
        store_id: String,
    },
    Get {
        store_id: String,
        id: String,
    },
    Start {
        store_id: String,
        application_key: String,
        outcome_key: String,
        outcome: SourceVersion,
        criteria: Vec<SourceVersion>,
        surfaces: Vec<CareerOutputSurface>,
    },
    Record {
        store_id: String,
        id: String,
        expected_revision: i64,
        record: SourceVersion,
    },
    BindApply {
        store_id: String,
        id: String,
        expected_revision: i64,
        run_id: String,
        prepared_run_digest: String,
    },
    ConfirmApply {
        store_id: String,
        id: String,
        expected_revision: i64,
    },
    Pause {
        store_id: String,
        id: String,
        expected_revision: i64,
    },
    Resume {
        store_id: String,
        id: String,
        expected_revision: i64,
    },
    Rebase {
        store_id: String,
        id: String,
        expected_revision: i64,
        outcome: SourceVersion,
        criteria: Vec<SourceVersion>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Progress {
    outcome: SourceVersion,
    initial_criteria: Vec<SourceVersion>,
    criteria: Vec<SourceVersion>,
    surfaces: Vec<CareerOutputSurface>,
    stage: Stage,
    paused: bool,
    reflection: Option<SourceVersion>,
    review: Option<SourceVersion>,
    apply: Option<ApplyBinding>,
    verification: Option<SourceVersion>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum Stage {
    Reflection,
    Review,
    CriteriaApply,
    OutputVerification,
    Complete,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct ApplyBinding {
    run_id: String,
    prepared_run_digest: String,
    apply_id: Option<String>,
}

/// A single fenced section in a native original. Prose stays in that original.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum WorkflowRecord {
    Reflection {
        application_key: String,
        outcome_key: String,
        findings: Vec<Finding>,
    },
    Review {
        reflection: SourceVersion,
        decisions: Vec<Decision>,
    },
    Verification {
        review: SourceVersion,
        criteria: Vec<SourceVersion>,
        checks: Vec<OutputCheck>,
    },
    OutputReview {
        review: SourceVersion,
        surface: CareerOutputSurface,
        criteria: Vec<SourceVersion>,
        artifact: Option<SourceVersion>,
        result: CheckResult,
        observations: String,
    },
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub id: String,
    pub kind: FindingKind,
    pub statement: String,
    pub evidence: Vec<SourceVersion>,
}
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum FindingKind {
    ConfirmedFact,
    ArtifactObservation,
    Hypothesis,
    Unknown,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub finding_id: String,
    pub disposition: Disposition,
    pub reason: String,
    pub targets: Vec<String>,
    pub revisit_when: Option<String>,
}
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Disposition {
    Apply,
    Hold,
    Reject,
    AlreadyCovered,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputCheck {
    pub surface: CareerOutputSurface,
    pub artifact: Option<SourceVersion>,
    pub independent_review: SourceVersion,
    pub result: CheckResult,
}
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum CheckResult {
    Passed,
    NeedsRevision,
}

fn key(s: &str) -> Result<(), Error> {
    if s.is_empty()
        || s.len() > 160
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn text(s: &str) -> Result<(), Error> {
    if s.trim().is_empty() || s.len() > 4096 {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn logical(version: &SourceVersion) -> Result<String, Error> {
    version.validate().map_err(|_| Error::Invalid)?;
    if !matches!(version.state, StoredSourceState::Live { .. }) {
        return Err(Error::Invalid);
    }
    let path = version
        .logical_path
        .strip_prefix("vault/")
        .ok_or(Error::Invalid)?;
    let (scope, relative) = if path.starts_with("work/") {
        let mut pieces = path.splitn(3, '/');
        pieces.next();
        let slug = pieces.next().ok_or(Error::Invalid)?;
        (format!("work/{slug}"), pieces.next().ok_or(Error::Invalid)?)
    } else {
        let (s, p) = path.split_once('/').ok_or(Error::Invalid)?;
        (s.to_owned(), p)
    };
    scope.parse::<ContextScope>()?;
    validate_path(relative)?;
    if restricted(relative) || !relative.ends_with(".md") {
        return Err(Error::Invalid);
    }
    Ok(path.to_owned())
}
fn criteria_valid(criteria: &[SourceVersion]) -> Result<(), Error> {
    if criteria.is_empty() || criteria.len() > 12 {
        return Err(Error::Invalid);
    }
    let mut paths = BTreeSet::new();
    for v in criteria {
        logical(v)?;
        if !paths.insert(&v.logical_path) {
            return Err(Error::Invalid);
        }
    }
    Ok(())
}
fn surfaces_valid(surfaces: &[CareerOutputSurface]) -> Result<(), Error> {
    if surfaces.is_empty()
        || surfaces.len() > 3
        || surfaces.iter().any(|s| {
            !matches!(
                s,
                CareerOutputSurface::ApplicationEssay
                    | CareerOutputSurface::CareerDescription
                    | CareerOutputSurface::Portfolio
            )
        })
        || surfaces.iter().collect::<BTreeSet<_>>().len() != surfaces.len()
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
pub fn parse_record(content: &str) -> Result<WorkflowRecord, Error> {
    let marker = "```career-workflow\n";
    if content.len() > 65536 || content.matches(marker).count() != 1 {
        return Err(Error::Invalid);
    }
    let rest = content.split_once(marker).ok_or(Error::Invalid)?.1;
    let json = rest.split_once("\n```").ok_or(Error::Invalid)?.0;
    serde_json::from_str(json).map_err(|_| Error::Invalid)
}
async fn bound_source(
    conn: &mut PgConnection,
    v: &SourceVersion,
    body: bool,
) -> Result<String, Error> {
    let path = logical(v)?;
    let row=sqlx::query("SELECT material_id::text,revision,content_digest,content_digest=encode(sha256(content),'hex') AND byte_len=octet_length(content) AS valid,CASE WHEN $2 AND byte_len<=65536 AND octet_length(content)<=65536 THEN content END AS content FROM context_materials WHERE source_path=$1 AND NOT deleted AND NOT restricted").bind(path).bind(body).fetch_optional(&mut *conn).await.map_err(|_| Error::Storage)?.ok_or(Error::Conflict)?;
    let state = StoredSourceState::Live {
        material_id: row.get("material_id"),
        revision: u64::try_from(row.get::<i64, _>("revision")).map_err(|_| Error::Storage)?,
        content_digest: row.get("content_digest"),
    };
    if state != v.state {
        return Err(Error::Conflict);
    }
    if !row.get::<bool, _>("valid") {
        return Err(Error::Storage);
    }
    if !body {
        return Ok(String::new());
    }
    let bytes: Vec<u8> = row
        .get::<Option<Vec<u8>>, _>("content")
        .ok_or(Error::Limit)?;
    if let StoredSourceState::Live { content_digest, .. } = &state
        && digest(&bytes) != *content_digest
    {
        return Err(Error::Storage);
    }
    String::from_utf8(bytes).map_err(|_| Error::Invalid)
}
async fn current(conn: &mut PgConnection, v: &SourceVersion) -> Result<(), Error> {
    bound_source(conn, v, false).await.map(|_| ())
}
async fn record_body(conn: &mut PgConnection, v: &SourceVersion) -> Result<String, Error> {
    bound_source(conn, v, true).await
}
trait SourceReader: Send {
    fn read<'a>(
        &'a mut self,
        version: &'a SourceVersion,
        body: bool,
    ) -> impl std::future::Future<Output = Result<String, Error>> + Send + 'a;
}
impl SourceReader for PgConnection {
    fn read<'a>(
        &'a mut self,
        version: &'a SourceVersion,
        body: bool,
    ) -> impl std::future::Future<Output = Result<String, Error>> + Send + 'a {
        bound_source(self, version, body)
    }
}
fn sources(p: &Progress) -> Vec<&SourceVersion> {
    let mut refs = vec![&p.outcome];
    refs.extend(&p.criteria);
    refs.extend(p.reflection.as_ref());
    refs.extend(p.review.as_ref());
    refs.extend(p.verification.as_ref());
    refs
}
async fn fresh(conn: &mut PgConnection, p: &Progress) -> Result<(), Error> {
    fresh_except(conn, p, &[]).await
}
async fn fresh_except(
    conn: &mut PgConnection,
    p: &Progress,
    apply_targets: &[String],
) -> Result<(), Error> {
    fresh_with(p, apply_targets, conn).await
}
async fn fresh_with(
    p: &Progress,
    apply_targets: &[String],
    source: &mut impl SourceReader,
) -> Result<(), Error> {
    for v in sources(p) {
        if apply_targets.contains(&v.logical_path) {
            continue;
        }
        let reflection = p.reflection.as_ref() == Some(v);
        let verification = p.verification.as_ref() == Some(v);
        let body = source.read(v, reflection || verification).await?;
        if reflection {
            let WorkflowRecord::Reflection { findings, .. } = parse_record(&body)? else {
                return Err(Error::Invalid);
            };
            for evidence in findings.into_iter().flat_map(|f| f.evidence) {
                // A confirmed criteria apply intentionally leaves its prior input in the reflection.
                let replaced_criterion = p.apply.as_ref().is_some_and(|a| a.apply_id.is_some())
                    && p.initial_criteria.contains(&evidence)
                    && p.criteria
                        .iter()
                        .any(|v| v.logical_path == evidence.logical_path && v != &evidence);
                if !replaced_criterion && !apply_targets.contains(&evidence.logical_path) {
                    source.read(&evidence, false).await?;
                }
            }
        }
        if verification {
            let WorkflowRecord::Verification { checks, .. } = parse_record(&body)? else {
                return Err(Error::Invalid);
            };
            for check in checks {
                source.read(&check.independent_review, false).await?;
                if let Some(artifact) = check.artifact {
                    source.read(&artifact, false).await?;
                }
            }
        }
    }
    Ok(())
}
async fn start_with(
    requested: &Progress,
    existing: &Progress,
    source: &mut impl SourceReader,
) -> Result<(), Error> {
    // Never silently replace the input bindings of an existing outcome.
    if existing.outcome != requested.outcome
        || existing.surfaces != requested.surfaces
        || existing.initial_criteria != requested.initial_criteria
    {
        return Err(Error::Conflict);
    }
    fresh_with(existing, &[], source).await
}
async fn health_with(
    p: &Progress,
    gate: Result<(), Error>,
    source: &mut impl SourceReader,
) -> Result<&'static str, Error> {
    match gate {
        Err(Error::ContextPending) => Ok("native-apply-pending"),
        Err(e) => Err(e),
        Ok(()) if p.stage == Stage::CriteriaApply && p.apply.is_some() => {
            Ok("apply-confirmation-needed")
        }
        Ok(()) => match fresh_with(p, &[], source).await {
            Ok(()) => Ok("ready"),
            Err(Error::Conflict | Error::NotFound) => Ok("source-drift"),
            Err(e) => Err(e),
        },
    }
}
async fn review_targets(conn: &mut PgConnection, p: &Progress) -> Result<Vec<String>, Error> {
    let record = record_body(conn, p.review.as_ref().ok_or(Error::Conflict)?).await?;
    let WorkflowRecord::Review { decisions, .. } = parse_record(&record)? else {
        return Err(Error::Invalid);
    };
    Ok(decisions
        .into_iter()
        .filter(|d| d.disposition == Disposition::Apply)
        .flat_map(|d| d.targets)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}
// Observe the authoritative typed Core bytes and receipt; do not construct a new acceptance contract.
async fn applied(
    conn: &mut PgConnection,
    store_id: &str,
    binding: &ApplyBinding,
    required: &[&SourceVersion],
    targets: &[String],
    selected: Option<&SourceVersion>,
) -> Result<(String, Vec<SourceVersion>), Error> {
    let rows=sqlx::query("SELECT *,apply_id::text AS id FROM context_apply_batches WHERE store_id=$1::uuid AND core_run_id=$2 AND prepared_run_digest=$3").bind(store_id).bind(&binding.run_id).bind(&binding.prepared_run_digest).fetch_all(&mut *conn).await.map_err(|_|Error::Storage)?;
    if rows.len() != 1 {
        return Err(Error::Conflict);
    }
    let row = &rows[0];
    if row.get::<String, _>("state") != "finalized"
        || row
            .get::<Option<String>, _>("final_core_receipt_digest")
            .is_none()
    {
        return Err(Error::ContextPending);
    }
    let bytes = row
        .get::<Option<Vec<u8>>, _>("core_contract")
        .ok_or(Error::Conflict)?;
    if row
        .get::<Option<String>, _>("core_contract_digest")
        .as_deref()
        != Some(digest(&bytes).as_str())
    {
        return Err(Error::Conflict);
    }
    let contract: Value = serde_json::from_slice(&bytes).map_err(|_| Error::Conflict)?;
    let receipt: ContextCommitReceipt = serde_json::from_value(
        row.get::<Option<Value>, _>("commit_receipt")
            .ok_or(Error::Conflict)?,
    )
    .map_err(|_| Error::Conflict)?;
    receipt.validate().map_err(|_| Error::Conflict)?;
    let attempt = &contract["attempt"];
    if contract["store_identity"]["store_id"] != store_id
        || contract["contract_digest"] != receipt.contract_digest()
        || attempt["run_identifier"] != binding.run_id
        || attempt["prepared_run_digest"] != binding.prepared_run_digest
        || attempt["attempt_identifier"] != receipt.attempt_identifier()
        || attempt["expected_batch_identifier"] != receipt.batch_identifier()
        || attempt["expected_journal_relative_path"] != receipt.journal_relative_path()
    {
        return Err(Error::Conflict);
    }
    let expected: Value = row.get("expected_source_versions");
    let previous: Vec<SourceVersion> =
        serde_json::from_value(expected["targets"].clone()).map_err(|_| Error::Conflict)?;
    let non_targets: Vec<SourceVersion> =
        serde_json::from_value(expected["non_targets"].clone()).map_err(|_| Error::Conflict)?;
    if receipt.previous() != previous
        || contract["non_target_versions"] != expected["non_targets"]
        || contract["targets"] != row.get::<Value, _>("context_targets")
        || previous
            .iter()
            .map(|v| &v.logical_path)
            .collect::<BTreeSet<_>>()
            != targets.iter().collect::<BTreeSet<_>>()
        || required
            .iter()
            .any(|v| !previous.iter().chain(&non_targets).any(|b| b == *v))
    {
        return Err(Error::Conflict);
    }
    let contract_targets = contract["targets"].as_array().ok_or(Error::Conflict)?;
    if contract_targets.len() != receipt.resulting().len() {
        return Err(Error::Conflict);
    }
    if selected.is_some_and(|v| !receipt.resulting().contains(v)) {
        return Err(Error::Conflict);
    }
    let apply_id: String = row.get("id");
    for ((prior, result), t) in receipt
        .previous()
        .iter()
        .zip(receipt.resulting())
        .zip(contract_targets)
    {
        let StoredSourceState::Live { content_digest, .. } = &result.state else {
            return Err(Error::Conflict);
        };
        if t["previous"] != serde_json::to_value(prior).map_err(|_| Error::Storage)?
            || t["resulting_content_digest"] != *content_digest
            || t["operation"] == "delete"
        {
            return Err(Error::Conflict);
        }
        let (
            StoredSourceState::Live {
                material_id,
                revision,
                ..
            },
            StoredSourceState::Live {
                material_id: new_id,
                revision: new_rev,
                content_digest,
            },
        ) = (&prior.state, &result.state)
        else {
            // Output artifacts may be newly created; criteria must already exist.
            if matches!(prior.state, StoredSourceState::Missing)
                && matches!(result.state, StoredSourceState::Live { revision: 1, .. })
            {
                if selected.is_none_or(|v| v == result) {
                    current(conn, result).await?;
                }
                continue;
            }
            return Err(Error::Conflict);
        };
        if material_id != new_id
            || revision.checked_add(1) != Some(*new_rev)
            || t["previous"] != serde_json::to_value(prior).map_err(|_| Error::Storage)?
            || t["resulting_content_digest"] != *content_digest
            || t["operation"] == "delete"
        {
            return Err(Error::Conflict);
        }
        if selected.is_none_or(|v| v == result) {
            current(conn, result).await?;
        }
    }
    for result in receipt
        .resulting()
        .iter()
        .filter(|v| selected.is_none_or(|s| s == *v))
    {
        let path = logical(result)?;
        let ok:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM context_materials m JOIN context_material_versions v USING(material_id,revision) WHERE m.source_path=$1 AND m.last_apply_id=$2::uuid AND v.apply_id=$2::uuid AND m.content=v.content AND m.content_digest=v.content_digest AND NOT m.deleted AND NOT v.deleted)").bind(path).bind(&apply_id).fetch_one(&mut *conn).await.map_err(|_|Error::Storage)?;
        if !ok {
            return Err(Error::Conflict);
        }
    }
    Ok((apply_id, receipt.resulting().to_vec()))
}

impl Store {
    async fn career_health(
        &self,
        conn: &mut PgConnection,
        p: &Progress,
    ) -> Result<&'static str, Error> {
        let gate = self.context_gate_in(conn, false).await;
        health_with(p, gate, conn).await
    }
    pub async fn list_career_workflows(&self) -> Result<Value, Error> {
        // The progress dashboard remains available while native source reads are recovery-gated.
        let store_id: String =
            sqlx::query_scalar("SELECT store_id::text FROM context_store WHERE singleton")
                .fetch_one(self.pool())
                .await
                .map_err(|_| Error::Storage)?;
        self.career_workflow(CareerCommand::List { store_id }).await
    }
    pub async fn career_workflow(&self, command: CareerCommand) -> Result<Value, Error> {
        let mut tx = self.pool().begin().await.map_err(|_| Error::Storage)?;
        let result = self.career_in(&mut tx, command).await.and_then(|value| {
            if serde_json::to_vec(&value)
                .map_err(|_| Error::Storage)?
                .len()
                > crate::domain::MAX_RESPONSE_BYTES
            {
                return Err(Error::Limit);
            }
            Ok(value)
        });
        self.finish_context(tx, result).await
    }
    async fn career_in(
        &self,
        conn: &mut PgConnection,
        command: CareerCommand,
    ) -> Result<Value, Error> {
        let store_id = match &command {
            CareerCommand::List { store_id }
            | CareerCommand::Get { store_id, .. }
            | CareerCommand::Start { store_id, .. }
            | CareerCommand::Record { store_id, .. }
            | CareerCommand::BindApply { store_id, .. }
            | CareerCommand::ConfirmApply { store_id, .. }
            | CareerCommand::Pause { store_id, .. }
            | CareerCommand::Resume { store_id, .. }
            | CareerCommand::Rebase { store_id, .. } => store_id.clone(),
        };
        uuid::Uuid::parse_str(&store_id).map_err(|_| Error::Invalid)?;
        let identity: String =
            sqlx::query_scalar("SELECT store_id::text FROM context_store WHERE singleton")
                .fetch_one(&mut *conn)
                .await
                .map_err(|_| Error::Storage)?;
        if identity != store_id {
            return Err(Error::Conflict);
        }
        let installed: bool =
            sqlx::query_scalar("SELECT to_regclass('public.career_workflows') IS NOT NULL")
                .fetch_one(&mut *conn)
                .await
                .map_err(|_| Error::Storage)?;
        if !installed {
            return Err(Error::Baseline);
        }
        if matches!(command, CareerCommand::List { .. }) {
            let rows=sqlx::query("SELECT id::text,application_key,outcome_key,revision,jsonb_build_object('stage',progress->'stage','paused',progress->'paused') AS progress FROM career_workflows WHERE store_id=$1::uuid ORDER BY updated_at DESC,id LIMIT 100").bind(&store_id).fetch_all(&mut *conn).await.map_err(|_|Error::Storage)?;
            return Ok(
                json!({"workflows":rows.iter().map(|r|json!({"id":r.get::<String,_>("id"),"application_key":r.get::<String,_>("application_key"),"outcome_key":r.get::<String,_>("outcome_key"),"revision":r.get::<i64,_>("revision"),"progress":r.get::<Value,_>("progress")})).collect::<Vec<_>>(),"limit":100,"store_id":store_id}),
            );
        }
        if let CareerCommand::Start {
            application_key,
            outcome_key,
            outcome,
            criteria,
            surfaces,
            ..
        } = &command
        {
            key(application_key)?;
            key(outcome_key)?;
            logical(outcome)?;
            criteria_valid(criteria)?;
            surfaces_valid(surfaces)?;
            self.context_gate_in(conn, false).await?;
            let p = Progress {
                outcome: outcome.clone(),
                initial_criteria: criteria.clone(),
                criteria: criteria.clone(),
                surfaces: surfaces.clone(),
                stage: Stage::Reflection,
                paused: false,
                reflection: None,
                review: None,
                apply: None,
                verification: None,
            };
            let row=sqlx::query("INSERT INTO career_workflows(store_id,application_key,outcome_key,progress) VALUES($1::uuid,$2,$3,$4) ON CONFLICT(store_id,application_key,outcome_key) DO UPDATE SET application_key=career_workflows.application_key RETURNING id::text,revision,progress").bind(&store_id).bind(application_key).bind(outcome_key).bind(serde_json::to_value(&p).map_err(|_|Error::Storage)?).fetch_one(&mut *conn).await.map_err(|_|Error::Storage)?;
            let existing: Progress =
                serde_json::from_value(row.get("progress")).map_err(|_| Error::Storage)?;
            start_with(&p, &existing, conn).await?;
            return Ok(
                json!({"id":row.get::<String,_>("id"),"revision":row.get::<i64,_>("revision"),"application_key":application_key,"outcome_key":outcome_key,"progress":existing,"health":"ready"}),
            );
        }
        let (id, revision) = match &command {
            CareerCommand::Get { id, .. } => (id, None),
            CareerCommand::Record {
                id,
                expected_revision,
                ..
            }
            | CareerCommand::BindApply {
                id,
                expected_revision,
                ..
            }
            | CareerCommand::ConfirmApply {
                id,
                expected_revision,
                ..
            }
            | CareerCommand::Pause {
                id,
                expected_revision,
                ..
            }
            | CareerCommand::Resume {
                id,
                expected_revision,
                ..
            }
            | CareerCommand::Rebase {
                id,
                expected_revision,
                ..
            } => (id, Some(*expected_revision)),
            _ => return Err(Error::Invalid),
        };
        let id = id.clone();
        uuid::Uuid::parse_str(&id).map_err(|_| Error::Invalid)?;
        let row=sqlx::query("SELECT revision,application_key,outcome_key,progress FROM career_workflows WHERE id=$1::uuid AND store_id=$2::uuid FOR UPDATE").bind(&id).bind(&store_id).fetch_optional(&mut *conn).await.map_err(|_|Error::Storage)?.ok_or(Error::NotFound)?;
        let actual: i64 = row.get("revision");
        if revision.is_some_and(|r| r != actual) {
            return Err(Error::Conflict);
        }
        let mut p: Progress =
            serde_json::from_value(row.get("progress")).map_err(|_| Error::Storage)?;
        let application_key: String = row.get("application_key");
        let outcome_key: String = row.get("outcome_key");
        if matches!(command, CareerCommand::Get { .. }) {
            let health = self.career_health(conn, &p).await?;
            return Ok(
                json!({"id":id,"revision":actual,"application_key":application_key,"outcome_key":outcome_key,"progress":p,"health":health}),
            );
        }
        let pausing = matches!(command, CareerCommand::Pause { .. });
        if !matches!(command, CareerCommand::Pause { .. }) {
            self.context_gate_in(conn, false).await?;
        }
        if p.paused
            && !matches!(
                command,
                CareerCommand::Resume { .. } | CareerCommand::Rebase { .. }
            )
        {
            return Err(Error::Conflict);
        }
        if matches!(command, CareerCommand::Resume { .. })
            && p.stage == Stage::CriteriaApply
            && p.apply.is_some()
        {
            let targets = review_targets(conn, &p).await?;
            fresh_except(conn, &p, &targets).await?;
        } else if !matches!(
            command,
            CareerCommand::ConfirmApply { .. }
                | CareerCommand::Pause { .. }
                | CareerCommand::Rebase { .. }
        ) {
            fresh(conn, &p).await?;
        }
        match command {
            CareerCommand::Pause { .. } => p.paused = true,
            CareerCommand::Resume { .. } => p.paused = false,
            CareerCommand::Rebase {
                outcome, criteria, ..
            } => {
                if p.apply.is_some() || p.stage == Stage::Complete {
                    return Err(Error::Conflict);
                }
                logical(&outcome)?;
                criteria_valid(&criteria)?;
                p.outcome = outcome;
                p.initial_criteria = criteria.clone();
                p.criteria = criteria;
                p.reflection = None;
                p.review = None;
                p.verification = None;
                p.stage = Stage::Reflection;
                p.paused = false;
                fresh(conn, &p).await?;
            }
            CareerCommand::BindApply {
                run_id,
                prepared_run_digest,
                ..
            } => {
                if p.stage != Stage::CriteriaApply || p.apply.is_some() {
                    return Err(Error::Conflict);
                }
                key(&run_id)?;
                if prepared_run_digest.len() != 64
                    || !prepared_run_digest
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    return Err(Error::Invalid);
                }
                let existing: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM context_apply_batches WHERE core_run_id=$1)",
                )
                .bind(&run_id)
                .fetch_one(&mut *conn)
                .await
                .map_err(|_| Error::Storage)?;
                if existing {
                    return Err(Error::Conflict);
                }
                p.apply = Some(ApplyBinding {
                    run_id,
                    prepared_run_digest,
                    apply_id: None,
                });
            }
            CareerCommand::ConfirmApply { .. } => {
                if p.stage != Stage::CriteriaApply {
                    return Err(Error::Conflict);
                }
                let targets = review_targets(conn, &p).await?;
                // Non-target originals must still be current; target revisions are checked against receipt history.
                fresh_except(conn, &p, &targets).await?;
                let binding = p.apply.as_ref().ok_or(Error::Conflict)?;
                let (apply_id, resulting) = applied(
                    conn,
                    &store_id,
                    binding,
                    &p.criteria
                        .iter()
                        .filter(|v| targets.contains(&v.logical_path))
                        .collect::<Vec<_>>(),
                    &targets,
                    None,
                )
                .await?;
                for v in &mut p.criteria {
                    if let Some(updated) =
                        resulting.iter().find(|r| r.logical_path == v.logical_path)
                    {
                        *v = updated.clone();
                    }
                }
                p.apply.as_mut().ok_or(Error::Conflict)?.apply_id = Some(apply_id);
                p.stage = Stage::OutputVerification;
            }
            CareerCommand::Record { record, .. } => {
                logical(&record)?;
                if sources(&p)
                    .iter()
                    .any(|v| v.logical_path == record.logical_path)
                {
                    return Err(Error::Invalid);
                }
                let body = record_body(conn, &record).await?;
                match parse_record(&body)? {
                    WorkflowRecord::Reflection {
                        application_key: a,
                        outcome_key: o,
                        findings,
                    } if p.stage == Stage::Reflection => {
                        if a != application_key
                            || o != outcome_key
                            || findings.is_empty()
                            || findings.len() > 32
                        {
                            return Err(Error::Invalid);
                        }
                        let mut ids = BTreeSet::new();
                        for f in findings {
                            key(&f.id)?;
                            text(&f.statement)?;
                            if !ids.insert(f.id)
                                || f.evidence.len() > 12
                                || f.kind == FindingKind::ConfirmedFact && f.evidence.is_empty()
                            {
                                return Err(Error::Invalid);
                            }
                            for v in f.evidence {
                                current(conn, &v).await?;
                            }
                        }
                        p.reflection = Some(record);
                        p.stage = Stage::Review;
                    }
                    WorkflowRecord::Review {
                        reflection,
                        decisions,
                    } if p.stage == Stage::Review => {
                        if Some(&reflection) != p.reflection.as_ref() {
                            return Err(Error::Conflict);
                        }
                        let WorkflowRecord::Reflection { findings, .. } =
                            parse_record(&record_body(conn, &reflection).await?)?
                        else {
                            return Err(Error::Invalid);
                        };
                        if decisions.len() != findings.len() {
                            return Err(Error::Invalid);
                        }
                        let mut ids = BTreeSet::new();
                        let mut any_apply = false;
                        for d in decisions {
                            text(&d.reason)?;
                            let f = findings
                                .iter()
                                .find(|f| f.id == d.finding_id)
                                .ok_or(Error::Invalid)?;
                            if !ids.insert(d.finding_id)
                                || d.targets.len() > 12
                                || d.targets.iter().collect::<BTreeSet<_>>().len()
                                    != d.targets.len()
                                || d.targets
                                    .iter()
                                    .any(|t| !p.criteria.iter().any(|v| &v.logical_path == t))
                            {
                                return Err(Error::Invalid);
                            }
                            if d.disposition == Disposition::Apply {
                                any_apply = true;
                                if d.targets.is_empty()
                                    || matches!(
                                        f.kind,
                                        FindingKind::Hypothesis | FindingKind::Unknown
                                    )
                                {
                                    return Err(Error::Invalid);
                                }
                            }
                            if matches!(
                                d.disposition,
                                Disposition::Apply | Disposition::AlreadyCovered
                            ) && d.targets.is_empty()
                            {
                                return Err(Error::Invalid);
                            }
                            if d.disposition == Disposition::Hold {
                                text(d.revisit_when.as_deref().ok_or(Error::Invalid)?)?;
                            }
                            for v in &f.evidence {
                                current(conn, v).await?;
                            }
                        }
                        p.review = Some(record);
                        p.stage = if any_apply {
                            Stage::CriteriaApply
                        } else {
                            Stage::OutputVerification
                        };
                    }
                    WorkflowRecord::Verification {
                        review,
                        criteria,
                        checks,
                    } if p.stage == Stage::OutputVerification => {
                        if Some(&review) != p.review.as_ref()
                            || criteria != p.criteria
                            || checks.len() != p.surfaces.len()
                        {
                            return Err(Error::Conflict);
                        }
                        let mut surfaces = BTreeSet::new();
                        for check in checks {
                            if check.result != CheckResult::Passed
                                || !p.surfaces.contains(&check.surface)
                                || !surfaces.insert(check.surface)
                                || check.independent_review.logical_path == record.logical_path
                                || check.artifact.as_ref().is_some_and(|a| {
                                    a.logical_path == check.independent_review.logical_path
                                })
                            {
                                return Err(Error::Invalid);
                            }
                            let reviewed = record_body(conn, &check.independent_review).await?;
                            let WorkflowRecord::OutputReview {
                                review,
                                surface,
                                criteria,
                                artifact,
                                result,
                                observations,
                            } = parse_record(&reviewed)?
                            else {
                                return Err(Error::Invalid);
                            };
                            text(&observations)?;
                            if Some(&review) != p.review.as_ref()
                                || surface != check.surface
                                || criteria != p.criteria
                                || artifact != check.artifact
                                || result != CheckResult::Passed
                            {
                                return Err(Error::Conflict);
                            }
                            if let Some(artifact) = check.artifact {
                                current(conn, &artifact).await?;
                                let path = logical(&artifact)?;
                                let row=sqlx::query("SELECT a.core_run_id,a.prepared_run_digest FROM context_materials m JOIN context_apply_batches a ON a.apply_id=m.last_apply_id WHERE m.source_path=$1").bind(path).fetch_optional(&mut *conn).await.map_err(|_|Error::Storage)?.ok_or(Error::Conflict)?;
                                let binding = ApplyBinding {
                                    run_id: row.get("core_run_id"),
                                    prepared_run_digest: row.get("prepared_run_digest"),
                                    apply_id: None,
                                };
                                // One batch may create multiple surfaces. Check this artifact's effect plus exact criteria input bindings.
                                let paths:Vec<String>=sqlx::query_scalar("SELECT jsonb_array_elements(context_targets)->'previous'->>'logical_path' FROM context_apply_batches WHERE core_run_id=$1 AND prepared_run_digest=$2").bind(&binding.run_id).bind(&binding.prepared_run_digest).fetch_all(&mut *conn).await.map_err(|_|Error::Storage)?;
                                let (_, outputs) = applied(
                                    conn,
                                    &store_id,
                                    &binding,
                                    &p.criteria.iter().collect::<Vec<_>>(),
                                    &paths,
                                    Some(&artifact),
                                )
                                .await?;
                                if !outputs.contains(&artifact) {
                                    return Err(Error::Conflict);
                                }
                            }
                        }
                        p.verification = Some(record);
                        p.stage = Stage::Complete;
                    }
                    _ => return Err(Error::Conflict),
                }
            }
            _ => return Err(Error::Invalid),
        }
        let next = actual.checked_add(1).ok_or(Error::Limit)?;
        let value = serde_json::to_value(&p).map_err(|_| Error::Storage)?;
        if serde_json::to_vec(&value)
            .map_err(|_| Error::Storage)?
            .len()
            > 65536
        {
            return Err(Error::Limit);
        }
        sqlx::query("UPDATE career_workflows SET progress=$2,revision=$3,updated_at=now() WHERE id=$1::uuid").bind(&id).bind(value).bind(next).execute(&mut *conn).await.map_err(|_|Error::Storage)?;
        let health = if pausing {
            self.career_health(conn, &p).await?
        } else {
            "ready"
        };
        Ok(
            json!({"id":id,"revision":next,"application_key":application_key,"outcome_key":outcome_key,"progress":p,"health":health}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    // Already-recorded synthetic progress; Core acceptance remains an integration-test concern.
    struct Originals(BTreeMap<String, (SourceVersion, String)>);
    impl Originals {
        fn put(&mut self, path: &str, body: String) -> SourceVersion {
            let (material_id, revision) = match self.0.get(path).map(|(v, _)| &v.state) {
                Some(StoredSourceState::Live {
                    material_id,
                    revision,
                    ..
                }) => (material_id.clone(), revision + 1),
                _ => (uuid::Uuid::new_v4().to_string(), 1),
            };
            let version = SourceVersion {
                logical_path: path.to_owned(),
                state: StoredSourceState::Live {
                    material_id,
                    revision,
                    content_digest: digest(body.as_bytes()),
                },
            };
            self.0.insert(path.to_owned(), (version.clone(), body));
            version
        }
        fn read(&self, version: &SourceVersion, body: bool) -> Result<String, Error> {
            logical(version)?;
            let (actual, content) = self.0.get(&version.logical_path).ok_or(Error::Conflict)?;
            if actual != version {
                return Err(Error::Conflict);
            }
            Ok(if body { content.clone() } else { String::new() })
        }
        fn drift(&mut self, version: &SourceVersion, deleted: bool) {
            if deleted {
                self.0.remove(&version.logical_path);
            } else {
                self.put(&version.logical_path, "changed original\n".into());
            }
        }
    }
    impl SourceReader for &Originals {
        fn read<'a>(
            &'a mut self,
            version: &'a SourceVersion,
            body: bool,
        ) -> impl std::future::Future<Output = Result<String, Error>> + Send + 'a {
            std::future::ready(Originals::read(self, version, body))
        }
    }
    fn fenced(value: Value) -> String {
        format!("```career-workflow\n{value}\n```\n")
    }
    struct Fixture {
        originals: Originals,
        progress: Progress,
        evidence: SourceVersion,
        outputs: Vec<(Option<SourceVersion>, SourceVersion)>,
    }
    impl Fixture {
        fn complete(native: bool, applied: bool) -> Self {
            let mut originals = Originals(BTreeMap::new());
            let outcome = originals.put("vault/personal/outcome.md", "synthetic outcome\n".into());
            let initial = originals.put("vault/personal/criteria.md", "initial criterion\n".into());
            let evidence =
                originals.put("vault/personal/evidence.md", "synthetic evidence\n".into());
            let criteria = if applied {
                originals.put(&initial.logical_path, "reviewed criterion\n".into())
            } else {
                initial.clone()
            };
            let reflection = originals.put("vault/personal/reflection.md", fenced(json!({
                "kind":"reflection", "application_key":"synthetic", "outcome_key":"result-1",
                "findings":[{"id":"fact", "kind":"confirmed-fact", "statement":"synthetic finding",
                    "evidence":[evidence, initial]}]
            })));
            let review = originals.put("vault/personal/review.md", fenced(json!({
                "kind":"review", "reflection":reflection,
                "decisions":[{"finding_id":"fact", "disposition":if applied {"apply"} else {"already-covered"},
                    "reason":"synthetic decision", "targets":[initial.logical_path], "revisit_when":null}]
            })));
            let surfaces = vec![
                CareerOutputSurface::ApplicationEssay,
                CareerOutputSurface::CareerDescription,
                CareerOutputSurface::Portfolio,
            ];
            let mut outputs = Vec::new();
            let mut checks = Vec::new();
            for (index, surface) in surfaces.iter().enumerate() {
                let artifact = native.then(|| {
                    originals.put(
                        &format!("vault/personal/output-{index}.md"),
                        "synthetic output\n".into(),
                    )
                });
                let independent_review = originals.put(
                    &format!("vault/personal/output-review-{index}.md"),
                    fenced(json!({
                        "kind":"output-review", "review":review, "surface":surface,
                        "criteria":[criteria], "artifact":artifact, "result":"passed",
                        "observations":"synthetic independent observations"
                    })),
                );
                checks.push(json!({"surface":surface, "artifact":artifact,
                    "independent_review":independent_review, "result":"passed"}));
                outputs.push((artifact, independent_review));
            }
            let verification = originals.put(
                "vault/personal/verification.md",
                fenced(json!({
                    "kind":"verification", "review":review, "criteria":[criteria], "checks":checks
                })),
            );
            Self {
                originals,
                evidence,
                outputs,
                progress: Progress {
                    outcome,
                    initial_criteria: vec![initial],
                    criteria: vec![criteria],
                    surfaces,
                    stage: Stage::Complete,
                    paused: false,
                    reflection: Some(reflection),
                    review: Some(review),
                    apply: applied.then(|| ApplyBinding {
                        run_id: "synthetic-apply".into(),
                        prepared_run_digest: "a".repeat(64),
                        apply_id: Some(uuid::Uuid::new_v4().to_string()),
                    }),
                    verification: Some(verification),
                },
            }
        }
        fn requested_start(&self) -> Progress {
            Progress {
                criteria: self.progress.initial_criteria.clone(),
                stage: Stage::Reflection,
                paused: false,
                reflection: None,
                review: None,
                apply: None,
                verification: None,
                ..self.progress.clone()
            }
        }
        async fn fresh(&self) -> Result<(), Error> {
            fresh_with(&self.progress, &[], &mut &self.originals).await
        }
        async fn start(&self, requested: &Progress) -> Result<(), Error> {
            start_with(requested, &self.progress, &mut &self.originals).await
        }
        async fn health(&self) -> Result<&'static str, Error> {
            health_with(&self.progress, Ok(()), &mut &self.originals).await
        }
    }

    #[tokio::test]
    async fn completed_native_and_external_outputs_and_start_stay_current() {
        for native in [false, true] {
            for applied in [false, true] {
                let mut fixture = Fixture::complete(native, applied);
                let before = fixture.progress.clone();
                assert_eq!(fixture.fresh().await, Ok(()));
                // The initial criterion may now be historical; only returned progress is current.
                assert_eq!(fixture.start(&fixture.requested_start()).await, Ok(()));
                assert_eq!(fixture.progress, before);
                assert_eq!(fixture.health().await, Ok("ready"));
                fixture.progress.paused = true;
                assert_eq!(fixture.health().await, Ok("ready"));
            }
        }
    }

    #[tokio::test]
    async fn changed_or_deleted_outputs_invalidate_completed_progress_and_start() {
        for native in [false, true] {
            for index in 0..3 {
                for artifact in [false, true] {
                    if artifact && !native {
                        continue;
                    }
                    for deleted in [false, true] {
                        let mut fixture = Fixture::complete(native, false);
                        let before = fixture.progress.clone();
                        let output = &fixture.outputs[index];
                        let version = if artifact {
                            output.0.as_ref().unwrap()
                        } else {
                            &output.1
                        };
                        fixture.originals.drift(version, deleted);
                        assert_eq!(fixture.fresh().await, Err(Error::Conflict));
                        assert_eq!(
                            fixture.start(&fixture.requested_start()).await,
                            Err(Error::Conflict)
                        );
                        // Freshness does not rewrite the historical completion record.
                        assert_eq!(fixture.progress, before);
                        assert_eq!(fixture.health().await, Ok("source-drift"));
                        fixture.progress.paused = true;
                        assert_eq!(fixture.health().await, Ok("source-drift"));
                        assert_eq!(fixture.progress.stage, Stage::Complete);
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn reflection_evidence_stays_bound_after_review_and_completion() {
        for stage in [Stage::Review, Stage::OutputVerification, Stage::Complete] {
            for deleted in [false, true] {
                let mut fixture = Fixture::complete(true, false);
                fixture.progress.stage = stage;
                if stage != Stage::Complete {
                    fixture.progress.verification = None;
                }
                if stage == Stage::Review {
                    fixture.progress.review = None;
                }
                assert_eq!(fixture.fresh().await, Ok(()));
                fixture.originals.drift(&fixture.evidence, deleted);
                assert_eq!(fixture.fresh().await, Err(Error::Conflict));
                fixture.progress.paused = true;
                assert_eq!(fixture.health().await, Ok("source-drift"));
            }
        }
    }

    #[tokio::test]
    async fn criteria_apply_preserves_only_its_exact_initial_evidence() {
        let mut fixture = Fixture::complete(true, true);
        assert_eq!(fixture.fresh().await, Ok(()));
        let binding = fixture.progress.apply.clone();
        fixture.progress.apply.as_mut().unwrap().apply_id = None;
        assert_eq!(fixture.fresh().await, Err(Error::Conflict));
        fixture.progress.apply = binding;
        let initial = fixture.progress.initial_criteria.clone();
        if let StoredSourceState::Live { revision, .. } =
            &mut fixture.progress.initial_criteria[0].state
        {
            *revision += 10;
        }
        assert_eq!(fixture.fresh().await, Err(Error::Conflict));
        fixture.progress.initial_criteria = initial;
        fixture.originals.drift(&fixture.evidence, false);
        assert_eq!(fixture.fresh().await, Err(Error::Conflict));

        let mut fixture = Fixture::complete(true, true);
        fixture
            .originals
            .drift(&fixture.progress.criteria[0], false);
        assert_eq!(fixture.fresh().await, Err(Error::Conflict));
    }

    #[tokio::test]
    async fn pending_apply_exempts_only_selected_targets() {
        let mut fixture = Fixture::complete(true, true);
        fixture.progress.stage = Stage::CriteriaApply;
        fixture.progress.verification = None;
        fixture.progress.criteria = fixture.progress.initial_criteria.clone();
        fixture.progress.apply.as_mut().unwrap().apply_id = None;
        let targets = vec![fixture.progress.criteria[0].logical_path.clone()];
        assert_eq!(fixture.fresh().await, Err(Error::Conflict));
        assert_eq!(
            fresh_with(&fixture.progress, &targets, &mut &fixture.originals).await,
            Ok(())
        );
        fixture.originals.drift(&fixture.evidence, true);
        assert_eq!(
            fresh_with(&fixture.progress, &targets, &mut &fixture.originals).await,
            Err(Error::Conflict)
        );
    }

    #[tokio::test]
    async fn start_keeps_input_bindings_and_validates_new_progress() {
        let fixture = Fixture::complete(true, false);
        let requested = fixture.requested_start();
        assert_eq!(
            start_with(&requested, &requested, &mut &fixture.originals).await,
            Ok(())
        );
        for input in 0..3 {
            let mut changed = requested.clone();
            match input {
                0 => changed.outcome.logical_path = "vault/personal/other-outcome.md".into(),
                1 => {
                    changed.initial_criteria[0].logical_path =
                        "vault/personal/other-criterion.md".into()
                }
                _ => changed.surfaces.truncate(2),
            }
            assert_eq!(fixture.start(&changed).await, Err(Error::Conflict));
        }
        let mut fixture = fixture;
        fixture.originals.drift(&requested.outcome, true);
        assert_eq!(
            start_with(&requested, &requested, &mut &fixture.originals).await,
            Err(Error::Conflict)
        );
    }

    #[tokio::test]
    async fn current_workflow_records_must_keep_their_kind() {
        for reflection in [false, true] {
            let mut fixture = Fixture::complete(true, false);
            let record = if reflection {
                &mut fixture.progress.reflection
            } else {
                &mut fixture.progress.verification
            };
            *record = Some(fixture.originals.put(
                &record.as_ref().unwrap().logical_path,
                fenced(json!({"kind":"review", "reflection":fixture.evidence, "decisions":[]})),
            ));
            assert_eq!(fixture.fresh().await, Err(Error::Invalid));
        }
    }

    #[tokio::test]
    async fn health_preserves_recovery_gates_and_does_not_hide_errors() {
        struct Unreadable;
        impl SourceReader for Unreadable {
            fn read<'a>(
                &'a mut self,
                _: &'a SourceVersion,
                _: bool,
            ) -> impl std::future::Future<Output = Result<String, Error>> + Send + 'a {
                std::future::ready(Err(Error::Storage))
            }
        }
        let mut fixture = Fixture::complete(true, true);
        fixture.progress.paused = true;
        assert_eq!(
            health_with(
                &fixture.progress,
                Err(Error::ContextPending),
                &mut Unreadable
            )
            .await,
            Ok("native-apply-pending")
        );
        assert_eq!(
            health_with(&fixture.progress, Err(Error::Forbidden), &mut Unreadable).await,
            Err(Error::Forbidden)
        );
        assert_eq!(
            health_with(&fixture.progress, Ok(()), &mut Unreadable).await,
            Err(Error::Storage)
        );
        fixture.progress.stage = Stage::CriteriaApply;
        assert_eq!(
            health_with(&fixture.progress, Ok(()), &mut Unreadable).await,
            Ok("apply-confirmation-needed")
        );
    }
}
