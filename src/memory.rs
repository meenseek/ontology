// One predicate owns evidence freshness for memory retrieval and the derived graph.
// The aliases m/x/e/s/p are owner memory, evidence, entity, source and projection.
macro_rules! evidence_current {
    () => {
        "CASE WHEN left(x->>'entity_id',2)='m_' THEN EXISTS(SELECT 1 FROM memories native WHERE native.scope=m.scope AND native.id=x->>'entity_id' AND native.id=x->>'source_id' AND native.status='accepted' AND native.revision=(x->>'generation')::bigint AND (native.document->>'effective_from' IS NULL OR (native.document->>'effective_from')::bigint<=extract(epoch FROM now())) AND (native.document->>'effective_until' IS NULL OR (native.document->>'effective_until')::bigint>extract(epoch FROM now())) AND x->>'source_revision'=x->>'content_digest' AND encode(sha256(convert_to(native.document->>'body','UTF8')),'hex')=x->>'content_digest') ELSE COALESCE(s.status='ok' AND p.present AND p.source_revision=s.verified_revision AND s.id=x->>'source_id' AND p.source_revision=x->>'source_revision' AND p.content_digest=x->>'content_digest' AND s.generation=(x->>'generation')::bigint,false) END"
    };
}
pub(crate) use evidence_current;
macro_rules! memory_query { ($tail:literal) => { concat!(r#"WITH visible AS (
 SELECT m.*, ((document->>'effective_from' IS NULL OR (document->>'effective_from')::bigint<=extract(epoch FROM now())) AND (document->>'effective_until' IS NULL OR (document->>'effective_until')::bigint>extract(epoch FROM now()))) AS effective,
 NOT EXISTS(SELECT 1 FROM jsonb_array_elements(document->'evidence') x LEFT JOIN entities e ON e.scope=m.scope AND e.id=x->>'entity_id' LEFT JOIN sources s ON s.scope=e.scope AND s.id=e.source_id LEFT JOIN source_records p ON p.scope=e.scope AND p.entity_id=e.id WHERE NOT "#, evidence_current!(), r#") AS supported,
 (to_jsonb(m)-'document') || document || jsonb_build_object('subject_name',(SELECT name FROM subjects WHERE scope=m.scope AND id=m.subject_id),'grouping',(SELECT jsonb_build_object('mode',g.mode,'state',g.state,'suggestions',g.suggestions,'reason',g.reason) FROM memory_grouping g WHERE g.memory_id=m.id),'evidence',COALESCE((SELECT jsonb_agg(x || jsonb_build_object('current',"#, evidence_current!(), r#")) FROM jsonb_array_elements(document->'evidence') x LEFT JOIN entities e ON e.scope=m.scope AND e.id=x->>'entity_id' LEFT JOIN sources s ON s.scope=e.scope AND s.id=e.source_id LEFT JOIN source_records p ON p.scope=e.scope AND p.entity_id=e.id),'[]'::jsonb),'support',CASE WHEN jsonb_array_length(document->'evidence')=0 THEN 'user-recorded' ELSE 'source-linked' END) AS value
 FROM memories m WHERE m.scope=$1
)"#, $tail) }; }
use crate::{
    domain::{Error, Scope, validate_id, validate_search},
    store::{Store, digest},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

pub const MAX_INPUT_BYTES: usize = 16_384;
const MAX_REVISION: i64 = 9_007_199_254_740_990;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryKind {
    #[default]
    Record,
    Fact,
    Decision,
    Preference,
    Idea,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    pub entity_id: String,
    pub source_revision: String,
    pub content_digest: String,
    pub generation: i64,
}
impl EvidenceRef {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        evidence_id(&self.entity_id)?;
        if self.generation < 0
            || !matches!(self.source_revision.len(), 40 | 64)
            || !self
                .source_revision
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || self.content_digest.len() != 64
            || !self
                .content_digest
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(Error::Invalid);
        }
        if self.entity_id.starts_with("m_") {
            revision_valid(self.generation)?;
            if self.source_revision != self.content_digest {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }

    pub(crate) fn from_metadata(value: &Value) -> Result<Self, Error> {
        serde_json::from_value(json!({"entity_id":value["entity_id"],"source_revision":value["source_revision"],"content_digest":value["content_digest"],"generation":value["generation"]})).map_err(|_| Error::Storage)
    }
}

pub(crate) struct PrepareMemory<'a> {
    pub origin: &'a str,
    pub title_from_body: bool,
    pub target_id: Option<&'a str>,
    pub persist_evidence: bool,
    pub curation: Option<Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryInput {
    #[serde(default)]
    pub kind: MemoryKind,
    #[serde(default)]
    pub title: String,
    pub body: String,
    pub subject_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grouping_preference: Option<GroupingPreference>,
    pub effective_from: Option<i64>,
    pub effective_until: Option<i64>,
    #[serde(default)]
    pub evidence: Vec<EvidenceRef>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupingPreference {
    Auto,
    Off,
    Manual,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryStatus {
    Accepted,
    Proposed,
    Withdrawn,
}
impl MemoryStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Proposed => "proposed",
            Self::Withdrawn => "withdrawn",
        }
    }
}
fn default_limit() -> usize {
    20
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
pub enum BrainCommand {
    Curation {
        scope: Scope,
        command: crate::curation::CurationCommand,
    },
    Remember {
        scope: Scope,
        idempotency_key: String,
        memory: MemoryInput,
    },
    Propose {
        scope: Scope,
        idempotency_key: String,
        memory: MemoryInput,
    },
    Correct {
        scope: Scope,
        id: String,
        revision: i64,
        memory: MemoryInput,
    },
    Accept {
        scope: Scope,
        id: String,
        revision: i64,
    },
    Withdraw {
        scope: Scope,
        id: String,
        revision: i64,
    },
    Forget {
        scope: Scope,
        id: String,
        revision: i64,
    },
    Read {
        scope: Scope,
        id: String,
    },
    History {
        scope: Scope,
        id: String,
        before_revision: Option<i64>,
        #[serde(default = "default_limit")]
        limit: usize,
    },
    List {
        scope: Scope,
        status: Option<MemoryStatus>,
        subject_id: Option<String>,
        #[serde(default)]
        query: String,
        after: Option<String>,
        #[serde(default = "default_limit")]
        limit: usize,
    },
    Search {
        scope: Scope,
        #[serde(default)]
        query: String,
        #[serde(default = "default_limit")]
        limit: usize,
    },
    EvidenceRead {
        scope: Scope,
        id: String,
        revision: i64,
        entity_id: String,
    },
    SubjectCreate {
        scope: Scope,
        idempotency_key: String,
        name: String,
    },
    Subjects {
        scope: Scope,
        #[serde(default)]
        query: String,
        after: Option<String>,
        #[serde(default = "default_limit")]
        limit: usize,
    },
    Evidence {
        scope: Scope,
        #[serde(default)]
        query: String,
        #[serde(default = "default_limit")]
        limit: usize,
    },
    GroupingRetry {
        scope: Scope,
        id: String,
    },
    GroupingSet {
        scope: Scope,
        id: String,
        revision: i64,
        subject_id: Option<String>,
        mode: GroupingPreference,
    },
}
pub(crate) fn text_valid(text: &str, bytes: usize, multiline: bool) -> bool {
    !text.trim().is_empty()
        && text.len() <= bytes
        && !text
            .chars()
            .any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\t')))
}
pub(crate) fn key_valid(key: &str) -> Result<(), Error> {
    if !(8..=128).contains(&key.len())
        || !key
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
pub(crate) fn native_id(id: &str, prefix: &str) -> Result<(), Error> {
    if id.len() != 38 || !id.starts_with(prefix) || Uuid::parse_str(&id[2..]).is_err() {
        return Err(Error::Invalid);
    }
    Ok(())
}
pub(crate) fn evidence_id(id: &str) -> Result<(), Error> {
    if id.starts_with("m_") {
        native_id(id, "m_")
    } else {
        validate_id(id)
    }
}
pub(crate) fn revision_valid(revision: i64) -> Result<(), Error> {
    if !(1..=MAX_REVISION).contains(&revision) {
        Err(Error::Invalid)
    } else {
        Ok(())
    }
}
fn limit_valid(limit: usize) -> Result<(), Error> {
    if !(1..=20).contains(&limit) {
        Err(Error::Invalid)
    } else {
        Ok(())
    }
}
impl MemoryInput {
    pub(crate) fn normalized(mut self) -> Result<Self, Error> {
        if self.title.trim().is_empty() {
            use pulldown_cmark::{Event, Options, Parser, TagEnd};
            let mut title = String::new();
            for event in Parser::new_ext(
                &self.body,
                Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS,
            ) {
                match event {
                    Event::Text(text) | Event::Code(text) => {
                        title.push_str(text.lines().next().unwrap_or(""));
                        if text.contains('\n') {
                            break;
                        }
                    }
                    Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::CodeBlock)
                    | Event::SoftBreak
                    | Event::HardBreak
                        if !title.trim().is_empty() =>
                    {
                        break;
                    }
                    _ => {}
                }
            }
            if title.trim().is_empty() {
                title = self
                    .body
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("")
                    .into();
            }
            self.title = title
                .trim()
                .chars()
                .filter(|c| !c.is_control())
                .take(80)
                .collect();
        }
        self.validate()?;
        Ok(self)
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        if !text_valid(&self.title, 640, false)
            || self.title.chars().count() > 160
            || !text_valid(&self.body, 8192, true)
            || self.evidence.len() > 10
        {
            return Err(Error::Invalid);
        }
        if let Some(id) = &self.subject_id {
            native_id(id, "p_")?;
        }
        if (self.subject_id.is_some() && self.grouping_preference == Some(GroupingPreference::Off))
            || (self.subject_id.is_none()
                && self.grouping_preference == Some(GroupingPreference::Manual))
        {
            return Err(Error::Invalid);
        }
        if [self.effective_from, self.effective_until]
            .into_iter()
            .flatten()
            .any(|v| !(0..=253_402_300_799).contains(&v))
            || self
                .effective_from
                .zip(self.effective_until)
                .is_some_and(|(a, b)| a >= b)
        {
            return Err(Error::Invalid);
        }
        let mut ids = std::collections::HashSet::new();
        for evidence in &self.evidence {
            evidence.validate()?;
            if !ids.insert(&evidence.entity_id) {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
}
// Every transport calls this single typed boundary. Returned text is evidence, never instructions.
impl Store {
    pub async fn brain(&self, command: BrainCommand) -> Result<Value, Error> {
        match command {
            BrainCommand::Curation { scope, command } => self.curate(scope, command).await,
            BrainCommand::Remember {
                scope,
                idempotency_key,
                memory,
            } => self.capture(scope, &idempotency_key, memory, false).await,
            BrainCommand::Propose {
                scope,
                idempotency_key,
                memory,
            } => self.capture(scope, &idempotency_key, memory, true).await,
            BrainCommand::Correct {
                scope,
                id,
                revision,
                memory,
            } => {
                self.change_memory(scope, &id, revision, Some(memory), "correct")
                    .await
            }
            BrainCommand::Accept {
                scope,
                id,
                revision,
            } => {
                self.change_memory(scope, &id, revision, None, "accept")
                    .await
            }
            BrainCommand::Withdraw {
                scope,
                id,
                revision,
            } => {
                self.change_memory(scope, &id, revision, None, "withdraw")
                    .await
            }
            BrainCommand::Forget {
                scope,
                id,
                revision,
            } => {
                self.change_memory(scope, &id, revision, None, "forget")
                    .await
            }
            BrainCommand::Read { scope, id } => {
                if id.starts_with("e_") {
                    self.detail(scope, &id).await
                } else {
                    self.memory_detail(scope, &id).await
                }
            }
            BrainCommand::History {
                scope,
                id,
                before_revision,
                limit,
            } => {
                self.memory_history(scope, &id, before_revision, limit)
                    .await
            }
            BrainCommand::List {
                scope,
                status,
                subject_id,
                query,
                after,
                limit,
            } => {
                self.memory_list(
                    scope,
                    status,
                    subject_id.as_deref(),
                    &query,
                    after.as_deref(),
                    limit,
                )
                .await
            }
            BrainCommand::Search {
                scope,
                query,
                limit,
            } => {
                limit_valid(limit)?;
                self.graph(crate::graph::GraphQuery {
                    scope,
                    q: query,
                    focus: None,
                    limit,
                })
                .await
            }
            BrainCommand::EvidenceRead {
                scope,
                id,
                revision,
                entity_id,
            } => self.evidence_read(scope, &id, revision, &entity_id).await,
            BrainCommand::SubjectCreate {
                scope,
                idempotency_key,
                name,
            } => self.subject_create(scope, &idempotency_key, &name).await,
            BrainCommand::Subjects {
                scope,
                query,
                after,
                limit,
            } => self.subjects(scope, &query, after.as_deref(), limit).await,
            BrainCommand::Evidence {
                scope,
                query,
                limit,
            } => self.evidence_options(scope, &query, limit).await,
            BrainCommand::GroupingRetry { scope, id } => self.grouping_retry(scope, &id).await,
            BrainCommand::GroupingSet {
                scope,
                id,
                revision,
                subject_id,
                mode,
            } => {
                self.grouping_set(scope, &id, revision, subject_id.as_deref(), mode)
                    .await
            }
        }
    }
    async fn subject_create(&self, scope: Scope, key: &str, name: &str) -> Result<Value, Error> {
        key_valid(key)?;
        if !text_valid(name, 320, false) || name.trim() != name {
            return Err(Error::Invalid);
        }
        self.count(1);
        let value: Value = sqlx::query_scalar("INSERT INTO subjects(id,scope,name,key_digest,payload_digest) VALUES($1,$2,$3,$4,$5) ON CONFLICT(scope,key_digest) DO UPDATE SET key_digest=EXCLUDED.key_digest WHERE subjects.payload_digest=EXCLUDED.payload_digest RETURNING jsonb_build_object('id',id,'scope',scope,'name',name,'created_at',created_at)")
            .bind(format!("p_{}",Uuid::new_v4())).bind(scope.as_str()).bind(name).bind(digest(key.as_bytes())).bind(digest(name.as_bytes())).fetch_optional(self.pool()).await.map_err(|_|Error::Storage)?.ok_or(Error::Conflict)?;
        Ok(value)
    }
    async fn subjects(
        &self,
        scope: Scope,
        query: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Value, Error> {
        validate_search(query)?;
        limit_valid(limit)?;
        if let Some(id) = after {
            native_id(id, "p_")?;
        }
        self.count(1);
        let rows:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',id,'scope',scope,'name',name,'created_at',created_at) FROM subjects WHERE scope=$1 AND strpos(lower(name),lower($2))>0 AND ($3::text IS NULL OR id>$3) ORDER BY id LIMIT $4")
            .bind(scope.as_str()).bind(query).bind(after).bind((limit+1) as i64).fetch_all(self.pool()).await.map_err(|_|Error::Storage)?;
        Ok(page(rows, limit))
    }
    async fn evidence_options(
        &self,
        scope: Scope,
        query: &str,
        limit: usize,
    ) -> Result<Value, Error> {
        validate_search(query)?;
        limit_valid(limit)?;
        self.count(1);
        let rows:Vec<Value>=sqlx::query_scalar(memory_query!(r#"SELECT option FROM (
 SELECT e.id,s.path,false AS native,jsonb_build_object('entity_id',e.id,'source_id',s.id,'kind',s.kind,'path',s.path,'repository',s.repository,'source_revision',p.source_revision,'content_digest',p.content_digest,'generation',s.generation) AS option
 FROM entities e JOIN sources s ON s.scope=e.scope AND s.id=e.source_id JOIN source_records p ON p.scope=e.scope AND p.entity_id=e.id
 WHERE e.scope=$1 AND s.status='ok' AND p.present AND p.content_digest IS NOT NULL AND p.source_revision=s.verified_revision AND strpos(lower(s.path),lower($2))>0
 UNION ALL
 SELECT m.id,m.document->>'title',true,jsonb_build_object('entity_id',m.id,'source_id',m.id,'kind','record','path',m.document->>'title','repository','분신','source_revision',encode(sha256(convert_to(m.document->>'body','UTF8')),'hex'),'content_digest',encode(sha256(convert_to(m.document->>'body','UTF8')),'hex'),'generation',m.revision)
 FROM visible m WHERE m.scope=$1 AND m.status='accepted' AND m.effective AND m.supported AND strpos(lower(concat(m.document->>'title',' ',m.document->>'body')),lower($2))>0
 ) candidates ORDER BY native,path,id LIMIT $3"#))
            .bind(scope.as_str()).bind(query).bind(limit as i64).fetch_all(self.pool()).await.map_err(|_|Error::Storage)?;
        Ok(json!({"items":rows,"limit":limit}))
    }
    pub(crate) async fn prepare_memory(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        scope: Scope,
        input: &MemoryInput,
        options: PrepareMemory<'_>,
    ) -> Result<Value, Error> {
        let observation = self.dependency_start("prepare-memory", &(scope, input))?;
        let result = async {
        input.validate()?;
        if let Some(subject) = &input.subject_id {
            self.count(1);
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM subjects WHERE scope=$1 AND id=$2)",
            )
            .bind(scope.as_str())
            .bind(subject)
            .fetch_one(&mut **tx)
            .await
            .map_err(|_| Error::Storage)?;
            if !exists {
                return Err(Error::Invalid);
            }
        }
        // A native record already contains its complete flat basis. Read direct native
        // records and that basis together, under the caller's scope mutation lock.
        let native_ids: Vec<_> = input
            .evidence
            .iter()
            .filter(|e| e.entity_id.starts_with("m_"))
            .map(|e| e.entity_id.as_str())
            .collect();
        let native_rows = if native_ids.is_empty() {
            Vec::new()
        } else {
            self.count(1);
            sqlx::query(r#"WITH requested AS MATERIALIZED (SELECT document FROM memories WHERE scope=$1 AND id=ANY($2))
 SELECT n.id,n.revision,n.status,n.document,
 (n.document->>'effective_from' IS NULL OR (n.document->>'effective_from')::bigint<=extract(epoch FROM now())) AND (n.document->>'effective_until' IS NULL OR (n.document->>'effective_until')::bigint>extract(epoch FROM now())) AS effective
 FROM memories n WHERE n.scope=$1 AND (n.id=ANY($2) OR n.id IN (
 SELECT x->>'entity_id' FROM requested r CROSS JOIN LATERAL jsonb_array_elements(r.document->'evidence') x WHERE left(x->>'entity_id',2)='m_')) ORDER BY n.id FOR SHARE OF n"#)
                .bind(scope.as_str()).bind(native_ids).fetch_all(&mut **tx).await.map_err(|_| Error::Storage)?
        };
        let mut references = input.evidence.clone();
        let mut index = 0;
        while index < references.len() {
            let expected = references[index].clone();
            expected.validate()?;
            if options.target_id == Some(expected.entity_id.as_str()) {
                return Err(Error::Invalid);
            }
            if expected.entity_id.starts_with("m_") {
                let row = native_rows
                    .iter()
                    .find(|r| r.get::<String, _>("id") == expected.entity_id)
                    .ok_or(Error::Invalid)?;
                let document: Value = row.get("document");
                let body = document["body"].as_str().ok_or(Error::Storage)?;
                if row.get::<String, _>("status") != "accepted"
                    || !row.get::<bool, _>("effective")
                    || row.get::<i64, _>("revision") != expected.generation
                    || digest(body.as_bytes()) != expected.content_digest
                {
                    return Err(Error::Conflict);
                }
                for value in document["evidence"].as_array().ok_or(Error::Storage)? {
                    let dependency = EvidenceRef::from_metadata(value)?;
                    dependency.validate()?;
                    if let Some(existing) = references
                        .iter()
                        .find(|r| r.entity_id == dependency.entity_id)
                    {
                        if existing != &dependency {
                            return Err(Error::Conflict);
                        }
                    } else {
                        if references.len() == 10 {
                            return Err(Error::Invalid);
                        }
                        references.push(dependency);
                    }
                }
            }
            index += 1;
        }
        let source_ids: Vec<_> = references
            .iter()
            .filter(|e| !e.entity_id.starts_with("m_"))
            .map(|e| e.entity_id.as_str())
            .collect();
        let rows = if source_ids.is_empty() {
            Vec::new()
        } else {
            let observation =
                self.dependency_start("memory-source-rows", &(scope.as_str(), &source_ids))?;
            self.count(1);
            let result=sqlx::query("SELECT e.id,s.id AS source_id,s.kind,s.repository,s.path,s.status,s.generation,s.verified_revision,p.present,p.source_revision,p.content_digest,p.content FROM entities e JOIN sources s ON s.scope=e.scope AND s.id=e.source_id JOIN source_records p ON p.scope=e.scope AND p.entity_id=e.id WHERE e.scope=$1 AND e.id=ANY($2) ORDER BY s.id FOR SHARE OF s,p")
                .bind(scope.as_str()).bind(source_ids).fetch_all(&mut **tx).await.map_err(|_|Error::Storage);
            self.dependency_finish_rows(observation, &result);
            result?
        };
        let mut evidence = Vec::new();
        let mut contents = Vec::new();
        for expected in &references {
            if expected.entity_id.starts_with("m_") {
                let row = native_rows
                    .iter()
                    .find(|r| r.get::<String, _>("id") == expected.entity_id)
                    .ok_or(Error::Invalid)?;
                let document: Value = row.get("document");
                evidence.push(json!({"entity_id":expected.entity_id,"source_id":expected.entity_id,"kind":"record","repository":"분신","path":document["title"],"semantics":{"kind":document["kind"],"origin":document["origin"],"title_from_body":document["title_from_body"],"effective_from":document["effective_from"],"effective_until":document["effective_until"],"applicability":document["curation"]["applicability"]},"source_revision":expected.source_revision,"content_digest":expected.content_digest,"generation":expected.generation}));
                contents.push(json!({"digest":expected.content_digest,"content":document["body"]}));
                continue;
            }
            let row = rows
                .iter()
                .find(|r| r.get::<String, _>("id") == expected.entity_id)
                .ok_or(Error::Invalid)?;
            if row.get::<String, _>("status") != "ok"
                || !row.get::<bool, _>("present")
                || row.get::<Option<String>, _>("source_revision")
                    != row.get::<Option<String>, _>("verified_revision")
                || row.get::<Option<String>, _>("source_revision").as_deref()
                    != Some(&expected.source_revision)
                || row.get::<Option<String>, _>("content_digest").as_deref()
                    != Some(&expected.content_digest)
                || row.get::<i64, _>("generation") != expected.generation
            {
                return Err(Error::Conflict);
            }
            evidence.push(json!({"entity_id":expected.entity_id,"source_id":row.get::<String,_>("source_id"),"kind":row.get::<String,_>("kind"),"repository":row.get::<String,_>("repository"),"path":row.get::<String,_>("path"),"source_revision":expected.source_revision,"content_digest":expected.content_digest,"generation":expected.generation}));
            contents.push(json!({"digest":expected.content_digest,"content":row.get::<Option<String>,_>("content").ok_or(Error::Storage)?}));
        }
        let mut document = json!({"kind":input.kind,"title":input.title,"title_from_body":options.title_from_body,"body":input.body,"origin":options.origin,"effective_from":input.effective_from,"effective_until":input.effective_until,"evidence":evidence});
        if let Some(curation) = options.curation {
            document["curation"] = curation;
        }
        self.check_memory_size(tx, &document).await?;
        if options.persist_evidence && !contents.is_empty() {
            self.count(1);
            sqlx::query("INSERT INTO evidence_contents(scope,digest,content) SELECT DISTINCT $1,digest,content FROM jsonb_to_recordset($2) AS x(digest text,content text) ON CONFLICT DO NOTHING")
                .bind(scope.as_str()).bind(json!(contents)).execute(&mut **tx).await.map_err(|_|Error::Storage)?;
        }
        Ok(document)
        }.await;
        self.dependency_finish(observation, &result);
        result
    }
    async fn check_memory_size(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        document: &Value,
    ) -> Result<(), Error> {
        // Match the database constraint, including JSONB spacing and escaped text.
        self.count(1);
        let fits: bool = sqlx::query_scalar("SELECT octet_length($1::jsonb::text) <= 24576")
            .bind(document)
            .fetch_one(&mut **tx)
            .await
            .map_err(|_| Error::Storage)?;
        if !fits {
            return Err(Error::Limit);
        }
        Ok(())
    }
    async fn capture(
        &self,
        scope: Scope,
        key: &str,
        input: MemoryInput,
        proposal: bool,
    ) -> Result<Value, Error> {
        key_valid(key)?;
        let title_from_body = input.title.trim().is_empty();
        let input = input.normalized()?;
        let key_hash = digest(key.as_bytes());
        // Empty title expresses automatic naming; keep that intent in the request identity.
        // Explicit titles retain the original serialized input and its existing idempotency digest.
        let mut identity_input = input.clone();
        if title_from_body {
            identity_input.title.clear();
        }
        let payload_hash =
            digest(&serde_json::to_vec(&(proposal, &identity_input)).map_err(|_| Error::Invalid)?);
        self.count(1);
        let mut tx = self.pool().begin().await.map_err(|_| Error::Storage)?;
        self.lock_memories(&mut tx, scope).await?;
        self.count(1);
        if let Some(row)=sqlx::query("SELECT memory_id,payload_digest FROM memory_creations WHERE scope=$1 AND key_digest=$2").bind(scope.as_str()).bind(&key_hash).fetch_optional(&mut *tx).await.map_err(|_|Error::Storage)? {
            if row.get::<String,_>("payload_digest")!=payload_hash { return Err(Error::Conflict); }
            let id=row.get::<String,_>("memory_id");
            self.count(1);tx.commit().await.map_err(|_|Error::Storage)?;
            return self.memory_detail(scope,&id).await.map_err(|e|if e==Error::NotFound { Error::Gone } else {e});
        }
        let document = self
            .prepare_memory(
                &mut tx,
                scope,
                &input,
                PrepareMemory {
                    origin: if proposal { "assistant" } else { "user" },
                    title_from_body,
                    target_id: None,
                    persist_evidence: true,
                    curation: None,
                },
            )
            .await?;
        let id = format!("m_{}", Uuid::new_v4());
        self.count(1);
        sqlx::query("INSERT INTO memory_creations(scope,key_digest,payload_digest,memory_id) VALUES($1,$2,$3,$4)").bind(scope.as_str()).bind(key_hash).bind(payload_hash).bind(&id).execute(&mut *tx).await.map_err(|_|Error::Storage)?;
        self.count(1);
        sqlx::query("INSERT INTO memories(id,scope,subject_id,revision,status,document) VALUES($1,$2,$3,1,$4,$5)").bind(&id).bind(scope.as_str()).bind(&input.subject_id).bind(if proposal {"proposed"} else {"accepted"}).bind(document).execute(&mut *tx).await.map_err(|_|Error::Storage)?;
        crate::grouping::on_capture(&mut tx, scope, &id, &input).await?;
        self.append_history(&mut tx, scope, &id).await?;
        self.pin_evidence(&mut tx, scope, &id, None).await?;
        self.count(1);
        tx.commit().await.map_err(|_| Error::Storage)?;
        self.memory_detail(scope, &id).await
    }
    pub(crate) async fn append_history(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        scope: Scope,
        id: &str,
    ) -> Result<(), Error> {
        self.count(1);
        sqlx::query("INSERT INTO memory_history(scope,memory_id,revision,status,subject_id,document) SELECT scope,id,revision,status,subject_id,document FROM memories WHERE scope=$1 AND id=$2").bind(scope.as_str()).bind(id).execute(&mut **tx).await.map_err(|_|Error::Storage)?;
        Ok(())
    }
    async fn revalidate_memory_evidence(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        scope: Scope,
        id: &str,
        document: &Value,
    ) -> Result<(), Error> {
        let observation = self.dependency_start(
            "memory-evidence-revalidate",
            &(scope, id, &document["evidence"]),
        )?;
        let result=async {
        let evidence = document["evidence"].as_array().ok_or(Error::Storage)?;
        if evidence.is_empty() {
            return Ok(());
        }
        let native: Vec<_> = evidence
            .iter()
            .filter_map(|e| e["entity_id"].as_str())
            .filter(|id| id.starts_with("m_"))
            .collect();
        let sources: Vec<_> = evidence
            .iter()
            .filter_map(|e| e["source_id"].as_str())
            .filter(|id| id.starts_with("s_"))
            .collect();
        if !native.is_empty() {
            let observation=self.dependency_start("accept-native-locks",&(scope.as_str(),&native))?;
            self.count(1);
            let result=sqlx::query("SELECT id FROM memories WHERE scope=$1 AND id=ANY($2) ORDER BY id FOR SHARE").bind(scope.as_str()).bind(native).fetch_all(&mut **tx).await.map_err(|_|Error::Storage);
            self.dependency_finish_rows(observation,&result);result?;
        }
        if !sources.is_empty() {
            let observation=self.dependency_start("accept-source-locks",&(scope.as_str(),&sources))?;
            self.count(1);
            let result=sqlx::query("SELECT s.id FROM sources s JOIN source_records p ON p.scope=s.scope JOIN entities e ON e.scope=s.scope AND e.source_id=s.id AND e.id=p.entity_id WHERE s.scope=$1 AND s.id=ANY($2) ORDER BY s.id FOR SHARE OF s,p").bind(scope.as_str()).bind(sources).fetch_all(&mut **tx).await.map_err(|_| Error::Storage);
            self.dependency_finish_rows(observation,&result);result?;
        }
        let current_observation = self.dependency_start("accept-current", &(scope.as_str(), id))?;
        self.count(1);
        let current: Result<bool, Error> = sqlx::query_scalar(concat!("SELECT NOT EXISTS(SELECT 1 FROM jsonb_array_elements(m.document->'evidence') x LEFT JOIN entities e ON e.scope=m.scope AND e.id=x->>'entity_id' LEFT JOIN sources s ON s.scope=e.scope AND s.id=e.source_id LEFT JOIN source_records p ON p.scope=e.scope AND p.entity_id=e.id WHERE NOT ", evidence_current!(), ") FROM memories m WHERE m.scope=$1 AND m.id=$2"))
            .bind(scope.as_str()).bind(id).fetch_one(&mut **tx).await.map_err(|_| Error::Storage);
        self.dependency_finish(current_observation, &current);
        if !current? {
            return Err(Error::Conflict);
        }
        Ok(())

        }.await;
        self.dependency_finish(observation, &result);
        result
    }
    async fn change_memory(
        &self,
        scope: Scope,
        id: &str,
        revision: i64,
        input: Option<MemoryInput>,
        action: &str,
    ) -> Result<Value, Error> {
        let observation =
            self.dependency_start("change-memory", &(scope, id, revision, &input, action))?;
        let result = async {
        native_id(id, "m_")?;
        revision_valid(revision)?;
        let title_from_body = input
            .as_ref()
            .is_some_and(|input| input.title.trim().is_empty());
        let input = input.map(MemoryInput::normalized).transpose()?;
        self.count(1);
        let mut tx = self.pool().begin().await.map_err(|_| Error::Storage)?;
        self.lock_memories(&mut tx, scope).await?;
        self.count(1);
        let row=sqlx::query("SELECT revision,status,subject_id,document FROM memories WHERE scope=$1 AND id=$2 FOR UPDATE").bind(scope.as_str()).bind(id).fetch_optional(&mut *tx).await.map_err(|_|Error::Storage)?;
        let Some(row) = row else {
            if action == "forget" {
                self.count(1);
                let gone: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM memory_creations WHERE scope=$1 AND memory_id=$2)",
                )
                .bind(scope.as_str())
                .bind(id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|_| Error::Storage)?;
                if gone {
                    return Ok(json!({"forgotten":true,"id":id}));
                }
            }
            return Err(Error::NotFound);
        };
        if row.get::<i64, _>("revision") != revision {
            return Err(Error::Conflict);
        }
        if action == "forget" {
            self.count(1);
            sqlx::query("WITH forgotten AS (UPDATE curation_reviews SET candidate=jsonb_build_object('source_id',source_id,'basis',candidate->'basis'),review=review-'reason',result=jsonb_build_object('outcome','forgotten','memory_id',$2,'review_id',id) WHERE scope=$1 AND result->>'memory_id'=$2 RETURNING id) DELETE FROM curation_reviews r WHERE r.scope=$1 AND r.id NOT IN (SELECT id FROM forgotten) AND (r.source_id=$2 OR r.candidate->'finding'->'target'->>'id'=$2 OR EXISTS(SELECT 1 FROM jsonb_array_elements(r.candidate->'basis') x WHERE x->>'entity_id'=$2))")
                .bind(scope.as_str()).bind(id).execute(&mut *tx).await.map_err(|_| Error::Storage)?;
            self.count(1);
            sqlx::query("WITH removed_references AS (DELETE FROM evidence_snapshots WHERE scope=$1 AND entity_id=$2) DELETE FROM memories WHERE scope=$1 AND id=$2")
                .bind(scope.as_str())
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|_| Error::Storage)?;
            self.count(1);
            sqlx::query("DELETE FROM evidence_contents c WHERE scope=$1 AND NOT EXISTS(SELECT 1 FROM evidence_snapshots r WHERE r.scope=c.scope AND r.digest=c.digest)")
                .bind(scope.as_str()).execute(&mut *tx).await.map_err(|_| Error::Storage)?;
            self.count(1);
            tx.commit().await.map_err(|_| Error::Storage)?;
            return Ok(json!({"forgotten":true,"id":id}));
        }
        let old_status = row.get::<String, _>("status");
        let mut document: Value = row.get("document");
        let mut subject_id: Option<String> = row.get("subject_id");
        let status = match action {
            "accept" if old_status == "proposed" => "accepted",
            "withdraw" if old_status != "withdrawn" => "withdrawn",
            "correct" if old_status != "withdrawn" => old_status.as_str(),
            _ => return Err(Error::Conflict),
        };
        if action == "accept" && input.is_none() {
            self.revalidate_memory_evidence(&mut tx, scope, id, &document)
                .await?;
        }
        if let Some(input) = input {
            document = self
                .prepare_memory(
                    &mut tx,
                    scope,
                    &input,
                    PrepareMemory {
                        origin: document["origin"].as_str().ok_or(Error::Storage)?,
                        title_from_body,
                        target_id: Some(id),
                        persist_evidence: true,
                        curation: None,
                    },
                )
                .await?;
            subject_id = crate::grouping::on_correct(
                &mut tx, scope, id, revision + 1, &input, subject_id.as_deref(),
            ).await?;
        }
        if action == "withdraw" {
            crate::grouping::on_withdraw(&mut tx, scope, id).await?;
        } else if action == "accept" {
            crate::grouping::on_accept(&mut tx, scope, id, revision + 1).await?;
        }
        self.count(1);
        sqlx::query("UPDATE memories SET revision=revision+1,status=$3,subject_id=$4,document=$5,updated_at=now() WHERE scope=$1 AND id=$2").bind(scope.as_str()).bind(id).bind(status).bind(subject_id).bind(document).execute(&mut *tx).await.map_err(|_|Error::Storage)?;
        self.append_history(&mut tx, scope, id).await?;
        self.pin_evidence(
            &mut tx,
            scope,
            id,
            if action == "correct" {
                None
            } else {
                Some(revision)
            },
        )
        .await?;
        self.count(1);
        tx.commit().await.map_err(|_| Error::Storage)?;
        self.memory_detail(scope, id).await
        }.await;
        self.dependency_finish(observation, &result);
        result
    }
    pub async fn memory_detail(&self, scope: Scope, id: &str) -> Result<Value, Error> {
        native_id(id, "m_")?;
        self.count(1);
        sqlx::query_scalar(memory_query!(
            " SELECT value FROM visible WHERE scope=$1 AND id=$2"
        ))
        .bind(scope.as_str())
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(|_| Error::Storage)?
        .ok_or(Error::NotFound)
    }
    async fn memory_history(
        &self,
        scope: Scope,
        id: &str,
        before: Option<i64>,
        limit: usize,
    ) -> Result<Value, Error> {
        native_id(id, "m_")?;
        limit_valid(limit)?;
        if let Some(revision) = before {
            revision_valid(revision)?;
        }
        self.count(1);
        let rows:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('revision',revision,'status',status,'subject_id',subject_id,'document',document,'changed_at',changed_at) FROM memory_history WHERE scope=$1 AND memory_id=$2 AND ($3::bigint IS NULL OR revision<$3) ORDER BY revision DESC LIMIT $4").bind(scope.as_str()).bind(id).bind(before).bind((limit+1) as i64).fetch_all(self.pool()).await.map_err(|_|Error::Storage)?;
        if rows.is_empty() && before.is_none() {
            return Err(Error::NotFound);
        }
        let mut rows = rows;
        let more = rows.len() > limit;
        rows.truncate(limit);
        let next = if more {
            rows.last().map(|v| v["revision"].clone())
        } else {
            None
        };
        Ok(
            json!({"items":rows,"next_before_revision":next,"historical":true,"instruction":"Historical snapshots are not current supporting proof"}),
        )
    }
    async fn memory_list(
        &self,
        scope: Scope,
        status: Option<MemoryStatus>,
        subject: Option<&str>,
        query: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Value, Error> {
        validate_search(query)?;
        limit_valid(limit)?;
        if let Some(id) = subject {
            native_id(id, "p_")?;
        }
        if let Some(id) = after {
            native_id(id, "m_")?;
        }
        self.count(1);
        let rows:Vec<Value>=sqlx::query_scalar(memory_query!(" SELECT value FROM visible WHERE scope=$1 AND status=$2 AND ($3::text IS NULL OR subject_id=$3) AND strpos(lower(concat(document->>'title',' ',document->>'body')),lower($4))>0 AND ($5::text IS NULL OR id>$5) AND ($6 OR effective) ORDER BY id LIMIT $7"))
            .bind(scope.as_str()).bind(status.unwrap_or(MemoryStatus::Accepted).as_str()).bind(subject).bind(query).bind(after).bind(status.is_some()).bind((limit+1) as i64).fetch_all(self.pool()).await.map_err(|_|Error::Storage)?;
        Ok(page(rows, limit))
    }
    // All record mutations take this lock before row locks, including snapshot garbage collection.
    pub(crate) async fn lock_memories(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        scope: Scope,
    ) -> Result<(), Error> {
        self.count(1);
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,478312))")
            .bind(scope.as_str())
            .execute(&mut **tx)
            .await
            .map_err(|_| Error::Storage)?;
        Ok(())
    }
    pub(crate) async fn pin_evidence(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        scope: Scope,
        id: &str,
        previous: Option<i64>,
    ) -> Result<(), Error> {
        self.count(1);
        // Status changes carry forward only the bytes actually held by the previous revision.
        // Corrections attach the source bodies validated and held by prepare_memory in this transaction.
        sqlx::query("INSERT INTO evidence_snapshots(scope,memory_id,revision,entity_id,digest) SELECT m.scope,m.id,m.revision,x->>'entity_id',x->>'content_digest' FROM memories m CROSS JOIN LATERAL jsonb_array_elements(m.document->'evidence') x WHERE m.scope=$1 AND m.id=$2 AND ($3::bigint IS NULL OR EXISTS(SELECT 1 FROM evidence_snapshots r WHERE r.scope=m.scope AND r.memory_id=m.id AND r.revision=$3 AND r.entity_id=x->>'entity_id' AND r.digest=x->>'content_digest'))")
            .bind(scope.as_str()).bind(id).bind(previous).execute(&mut **tx).await.map_err(|_| Error::Storage)?;
        Ok(())
    }
    async fn evidence_read(
        &self,
        scope: Scope,
        id: &str,
        revision: i64,
        entity_id: &str,
    ) -> Result<Value, Error> {
        native_id(id, "m_")?;
        revision_valid(revision)?;
        evidence_id(entity_id)?;
        self.count(1);
        sqlx::query_scalar("SELECT jsonb_build_object('memory_id',h.memory_id,'revision',h.revision,'evidence',x,'content',c.content,'available',c.content IS NOT NULL,'historical',true) FROM memory_history h CROSS JOIN LATERAL jsonb_array_elements(h.document->'evidence') x LEFT JOIN evidence_snapshots r ON r.scope=h.scope AND r.memory_id=h.memory_id AND r.revision=h.revision AND r.entity_id=x->>'entity_id' LEFT JOIN evidence_contents c ON c.scope=r.scope AND c.digest=r.digest WHERE h.scope=$1 AND h.memory_id=$2 AND h.revision=$3 AND x->>'entity_id'=$4")
            .bind(scope.as_str()).bind(id).bind(revision).bind(entity_id).fetch_optional(self.pool()).await.map_err(|_| Error::Storage)?.ok_or(Error::NotFound)
    }
}
fn page(mut rows: Vec<Value>, limit: usize) -> Value {
    let more = rows.len() > limit;
    rows.truncate(limit);
    let next = if more {
        rows.last().map(|v| v["id"].clone())
    } else {
        None
    };
    json!({"items":rows,"next_after":next,"limit":limit})
}
// One set query, including all citations, for 0/1/many memories. Scope predicates apply to every join.
