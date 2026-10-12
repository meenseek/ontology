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
fn sources(p: &Progress) -> Vec<&SourceVersion> {
    let mut refs = vec![&p.outcome];
    refs.extend(&p.criteria);
    refs.extend(p.reflection.as_ref());
    refs.extend(p.review.as_ref());
    refs.extend(p.verification.as_ref());
    refs
}
async fn fresh(conn: &mut PgConnection, p: &Progress) -> Result<(), Error> {
    for v in sources(p) {
        current(conn, v).await?;
    }
    Ok(())
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
            fresh(conn, &p).await?;
            let row=sqlx::query("INSERT INTO career_workflows(store_id,application_key,outcome_key,progress) VALUES($1::uuid,$2,$3,$4) ON CONFLICT(store_id,application_key,outcome_key) DO UPDATE SET application_key=career_workflows.application_key RETURNING id::text,revision,progress").bind(&store_id).bind(application_key).bind(outcome_key).bind(serde_json::to_value(&p).map_err(|_|Error::Storage)?).fetch_one(&mut *conn).await.map_err(|_|Error::Storage)?;
            let existing: Progress =
                serde_json::from_value(row.get("progress")).map_err(|_| Error::Storage)?;
            // Never silently replace the input bindings of an existing outcome.
            if existing.outcome != p.outcome
                || existing.surfaces != p.surfaces
                || existing.initial_criteria != p.initial_criteria
            {
                return Err(Error::Conflict);
            }
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
            let health = match self.context_gate_in(conn, false).await {
                Err(Error::ContextPending) => "native-apply-pending",
                Err(e) => return Err(e),
                Ok(()) if p.stage == Stage::CriteriaApply && p.apply.is_some() => {
                    "apply-confirmation-needed"
                }
                Ok(()) => match fresh(conn, &p).await {
                    Ok(()) => "ready",
                    Err(Error::Conflict | Error::NotFound) => "source-drift",
                    Err(e) => return Err(e),
                },
            };
            return Ok(
                json!({"id":id,"revision":actual,"application_key":application_key,"outcome_key":outcome_key,"progress":p,"health":health}),
            );
        }
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
            for v in sources(&p)
                .into_iter()
                .filter(|v| !targets.contains(&v.logical_path))
            {
                current(conn, v).await?;
            }
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
                for v in sources(&p)
                    .into_iter()
                    .filter(|v| !targets.contains(&v.logical_path))
                {
                    current(conn, v).await?;
                }
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
        Ok(
            json!({"id":id,"revision":next,"application_key":application_key,"outcome_key":outcome_key,"progress":p,"health":"ready"}),
        )
    }
}
