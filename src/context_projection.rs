//! Immutable search projections and bounded, exact document reads from the canonical store.
use crate::{
    context::{ContextScope, MAX_FILES, MAX_TOTAL_BYTES, restricted, validate_path},
    domain::{Error, MAX_DOCUMENT_BYTES, MAX_RESPONSE_BYTES},
    store::{Store, digest},
};
use context_core::{
    Scope,
    document::parse_markdown_bytes,
    redaction::redact_secrets,
    search_text::{expanded_query_terms, expanded_search_text},
    vault::normalize_exact_read_paths,
};
use pulldown_cmark::{Event, Parser, Tag};
use serde_json::{Value, json};
use sqlx::{PgConnection, Row, postgres::PgRow};
use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

fn linked_markdown_paths(
    scope: &ContextScope,
    path: &str,
    body: &str,
    prefix: &str,
) -> BTreeSet<String> {
    let mut targets = BTreeSet::new();
    let parent = Path::new(path).parent().unwrap_or_else(|| Path::new(""));
    for event in Parser::new(body) {
        let Event::Start(Tag::Link { dest_url, .. }) = event else {
            continue;
        };
        let href = dest_url.as_ref();
        if href.is_empty()
            || href.starts_with(['/', '\\', '#'])
            || href.contains(['?', '#', '\\', '\0', ':'])
        {
            continue;
        }
        let mut parts = Vec::new();
        let mut valid = true;
        for component in parent.join(href).components() {
            match component {
                Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
                Component::CurDir => {}
                Component::ParentDir => {
                    if parts.pop().is_none() {
                        valid = false;
                        break;
                    }
                }
                _ => {
                    valid = false;
                    break;
                }
            }
        }
        if !valid {
            continue;
        }
        let target = parts.join("/");
        if target == path
            || !target.ends_with(".md")
            || restricted(&target)
            || validate_path(&target).is_err()
        {
            continue;
        }
        let full_path = format!("{}/{}", scope.as_str(), target);
        if full_path.starts_with(prefix) {
            targets.insert(full_path);
        }
    }
    targets
}

pub(crate) fn normalize_paths(
    scope: &ContextScope,
    paths: &[String],
) -> Result<Vec<String>, Error> {
    if paths.len() > 100
        || paths
            .iter()
            .any(|path| Path::new(path).is_absolute() || crate::context::restricted(path))
    {
        return Err(Error::Invalid);
    }
    let (core_scope, prefix) = match scope.as_str() {
        "personal" => (Scope::Personal, ""),
        "profile" => (Scope::Profile, ""),
        value => (
            Scope::Work,
            value.strip_prefix("work/").ok_or(Error::Invalid)?,
        ),
    };
    let requested: Vec<PathBuf> = paths
        .iter()
        .map(|p| {
            if prefix.is_empty() {
                PathBuf::from(p)
            } else {
                PathBuf::from(format!("{prefix}/{p}"))
            }
        })
        .collect();
    normalize_exact_read_paths(core_scope, &requested)
        .map_err(|_| Error::Invalid)?
        .into_iter()
        .map(|p| {
            p.to_str()
                .and_then(|p| p.strip_prefix(&format!("{}/", scope.as_str())))
                .map(str::to_owned)
                .ok_or(Error::Invalid)
        })
        .collect()
}
pub(crate) fn eligible(scope: &ContextScope, path: &str, restricted: bool) -> bool {
    !restricted
        && Path::new(path)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md"))
        && normalize_paths(scope, &[path.to_owned()]).is_ok()
}
fn redacted_optional(value: Option<&str>) -> Value {
    value
        .map(|v| Value::String(redact_secrets(v)))
        .unwrap_or(Value::Null)
}
fn redacted_list(values: &[String]) -> Vec<String> {
    values.iter().map(|v| redact_secrets(v)).collect()
}

/// No I/O: callers supply only already verified, explicitly selected original bytes.
pub(crate) fn projection(
    scope: &ContextScope,
    path: &str,
    sha: &str,
    restricted: bool,
    deleted: bool,
    bytes: Option<&[u8]>,
) -> Result<Value, Error> {
    let mut payload = json!({"source_digest":sha,"status":"excluded","terms":[]});
    if deleted {
        payload["status"] = json!("tombstone");
        return Ok(payload);
    }
    if !eligible(scope, path, restricted) {
        return Ok(payload);
    }
    payload["status"] = json!("unavailable");
    let Some(bytes) = bytes else {
        return Ok(payload);
    };
    if digest(bytes) != sha {
        return Err(Error::Storage);
    }
    let Ok(parsed) = parse_markdown_bytes(Path::new(&format!("{}/{path}", scope.as_str())), bytes)
    else {
        return Ok(payload);
    };
    if parsed
        .declared_scope()
        .is_some_and(|s| s != scope.as_str().split('/').next().unwrap_or(""))
    {
        return Ok(payload);
    }
    let doc = parsed.document();
    let ontology = doc.ontology();
    let title = redact_secrets(doc.title());
    let body = redact_secrets(doc.body());
    let aliases = redacted_list(doc.aliases());
    let mut related: BTreeSet<String> = redacted_list(ontology.related()).into_iter().collect();
    if let Some(prefix) = ontology.related_link_prefix() {
        related.extend(
            linked_markdown_paths(scope, path, doc.body(), prefix)
                .into_iter()
                .map(|target| redact_secrets(&target)),
        );
    }
    let ont = json!({"type":redacted_optional(ontology.document_type()),"domain":redacted_optional(ontology.domain()),"status":redacted_optional(ontology.status()),"confidence":redacted_optional(ontology.confidence()),"entities":redacted_list(ontology.entities()),"applies_to":redacted_list(ontology.applies_to()),"related":related,"relations":ontology.relations().iter().map(|r|json!({"from":redact_secrets(r.from()),"type":redact_secrets(r.relation_type()),"to":redact_secrets(r.to())})).collect::<Vec<_>>()});
    let ontology_text = redact_secrets(&doc.ontology_search_text());
    let alias_text = aliases.join(" ");
    let text = expanded_search_text(&[&title, &body, &alias_text, &ontology_text]);
    let terms: BTreeSet<String> = text.split_whitespace().map(str::to_owned).collect();
    payload = json!({"source_digest":sha,"status":"searchable","title":title,"body":body,"language":redacted_optional(doc.language()),"aliases":aliases,"exportable":doc.exportable(),"ontology":ont,"terms":terms});
    if serde_json::to_vec(&payload)
        .map_err(|_| Error::Storage)?
        .len()
        > 32 * 1024 * 1024
    {
        return Err(Error::Limit);
    }
    Ok(payload)
}

#[cfg(test)]
mod link_tests {
    use super::*;

    #[test]
    fn opted_in_source_map_links_become_deduplicated_relations() {
        let scope: ContextScope = "personal".parse().unwrap();
        let content = "---\nontology: true\nrelated_from_links: true\nrelated_link_prefix: personal/knowledge/notion/pages/\nrelated: [personal/knowledge/notion/pages/a.md]\n---\n[one](<pages/a.md>) [two](<pages/b.md>) [포트폴리오](<pages/[SK 하이닉스] R&D 합격 포트폴리오.md>) [sort](notion-page-sort.md) [outside](../../../../profile/private.md) [web](https://example.com/page.md) [self](index.md)\n\n`[code](pages/c.md)`\n".as_bytes();
        let result = projection(
            &scope,
            "knowledge/notion/index.md",
            &digest(content),
            false,
            false,
            Some(content),
        )
        .unwrap();
        assert_eq!(
            result["ontology"]["related"],
            json!([
                "personal/knowledge/notion/pages/[SK 하이닉스] R&D 합격 포트폴리오.md",
                "personal/knowledge/notion/pages/a.md",
                "personal/knowledge/notion/pages/b.md"
            ])
        );
    }

    #[test]
    fn markdown_links_do_not_create_relations_without_opt_in() {
        let scope: ContextScope = "personal".parse().unwrap();
        let content = b"---\nontology: true\n---\n[one](pages/a.md)\n";
        let result = projection(
            &scope,
            "knowledge/notion/index.md",
            &digest(content),
            false,
            false,
            Some(content),
        )
        .unwrap();
        assert_eq!(result["ontology"]["related"], json!([]));
    }
}

/// A bounded page shares one validation query and one append query. The caller holds
/// the exclusive context gate and the surrounding transaction through publication.
pub(crate) async fn append_projections(
    store: &Store,
    connection: &mut PgConnection,
    records: &[Value],
) -> Result<(), Error> {
    if records.is_empty() {
        return Ok(());
    }
    if records.len() > 100 {
        return Err(Error::Limit);
    }
    let records = Value::Array(records.to_vec());
    store.count(1);
    let conflict: bool = sqlx::query_scalar(
        r#"
        WITH incoming AS (
            SELECT material_id::uuid AS material_id,revision,payload
            FROM jsonb_to_recordset($1) AS x(material_id text,revision bigint,payload jsonb)
        )
        SELECT EXISTS (
            SELECT 1 FROM incoming i JOIN context_projection_versions p USING(material_id,revision)
            WHERE p.payload IS DISTINCT FROM i.payload
               OR p.payload_digest<>encode(sha256(convert_to(p.payload::text,'UTF8')),'hex')
        )
    "#,
    )
    .bind(&records)
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| Error::Storage)?;
    if conflict {
        return Err(Error::Storage);
    }
    store.count(1);
    sqlx::query(r#"
        INSERT INTO context_projection_versions(material_id,revision,payload,payload_digest)
        SELECT material_id::uuid,revision,payload,encode(sha256(convert_to(payload::text,'UTF8')),'hex')
        FROM jsonb_to_recordset($1) AS x(material_id text,revision bigint,payload jsonb)
        ON CONFLICT(material_id,revision) DO NOTHING
    "#).bind(records).execute(connection).await.map_err(|_|Error::Storage)?;
    Ok(())
}

#[derive(Debug)]
pub(crate) struct ExactMetadata {
    pub path: String,
    pub material_id: String,
    pub revision: i64,
    pub sha: String,
    pub byte_len: i64,
    pub restricted: bool,
    pub deleted: bool,
    pub origin: String,
    pub source_path: String,
    pub source_digest: Option<String>,
}
impl ExactMetadata {
    pub(crate) fn from_row(row: &PgRow) -> Self {
        Self {
            path: row.get("path"),
            material_id: row.get("material_id"),
            revision: row.get("revision"),
            sha: row.get("content_digest"),
            byte_len: row.get("byte_len"),
            restricted: row.get("restricted"),
            deleted: row.get("deleted"),
            origin: row.get("origin_kind"),
            source_path: row.get("source_path"),
            source_digest: row.get("source_digest"),
        }
    }
}
/// The caller owns the shared context gate through subsequent publication.
pub(crate) async fn exact_metadata(
    store: &Store,
    conn: &mut PgConnection,
    scope: &ContextScope,
    paths: &[String],
) -> Result<Vec<ExactMetadata>, Error> {
    let paths = normalize_paths(scope, paths)?;
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    store.count(1);
    Ok(sqlx::query("SELECT path,material_id::text,revision,content_digest,byte_len,restricted,deleted,origin_kind,source_path,source_digest FROM context_materials WHERE scope=$1 AND path=ANY($2) ORDER BY path COLLATE \"C\"")
        .bind(scope.as_str()).bind(paths).fetch_all(conn).await.map_err(|_|Error::Storage)?.iter().map(ExactMetadata::from_row).collect())
}
/// Empty/invalid input makes no body query; every valid live batch makes exactly one.
pub(crate) async fn exact_documents(
    store: &Store,
    conn: &mut PgConnection,
    scope: &ContextScope,
    paths: &[String],
) -> Result<Value, Error> {
    let paths = normalize_paths(scope, paths)?;
    if paths.is_empty() {
        return Ok(json!({"documents":[]}));
    }
    let metadata = exact_metadata(store, conn, scope, &paths).await?;
    if metadata.len() != paths.len()
        || metadata
            .iter()
            .any(|m| m.deleted || !eligible(scope, &m.path, m.restricted))
    {
        return Err(Error::NotFound);
    }
    if metadata
        .iter()
        .any(|m| m.byte_len < 0 || m.byte_len > MAX_DOCUMENT_BYTES as i64)
    {
        return Err(Error::Limit);
    }
    store.count(1);
    let store_id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
        .fetch_one(&mut *conn)
        .await
        .map_err(|_| Error::Storage)?;
    store.count(1);
    let bodies=sqlx::query("SELECT path,CASE WHEN byte_len BETWEEN 0 AND 65536 AND octet_length(content)<=65536 THEN content END AS content FROM context_materials WHERE scope=$1 AND path=ANY($2) AND NOT deleted ORDER BY path COLLATE \"C\"")
        .bind(scope.as_str()).bind(&paths).fetch_all(conn).await.map_err(|_|Error::Storage)?;
    if bodies.len() != metadata.len() {
        return Err(Error::Conflict);
    }
    let mut documents = Vec::with_capacity(metadata.len());
    for (m, row) in metadata.iter().zip(&bodies) {
        let bytes: Vec<u8> = row
            .get::<Option<Vec<u8>>, _>("content")
            .ok_or(Error::Limit)?;
        if m.path != row.get::<String, _>("path") || m.byte_len != bytes.len() as i64 {
            return Err(Error::Storage);
        }
        let p = projection(
            scope,
            &m.path,
            &m.sha,
            m.restricted,
            m.deleted,
            Some(&bytes),
        )?;
        if p["status"] != "searchable" {
            return Err(Error::Invalid);
        }
        if serde_json::to_vec(&p).map_err(|_| Error::Storage)?.len() > MAX_DOCUMENT_BYTES {
            return Err(Error::Limit);
        }
        documents.push(json!({"scope":scope,"path":format!("{}/{}",scope.as_str(),m.path),"title":p["title"],"body":p["body"],"source_digest":m.sha,"material_id":m.material_id,"store_id":store_id,"revision":m.revision,"origin_kind":m.origin,"origin_source_digest":m.source_digest,"source_path":m.source_path}));
    }
    bounded(json!({"documents":documents}))
}
fn bounded(value: Value) -> Result<Value, Error> {
    if serde_json::to_vec(&value)
        .map_err(|_| Error::Storage)?
        .len()
        > MAX_RESPONSE_BYTES
    {
        Err(Error::Limit)
    } else {
        Ok(value)
    }
}
fn scopes(scopes: &[ContextScope]) -> Result<Vec<String>, Error> {
    let values: BTreeSet<_> = scopes.iter().map(|s| s.as_str().to_owned()).collect();
    if values.is_empty() || values.len() > 64 || values.len() != scopes.len() {
        return Err(Error::Invalid);
    }
    Ok(values.into_iter().collect())
}
async fn manifest(
    store: &Store,
    conn: &mut PgConnection,
    scopes: &[String],
) -> Result<(Vec<PgRow>, String), Error> {
    store.count(1);
    let rows=sqlx::query("SELECT m.material_id::text,m.revision,m.scope,m.path,m.content_digest,m.byte_len,m.restricted,m.deleted,p.payload->>'status' AS status,p.payload_digest,encode(sha256(convert_to(p.payload::text,'UTF8')),'hex') AS actual_projection_digest FROM context_materials m LEFT JOIN context_projection_versions p USING(material_id,revision) WHERE m.scope=ANY($1) ORDER BY m.material_id,m.revision,m.path LIMIT 10001")
        .bind(scopes).fetch_all(conn).await.map_err(|_|Error::Storage)?;
    if rows.len() > MAX_FILES {
        return Err(Error::Limit);
    }
    let values: Vec<_> = rows
        .iter()
        .map(|r| {
            json!([
                r.get::<String, _>("material_id"),
                r.get::<i64, _>("revision"),
                r.get::<String, _>("scope"),
                r.get::<String, _>("path"),
                r.get::<String, _>("content_digest"),
                r.get::<Option<String>, _>("status")
            ])
        })
        .collect();
    let sha = digest(&serde_json::to_vec(&values).map_err(|_| Error::Storage)?);
    Ok((rows, sha))
}
impl Store {
    pub async fn read_context_documents(
        &self,
        scope: &ContextScope,
        paths: &[String],
    ) -> Result<Value, Error> {
        if paths.is_empty() {
            return Err(Error::Invalid);
        }
        normalize_paths(scope, paths)?;
        let mut tx = self.lock_context(false).await?;
        let result = exact_documents(self, &mut tx, scope, paths).await;
        self.finish_context(tx, result).await
    }
    pub async fn context_identity(&self) -> Result<Value, Error> {
        let mut tx = self.lock_context(false).await?;
        self.count(1);
        let result = sqlx::query_scalar::<_, String>("SELECT store_id::text FROM context_store")
            .fetch_one(&mut *tx)
            .await
            .map(|id| json!({"store_id":id}))
            .map_err(|_| Error::Storage);
        self.finish_context(tx, result).await
    }
    pub async fn projection_status(&self, selected: &[ContextScope]) -> Result<Value, Error> {
        let scopes = scopes(selected)?;
        let mut tx = self.lock_context(false).await?;
        let result=async {let(rows,sha)=manifest(self,&mut tx,&scopes).await?;let mut counts=std::collections::BTreeMap::<String,usize>::new();
            for r in &rows {
                if r.get::<Option<String>,_>("payload_digest")!=r.get::<Option<String>,_>("actual_projection_digest") {return Err(Error::Storage);}
                let status=r.get::<Option<String>,_>("status").unwrap_or_else(||"missing".into());
                *counts.entry(status).or_default()+=1;
            }
            Ok(json!({"scopes":scopes,"manifest_digest":sha,"total":rows.len(),"counts":counts,"ready":!counts.contains_key("missing")&&!counts.contains_key("unavailable")})) }.await;
        self.finish_context(tx, result).await
    }
    pub async fn project_context(
        &self,
        selected: &[ContextScope],
        expected: &str,
    ) -> Result<Value, Error> {
        let scopes = scopes(selected)?;
        if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Invalid);
        }
        let mut tx = self.lock_context(true).await?;
        let result=async{let(rows,sha)=manifest(self,&mut tx,&scopes).await?;if sha!=expected{return Err(Error::Conflict);}
            let mut total=0u64;let mut inserted=0;
            for page in rows.chunks(100) {
                let mut ids=Vec::new();
                for r in page {let scope:ContextScope=r.get::<String,_>("scope").parse()?;let length=r.get::<i64,_>("byte_len");
                    if length<0 {return Err(Error::Storage);} total=total.checked_add(length as u64).ok_or(Error::Limit)?;if total>MAX_TOTAL_BYTES{return Err(Error::Limit);}
                    if !r.get::<bool,_>("deleted")&&eligible(&scope,&r.get::<String,_>("path"),r.get("restricted"))&&length<=10*1024*1024 {ids.push(r.get::<String,_>("material_id"));}
                    if r.get::<Option<String>,_>("payload_digest")!=r.get::<Option<String>,_>("actual_projection_digest"){return Err(Error::Storage);}
                }
                let bodies=if ids.is_empty(){Vec::new()}else{self.count(1);sqlx::query("SELECT material_id::text,CASE WHEN byte_len BETWEEN 0 AND 10485760 AND octet_length(content)<=10485760 THEN content END AS content FROM context_materials WHERE material_id::text=ANY($1)").bind(&ids).fetch_all(&mut *tx).await.map_err(|_|Error::Storage)?};
                let mut projections=Vec::with_capacity(page.len());
                for r in page {let scope:ContextScope=r.get::<String,_>("scope").parse()?;let id:String=r.get("material_id");
                    let body=bodies.iter().find(|b|b.get::<String,_>("material_id")==id).and_then(|b|b.get::<Option<Vec<u8>>,_>("content"));
                    let p=projection(&scope,&r.get::<String,_>("path"),&r.get::<String,_>("content_digest"),r.get("restricted"),r.get("deleted"),body.as_deref())?;
                    projections.push(json!({"material_id":id,"revision":r.get::<i64,_>("revision"),"payload":p}));
                    if r.get::<Option<String>,_>("status").is_none(){inserted+=1;}
                }
                append_projections(self,&mut tx,&projections).await?;
            } Ok(json!({"inserted":inserted,"verified":rows.len(),"manifest_digest":sha}))}.await;
        self.finish_context(tx, result).await
    }
    pub async fn semantic_context(
        &self,
        selected: &[ContextScope],
        query: &str,
        limit: usize,
        edges: bool,
    ) -> Result<Value, Error> {
        let scopes = scopes(selected)?;
        if query.trim().is_empty()
            || query.chars().count() > 120
            || query.chars().any(char::is_control)
            || !(1..=100).contains(&limit)
        {
            return Err(Error::Invalid);
        }
        let groups = expanded_query_terms(&redact_secrets(query));
        if groups.is_empty() {
            return Err(Error::Invalid);
        }
        let mut tx = self.lock_context(false).await?;
        let result=async{
            let (readiness, _)=manifest(self,&mut tx,&scopes).await?;
            for row in &readiness {
                let selected:ContextScope=row.get::<String,_>("scope").parse()?;
                if !row.get::<bool,_>("deleted") && eligible(&selected,&row.get::<String,_>("path"),row.get("restricted"))
                    && (row.get::<Option<String>,_>("status").is_none_or(|s|s=="unavailable") || row.get::<Option<String>,_>("payload_digest")!=row.get::<Option<String>,_>("actual_projection_digest")) {
                    return Err(Error::ContextProjectionUnavailable);
                }
            }
            if edges {
                self.count(1);
                let rows=sqlx::query(r#"
                    WITH edges AS (
                        SELECT DISTINCT m.scope,m.path,m.material_id,m.revision,
                            r->>'from' AS from_entity,r->>'type' AS relation_type,r->>'to' AS to_entity
                        FROM context_materials m JOIN context_projection_versions p USING(material_id,revision)
                        CROSS JOIN LATERAL jsonb_array_elements(p.payload->'ontology'->'relations') r
                        WHERE NOT m.deleted AND m.scope=ANY($1) AND p.payload->>'status'='searchable'
                          AND (r->>'from'=$2 OR r->>'to'=$2)
                    )
                    SELECT scope,path,material_id::text,revision,from_entity,relation_type,to_entity
                    FROM edges ORDER BY scope COLLATE "C",path COLLATE "C",from_entity COLLATE "C",relation_type COLLATE "C",to_entity COLLATE "C"
                    LIMIT $3
                "#).bind(&scopes).bind(query).bind(limit as i64).fetch_all(&mut *tx).await.map_err(|_|Error::Storage)?;
                let items:Vec<_>=rows.iter().map(|r|json!({"scope":r.get::<String,_>("scope"),"path":r.get::<String,_>("path"),"material_id":r.get::<String,_>("material_id"),"revision":r.get::<i64,_>("revision"),"relation":{"from":r.get::<String,_>("from_entity"),"type":r.get::<String,_>("relation_type"),"to":r.get::<String,_>("to_entity")}})).collect();
                return bounded(json!({"items":items,"limit":limit}));
            }
            let terms:Vec<String>=groups.iter().flatten().cloned().collect();
            let mut q=sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT m.scope,m.path,m.material_id::text,m.revision,jsonb_build_object('source_digest',p.payload->'source_digest','title',left(p.payload->>'title',4096),'body',left(p.payload->>'body',400),'exportable',p.payload->'exportable') AS payload,(SELECT count(*) FROM jsonb_array_elements_text(p.payload->'terms') t WHERE t=ANY(");
            q.push_bind(&terms).push(")) AS relevance FROM context_materials m JOIN context_projection_versions p USING(material_id,revision) WHERE NOT m.deleted AND m.scope=ANY(");
            q.push_bind(&scopes).push(") AND p.payload->>'status'='searchable'");
            for group in &groups {q.push(" AND (p.payload->'terms') ?| ").push_bind(group);}
            q.push(" ORDER BY relevance DESC,m.scope COLLATE \"C\",m.path COLLATE \"C\" LIMIT ").push_bind(limit as i64);
            self.count(1);
            let rows=q.build().fetch_all(&mut *tx).await.map_err(|_|Error::Storage)?;
            let items:Vec<_>=rows.iter().map(|r|{
                let p:Value=r.get("payload");
                json!({"scope":r.get::<String,_>("scope"),"path":r.get::<String,_>("path"),"material_id":r.get::<String,_>("material_id"),"revision":r.get::<i64,_>("revision"),"source_digest":p["source_digest"],"title":p["title"],"snippet":p["body"],"exportable":p["exportable"]})
            }).collect();
            bounded(json!({"items":items,"limit":limit}))
        }.await;
        self.finish_context(tx, result).await
    }
}
