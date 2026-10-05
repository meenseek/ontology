//! Purpose definitions and document membership belong to existing app subjects.
//! Canonical native identity is a material UUID; no source body is written here.
use crate::{
    context::ContextScope,
    context_projection::eligible,
    domain::{Error, Scope, validate_id},
    memory::{native_id, revision_valid, text_valid},
    store::Store,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubjectDefinition {
    pub purpose: String,
    pub include: String,
    pub exclude: String,
}
impl SubjectDefinition {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if !text_valid(&self.purpose, 1024, true)
            || !text_valid(&self.include, 2048, true)
            || !text_valid(&self.exclude, 2048, true)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentIdentity {
    pub material_id: Option<String>,
    pub entity_id: Option<String>,
}
impl DocumentIdentity {
    fn key(&self) -> Result<String, Error> {
        match (&self.material_id, &self.entity_id) {
            (Some(id), None) => Uuid::parse_str(id)
                .map(|id| id.to_string())
                .map_err(|_| Error::Invalid),
            (None, Some(id)) => {
                validate_id(id)?;
                Ok(id.clone())
            }
            _ => Err(Error::Invalid),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentSubjectInput {
    pub identity: DocumentIdentity,
    pub revision: i64,
    pub source_revision: String,
    pub content_digest: String,
    pub subject_id: Option<String>,
    pub subject_revision: Option<i64>,
    pub reason: String,
}
pub(crate) fn expected_revision(revision: i64) -> Result<(), Error> {
    if revision == 0 {
        Ok(())
    } else {
        revision_valid(revision)
    }
}

impl Store {
    pub(crate) async fn subject_define(
        &self,
        scope: Scope,
        id: &str,
        revision: i64,
        name: &str,
        definition: SubjectDefinition,
    ) -> Result<Value, Error> {
        native_id(id, "p_")?;
        expected_revision(revision)?;
        definition.validate()?;
        if !text_valid(name, 320, false) || name.trim() != name {
            return Err(Error::Invalid);
        }
        let definition = serde_json::to_value(definition).map_err(|_| Error::Invalid)?;
        let mut tx = self.pool().begin().await.map_err(|_| Error::Storage)?;
        self.lock_memories(&mut tx, scope).await?;
        self.count(1);
        let old = sqlx::query(
            "SELECT revision,name,definition FROM subjects WHERE scope=$1 AND id=$2 FOR UPDATE",
        )
        .bind(scope.as_str())
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| Error::Storage)?
        .ok_or(Error::NotFound)?;
        if old.get::<i64, _>("revision") != revision {
            return Err(Error::Conflict);
        }
        if old.get::<String, _>("name") != name
            || old.get::<Option<Value>, _>("definition").as_ref() != Some(&definition)
        {
            self.count(1);
            sqlx::query("UPDATE subjects SET name=$3,definition=$4,revision=revision+1 WHERE scope=$1 AND id=$2")
                .bind(scope.as_str()).bind(id).bind(name).bind(&definition).execute(&mut *tx).await.map_err(|_|Error::Storage)?;
            self.count(1);
            sqlx::query("INSERT INTO subject_history(scope,subject_id,revision,name,definition,action) SELECT scope,id,revision,name,definition,'define' FROM subjects WHERE scope=$1 AND id=$2")
                .bind(scope.as_str()).bind(id).execute(&mut *tx).await.map_err(|_|Error::Storage)?;
        }
        self.count(1);
        let value: Value = sqlx::query_scalar("SELECT to_jsonb(s)-'key_digest'-'payload_digest' FROM subjects s WHERE scope=$1 AND id=$2")
            .bind(scope.as_str()).bind(id).fetch_one(&mut *tx).await.map_err(|_|Error::Storage)?;
        tx.commit().await.map_err(|_| Error::Storage)?;
        Ok(value)
    }

    // Importers hold the source row before changing Git content. Native writes
    // lock the material; classification takes that same source's shared lock.
    async fn purpose_source(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        scope: Scope,
        identity: &DocumentIdentity,
    ) -> Result<Value, Error> {
        identity.key()?;
        self.count(1);
        if let Some(material) = &identity.material_id {
            let row = sqlx::query("SELECT m.material_id::text,m.scope,m.path,m.revision::text AS source_revision,m.content_digest,m.restricted FROM context_materials m WHERE m.material_id=$1::uuid AND NOT m.deleted AND NOT m.restricted AND m.path LIKE '%.md' AND ($2='personal' OR EXISTS(SELECT 1 FROM context_source_bindings b JOIN sources s ON s.id=b.source_id WHERE b.material_id=m.material_id AND s.scope=$2)) FOR SHARE OF m")
                .bind(material).bind(scope.as_str()).fetch_optional(&mut **tx).await.map_err(|_|Error::Storage)?.ok_or(Error::NotFound)?;
            let native_scope: ContextScope = row
                .get::<String, _>("scope")
                .parse()
                .map_err(|_| Error::Storage)?;
            if !eligible(
                &native_scope,
                &row.get::<String, _>("path"),
                row.get("restricted"),
            ) {
                return Err(Error::NotFound);
            }
            return Ok(
                json!({"material_id":row.get::<String,_>("material_id"),"entity_id":null,
                "source_revision":row.get::<String,_>("source_revision"),"content_digest":row.get::<String,_>("content_digest")}),
            );
        }
        let entity = identity.entity_id.as_deref().ok_or(Error::Invalid)?;
        let source: Option<String> = sqlx::query_scalar("SELECT s.id FROM entities e JOIN sources s ON s.id=e.source_id AND s.scope=e.scope WHERE e.scope=$1 AND e.id=$2 AND s.kind='git' FOR SHARE OF s")
            .bind(scope.as_str()).bind(entity).fetch_optional(&mut **tx).await.map_err(|_|Error::Storage)?;
        if source.is_none() {
            return Err(Error::NotFound);
        }
        self.count(1);
        sqlx::query_scalar("SELECT jsonb_build_object('material_id',NULL,'entity_id',e.id,'source_revision',p.source_revision,'content_digest',p.content_digest) FROM entities e JOIN source_records p ON p.entity_id=e.id AND p.scope=e.scope JOIN sources s ON s.id=e.source_id WHERE e.scope=$1 AND e.id=$2 AND s.id=$3 AND s.status='ok' AND p.present AND p.content_digest IS NOT NULL AND p.source_revision=s.verified_revision AND NOT EXISTS(SELECT 1 FROM context_source_bindings b WHERE b.source_id=s.id) FOR SHARE OF e,p")
            .bind(scope.as_str()).bind(entity).bind(source).fetch_optional(&mut **tx).await.map_err(|_|Error::Storage)?.ok_or(Error::NotFound)
    }

    async fn purpose_membership_value(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        scope: Scope,
        key: &str,
        source: Value,
    ) -> Result<Value, Error> {
        self.count(1);
        let stored: Option<Value> = sqlx::query_scalar("SELECT to_jsonb(d)||jsonb_build_object('subject_name',s.name,'definition',s.definition,'current_subject_revision',s.revision,'review_needed',d.subject_id IS NOT NULL AND d.subject_revision IS DISTINCT FROM s.revision) FROM document_subjects d LEFT JOIN subjects s ON s.scope=d.scope AND s.id=d.subject_id WHERE d.scope=$1 AND d.source_id=$2")
            .bind(scope.as_str()).bind(key).fetch_optional(&mut **tx).await.map_err(|_|Error::Storage)?;
        let mut value = stored.unwrap_or_else(||json!({"revision":0,"subject_id":null,"subject_revision":null,"reason":null,"review_needed":false}));
        if value["revision"] != 0
            && (value["source_revision"] != source["source_revision"]
                || value["content_digest"] != source["content_digest"])
        {
            value["review_needed"] = json!(true);
        }
        value["current_source"] = source;
        Ok(value)
    }

    pub(crate) async fn document_subject(
        &self,
        scope: Scope,
        identity: DocumentIdentity,
    ) -> Result<Value, Error> {
        let key = identity.key()?;
        let mut tx = self.pool().begin().await.map_err(|_| Error::Storage)?;
        self.lock_memories(&mut tx, scope).await?;
        let source = self.purpose_source(&mut tx, scope, &identity).await?;
        let value = self
            .purpose_membership_value(&mut tx, scope, &key, source)
            .await?;
        tx.commit().await.map_err(|_| Error::Storage)?;
        Ok(value)
    }

    pub(crate) async fn document_subject_set(
        &self,
        scope: Scope,
        input: DocumentSubjectInput,
    ) -> Result<Value, Error> {
        let key = input.identity.key()?;
        expected_revision(input.revision)?;
        if !text_valid(&input.reason, 2048, true)
            || (input.subject_id.is_some() != input.subject_revision.is_some())
        {
            return Err(Error::Invalid);
        }
        if let Some(id) = &input.subject_id {
            native_id(id, "p_")?;
            expected_revision(input.subject_revision.ok_or(Error::Invalid)?)?;
        }
        let mut tx = self.pool().begin().await.map_err(|_| Error::Storage)?;
        self.lock_memories(&mut tx, scope).await?;
        let source = self.purpose_source(&mut tx, scope, &input.identity).await?;
        if source["source_revision"] != input.source_revision
            || source["content_digest"] != input.content_digest
        {
            return Err(Error::Conflict);
        }
        if let Some(subject) = &input.subject_id {
            self.count(1);
            let revision: Option<i64> = sqlx::query_scalar(
                "SELECT revision FROM subjects WHERE scope=$1 AND id=$2 FOR SHARE",
            )
            .bind(scope.as_str())
            .bind(subject)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| Error::Storage)?;
            if revision != input.subject_revision {
                return Err(Error::Conflict);
            }
        }
        self.count(1);
        let old: Option<Value> = sqlx::query_scalar("SELECT to_jsonb(d) FROM document_subjects d WHERE scope=$1 AND source_id=$2 FOR UPDATE")
            .bind(scope.as_str()).bind(&key).fetch_optional(&mut *tx).await.map_err(|_|Error::Storage)?;
        if old
            .as_ref()
            .and_then(|v| v["revision"].as_i64())
            .unwrap_or(0)
            != input.revision
        {
            return Err(Error::Conflict);
        }
        let same = old.as_ref().is_some_and(|v| {
            v["subject_id"] == json!(input.subject_id)
                && v["subject_revision"] == json!(input.subject_revision)
                && v["source_revision"] == input.source_revision
                && v["content_digest"] == input.content_digest
                && v["reason"] == input.reason
        });
        if !same {
            self.count(1);
            sqlx::query("INSERT INTO document_subjects(scope,material_id,entity_id,subject_id,subject_revision,revision,source_revision,content_digest,reason) VALUES($1,$2::uuid,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT(scope,source_id) DO UPDATE SET subject_id=EXCLUDED.subject_id,subject_revision=EXCLUDED.subject_revision,revision=EXCLUDED.revision,source_revision=EXCLUDED.source_revision,content_digest=EXCLUDED.content_digest,reason=EXCLUDED.reason,updated_at=now()")
                .bind(scope.as_str()).bind(input.identity.material_id).bind(&input.identity.entity_id).bind(&input.subject_id).bind(input.subject_revision).bind(input.revision+1).bind(&input.source_revision).bind(&input.content_digest).bind(&input.reason).execute(&mut *tx).await.map_err(|_|Error::Storage)?;
            self.append_document_subject_history(&mut tx, scope, &key)
                .await?;
        }
        let value = self
            .purpose_membership_value(&mut tx, scope, &key, source)
            .await?;
        tx.commit().await.map_err(|_| Error::Storage)?;
        Ok(value)
    }

    pub(crate) async fn append_document_subject_history(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        scope: Scope,
        key: &str,
    ) -> Result<(), Error> {
        self.count(1);
        sqlx::query("INSERT INTO document_subject_history(scope,source_id,material_id,entity_id,revision,subject_id,subject_revision,source_revision,content_digest,reason) SELECT scope,source_id,material_id,entity_id,revision,subject_id,subject_revision,source_revision,content_digest,reason FROM document_subjects WHERE scope=$1 AND source_id=$2")
            .bind(scope.as_str()).bind(key).execute(&mut **tx).await.map_err(|_|Error::Storage)?;
        Ok(())
    }

    pub(crate) async fn document_subject_history(
        &self,
        scope: Scope,
        identity: DocumentIdentity,
        before: Option<i64>,
        limit: usize,
    ) -> Result<Value, Error> {
        let key = identity.key()?;
        if !(1..=20).contains(&limit) {
            return Err(Error::Invalid);
        }
        if let Some(revision) = before {
            revision_valid(revision)?;
        }
        let mut tx = self.pool().begin().await.map_err(|_| Error::Storage)?;
        self.lock_memories(&mut tx, scope).await?;
        self.purpose_source(&mut tx, scope, &identity).await?;
        self.count(1);
        let mut rows: Vec<Value> = sqlx::query_scalar("SELECT to_jsonb(h)||jsonb_build_object('subject_name',s.name,'definition',s.definition) FROM document_subject_history h LEFT JOIN subject_history s ON s.scope=h.scope AND s.subject_id=h.subject_id AND s.revision=h.subject_revision WHERE h.scope=$1 AND h.source_id=$2 AND ($3::bigint IS NULL OR h.revision<$3) ORDER BY h.revision DESC LIMIT $4")
            .bind(scope.as_str()).bind(key).bind(before).bind((limit+1) as i64).fetch_all(&mut *tx).await.map_err(|_|Error::Storage)?;
        let more = rows.len() > limit;
        rows.truncate(limit);
        let next = if more {
            rows.last().map(|v| v["revision"].clone())
        } else {
            None
        };
        tx.commit().await.map_err(|_| Error::Storage)?;
        Ok(json!({"items":rows,"next_before_revision":next}))
    }
}
