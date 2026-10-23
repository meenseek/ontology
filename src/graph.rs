//! Read-only, bounded graph projection. Every request observes one PostgreSQL snapshot.
use crate::{
    domain::{AREAS, Error, MAX_RESPONSE_BYTES, Scope, validate_id, validate_search},
    memory::{evidence_current, memory_content_updated_at},
    store::Store,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;
use uuid::Uuid;

pub const MAX_GRAPH_NODES: usize = 800;
pub const MAX_GRAPH_LINKS: usize = 2_000;
fn default_limit() -> usize {
    MAX_GRAPH_NODES
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphQuery {
    pub scope: Scope,
    #[serde(default)]
    pub q: String,
    pub focus: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}
impl GraphQuery {
    fn validate(&self) -> Result<(), Error> {
        validate_search(&self.q)?;
        if !(1..=MAX_GRAPH_NODES).contains(&self.limit) {
            return Err(Error::Invalid);
        }
        if let Some(id) = &self.focus {
            if id.starts_with("e_") {
                validate_id(id)?;
            } else if let Some(uuid) = id
                .strip_prefix("m_")
                .or_else(|| id.strip_prefix("p_"))
                .or_else(|| id.strip_prefix("c_"))
            {
                if id.len() != 38 || Uuid::parse_str(uuid).is_err() {
                    return Err(Error::Invalid);
                }
            } else if let Some(topic) = id.strip_prefix("t_") {
                if topic.parse::<i64>().is_err()
                    || topic.starts_with('0')
                    || !topic.bytes().all(|c| c.is_ascii_digit())
                {
                    return Err(Error::Invalid);
                }
            } else if let Some(area) = id.strip_prefix("a_") {
                if self.scope != Scope::Meenseek || !AREAS.iter().any(|(id, _)| *id == area) {
                    return Err(Error::Invalid);
                }
            } else {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
}

// Original revision timestamps are distinct from consumer refresh timestamps.
macro_rules! original_content_updated_at {
    ($alias:literal) => {
        concat!("(SELECT v.recorded_at FROM context_material_versions v LEFT JOIN context_material_versions previous ON previous.material_id=v.material_id AND previous.revision=v.revision-1 WHERE v.material_id=", $alias, ".material_id AND v.content_digest IS DISTINCT FROM previous.content_digest ORDER BY v.revision DESC LIMIT 1)")
    };
}
// OFFSET 0 keeps PostgreSQL from inlining the per-document text normalization
// into each term predicate. Large original bodies are normalized once per row.
// Order terms before testing excerpt hits so LIMIT stops at the first hit rather
// than calculating every body position and sorting the entire set afterward.
const GRAPH_SQL: &str = concat!(
    r#"WITH terms AS MATERIALIZED (
 SELECT lower(word) AS term,ordinality FROM regexp_split_to_table(btrim($2),'[[:space:]]+') WITH ORDINALITY AS words(word,ordinality)
 WHERE word<>''
), evidence AS MATERIALIZED (
 SELECT m.id AS memory_id,x->>'entity_id' AS entity_id,"#,
    evidence_current!(),
    r#" AS current
 FROM memories m CROSS JOIN LATERAL jsonb_array_elements(m.document->'evidence') x
 LEFT JOIN entities e ON e.scope=m.scope AND e.id=x->>'entity_id'
 LEFT JOIN sources s ON s.scope=e.scope AND s.id=e.source_id
 LEFT JOIN source_records p ON p.scope=e.scope AND p.entity_id=e.id
 WHERE m.scope=$1
) , stale_memories AS MATERIALIZED (
 SELECT DISTINCT memory_id FROM evidence WHERE NOT current
), bound_materials AS MATERIALIZED (
 SELECT DISTINCT ON (b.material_id) b.material_id,e.id
 FROM context_source_bindings b JOIN sources s ON s.id=b.source_id AND s.scope=$1
 JOIN entities e ON e.scope=s.scope AND e.source_id=s.id
 JOIN source_records r ON r.scope=e.scope AND r.entity_id=e.id
 ORDER BY b.material_id,e.id
), current_projections AS MATERIALIZED (
 -- Decode each visible current projection once. Extracting fields from the large
 -- JSON payload in every consumer repeatedly detoasts bodies, including old revisions.
 SELECT m.material_id,m.revision,projection.*
 FROM context_materials m JOIN context_projection_versions p USING(material_id,revision)
 CROSS JOIN LATERAL jsonb_to_record(p.payload)
 projection(title text,status text,source_digest text,body text,aliases text,source_references jsonb,ontology jsonb)
 WHERE NOT m.deleted AND NOT m.restricted
 AND ($1='personal' OR EXISTS(SELECT 1 FROM bound_materials b WHERE b.material_id=m.material_id))
), imported_references AS MATERIALIZED (
 SELECT source.id,source.scope,s.repository,reference.target_path
 FROM entities source JOIN sources s ON s.scope=source.scope AND s.id=source.source_id
 JOIN source_records r ON r.scope=source.scope AND r.entity_id=source.id
 CROSS JOIN LATERAL unnest(r.reference_paths) reference(target_path)
 WHERE source.scope=$1 AND s.kind='git' AND s.status='ok' AND r.present AND r.source_revision=s.verified_revision
 AND NOT EXISTS(SELECT 1 FROM context_source_bindings b WHERE b.source_id=s.id)
), source_nodes AS MATERIALIZED (
 SELECT e.id,'document' AS kind,
 jsonb_build_object('id',e.id,'scope',e.scope,'kind','document','label',s.path,'title',cp.title,'repository',s.repository,
 'revision',COALESCE(cm.revision,e.revision)::text,'content_digest',COALESCE(cm.content_digest,p.content_digest),'source_revision',p.source_revision,
 'generation',s.generation::text,'status',CASE WHEN cm.material_id IS NULL THEN s.status ELSE 'ok' END,
 'present',CASE WHEN cm.material_id IS NULL THEN p.present ELSE true END,
 'current',CASE WHEN cm.material_id IS NULL THEN (s.status='ok' AND p.present AND p.source_revision=s.verified_revision) ELSE true END,'source_kind',s.kind,
 'last_success_at',s.last_success_at,'observed_at',p.observed_at,
 'context_scope',cm.scope,'context_path',cm.path,'material_id',cm.material_id,
 'purpose_source_revision',CASE WHEN cm.material_id IS NULL THEN p.source_revision ELSE cm.revision::text END,
 'created_at',cm.created_at,'content_updated_at',"#,
    original_content_updated_at!("cm"),
    r#",
 'excerpt',CASE WHEN excerpt_hit.at IS NOT NULL THEN '…' || substring(current_body.body FROM greatest(1,excerpt_hit.at-80) FOR 400) || '…' END) AS value,
 NOT EXISTS(SELECT 1 FROM terms WHERE strpos(search_text.text,term)=0) AS matched
 FROM entities e JOIN sources s ON s.scope=e.scope AND s.id=e.source_id
 JOIN source_records p ON p.scope=e.scope AND p.entity_id=e.id
 LEFT JOIN context_source_bindings cb ON cb.source_id=s.id
 LEFT JOIN context_materials cm ON cm.material_id=cb.material_id AND NOT cm.deleted AND NOT cm.restricted
 LEFT JOIN current_projections cp ON cp.material_id=cm.material_id AND cp.revision=cm.revision
 CROSS JOIN LATERAL (SELECT CASE WHEN $2<>'' THEN CASE WHEN cm.material_id IS NULL THEN p.content ELSE cp.body END END AS body,
 CASE WHEN $2<>'' THEN lower(CASE WHEN cm.material_id IS NULL THEN p.content ELSE cp.body END) END AS lower_body OFFSET 0) current_body
 CROSS JOIN LATERAL (SELECT CASE WHEN $2<>'' THEN lower(concat(s.path,' ',cm.path,' ',cm.search_text,' ',CASE WHEN cm.material_id IS NULL THEN p.content ELSE concat(cp.title,' ',cp.body,' ',cp.aliases) END,' ',
   (SELECT string_agg(t.name,' ') FROM entity_topics et JOIN topics t ON t.scope=et.scope AND t.id=et.topic_id WHERE et.scope=e.scope AND et.entity_id=e.id))) END AS text OFFSET 0) search_text
 LEFT JOIN LATERAL (
 SELECT hit.at FROM (SELECT term FROM terms ORDER BY ordinality OFFSET 0) ordered_terms
 CROSS JOIN LATERAL (SELECT strpos(current_body.lower_body,term) AS at OFFSET 0) hit
 WHERE hit.at>0 LIMIT 1
 ) excerpt_hit ON true
 WHERE e.scope=$1 AND (cb.source_id IS NULL OR (cm.material_id IS NOT NULL AND cm.path LIKE '%.md'
 AND cp.status IN ('searchable','unavailable') AND cp.source_digest=cm.content_digest))
 UNION ALL
 SELECT 'c_'||m.material_id::text,'document',
 jsonb_build_object('id','c_'||m.material_id::text,'scope',$1,'kind','document',
 'label',m.path,'title',p.title,'repository',m.scope,'source_kind','original','context_scope',m.scope,'context_path',m.path,
 'revision',m.revision::text,'content_digest',m.content_digest,'material_id',m.material_id,'purpose_source_revision',m.revision::text,'status','ok','present',true,'current',true,
 'created_at',m.created_at,'content_updated_at',"#,
    original_content_updated_at!("m"),
    r#",
 'excerpt',CASE WHEN excerpt_hit.at IS NOT NULL THEN '…' || substring(content.body FROM greatest(1,excerpt_hit.at-80) FOR 400) || '…' END),
 NOT EXISTS(SELECT 1 FROM terms WHERE strpos(content.search_text,term)=0)
 FROM context_materials m LEFT JOIN current_projections p USING(material_id,revision)
 CROSS JOIN LATERAL (SELECT CASE WHEN $2<>'' THEN p.body END AS body,
 CASE WHEN $2<>'' THEN lower(p.body) END AS lower_body,
 CASE WHEN $2<>'' THEN lower(concat(m.path,' ',m.search_text,' ',p.title,' ',p.body)) END AS search_text OFFSET 0) content
 LEFT JOIN LATERAL (
 SELECT hit.at FROM (SELECT term FROM terms ORDER BY ordinality OFFSET 0) ordered_terms
 CROSS JOIN LATERAL (SELECT strpos(content.lower_body,term) AS at OFFSET 0) hit
 WHERE hit.at>0 LIMIT 1
 ) excerpt_hit ON true
 WHERE $1='personal' AND NOT m.deleted AND NOT m.restricted
 AND m.path LIKE '%.md' AND p.status IN ('searchable','unavailable')
 AND p.source_digest=m.content_digest
 AND NOT EXISTS (SELECT 1 FROM bound_materials b WHERE b.material_id=m.material_id)
 UNION ALL
 SELECT m.id,'memory',jsonb_build_object('id',m.id,'scope',m.scope,'kind','memory',
 'label',m.document->>'title','revision',m.revision::text,'status',m.status,
 'memory_kind',m.document->>'kind','subject_id',m.subject_id,
 'created_at',m.created_at,'content_updated_at',"#,
    memory_content_updated_at!(),
    r#",
 'temporal',CASE WHEN (m.document->>'effective_from')::bigint>extract(epoch FROM now()) THEN 'future'
 WHEN (m.document->>'effective_until')::bigint<=extract(epoch FROM now()) THEN 'expired' ELSE 'current' END,
 'supported',stale.memory_id IS NULL,
 'support',CASE WHEN jsonb_array_length(m.document->'evidence')=0 THEN 'user-recorded' ELSE 'source-linked' END,
 'matched_revision',CASE WHEN old.revision IS NOT NULL THEN old.revision::text ELSE m.revision::text END,
 'historical_match',old.revision IS NOT NULL,
 'excerpt',CASE WHEN excerpt_hit.at IS NOT NULL THEN '…' || substring(COALESCE(old.document,m.document)->>'body' FROM greatest(1,excerpt_hit.at-80) FOR 400) || '…' END),
 (NOT EXISTS(SELECT 1 FROM terms WHERE strpos(lower(concat(m.document->>'title',' ',m.document->>'body')),term)=0) OR old.revision IS NOT NULL)
 FROM memories m LEFT JOIN stale_memories stale ON stale.memory_id=m.id
 LEFT JOIN LATERAL (
   SELECT h.revision,h.document FROM memory_history h
   WHERE EXISTS(SELECT 1 FROM terms WHERE strpos(lower(concat(m.document->>'title',' ',m.document->>'body')),term)=0)
     AND h.scope=m.scope AND h.memory_id=m.id AND h.revision<m.revision
     AND NOT EXISTS(SELECT 1 FROM terms WHERE strpos(lower(concat(h.document->>'title',' ',h.document->>'body')),term)=0)
   ORDER BY h.revision DESC LIMIT 1
 ) old ON true
 LEFT JOIN LATERAL (SELECT strpos(lower(COALESCE(old.document,m.document)->>'body'),term) AS at FROM terms WHERE strpos(lower(COALESCE(old.document,m.document)->>'body'),term)>0 ORDER BY ordinality LIMIT 1) excerpt_hit ON true
 WHERE m.scope=$1
 UNION ALL
 SELECT 't_'||t.id,'topic',jsonb_build_object('id','t_'||t.id,'scope',t.scope,'kind','topic','label',t.name),
 NOT EXISTS(SELECT 1 FROM terms WHERE strpos(lower(t.name),term)=0)
 FROM topics t WHERE t.scope=$1 AND EXISTS(SELECT 1 FROM entity_topics et WHERE et.scope=t.scope AND et.topic_id=t.id)
 UNION ALL
 SELECT p.id,'subject',jsonb_build_object('id',p.id,'scope',p.scope,'kind','subject','label',p.name,'revision',p.revision::text,'definition',p.definition),
 NOT EXISTS(SELECT 1 FROM terms WHERE strpos(lower(p.name),term)=0) FROM subjects p WHERE p.scope=$1
 UNION ALL
 SELECT 'a_'||a.id,'area',jsonb_build_object('id','a_'||a.id,'scope',$1,'kind','area','label',a.label),
 NOT EXISTS(SELECT 1 FROM terms WHERE strpos(lower(a.label),term)=0) FROM areas a
 WHERE $1='meenseek' AND EXISTS(SELECT 1 FROM entity_areas ea WHERE ea.scope=$1 AND ea.area=a.id)
), classified_nodes AS MATERIALIZED (
 SELECT n.id,n.kind,n.matched,n.value ||
 CASE WHEN n.kind='document' THEN jsonb_build_object('subject_id',d.subject_id,'classification_revision',COALESCE(d.revision,0),
 'classification_review_needed',d.source_id IS NOT NULL AND (d.source_revision IS DISTINCT FROM n.value->>'purpose_source_revision'
 OR d.content_digest IS DISTINCT FROM n.value->>'content_digest' OR (d.subject_id IS NOT NULL AND (d.subject_revision IS DISTINCT FROM s.revision
 OR COALESCE(g.mode='auto' AND g.state IN ('pending','processing','suggested','unmatched','error','ineligible'),false))))) ELSE '{}'::jsonb END ||
 CASE WHEN s.id IS NOT NULL THEN jsonb_build_object('subject_name',s.name,'subject_revision',s.revision) ELSE '{}'::jsonb END AS value
 FROM source_nodes n
 LEFT JOIN document_subjects d ON n.kind='document' AND d.scope=$1 AND d.source_id=
 CASE WHEN n.value->>'material_id' IS NOT NULL AND n.value->>'context_path' NOT LIKE 'journal/%' THEN n.value->>'material_id'
 WHEN n.value->>'material_id' IS NULL AND n.value->>'source_kind'='git' THEN n.id END
 LEFT JOIN subjects s ON s.scope=$1 AND s.id=COALESCE(d.subject_id,CASE WHEN n.kind='memory' THEN n.value->>'subject_id' WHEN n.kind='subject' THEN n.id END)
 LEFT JOIN document_grouping g ON g.scope=d.scope AND g.source_id=d.source_id
), purpose_counts AS MATERIALIZED (
 SELECT value->>'subject_id' AS subject_id,count(*) AS total FROM classified_nodes
 WHERE kind IN ('document','memory') AND value->>'subject_id' IS NOT NULL GROUP BY value->>'subject_id'
), nodes AS MATERIALIZED (
 SELECT n.id,n.kind,n.matched,n.value || CASE WHEN c.subject_id IS NOT NULL OR n.kind='subject'
 THEN jsonb_build_object('purpose_total',COALESCE(c.total,0)) ELSE '{}'::jsonb END AS value
 FROM classified_nodes n LEFT JOIN purpose_counts c ON c.subject_id=CASE WHEN n.kind='subject' THEN n.id ELSE n.value->>'subject_id' END
), native_references AS MATERIALIZED (
 -- Reject sources outside this graph before expanding JSON. The old target join
 -- expanded the same projection once per target, even for a one-node health read.
 SELECT COALESCE(b.id,'c_'||m.material_id::text) AS source,reference.target_path
 FROM context_materials m JOIN current_projections p USING(material_id,revision)
 LEFT JOIN bound_materials b ON b.material_id=m.material_id
 CROSS JOIN LATERAL jsonb_array_elements_text(COALESCE(p.source_references,'[]'::jsonb)) reference(target_path)
 WHERE NOT m.deleted AND NOT m.restricted
 AND p.status='searchable' AND p.source_digest=m.content_digest
 AND EXISTS(SELECT 1 FROM source_nodes visible WHERE visible.id=COALESCE(b.id,'c_'||m.material_id::text))
), links AS MATERIALIZED (
 SELECT left_id AS source,right_id AS target,'related' AS kind,true AS current
 FROM related_materials WHERE scope=$1
 UNION ALL SELECT memory_id,entity_id,'evidence',current FROM evidence
 UNION ALL SELECT et.entity_id,'t_'||et.topic_id,'topic',true FROM entity_topics et WHERE et.scope=$1
 UNION ALL SELECT m.id,m.subject_id,'subject',true FROM memories m WHERE m.scope=$1 AND m.subject_id IS NOT NULL
 UNION ALL SELECT id,value->>'subject_id','subject',true FROM nodes WHERE kind='document' AND value->>'subject_id' IS NOT NULL
 UNION ALL SELECT ea.entity_id,'a_'||ea.area,'area',true FROM entity_areas ea WHERE ea.scope=$1
 UNION ALL SELECT COALESCE(source_bound.id,'c_'||m.material_id::text),COALESCE(target_bound.id,'c_'||target.material_id::text),'related',true
 FROM context_materials m JOIN current_projections p USING(material_id,revision)
 CROSS JOIN LATERAL jsonb_array_elements_text(COALESCE(p.ontology->'related','[]'::jsonb)) related(source_path)
 JOIN context_materials target ON target.scope=m.scope AND target.source_path=related.source_path
 LEFT JOIN bound_materials source_bound ON source_bound.material_id=m.material_id
 LEFT JOIN bound_materials target_bound ON target_bound.material_id=target.material_id
 WHERE $1='personal' AND NOT m.deleted AND NOT m.restricted AND NOT target.deleted AND NOT target.restricted
 UNION ALL SELECT reference.source,COALESCE(target_bound.id,'c_'||target.material_id::text),'reference',true
 FROM native_references reference
 JOIN context_materials target ON target.scope||'/'||target.path=reference.target_path
 LEFT JOIN bound_materials target_bound ON target_bound.material_id=target.material_id
 WHERE NOT target.deleted AND NOT target.restricted
 UNION ALL SELECT reference.id,target.id,'reference',true
 FROM imported_references reference
 JOIN sources t ON t.scope=reference.scope AND t.repository=reference.repository AND t.path=reference.target_path AND t.kind='git'
 JOIN entities target ON target.scope=t.scope AND target.source_id=t.id
 JOIN source_records tr ON tr.scope=target.scope AND tr.entity_id=target.id
 WHERE t.status='ok' AND tr.present AND tr.source_revision=t.verified_revision
 AND NOT EXISTS(SELECT 1 FROM context_source_bindings b WHERE b.source_id=t.id)
), scoped_links AS MATERIALIZED (
 SELECT l.* FROM links l JOIN nodes s ON s.id=l.source JOIN nodes t ON t.id=l.target
), relation_tokens AS (
 SELECT source AS id,kind||':'||source||':'||target||':'||current::text AS token FROM scoped_links
 UNION ALL
 SELECT target AS id,kind||':'||source||':'||target||':'||current::text AS token FROM scoped_links
), relation_digests AS MATERIALIZED (
 SELECT id,md5(string_agg(token,'|' ORDER BY token)) AS digest FROM relation_tokens GROUP BY id
), focused_neighbors AS MATERIALIZED (
 SELECT DISTINCT CASE WHEN l.source=$3 THEN l.target ELSE l.source END AS id
 FROM scoped_links l WHERE l.source=$3 OR l.target=$3
), eligible AS MATERIALIZED (
 SELECT n.*,COALESCE(n.id=$3,false) AS focused,f.id IS NOT NULL AS neighbor
 FROM nodes n LEFT JOIN focused_neighbors f ON f.id=n.id
 WHERE n.matched OR n.id=$3 OR f.id IS NOT NULL
), selected AS MATERIALIZED (
 SELECT eligible.*,COALESCE(d.digest,md5('')) AS relation_digest
 FROM eligible LEFT JOIN relation_digests d ON d.id=eligible.id
 ORDER BY focused DESC,neighbor DESC,kind IN ('document','memory') DESC,CASE WHEN $2='' THEN (value->>'content_updated_at')::timestamptz END DESC NULLS LAST,(CASE WHEN $2='' THEN COALESCE(value->>'title',value->>'label') END) COLLATE "C",eligible.id LIMIT $4
), eligible_links AS MATERIALIZED (
 SELECT l.* FROM scoped_links l JOIN eligible s ON s.id=l.source JOIN eligible t ON t.id=l.target
), selected_links AS (
 SELECT l.* FROM eligible_links l JOIN selected s ON s.id=l.source JOIN selected t ON t.id=l.target
 ORDER BY (l.source=$3 OR l.target=$3) DESC NULLS LAST,l.kind,l.source,l.target LIMIT $5
)
SELECT jsonb_build_object('scope',$1,'query',$2,'purpose','discovery','instruction','Search excerpts locate records; they are not complete decision evidence or verified facts. Read the current record, applicable rules and linked sources before acting.',
 'focus',jsonb_build_object('id',$3,'found',EXISTS(SELECT 1 FROM nodes WHERE id=$3)),
 'nodes',COALESCE((SELECT jsonb_agg(value||jsonb_build_object('relation_digest',relation_digest,'search_match',matched) ORDER BY focused DESC,neighbor DESC,kind IN ('document','memory') DESC,id) FROM selected),'[]'::jsonb),
 'links',COALESCE((SELECT jsonb_agg(to_jsonb(l)) FROM selected_links l),'[]'::jsonb),
 'totals',jsonb_build_object('documents',(SELECT count(*) FROM nodes WHERE kind='document'),
 'memories',(SELECT count(*) FROM nodes WHERE kind='memory'),'markers',(SELECT count(*) FROM nodes WHERE kind NOT IN ('document','memory')),
 'links',(SELECT count(*) FROM scoped_links)),
 'matched',(SELECT count(*) FROM nodes WHERE matched),
 'eligible',jsonb_build_object('nodes',(SELECT count(*) FROM eligible),'links',(SELECT count(*) FROM eligible_links)))"#
);
impl Store {
    pub async fn graph(&self, query: GraphQuery) -> Result<Value, Error> {
        let observation = self.dependency_start(
            "consumer-graph",
            &(
                query.scope,
                &query.q,
                &query.focus,
                query.limit,
                MAX_GRAPH_LINKS,
            ),
        )?;
        let result = async {
            query.validate()?;
            self.count(1);
            let started = std::time::Instant::now();
            let value: Value = sqlx::query_scalar(GRAPH_SQL)
                .bind(query.scope.as_str())
                .bind(&query.q)
                .bind(&query.focus)
                .bind(query.limit as i64)
                .bind(MAX_GRAPH_LINKS as i64)
                .fetch_one(self.pool())
                .await
                .map_err(|error| {
                    // Error messages can embed SQL, source text or credentials.
                    // Keep only operational categories so a future failure can
                    // distinguish pool exhaustion, a DB timeout and transport loss.
                    let kind = match &error {
                        sqlx::Error::Database(_) => "database",
                        sqlx::Error::PoolTimedOut => "pool_timeout",
                        sqlx::Error::PoolClosed => "pool_closed",
                        sqlx::Error::Io(_) | sqlx::Error::Tls(_) => "transport",
                        _ => "other",
                    };
                    let code = error.as_database_error().and_then(|db| db.code());
                    eprintln!("{}", json!({"event":"consumer_graph_failed","kind":kind,"sqlstate":code,"elapsed_ms":started.elapsed().as_millis()}));
                    Error::Storage
                })?;
            bound_response(value, query.limit)
        }
        .await;
        self.dependency_finish(observation, &result);
        result
    }
}

// SQL bounds row counts; this additionally bounds UTF-8/JSON expansion. Keep the prioritized
// focus node first and report every omitted row. Never perform per-node follow-up queries.
fn bound_response(mut value: Value, limit: usize) -> Result<Value, Error> {
    let mut nodes = value["nodes"]
        .take()
        .as_array()
        .ok_or(Error::Storage)?
        .clone();
    let mut links = value["links"]
        .take()
        .as_array()
        .ok_or(Error::Storage)?
        .clone();
    let total_nodes = ["documents", "memories", "markers"]
        .into_iter()
        .try_fold(0_u64, |total, key| {
            total.checked_add(value["totals"][key].as_u64()?)
        })
        .ok_or(Error::Storage)?;
    let total_links = value["totals"]["links"].as_u64().ok_or(Error::Storage)?;
    let eligible_nodes = value["eligible"]["nodes"].as_u64().ok_or(Error::Storage)?;
    let eligible_links = value["eligible"]["links"].as_u64().ok_or(Error::Storage)?;
    let mut byte_limited = false;
    loop {
        let knowledge = nodes
            .iter()
            .filter(|n| matches!(n["kind"].as_str(), Some("document" | "memory")))
            .count();
        value["returned"] =
            json!({"knowledge":knowledge,"markers":nodes.len()-knowledge,"links":links.len()});
        value["omitted"] = json!({"nodes":total_nodes.saturating_sub(nodes.len() as u64),"links":total_links.saturating_sub(links.len() as u64)});
        value["truncated"] =
            json!(eligible_nodes > nodes.len() as u64 || eligible_links > links.len() as u64);
        value["limits"] = json!({"nodes":limit,"links":MAX_GRAPH_LINKS,"response_bytes":MAX_RESPONSE_BYTES,"byte_limited":byte_limited});
        value["nodes"] = json!(nodes);
        value["links"] = json!(links);
        if serde_json::to_vec(&value)
            .map_err(|_| Error::Storage)?
            .len()
            <= MAX_RESPONSE_BYTES
        {
            return Ok(value);
        }
        byte_limited = true;
        if !links.is_empty() {
            links.truncate(links.len() / 2);
        } else if nodes.len() > 1 {
            nodes.truncate(nodes.len() / 2);
            let ids = nodes
                .iter()
                .filter_map(|n| n["id"].as_str())
                .collect::<HashSet<_>>();
            links.retain(|l| {
                l["source"].as_str().is_some_and(|id| ids.contains(id))
                    && l["target"].as_str().is_some_and(|id| ids.contains(id))
            });
        } else {
            return Err(Error::Limit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{context::inventory, domain::ImportedRecord, domain::SourceKind, store::digest};
    use sqlx::ConnectOptions;

    #[tokio::test]
    async fn graph_decodes_current_projections_once_within_scope() {
        let url = std::env::var("TEST_DATABASE_URL").expect("explicit isolated PostgreSQL");
        assert!(
            crate::config::database_options(&url)
                .unwrap()
                .get_database()
                .is_some_and(|name| name.starts_with("ontology_test_"))
        );
        // Native Harness unit fixtures deliberately hold the shared context gate.
        // Give this workload its own DB inside verify.sh's disposable server.
        let admin = Store::connect(&url).await.unwrap();
        let database = format!("ontology_test_graph_{}", Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {database}")))
            .execute(admin.pool())
            .await
            .unwrap();
        let options = crate::config::database_options(&url)
            .unwrap()
            .database(&database);
        let store = Store::connect(options.to_url_lossy().as_str())
            .await
            .unwrap();
        store.initialize().await.unwrap();
        let original = ImportedRecord {
            source_id: format!("s_{}", digest(b"graph-reference-scope-source")),
            entity_id: format!("e_{}", digest(b"graph-reference-scope-entity")),
            scope: Scope::Meenseek,
            repository: "/synthetic/references".into(),
            path: "index.md".into(),
            kind: SourceKind::Git,
            source_revision: "a".repeat(40),
            digest: Some(digest(b"# Index\n")),
            content: Some("# Index\n".into()),
        };
        store.apply_import(&[original]).await.unwrap();
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("personal/notes")).unwrap();
        for i in 0..24 {
            std::fs::write(
                root.join(format!("personal/notes/{i}.md")),
                format!("# Reference {i}\n[Next]({}.md)\n", (i + 1) % 24),
            )
            .unwrap();
        }
        let scopes = ["personal".parse().unwrap()];
        let expected = inventory(&root, &scopes).unwrap();
        store
            .import_context(&root, &scopes, &expected.inventory_digest)
            .await
            .unwrap();
        // An old projection must not add decoding work or reintroduce old links.
        store
            .edit_context(
                &scopes[0],
                "notes/0.md",
                1,
                &digest(b"# Reference 0\n[Next](1.md)\n"),
                &format!(
                    "# Current reference\n[Next](1.md)\n{}",
                    "synthetic body ".repeat(2000)
                ),
            )
            .await
            .unwrap();
        fn function_rows(plan: &Value, function: &str) -> u64 {
            let own = if plan["Function Name"] == function {
                (plan["Actual Rows"].as_f64().unwrap() * plan["Actual Loops"].as_f64().unwrap())
                    .round() as u64
            } else {
                0
            };
            own + plan["Plans"].as_array().map_or(0, |children| {
                children
                    .iter()
                    .map(|child| function_rows(child, function))
                    .sum()
            })
        }
        for (scope, expected_decodes) in [("meenseek", 0), ("personal", 24)] {
            // Both SQL fragments are source constants; fixture values remain bound.
            let plan: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "EXPLAIN (ANALYZE, FORMAT JSON) {GRAPH_SQL}"
            )))
            .bind(scope)
            .bind("")
            .bind(None::<String>)
            .bind(1_i64)
            .bind(MAX_GRAPH_LINKS as i64)
            .fetch_one(store.pool())
            .await
            .unwrap();
            assert_eq!(
                function_rows(&plan[0]["Plan"], "jsonb_to_record"),
                expected_decodes,
                "decode each current in-scope projection once, independent of its history"
            );
            if scope == "meenseek" {
                assert_eq!(
                    function_rows(&plan[0]["Plan"], "jsonb_array_elements_text"),
                    0,
                    "unbound native references cannot consume the company graph/health budget"
                );
            }
        }
        let personal = store
            .graph(GraphQuery {
                scope: Scope::Personal,
                q: String::new(),
                focus: None,
                limit: 100,
            })
            .await
            .unwrap();
        assert_eq!(
            personal["totals"]["links"], 24,
            "visible native reference semantics are retained"
        );
        store.pool().close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!("DROP DATABASE {database}")))
            .execute(admin.pool())
            .await
            .unwrap();
        admin.pool().close().await;
    }
}
