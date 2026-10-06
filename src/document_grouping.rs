//! Discover canonical documents centrally; source producers never enqueue copies.
//! Durable intent/claims live here, membership/history remain in purpose.rs.
use crate::{
    context::ContextScope,
    context_projection::eligible,
    domain::{Error, Scope},
    grouping::{Judgment, run_judgment, validate_judgment},
    purpose::{DocumentIdentity, SubjectDefinition, expected_revision},
    store::{Store, digest},
};
use context_core::{document::parse_markdown_bytes, redaction::redact_secrets};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Row, Transaction};
use std::{path::Path, time::Duration};
use uuid::Uuid;

const POLICY: &str = include_str!("../docs/document-grouping.md");

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentGroupingRetry {
    pub identity: DocumentIdentity,
    pub revision: i64,
    pub source_revision: String,
    pub content_digest: String,
}

struct Claim {
    scope: Scope,
    identity: DocumentIdentity,
    key: String,
    source_revision: String,
    content_digest: String,
    membership_revision: i64,
    token: String,
}

fn model_record(path: &str, bytes: &[u8]) -> Result<Value, Error> {
    let (title, body) = match parse_markdown_bytes(Path::new(path), bytes) {
        Ok(parsed) => (
            parsed.document().title().to_owned(),
            parsed.document().body().to_owned(),
        ),
        Err(_) => (
            "문서".into(),
            std::str::from_utf8(bytes)
                .map_err(|_| Error::Invalid)?
                .to_owned(),
        ),
    };
    Ok(json!({"title":redact_secrets(&title),"body":redact_secrets(&body)}))
}

fn document_judgment(value: Judgment, definitions: &[Value]) -> Result<Judgment, Error> {
    let ids = definitions
        .iter()
        .filter_map(|s| s["id"].as_str().map(str::to_owned))
        .collect();
    let value = validate_judgment(value, &ids)?;
    if value.new_subject.is_some() {
        return Err(Error::Invalid);
    }
    if value.decision == "assign" {
        let definition = definitions
            .iter()
            .find(|s| s["id"].as_str() == value.subject_id.as_deref())
            .ok_or(Error::Invalid)?;
        serde_json::from_value::<SubjectDefinition>(definition["definition"].clone())
            .map_err(|_| Error::Invalid)?
            .validate()?;
    }
    Ok(value)
}

impl Store {
    pub(crate) async fn manual_document_grouping(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        scope: Scope,
        identity: &DocumentIdentity,
        source: &Value,
        revision: i64,
        assigned: bool,
    ) -> Result<(), Error> {
        let mode = if assigned { "manual" } else { "off" };
        sqlx::query("INSERT INTO document_grouping(scope,material_id,entity_id,mode,state,source_revision,content_digest,membership_revision) VALUES($1,$2::uuid,$3,$4,$4,$5,$6,$7) ON CONFLICT(scope,source_id) DO UPDATE SET mode=EXCLUDED.mode,state=EXCLUDED.state,source_revision=EXCLUDED.source_revision,content_digest=EXCLUDED.content_digest,membership_revision=EXCLUDED.membership_revision,attempts=0,claim_token=NULL,lease_until=NULL,suggestions='{}'::jsonb,reason=NULL,policy_digest=NULL,updated_at=now() WHERE document_grouping.mode<>EXCLUDED.mode OR document_grouping.state<>EXCLUDED.state OR document_grouping.source_revision<>EXCLUDED.source_revision OR document_grouping.content_digest<>EXCLUDED.content_digest OR document_grouping.membership_revision<>EXCLUDED.membership_revision")
            .bind(scope.as_str()).bind(&identity.material_id).bind(&identity.entity_id).bind(mode).bind(source["source_revision"].as_str().ok_or(Error::Storage)?).bind(source["content_digest"].as_str().ok_or(Error::Storage)?).bind(revision).execute(&mut **tx).await.map_err(|_|Error::Storage)?;
        Ok(())
    }
    // No source body is acquired during discovery. Ineligible identities and
    // binding aliases cannot be sent to the judge or applied as fallback copies.
    pub async fn discover_document_grouping(&self) -> Result<(), Error> {
        for scope in [Scope::Personal, Scope::Meenseek] {
            let mut tx = self.pool().begin().await.map_err(|_| Error::Storage)?;
            self.lock_memories(&mut tx, scope).await?;
            self.context_gate_in(&mut tx, false).await?;
            let native=sqlx::query("SELECT m.material_id::text,m.scope,m.path,m.revision::text AS source_revision,m.content_digest FROM context_materials m JOIN context_projection_versions p USING(material_id,revision) WHERE NOT m.deleted AND NOT m.restricted AND m.path LIKE '%.md' AND p.payload->>'status' IN ('searchable','unavailable') AND p.payload->>'source_digest'=m.content_digest AND ($1='personal' OR EXISTS(SELECT 1 FROM context_source_bindings b JOIN sources s ON s.id=b.source_id WHERE b.material_id=m.material_id AND s.scope=$1)) ORDER BY m.material_id")
                .bind(scope.as_str()).fetch_all(&mut *tx).await.map_err(|_|Error::Storage)?;
            let mut sources = Vec::new();
            for row in native {
                let native_scope: ContextScope = row
                    .get::<String, _>("scope")
                    .parse()
                    .map_err(|_| Error::Storage)?;
                if eligible(&native_scope, &row.get::<String, _>("path"), false) {
                    sources.push((
                        DocumentIdentity {
                            material_id: Some(row.get("material_id")),
                            entity_id: None,
                        },
                        row.get::<String, _>("source_revision"),
                        row.get::<String, _>("content_digest"),
                    ));
                }
            }
            let git=sqlx::query("SELECT e.id,p.source_revision,p.content_digest FROM entities e JOIN source_records p ON p.scope=e.scope AND p.entity_id=e.id JOIN sources s ON s.scope=e.scope AND s.id=e.source_id WHERE e.scope=$1 AND s.kind='git' AND s.status='ok' AND p.present AND p.content_digest IS NOT NULL AND p.source_revision=s.verified_revision AND NOT EXISTS(SELECT 1 FROM context_source_bindings b WHERE b.source_id=s.id) ORDER BY e.id")
                .bind(scope.as_str()).fetch_all(&mut *tx).await.map_err(|_|Error::Storage)?;
            for row in git {
                sources.push((
                    DocumentIdentity {
                        material_id: None,
                        entity_id: Some(row.get("id")),
                    },
                    row.get("source_revision"),
                    row.get("content_digest"),
                ));
            }
            let mut keys = Vec::with_capacity(sources.len());
            let mut batch = Vec::with_capacity(sources.len());
            for (identity, revision, hash) in sources {
                let key = identity.key()?;
                keys.push(key.clone());
                batch.push(json!({"material_id":identity.material_id,"entity_id":identity.entity_id,"source_id":key,"source_revision":revision,"content_digest":hash}));
            }
            // One indexed upsert per scope, rather than N network round trips
            // while holding the shared manual/memory mutation lock.
            sqlx::query("INSERT INTO document_grouping(scope,material_id,entity_id,mode,state,source_revision,content_digest,membership_revision) SELECT $1,x.material_id::uuid,x.entity_id,'auto','pending',x.source_revision,x.content_digest,COALESCE(d.revision,0) FROM jsonb_to_recordset($2::jsonb) AS x(material_id text,entity_id text,source_id text,source_revision text,content_digest text) LEFT JOIN document_subjects d ON d.scope=$1 AND d.source_id=x.source_id ORDER BY x.source_id ON CONFLICT(scope,source_id) DO UPDATE SET source_revision=EXCLUDED.source_revision,content_digest=EXCLUDED.content_digest,membership_revision=EXCLUDED.membership_revision,state=CASE WHEN document_grouping.content_digest IS DISTINCT FROM EXCLUDED.content_digest OR document_grouping.membership_revision IS DISTINCT FROM EXCLUDED.membership_revision OR document_grouping.state IN ('pending','processing') THEN 'pending' WHEN document_grouping.state='ineligible' THEN CASE WHEN document_grouping.attempts>=3 THEN 'error' ELSE 'pending' END ELSE document_grouping.state END,attempts=CASE WHEN document_grouping.content_digest IS DISTINCT FROM EXCLUDED.content_digest OR document_grouping.membership_revision IS DISTINCT FROM EXCLUDED.membership_revision OR document_grouping.state IN ('pending','processing') THEN 0 ELSE document_grouping.attempts END,claim_token=NULL,lease_until=NULL,suggestions=CASE WHEN document_grouping.content_digest IS DISTINCT FROM EXCLUDED.content_digest THEN '{}'::jsonb ELSE document_grouping.suggestions END,reason=CASE WHEN document_grouping.content_digest IS DISTINCT FROM EXCLUDED.content_digest THEN NULL ELSE document_grouping.reason END,updated_at=now() WHERE document_grouping.mode='auto' AND (document_grouping.source_revision IS DISTINCT FROM EXCLUDED.source_revision OR document_grouping.content_digest IS DISTINCT FROM EXCLUDED.content_digest OR document_grouping.membership_revision IS DISTINCT FROM EXCLUDED.membership_revision OR document_grouping.state='ineligible')")
                .bind(scope.as_str()).bind(json!(batch)).execute(&mut *tx).await.map_err(|_|Error::Storage)?;
            // Eligibility is not a new judgment. Preserve terminal decisions and
            // attempt budgets across projection loss/restoration. Only an
            // unfinished claim needs cancellation and later continuation.
            sqlx::query("UPDATE document_grouping SET state='ineligible',claim_token=NULL,lease_until=NULL,updated_at=now() WHERE scope=$1 AND mode='auto' AND state IN ('pending','processing') AND NOT(source_id=ANY($2::text[]))")
                .bind(scope.as_str()).bind(keys).execute(&mut *tx).await.map_err(|_|Error::Storage)?;
            tx.commit().await.map_err(|_| Error::Storage)?;
        }
        Ok(())
    }

    async fn claim_document_grouping(&self) -> Result<Option<Claim>, Error> {
        sqlx::query("UPDATE document_grouping SET state='error',claim_token=NULL,lease_until=NULL,reason='분류 작업이 반복 중단됐습니다. 명시적으로 재검토할 수 있습니다.',updated_at=now() WHERE mode='auto' AND state='processing' AND lease_until<clock_timestamp() AND attempts>=3")
            .execute(self.pool()).await.map_err(|_|Error::Storage)?;
        let token = Uuid::new_v4().to_string();
        let row=sqlx::query("WITH next AS (SELECT scope,source_id FROM document_grouping WHERE mode='auto' AND attempts<3 AND (state='pending' OR (state='processing' AND lease_until<clock_timestamp())) ORDER BY updated_at,scope,source_id FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE document_grouping g SET state='processing',claim_token=$1::uuid,lease_until=clock_timestamp()+interval '2 minutes',attempts=attempts+1,updated_at=now() FROM next WHERE g.scope=next.scope AND g.source_id=next.source_id RETURNING g.scope,g.source_id,g.material_id::text,g.entity_id,g.source_revision,g.content_digest,g.membership_revision")
            .bind(&token).fetch_optional(self.pool()).await.map_err(|_|Error::Storage)?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(Claim {
            scope: row.get::<String, _>("scope").parse()?,
            identity: DocumentIdentity {
                material_id: row.get("material_id"),
                entity_id: row.get("entity_id"),
            },
            key: row.get("source_id"),
            source_revision: row.get("source_revision"),
            content_digest: row.get("content_digest"),
            membership_revision: row.get("membership_revision"),
            token,
        }))
    }

    async fn lock_document_claim(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        claim: &Claim,
    ) -> Result<bool, Error> {
        // SELECT first, then wall-clock CAS: a lock wait must not extend a lease.
        sqlx::query(
            "SELECT source_id FROM document_grouping WHERE scope=$1 AND source_id=$2 FOR UPDATE",
        )
        .bind(claim.scope.as_str())
        .bind(&claim.key)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| Error::Storage)?;
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM document_grouping WHERE scope=$1 AND source_id=$2 AND mode='auto' AND state='processing' AND source_revision=$3 AND content_digest=$4 AND membership_revision=$5 AND claim_token=$6::uuid AND lease_until>clock_timestamp())")
            .bind(claim.scope.as_str()).bind(&claim.key).bind(&claim.source_revision).bind(&claim.content_digest).bind(claim.membership_revision).bind(&claim.token).fetch_one(&mut **tx).await.map_err(|_|Error::Storage)
    }

    async fn document_definitions(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        scope: Scope,
    ) -> Result<Vec<Value>, Error> {
        sqlx::query_scalar("SELECT jsonb_build_object('id',id,'name',name,'revision',revision,'definition',definition) FROM subjects WHERE scope=$1 ORDER BY id")
            .bind(scope.as_str()).fetch_all(&mut **tx).await.map_err(|_|Error::Storage)
    }

    async fn document_membership_revision(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        claim: &Claim,
    ) -> Result<i64, Error> {
        Ok(sqlx::query_scalar(
            "SELECT revision FROM document_subjects WHERE scope=$1 AND source_id=$2",
        )
        .bind(claim.scope.as_str())
        .bind(&claim.key)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| Error::Storage)?
        .unwrap_or(0))
    }

    async fn document_input(&self, claim: &Claim) -> Result<Option<(Value, Vec<Value>)>, Error> {
        let mut tx = self.pool().begin().await.map_err(|_| Error::Storage)?;
        self.lock_memories(&mut tx, claim.scope).await?;
        let source = self
            .purpose_source(&mut tx, claim.scope, &claim.identity)
            .await?;
        if source["source_revision"] != claim.source_revision
            || source["content_digest"] != claim.content_digest
            || !self.lock_document_claim(&mut tx, claim).await?
            || self.document_membership_revision(&mut tx, claim).await? != claim.membership_revision
        {
            return Ok(None);
        }
        let (path, bytes) = if let Some(id) = &claim.identity.material_id {
            let row=sqlx::query("SELECT scope||'/'||path AS path,content FROM context_materials WHERE material_id=$1::uuid")
                .bind(id).fetch_one(&mut *tx).await.map_err(|_|Error::Storage)?;
            (
                row.get::<String, _>("path"),
                row.get::<Vec<u8>, _>("content"),
            )
        } else {
            let row=sqlx::query("SELECT s.path,p.content FROM entities e JOIN sources s ON s.id=e.source_id AND s.scope=e.scope JOIN source_records p ON p.scope=e.scope AND p.entity_id=e.id WHERE e.scope=$1 AND e.id=$2")
                .bind(claim.scope.as_str()).bind(&claim.identity.entity_id).fetch_one(&mut *tx).await.map_err(|_|Error::Storage)?;
            (
                row.get::<String, _>("path"),
                row.get::<String, _>("content").into_bytes(),
            )
        };
        if digest(&bytes) != claim.content_digest {
            return Err(Error::Conflict);
        }
        let definitions = self.document_definitions(&mut tx, claim.scope).await?;
        let input = json!({"record":model_record(&path,&bytes)?,"subjects":definitions});
        tx.commit().await.map_err(|_| Error::Storage)?;
        Ok(Some((input, definitions)))
    }

    async fn finish_document_claim(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        claim: &Claim,
        state: &str,
        suggestions: Value,
        reason: &str,
        membership_revision: i64,
    ) -> Result<bool, Error> {
        let changed=sqlx::query("UPDATE document_grouping SET state=$7,suggestions=$8,reason=$9,membership_revision=$10,policy_digest=$11,claim_token=NULL,lease_until=NULL,updated_at=now() WHERE scope=$1 AND source_id=$2 AND mode='auto' AND state='processing' AND source_revision=$3 AND content_digest=$4 AND membership_revision=$5 AND claim_token=$6::uuid AND lease_until>clock_timestamp()")
            .bind(claim.scope.as_str()).bind(&claim.key).bind(&claim.source_revision).bind(&claim.content_digest).bind(claim.membership_revision).bind(&claim.token).bind(state).bind(suggestions).bind(reason).bind(membership_revision).bind(digest(POLICY.as_bytes())).execute(&mut **tx).await.map_err(|_|Error::Storage)?;
        Ok(changed.rows_affected() == 1)
    }

    async fn apply_document_judgment(
        &self,
        claim: &Claim,
        judgment: Judgment,
        definitions: &[Value],
    ) -> Result<(), Error> {
        let mut tx = self.pool().begin().await.map_err(|_| Error::Storage)?;
        self.lock_memories(&mut tx, claim.scope).await?;
        let source = self
            .purpose_source(&mut tx, claim.scope, &claim.identity)
            .await?;
        if source["source_revision"] != claim.source_revision
            || source["content_digest"] != claim.content_digest
            || !self.lock_document_claim(&mut tx, claim).await?
            || self.document_membership_revision(&mut tx, claim).await? != claim.membership_revision
        {
            return Ok(());
        }
        let current = self.document_definitions(&mut tx, claim.scope).await?;
        if current != definitions {
            self.finish_document_claim(
                &mut tx,
                claim,
                "pending",
                json!({}),
                "목적 정의가 바뀌어 최신 기준으로 재검토합니다.",
                claim.membership_revision,
            )
            .await?;
            // Definition drift is not a provider failure or a consumed attempt.
            sqlx::query("UPDATE document_grouping SET attempts=0 WHERE scope=$1 AND source_id=$2 AND state='pending' AND claim_token IS NULL")
                .bind(claim.scope.as_str()).bind(&claim.key).execute(&mut *tx).await.map_err(|_|Error::Storage)?;
            return tx.commit().await.map_err(|_| Error::Storage);
        }
        let judgment = document_judgment(judgment, &current)?;
        let mut membership = claim.membership_revision;
        let state = match judgment.decision.as_str() {
            "assign" => "assigned",
            "suggest" => "suggested",
            _ => "unmatched",
        };
        if judgment.decision == "assign" {
            let subject = current
                .iter()
                .find(|s| s["id"].as_str() == judgment.subject_id.as_deref())
                .ok_or(Error::Invalid)?;
            let same:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM document_subjects WHERE scope=$1 AND source_id=$2 AND subject_id=$3 AND subject_revision=$4 AND source_revision=$5 AND content_digest=$6 AND reason=$7)")
                .bind(claim.scope.as_str()).bind(&claim.key).bind(&judgment.subject_id).bind(subject["revision"].as_i64().ok_or(Error::Storage)?).bind(&claim.source_revision).bind(&claim.content_digest).bind(&judgment.reason).fetch_one(&mut *tx).await.map_err(|_|Error::Storage)?;
            if !same {
                membership += 1;
                sqlx::query("INSERT INTO document_subjects(scope,material_id,entity_id,subject_id,subject_revision,revision,source_revision,content_digest,reason) VALUES($1,$2::uuid,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT(scope,source_id) DO UPDATE SET subject_id=EXCLUDED.subject_id,subject_revision=EXCLUDED.subject_revision,revision=EXCLUDED.revision,source_revision=EXCLUDED.source_revision,content_digest=EXCLUDED.content_digest,reason=EXCLUDED.reason,updated_at=now()")
                .bind(claim.scope.as_str()).bind(&claim.identity.material_id).bind(&claim.identity.entity_id).bind(&judgment.subject_id).bind(subject["revision"].as_i64().ok_or(Error::Storage)?).bind(membership).bind(&claim.source_revision).bind(&claim.content_digest).bind(&judgment.reason).execute(&mut *tx).await.map_err(|_|Error::Storage)?;
                self.append_document_subject_history(&mut tx, claim.scope, &claim.key)
                    .await?;
            }
        }
        let names: serde_json::Map<String, Value> = judgment
            .candidate_ids
            .iter()
            .filter_map(|id| {
                current
                    .iter()
                    .find(|s| s["id"] == *id)
                    .map(|s| (id.clone(), s["name"].clone()))
            })
            .collect();
        let suggestions = json!({"candidate_ids":judgment.candidate_ids,"candidate_names":names});
        if !self
            .finish_document_claim(
                &mut tx,
                claim,
                state,
                suggestions,
                &judgment.reason,
                membership,
            )
            .await?
        {
            return tx.rollback().await.map_err(|_| Error::Storage);
        }
        tx.commit().await.map_err(|_| Error::Storage)
    }

    pub async fn document_grouping_once(&self) -> Result<bool, Error> {
        let Some(claim) = self.claim_document_grouping().await? else {
            return Ok(false);
        };
        let mut attempted_definitions = None;
        let result = async {
            let Some((input, definitions)) = self.document_input(&claim).await? else {
                return Ok(());
            };
            attempted_definitions = Some(definitions.clone());
            if input.to_string().len() > 65_536 {
                return Err(Error::Limit);
            }
            let judgment = if definitions.iter().all(|s| s["definition"].is_null()) {
                Judgment {
                    decision: "unmatched".into(),
                    subject_id: None,
                    candidate_ids: vec![],
                    new_subject: None,
                    reason: "정의된 목적 그룹이 없어 자동 배정하지 않았습니다.".into(),
                }
            } else {
                run_judgment(&input, POLICY, "문서").await?
            };
            let judgment = document_judgment(judgment, &definitions)?;
            self.apply_document_judgment(&claim, judgment, &definitions)
                .await
        }
        .await;
        if let Err(error) = result {
            let mut tx = self.pool().begin().await.map_err(|_| Error::Storage)?;
            self.lock_memories(&mut tx, claim.scope).await?;
            // A failed judge is also an old result. Check current source and
            // definitions before turning a live claim into a terminal failure.
            let current = self
                .purpose_source(&mut tx, claim.scope, &claim.identity)
                .await;
            let pending = matches!(&error, Error::ContextPending)
                || matches!(&current, Err(Error::ContextPending));
            let source = match current {
                Ok(source) => Some(source),
                Err(Error::ContextPending | Error::NotFound) => None,
                Err(error) => return Err(error),
            };
            if self.lock_document_claim(&mut tx, &claim).await? {
                let definitions_changed = if let Some(snapshot) = &attempted_definitions {
                    snapshot != &self.document_definitions(&mut tx, claim.scope).await?
                } else {
                    false
                };
                let drift = source.as_ref().is_some_and(|s| {
                    s["source_revision"] != claim.source_revision
                        || s["content_digest"] != claim.content_digest
                }) || definitions_changed;
                let (state, reason) = if pending {
                    ("pending", "원문 적용·복구 완료를 기다립니다.")
                } else if source.is_none() {
                    (
                        "ineligible",
                        "현재 원문이 문서 분류 범위에서 제외되었습니다.",
                    )
                } else if drift {
                    (
                        "pending",
                        "원문 또는 목적 정의가 바뀌어 최신 기준으로 재검토합니다.",
                    )
                } else if matches!(error, Error::Limit) {
                    (
                        "error",
                        "본문과 목적 정의 입력이 분류 한도를 넘어 수동 검토가 필요합니다. 내용을 자르지 않았습니다.",
                    )
                } else {
                    (
                        "error",
                        "문서 분류를 완료하지 못했습니다. 현재 원문을 확인하고 재검토할 수 있습니다.",
                    )
                };
                let changed = self
                    .finish_document_claim(
                        &mut tx,
                        &claim,
                        state,
                        json!({}),
                        reason,
                        claim.membership_revision,
                    )
                    .await?;
                if changed && state == "pending" {
                    let source_revision = source
                        .as_ref()
                        .and_then(|s| s["source_revision"].as_str())
                        .unwrap_or(&claim.source_revision);
                    let hash = source
                        .as_ref()
                        .and_then(|s| s["content_digest"].as_str())
                        .unwrap_or(&claim.content_digest);
                    sqlx::query("UPDATE document_grouping SET source_revision=$3,content_digest=$4,attempts=CASE WHEN $5 THEN greatest(attempts-1,0) ELSE 0 END WHERE scope=$1 AND source_id=$2")
                        .bind(claim.scope.as_str()).bind(&claim.key).bind(source_revision).bind(hash).bind(pending && !drift).execute(&mut *tx).await.map_err(|_|Error::Storage)?;
                }
            }
            tx.commit().await.map_err(|_| Error::Storage)?;
            if pending {
                return Err(Error::ContextPending);
            }
        }
        Ok(true)
    }

    pub(crate) async fn document_grouping_retry(
        &self,
        scope: Scope,
        input: DocumentGroupingRetry,
    ) -> Result<Value, Error> {
        expected_revision(input.revision)?;
        let key = input.identity.key()?;
        let mut tx = self.pool().begin().await.map_err(|_| Error::Storage)?;
        self.lock_memories(&mut tx, scope).await?;
        let source = self.purpose_source(&mut tx, scope, &input.identity).await?;
        if source["source_revision"] != input.source_revision
            || source["content_digest"] != input.content_digest
        {
            return Err(Error::Conflict);
        }
        let revision = sqlx::query_scalar::<_, i64>(
            "SELECT revision FROM document_subjects WHERE scope=$1 AND source_id=$2",
        )
        .bind(scope.as_str())
        .bind(&key)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| Error::Storage)?
        .unwrap_or(0);
        if revision != input.revision {
            return Err(Error::Conflict);
        }
        sqlx::query("INSERT INTO document_grouping(scope,material_id,entity_id,mode,state,source_revision,content_digest,membership_revision) VALUES($1,$2::uuid,$3,'auto','pending',$4,$5,$6) ON CONFLICT(scope,source_id) DO UPDATE SET mode='auto',state='pending',source_revision=$4,content_digest=$5,membership_revision=$6,attempts=0,claim_token=NULL,lease_until=NULL,suggestions='{}'::jsonb,reason=NULL,policy_digest=NULL,updated_at=now() WHERE document_grouping.mode<>'auto' OR document_grouping.state NOT IN ('pending','processing') OR document_grouping.source_revision IS DISTINCT FROM $4 OR document_grouping.content_digest IS DISTINCT FROM $5 OR document_grouping.membership_revision IS DISTINCT FROM $6")
            .bind(scope.as_str()).bind(&input.identity.material_id).bind(&input.identity.entity_id).bind(&input.source_revision).bind(&input.content_digest).bind(revision).execute(&mut *tx).await.map_err(|_|Error::Storage)?;
        tx.commit().await.map_err(|_| Error::Storage)?;
        self.document_subject(scope, input.identity).await
    }
}

pub async fn run_discovery_loop(store: Store) {
    loop {
        let _ = store.discover_document_grouping().await;
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

pub async fn run_loop(store: Store) {
    loop {
        match store.document_grouping_once().await {
            Ok(true) => {}
            Ok(false) | Err(_) => tokio::time::sleep(Duration::from_secs(3)).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn judge_input_redacts_both_parser_and_unavailable_fallback_without_truncation() {
        let body = "자료 내용 ".repeat(1500);
        let raw = format!("---\ntitle: 합성 문서\n---\nAPI_KEY=synthetic-secret\n{body}");
        let record = model_record("personal/notes/a.md", raw.as_bytes()).unwrap();
        assert_eq!(
            record["body"]
                .as_str()
                .unwrap()
                .matches("자료 내용")
                .count(),
            1500
        );
        assert!(!record.to_string().contains("synthetic-secret"));
        let bad = b"---\n[\n---\nAPI_KEY=synthetic-fallback\n";
        let record = model_record("bad.md", bad).unwrap();
        assert!(!record.to_string().contains("synthetic-fallback"));
        assert!(record["body"].as_str().unwrap().contains("[REDACTED]"));
    }
    #[test]
    fn real_but_undefined_id_cannot_receive_a_document() {
        let definition = json!({"id":"p_synthetic","definition":null});
        let judgment = Judgment {
            decision: "assign".into(),
            subject_id: Some("p_synthetic".into()),
            candidate_ids: vec![],
            new_subject: None,
            reason: "합성 목적 판정".into(),
        };
        assert!(matches!(
            document_judgment(judgment, &[definition]),
            Err(Error::Invalid)
        ));
    }
}
