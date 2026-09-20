//! Exact consumers of canonical Context material; no filesystem or subprocess transport.
use crate::{
    context::ContextScope,
    context_projection,
    domain::{Error, ImportedRecord, MAX_RESPONSE_BYTES, Scope, SourceKind},
    store::{Store, digest},
};
use serde_json::{Value, json};
use sqlx::{PgConnection, Row};
use std::collections::BTreeMap;

pub fn validate_store_id(value: &str) -> Result<(), Error> {
    let id = uuid::Uuid::parse_str(value).map_err(|_| Error::Invalid)?;
    if id.to_string() != value {
        return Err(Error::Invalid);
    }
    Ok(())
}
pub fn validate_paths(
    scope: &ContextScope,
    paths: &[String],
    required: bool,
) -> Result<Vec<String>, Error> {
    if required && paths.is_empty() {
        return Err(Error::Invalid);
    }
    if paths.iter().any(|p| {
        scope
            .as_str()
            .len()
            .checked_add(p.len())
            .and_then(|n| n.checked_add(1))
            .is_none_or(|n| n > 512)
    }) {
        return Err(Error::Limit);
    }
    let normalized = context_projection::normalize_paths(scope, paths)?;
    if normalized.len() != paths.len() || normalized.iter().any(|p| !paths.contains(p)) {
        return Err(Error::Invalid);
    }
    Ok(normalized)
}
pub fn identity(
    scope: Scope,
    kind: SourceKind,
    repository: &str,
    source: &str,
) -> (String, String) {
    let hash = digest(
        format!(
            "{}\0{}\0{repository}\0{source}",
            scope.as_str(),
            kind.as_str()
        )
        .as_bytes(),
    );
    (format!("s_{hash}"), format!("e_{hash}"))
}
pub fn revision_token(
    store: &str,
    material: &str,
    revision: i64,
    deleted: bool,
    sha: &str,
    origin: &str,
    source_digest: Option<&str>,
) -> Result<String, Error> {
    validate_store_id(store)?;
    validate_store_id(material)?;
    if revision < 1 || !lower_sha(sha) {
        return Err(Error::Invalid);
    }
    if origin == "imported-file" && revision == 1 && !deleted {
        if source_digest != Some(sha) {
            return Err(Error::Invalid);
        }
        return Ok(sha.to_owned());
    }
    if !matches!(origin, "imported-file" | "native") {
        return Err(Error::Invalid);
    }
    Ok(digest(
        format!(
            "context-source-v1:{store}:{material}:{revision}:{}:{sha}",
            if deleted { "deleted" } else { "live" }
        )
        .as_bytes(),
    ))
}
fn lower_sha(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
#[derive(Default)]
pub struct ContextReader {
    resolved: Vec<(String, SourceKind)>,
    bindings: Vec<Value>,
    pub body_calls: u64,
    pub response_bytes: u64,
}
impl ContextReader {
    pub fn new() -> Self {
        Self::default()
    }
    async fn read_in(
        &mut self,
        store: &Store,
        conn: &mut PgConnection,
        expected_store: &str,
        context_scope: &ContextScope,
        paths: &[String],
        scope: Scope,
    ) -> Result<Vec<ImportedRecord>, Error> {
        let observation = store.dependency_start(
            "context-read",
            &(expected_store, context_scope, paths, scope),
        )?;
        let result=async {
        self.resolved.clear();
        self.bindings.clear();
        self.body_calls = 0;
        self.response_bytes = 0;
        validate_store_id(expected_store)?;
        let paths = validate_paths(context_scope, paths, false)?;
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let observation=store.dependency_start("consumer-store-identity", &Vec::<String>::new())?;
        store.count(1);
        let result:Result<String,Error>=sqlx::query_scalar("SELECT store_id::text FROM context_store").fetch_one(&mut *conn).await.map_err(|_|Error::Storage);
        store.dependency_finish(observation,&result);let actual=result?;
        if actual != expected_store {
            return Err(Error::Conflict);
        }
        let a_observation=store.dependency_start("A-exact-metadata", &(context_scope,&paths))?;
            let metadata_result=context_projection::exact_metadata(store,conn,context_scope,&paths).await;
            let observed=metadata_result.as_ref().map(|rows|rows.iter().map(|m|json!({"path":m.path,"material_id":m.material_id,"revision":m.revision,"content_digest":m.sha,"byte_len":m.byte_len,"restricted":m.restricted,"deleted":m.deleted,"origin_kind":m.origin,"source_path":m.source_path,"source_digest":m.source_digest})).collect::<Vec<_>>()).map_err(|e|*e);
            store.dependency_finish(a_observation,&observed);
            let metadata=metadata_result?;
        let mut roots = BTreeMap::new();
        if metadata.iter().any(|m| m.origin == "imported-file") {
            let observation=store.dependency_start("consumer-imported-roots", &(context_scope.as_str(),&paths))?;
            store.count(1);
            let result=sqlx::query("SELECT material_id::text,source_root FROM context_materials WHERE scope=$1 AND path=ANY($2) AND origin_kind='imported-file'").bind(context_scope.as_str()).bind(&paths).fetch_all(&mut *conn).await.map_err(|_|Error::Storage);
            store.dependency_finish_rows(observation,&result);let rows=result?;
            for row in rows {
                roots.insert(
                    row.get::<String, _>("material_id"),
                    row.get::<String, _>("source_root"),
                );
            }
        }
        let mut records = Vec::with_capacity(metadata.len());
        let mut acquired = 0usize;
        for m in &metadata {
            validate_store_id(&m.material_id)?;
            if m.source_path != format!("{}/{}", context_scope.as_str(), m.path) || m.restricted {
                return Err(Error::Invalid);
            }
            let (kind, repository, key) = match m.origin.as_str() {
                "imported-file" => {
                    let root = roots.get(&m.material_id).ok_or(Error::Storage)?;
                    (SourceKind::Vault, root.clone(), m.source_path.clone())
                }
                "native" => (
                    SourceKind::Context,
                    format!("ontology-context:{actual}"),
                    m.material_id.clone(),
                ),
                _ => return Err(Error::Invalid),
            };
            let identity_repository = if kind == SourceKind::Context {
                actual.as_str()
            } else {
                repository.as_str()
            };
            let (source_id, entity_id) = identity(scope, kind, identity_repository, &key);
            self.resolved.push((source_id.clone(), kind));
            self.bindings
                .push(json!({"source_id":source_id,"material_id":m.material_id}));
            let source_revision = revision_token(
                &actual,
                &m.material_id,
                m.revision,
                m.deleted,
                &m.sha,
                &m.origin,
                m.source_digest.as_deref(),
            )?;
            if m.byte_len < 0 || m.byte_len > 65536 {
                return Err(Error::Limit);
            }
            if !m.deleted {
                acquired = acquired
                    .checked_add(m.byte_len as usize)
                    .ok_or(Error::Limit)?;
            }
            if acquired > MAX_RESPONSE_BYTES {
                return Err(Error::Limit);
            }
            records.push(ImportedRecord {
                source_id,
                entity_id,
                scope,
                repository,
                path: m.source_path.clone(),
                kind,
                source_revision,
                digest: None,
                content: None,
            });
        }
        if metadata.len() != paths.len() {
            return Err(Error::NotFound);
        }
        let live: Vec<_> = metadata
            .iter()
            .filter(|m| !m.deleted)
            .map(|m| m.path.clone())
            .collect();
        let value = if live.is_empty() {
            json!({"documents":[]})
        } else {
            // A owns the parser, redaction and exact live-body query.
            { let a_observation=store.dependency_start("A-exact-documents", &(context_scope,&live))?;
                let before_calls=store.calls();
                let result=context_projection::exact_documents(store,conn,context_scope,&live).await;
                self.body_calls=u64::from(store.calls().saturating_sub(before_calls)>=3);
                store.dependency_finish(a_observation,&result);result? }
        };
        self.response_bytes = serde_json::to_vec(&value)
            .map_err(|_| Error::Storage)?
            .len() as u64;
        if self.response_bytes > MAX_RESPONSE_BYTES as u64 {
            return Err(Error::Limit);
        }
        let documents = value["documents"].as_array().ok_or(Error::Invalid)?;
        if documents.len() != live.len() {
            return Err(Error::Invalid);
        }
        for (m, record) in metadata.iter().zip(&mut records) {
            if m.deleted {
                continue;
            }
            let d = documents
                .iter()
                .find(|d| d["path"] == m.source_path)
                .ok_or(Error::Invalid)?;
            if d["scope"] != context_scope.as_str()
                || d["store_id"] != actual
                || d["material_id"] != m.material_id
                || d["revision"] != m.revision
                || d["source_digest"] != m.sha
                || d["origin_kind"] != m.origin
                || d["source_path"] != m.source_path
            {
                return Err(Error::Invalid);
            }
            let title = d["title"].as_str().ok_or(Error::Invalid)?;
            let body = d["body"].as_str().ok_or(Error::Invalid)?;
            if title.contains('\0') || body.contains('\0') {
                return Err(Error::Invalid);
            }
            let content = format!("# {title}\n\n{body}");
            if content.len() > 65536 {
                return Err(Error::Limit);
            }
            record.digest = Some(digest(content.as_bytes()));
            record.content = Some(content);
        }
        if serde_json::to_vec(&records)
            .map_err(|_| Error::Invalid)?
            .len()
            > MAX_RESPONSE_BYTES
        {
            return Err(Error::Limit);
        }
        Ok(records)

        }.await;
        store.dependency_finish(observation, &result);
        result
    }
    pub async fn import(
        &mut self,
        store: &Store,
        expected_store: &str,
        context_scope: &ContextScope,
        paths: &[String],
        scope: Scope,
    ) -> Result<usize, Error> {
        self.resolved.clear();
        self.bindings.clear();
        self.body_calls = 0;
        self.response_bytes = 0;
        let observation = store.dependency_start(
            "context-import",
            &(expected_store, context_scope, paths, scope),
        )?;
        let result = async {
            validate_store_id(expected_store)?;
            validate_paths(context_scope, paths, false)?;
            if paths.is_empty() {
                return Ok(0);
            }
            let mut tx = store.lock_import().await?;
            if let Err(error) = store.context_gate_in(&mut tx, false).await {
                return store.finish_context(tx, Err(error)).await;
            }
            let result = self
                .refresh_in(store, &mut tx, expected_store, context_scope, paths, scope)
                .await;
            store.finish_import(tx).await?;
            result
        }
        .await;
        store.dependency_finish(observation, &result);
        result
    }
    pub(crate) async fn refresh_in(
        &mut self,
        store: &Store,
        conn: &mut PgConnection,
        expected_store: &str,
        context_scope: &ContextScope,
        paths: &[String],
        scope: Scope,
    ) -> Result<usize, Error> {
        store
            .consumer_control(&mut *conn, "SAVEPOINT context_consumer_source")
            .await?;
        let result=async {
            let records=self.read_in(store,conn,expected_store,context_scope,paths,scope).await?;
            store.apply_import_in(conn,&records).await?;
            if !self.bindings.is_empty(){
                let observation=store.dependency_start("consumer-bindings", &json!(self.bindings))?;
                store.count(1);
                let result=sqlx::query("INSERT INTO context_source_bindings(source_id,material_id) SELECT source_id,material_id::uuid FROM jsonb_to_recordset($1) AS x(source_id text,material_id text) ON CONFLICT(source_id) DO UPDATE SET material_id=EXCLUDED.material_id WHERE context_source_bindings.material_id=EXCLUDED.material_id RETURNING source_id")
                    .bind(json!(self.bindings)).fetch_all(&mut *conn).await.map_err(|_|Error::Storage);
                store.dependency_finish_rows(observation,&result);let rows=result?;
                if rows.len()!=records.len(){return Err(Error::Conflict);}
            }
            Ok(records.len())
        }.await;
        if let Err(error) = &result {
            store
                .consumer_control(&mut *conn, "ROLLBACK TO SAVEPOINT context_consumer_source")
                .await?;
            if !matches!(error, Error::Conflict | Error::ContextPending) {
                for kind in [SourceKind::Vault, SourceKind::Context] {
                    let ids: Vec<_> = self
                        .resolved
                        .iter()
                        .filter(|(_, k)| *k == kind)
                        .map(|(id, _)| id.clone())
                        .collect();
                    store.mark_failed_in(conn, &ids, kind).await?;
                }
            }
        }
        store
            .consumer_control(&mut *conn, "RELEASE SAVEPOINT context_consumer_source")
            .await?;
        result
    }
}
