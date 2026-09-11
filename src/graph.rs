//! Read-only, bounded graph projection. Every request observes one PostgreSQL snapshot.
use crate::{
    domain::{AREAS, Error, MAX_RESPONSE_BYTES, Scope, validate_id, validate_search},
    memory::evidence_current,
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
            } else if let Some(uuid) = id.strip_prefix("m_").or_else(|| id.strip_prefix("p_")) {
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

const GRAPH_SQL: &str = concat!(
    r#"WITH evidence AS MATERIALIZED (
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
), nodes AS MATERIALIZED (
 SELECT e.id,'document' AS kind,
 jsonb_build_object('id',e.id,'scope',e.scope,'kind','document','label',s.path,'repository',s.repository,
 'revision',e.revision::text,'content_digest',p.content_digest,'source_revision',p.source_revision,
 'generation',s.generation::text,'status',s.status,'present',p.present,'source_kind',s.kind,
 'last_success_at',s.last_success_at,'observed_at',p.observed_at) AS value,
 ($2='' OR strpos(lower(concat(s.path,' ',p.content)),lower($2))>0 OR EXISTS(
 SELECT 1 FROM entity_topics et JOIN topics t ON t.scope=et.scope AND t.id=et.topic_id
 WHERE et.scope=e.scope AND et.entity_id=e.id AND strpos(lower(t.name),lower($2))>0)) AS matched
 FROM entities e JOIN sources s ON s.scope=e.scope AND s.id=e.source_id
 JOIN source_records p ON p.scope=e.scope AND p.entity_id=e.id WHERE e.scope=$1
 UNION ALL
 SELECT m.id,'memory',jsonb_build_object('id',m.id,'scope',m.scope,'kind','memory',
 'label',m.document->>'title','revision',m.revision::text,'status',m.status,
 'memory_kind',m.document->>'kind','subject_id',m.subject_id,
 'temporal',CASE WHEN (m.document->>'effective_from')::bigint>extract(epoch FROM now()) THEN 'future'
 WHEN (m.document->>'effective_until')::bigint<=extract(epoch FROM now()) THEN 'expired' ELSE 'current' END,
 'supported',stale.memory_id IS NULL,
 'support',CASE WHEN jsonb_array_length(m.document->'evidence')=0 THEN 'user-recorded' ELSE 'source-linked' END),
 ($2='' OR strpos(lower(concat(m.document->>'title',' ',m.document->>'body')),lower($2))>0)
 FROM memories m LEFT JOIN stale_memories stale ON stale.memory_id=m.id WHERE m.scope=$1
 UNION ALL
 SELECT 't_'||t.id,'topic',jsonb_build_object('id','t_'||t.id,'scope',t.scope,'kind','topic','label',t.name),
 ($2='' OR strpos(lower(t.name),lower($2))>0)
 FROM topics t WHERE t.scope=$1 AND EXISTS(SELECT 1 FROM entity_topics et WHERE et.scope=t.scope AND et.topic_id=t.id)
 UNION ALL
 SELECT p.id,'subject',jsonb_build_object('id',p.id,'scope',p.scope,'kind','subject','label',p.name),
 ($2='' OR strpos(lower(p.name),lower($2))>0) FROM subjects p WHERE p.scope=$1
 UNION ALL
 SELECT 'a_'||a.id,'area',jsonb_build_object('id','a_'||a.id,'scope',$1,'kind','area','label',a.label),
 ($2='' OR strpos(lower(a.label),lower($2))>0) FROM areas a
 WHERE $1='meenseek' AND EXISTS(SELECT 1 FROM entity_areas ea WHERE ea.scope=$1 AND ea.area=a.id)
), links AS MATERIALIZED (
 SELECT left_id AS source,right_id AS target,'related' AS kind,true AS current
 FROM related_materials WHERE scope=$1
 UNION ALL SELECT memory_id,entity_id,'evidence',current FROM evidence
 UNION ALL SELECT et.entity_id,'t_'||et.topic_id,'topic',true FROM entity_topics et WHERE et.scope=$1
 UNION ALL SELECT m.id,m.subject_id,'subject',true FROM memories m WHERE m.scope=$1 AND m.subject_id IS NOT NULL
 UNION ALL SELECT ea.entity_id,'a_'||ea.area,'area',true FROM entity_areas ea WHERE ea.scope=$1
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
 ORDER BY focused DESC,neighbor DESC,kind IN ('document','memory') DESC,eligible.id LIMIT $4
), eligible_links AS MATERIALIZED (
 SELECT l.* FROM scoped_links l JOIN eligible s ON s.id=l.source JOIN eligible t ON t.id=l.target
), selected_links AS (
 SELECT l.* FROM eligible_links l JOIN selected s ON s.id=l.source JOIN selected t ON t.id=l.target
 ORDER BY (l.source=$3 OR l.target=$3) DESC NULLS LAST,l.kind,l.source,l.target LIMIT $5
)
SELECT jsonb_build_object('scope',$1,'query',$2,
 'focus',jsonb_build_object('id',$3,'found',EXISTS(SELECT 1 FROM nodes WHERE id=$3)),
 'nodes',COALESCE((SELECT jsonb_agg(value||jsonb_build_object('relation_digest',relation_digest) ORDER BY focused DESC,neighbor DESC,kind IN ('document','memory') DESC,id) FROM selected),'[]'::jsonb),
 'links',COALESCE((SELECT jsonb_agg(to_jsonb(l)) FROM selected_links l),'[]'::jsonb),
 'totals',jsonb_build_object('documents',(SELECT count(*) FROM nodes WHERE kind='document'),
 'memories',(SELECT count(*) FROM nodes WHERE kind='memory'),'markers',(SELECT count(*) FROM nodes WHERE kind NOT IN ('document','memory')),
 'links',(SELECT count(*) FROM scoped_links)),
 'matched',(SELECT count(*) FROM nodes WHERE matched),
 'eligible',jsonb_build_object('nodes',(SELECT count(*) FROM eligible),'links',(SELECT count(*) FROM eligible_links)))"#
);
impl Store {
    pub async fn graph(&self, query: GraphQuery) -> Result<Value, Error> {
        query.validate()?;
        self.count(1);
        let value: Value = sqlx::query_scalar(GRAPH_SQL)
            .bind(query.scope.as_str())
            .bind(&query.q)
            .bind(&query.focus)
            .bind(query.limit as i64)
            .bind(MAX_GRAPH_LINKS as i64)
            .fetch_one(self.pool())
            .await
            .map_err(|_| Error::Storage)?;
        bound_response(value, query.limit)
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
