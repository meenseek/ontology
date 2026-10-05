use crate::{
    config::database_options,
    domain::{
        AREAS, Classification, Error, ImportedRecord, LinkChange, Scope, SourceKind, validate_id,
        validate_search,
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{
    PgPool, Row,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

pub const BASELINE: &str = include_str!("../schema/baseline.sql");
pub const SOURCE_PROVIDERS_MIGRATION: &str =
    include_str!("../schema/migrations/001-source-providers.sql");
pub const EVIDENCE_SNAPSHOTS_MIGRATION: &str =
    include_str!("../schema/migrations/003-evidence-snapshots.sql");
pub const SECOND_BRAIN_MIGRATION: &str = include_str!("../schema/migrations/002-second-brain.sql");
const SOURCE_PROVIDERS_NAME: &str = "001-source-providers.sql";
/// Application helper boundaries, not PostgreSQL encoder or socket bytes.
/// Store-call deltas require an isolated Store workload, as native projection observations do.
#[derive(Clone, Debug, serde::Serialize)]
pub struct DependencyObservation {
    pub dependency: &'static str,
    pub request_digest: String,
    pub input_bytes: usize,
    pub returned_bytes: usize,
    pub returned_format: &'static str,
    pub store_call_delta: u64,
    pub succeeded: bool,
    pub started: u64,
    pub finished: u64,
}
pub(crate) struct DependencyStart {
    dependency: &'static str,
    request_digest: String,
    input_bytes: usize,
    calls: u64,
    started: u64,
}
#[derive(Default)]
struct DependencyObservations {
    entries: Vec<DependencyObservation>,
    sequence: u64,
    dropped: usize,
}
#[derive(Clone)]
pub struct Store {
    pool: PgPool,
    pub(crate) database_options: PgConnectOptions,
    calls: Arc<AtomicU64>,
    dependencies: Arc<std::sync::Mutex<DependencyObservations>>,
}
impl Store {
    pub async fn connect(url: &str) -> Result<Self, Error> {
        let options = database_options(url)?;
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(3))
            .after_connect(|connection, _| {
                Box::pin(async move {
                    sqlx::query("SET statement_timeout = '5s'")
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SET lock_timeout = '3s'")
                        .execute(&mut *connection)
                        .await?;
                    // The graph projection runs in milliseconds without JIT, but its
                    // estimated cost otherwise triggers seconds of JIT compilation.
                    sqlx::query("SET jit = off")
                        .execute(&mut *connection)
                        .await?;
                    Ok(())
                })
            })
            .connect_with(options.clone())
            .await
            .map_err(|_| Error::Storage)?;
        Ok(Self {
            database_options: options,
            pool,
            calls: Arc::new(AtomicU64::new(0)),
            dependencies: Arc::new(std::sync::Mutex::new(DependencyObservations::default())),
        })
    }
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::Relaxed)
    }
    pub(crate) fn count(&self, n: u64) {
        self.calls.fetch_add(n, Ordering::Relaxed);
    }
    pub fn dependency_observations(&self) -> Vec<DependencyObservation> {
        self.dependencies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entries
            .clone()
    }
    pub fn dependency_observations_dropped(&self) -> usize {
        self.dependencies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .dropped
    }
    pub(crate) fn dependency_start(
        &self,
        dependency: &'static str,
        input: &impl serde::Serialize,
    ) -> Result<DependencyStart, Error> {
        let bytes = serde_json::to_vec(input).map_err(|_| Error::Storage)?;
        let mut state = self
            .dependencies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.sequence = state.sequence.saturating_add(1);
        Ok(DependencyStart {
            dependency,
            request_digest: digest(&bytes),
            input_bytes: bytes.len(),
            calls: self.calls(),
            started: state.sequence,
        })
    }
    pub(crate) fn dependency_finish<T: serde::Serialize>(
        &self,
        start: DependencyStart,
        result: &Result<T, Error>,
    ) {
        let returned_bytes = result
            .as_ref()
            .ok()
            .and_then(|v| serde_json::to_vec(v).ok())
            .map_or(0, |v| v.len());
        self.record_dependency(
            start,
            returned_bytes,
            "serialized-helper-json",
            result.is_ok(),
        );
    }
    fn record_dependency(
        &self,
        start: DependencyStart,
        returned_bytes: usize,
        returned_format: &'static str,
        succeeded: bool,
    ) {
        let mut state = self
            .dependencies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.sequence = state.sequence.saturating_add(1);
        let finished = state.sequence;
        if state.entries.len() == 4096 {
            state.entries.remove(0);
            state.dropped = state.dropped.saturating_add(1);
        }
        state.entries.push(DependencyObservation {
            dependency: start.dependency,
            request_digest: start.request_digest,
            input_bytes: start.input_bytes,
            returned_bytes,
            returned_format,
            store_call_delta: self.calls().saturating_sub(start.calls),
            succeeded,
            started: start.started,
            finished,
        });
    }
    pub(crate) fn dependency_finish_rows(
        &self,
        start: DependencyStart,
        result: &Result<Vec<sqlx::postgres::PgRow>, Error>,
    ) {
        use sqlx::ValueRef;
        let bytes = result.as_ref().ok().and_then(|rows| {
            rows.iter().try_fold(0usize, |total, row| {
                (0..row.len()).try_fold(total, |total, index| {
                    let value = row.try_get_raw(index).ok()?;
                    if value.is_null() {
                        Some(total)
                    } else {
                        total.checked_add(row.try_get_unchecked::<&[u8], _>(index).ok()?.len())
                    }
                })
            })
        });
        self.record_dependency(
            start,
            bytes.unwrap_or(0),
            "postgres-application-row-values",
            result.is_ok() && bytes.is_some(),
        );
    }
    pub(crate) async fn consumer_control(
        &self,
        conn: &mut sqlx::PgConnection,
        statement: &'static str,
    ) -> Result<(), Error> {
        let observation = self.dependency_start(statement, &Vec::<String>::new())?;
        self.count(1);
        let result = sqlx::query(statement)
            .execute(conn)
            .await
            .map(|r| r.rows_affected())
            .map_err(|_| Error::Storage);
        self.dependency_finish(observation, &result);
        result.map(|_| ())
    }
    pub async fn initialize(&self) -> Result<(), Error> {
        self.initialize_baseline(BASELINE).await
    }
    pub async fn initialize_baseline(&self, baseline: &'static str) -> Result<(), Error> {
        let digest = digest(baseline.as_bytes());
        let mut tx = self.pool.begin().await.map_err(|_| Error::Storage)?;
        sqlx::query("SELECT pg_advisory_xact_lock(478310001)")
            .execute(&mut *tx)
            .await
            .map_err(|_| Error::Storage)?;
        let exists: bool =
            sqlx::query_scalar("SELECT to_regclass('public.ontology_baseline') IS NOT NULL")
                .fetch_one(&mut *tx)
                .await
                .map_err(|_| Error::Storage)?;
        if exists {
            let stored: Option<String> =
                sqlx::query_scalar("SELECT digest FROM ontology_baseline WHERE singleton = true")
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(|_| Error::Baseline)?;
            if stored.as_deref() != Some(&digest) {
                return Err(Error::Baseline);
            }
        } else {
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind IN ('r','p','v','m','S')").fetch_one(&mut *tx).await.map_err(|_| Error::Storage)?;
            if count != 0 {
                return Err(Error::Baseline);
            }
            sqlx::raw_sql(baseline)
                .execute(&mut *tx)
                .await
                .map_err(|_| Error::Baseline)?;
            sqlx::query("CREATE TABLE ontology_baseline (singleton boolean PRIMARY KEY CHECK(singleton), digest text NOT NULL)").execute(&mut *tx).await.map_err(|_| Error::Storage)?;
            sqlx::query("INSERT INTO ontology_baseline VALUES(true,$1)")
                .bind(digest)
                .execute(&mut *tx)
                .await
                .map_err(|_| Error::Storage)?;
            let areas = json!(
                AREAS
                    .iter()
                    .map(|(id, label)| json!({"id":id,"label":label}))
                    .collect::<Vec<_>>()
            );
            sqlx::query("INSERT INTO areas SELECT id,label FROM jsonb_to_recordset($1) AS x(id text,label text)").bind(areas).execute(&mut *tx).await.map_err(|_| Error::Storage)?;
        }
        sqlx::query("CREATE TABLE IF NOT EXISTS ontology_migrations (name text PRIMARY KEY, digest text NOT NULL)")
            .execute(&mut *tx).await.map_err(|_| Error::Baseline)?;
        let migrations = sqlx::query("SELECT name,digest FROM ontology_migrations ORDER BY name")
            .fetch_all(&mut *tx)
            .await
            .map_err(|_| Error::Baseline)?;
        let expected = [
            (SOURCE_PROVIDERS_NAME, SOURCE_PROVIDERS_MIGRATION),
            ("002-second-brain.sql", SECOND_BRAIN_MIGRATION),
            ("003-evidence-snapshots.sql", EVIDENCE_SNAPSHOTS_MIGRATION),
            (
                "004-curation-reviews.sql",
                include_str!("../schema/migrations/004-curation-reviews.sql"),
            ),
            (
                "005-context-materials.sql",
                include_str!("../schema/migrations/005-context-materials.sql"),
            ),
            (
                "006-context-history.sql",
                include_str!("../schema/migrations/006-context-history.sql"),
            ),
            (
                "007-context-native.sql",
                include_str!("../schema/migrations/007-context-native.sql"),
            ),
            (
                "008-context-consumers.sql",
                include_str!("../schema/migrations/008-context-consumers.sql"),
            ),
            (
                "009-context-manual-edits.sql",
                include_str!("../schema/migrations/009-context-manual-edits.sql"),
            ),
            (
                "010-profile-manual-edits.sql",
                include_str!("../schema/migrations/010-profile-manual-edits.sql"),
            ),
            (
                "011-personal-memory-grouping.sql",
                include_str!("../schema/migrations/011-personal-memory-grouping.sql"),
            ),
            (
                "012-context-projection-refresh.sql",
                include_str!("../schema/migrations/012-context-projection-refresh.sql"),
            ),
        ];
        if migrations.len() > expected.len() {
            return Err(Error::Baseline);
        }
        for (index, (name, sql)) in expected.iter().enumerate() {
            let hash = crate::store::digest(sql.as_bytes());
            if let Some(migration) = migrations.get(index) {
                if migration.get::<String, _>("name") != *name
                    || migration.get::<String, _>("digest") != hash
                {
                    return Err(Error::Baseline);
                }
            } else {
                sqlx::raw_sql(*sql)
                    .execute(&mut *tx)
                    .await
                    .map_err(|_| Error::Baseline)?;
                sqlx::query("INSERT INTO ontology_migrations(name,digest) VALUES($1,$2)")
                    .bind(name)
                    .bind(hash)
                    .execute(&mut *tx)
                    .await
                    .map_err(|_| Error::Baseline)?;
            }
        }
        tx.commit().await.map_err(|_| Error::Storage)
    }
    pub async fn list(
        &self,
        scope: Scope,
        query: &str,
        unclassified: bool,
        area: Option<&str>,
    ) -> Result<Value, Error> {
        let observation =
            self.dependency_start("source-list", &(scope, query, unclassified, area))?;
        let result=async {
        validate_search(query)?;
        crate::domain::validate_area(scope, area)?;
        self.count(1);
        sqlx::query_scalar(r#"
WITH scoped AS MATERIALIZED (
 SELECT e.id,e.revision,s.kind,s.path,s.status,p.present,(s.status='ok' AND p.present AND p.source_revision=s.verified_revision) AS current,left(COALESCE(p.content,''),200) AS excerpt,
 ARRAY(SELECT a.area FROM entity_areas a WHERE a.scope=$1 AND a.entity_id=e.id ORDER BY a.area) AS areas,
 ARRAY(SELECT t.name FROM entity_topics et JOIN topics t ON t.scope=et.scope AND t.id=et.topic_id WHERE et.scope=$1 AND et.entity_id=e.id ORDER BY t.name) AS topics
 FROM entities e JOIN sources s ON s.scope=e.scope AND s.id=e.source_id JOIN source_records p ON p.scope=e.scope AND p.entity_id=e.id
 WHERE e.scope=$1 AND ($4::text IS NULL OR EXISTS(SELECT 1 FROM entity_areas a WHERE a.scope=$1 AND a.entity_id=e.id AND a.area=$4)) AND (NOT $3 OR (NOT EXISTS(SELECT 1 FROM entity_areas a WHERE a.scope=$1 AND a.entity_id=e.id) AND NOT EXISTS(SELECT 1 FROM entity_topics t WHERE t.scope=$1 AND t.entity_id=e.id)))
 AND ($2='' OR strpos(lower(COALESCE(p.content,'') || ' ' || s.path),lower($2))>0 OR EXISTS(SELECT 1 FROM entity_topics et JOIN topics t ON t.scope=et.scope AND t.id=et.topic_id WHERE et.scope=$1 AND et.entity_id=e.id AND strpos(lower(t.name),lower($2))>0))
), page AS (SELECT * FROM scoped ORDER BY path,id LIMIT 100)
SELECT jsonb_build_object('items',COALESCE((SELECT jsonb_agg(to_jsonb(page) ORDER BY path,id) FROM page),'[]'::jsonb),'total',(SELECT count(*) FROM scoped),'limit',100,
 'areas',COALESCE((SELECT jsonb_agg(x) FROM (SELECT area,count(*) FROM scoped CROSS JOIN unnest(areas) area GROUP BY area ORDER BY area) x),'[]'::jsonb))
"#).bind(scope.as_str()).bind(query).bind(unclassified).bind(area).fetch_one(&self.pool).await.map_err(|_|Error::Storage)

 }.await;
        self.dependency_finish(observation, &result);
        result
    }
    pub async fn detail(&self, scope: Scope, id: &str) -> Result<Value, Error> {
        let observation = self.dependency_start("source-detail", &(scope, id))?;
        let result=async {
        validate_id(id)?;
        self.count(1);
        sqlx::query_scalar(r#"
SELECT jsonb_build_object('id',e.id,'revision',e.revision,
 'areas',ARRAY(SELECT a.area FROM entity_areas a WHERE a.scope=$1 AND a.entity_id=e.id ORDER BY a.area),
 'topics',ARRAY(SELECT t.name FROM entity_topics et JOIN topics t ON t.scope=et.scope AND t.id=et.topic_id WHERE et.scope=$1 AND et.entity_id=e.id ORDER BY t.name),
 'source',jsonb_build_object('kind',s.kind,'path',s.path,'repository',s.repository,'status',s.status,'last_attempt_at',s.last_attempt_at,'last_success_at',s.last_success_at,'verified_revision',s.verified_revision,'failure_code',s.failure_code),
 'current',(s.status='ok' AND p.present AND p.source_revision=s.verified_revision),'projection',to_jsonb(p)-'entity_id'-'scope',
 'related',COALESCE((SELECT jsonb_agg(x ORDER BY x.path,x.id) FROM (
 SELECT other.id,os.path FROM related_materials r JOIN entities other ON other.scope=r.scope AND other.id=CASE WHEN r.left_id=e.id THEN r.right_id ELSE r.left_id END
 JOIN sources os ON os.scope=other.scope AND os.id=other.source_id WHERE r.scope=$1 AND (r.left_id=e.id OR r.right_id=e.id) ORDER BY os.path,other.id LIMIT 100) x),'[]'::jsonb),
 'history',COALESCE((SELECT jsonb_agg(x ORDER BY x.id DESC) FROM (SELECT h.id,h.kind,h.revision,h.previous,h.confirmed,h.confirmed_at FROM confirmation_history h WHERE h.scope=$1 AND h.entity_id=e.id ORDER BY h.id DESC LIMIT 30) x),'[]'::jsonb))
FROM entities e JOIN sources s ON s.scope=e.scope AND s.id=e.source_id JOIN source_records p ON p.scope=e.scope AND p.entity_id=e.id WHERE e.scope=$1 AND e.id=$2
"#).bind(scope.as_str()).bind(id).fetch_optional(&self.pool).await.map_err(|_|Error::Storage)?.ok_or(Error::NotFound)

 }.await;
        self.dependency_finish(observation, &result);
        result
    }
    pub async fn classify(
        &self,
        scope: Scope,
        id: &str,
        change: Classification,
    ) -> Result<(), Error> {
        validate_id(id)?;
        change.validate(scope)?;
        self.count(1);
        let mut tx = self.pool.begin().await.map_err(|_| Error::Storage)?;
        self.count(1);
        let old=sqlx::query(r#"SELECT e.revision,
 ARRAY(SELECT a.area FROM entity_areas a WHERE a.scope=$1 AND a.entity_id=e.id ORDER BY a.area) AS areas,
 ARRAY(SELECT t.name FROM entity_topics et JOIN topics t ON t.scope=et.scope AND t.id=et.topic_id WHERE et.scope=$1 AND et.entity_id=e.id ORDER BY t.name) AS topics
 FROM entities e WHERE e.scope=$1 AND e.id=$2 FOR UPDATE"#).bind(scope.as_str()).bind(id).fetch_optional(&mut *tx).await.map_err(|_|Error::Storage)?.ok_or(Error::NotFound)?;
        let revision: i64 = old.get("revision");
        if revision != change.revision {
            self.count(1);
            return Err(Error::Conflict);
        }
        let previous = json!({"areas":old.get::<Vec<String>,_>("areas"),"topics":old.get::<Vec<String>,_>("topics")});
        self.count(1);
        sqlx::query("INSERT INTO topics(scope,name) SELECT $1,unnest($2::text[]) ON CONFLICT(scope,name) DO NOTHING").bind(scope.as_str()).bind(&change.topics).execute(&mut *tx).await.map_err(|_|Error::Storage)?;
        self.count(1);
        sqlx::query(r#"WITH removed AS (DELETE FROM entity_areas WHERE scope=$1 AND entity_id=$2 AND NOT(area=ANY($3))) INSERT INTO entity_areas SELECT $1,$2,unnest($3::text[]) ON CONFLICT DO NOTHING"#).bind(scope.as_str()).bind(id).bind(&change.areas).execute(&mut *tx).await.map_err(|_|Error::Storage)?;
        self.count(1);
        sqlx::query(r#"WITH removed AS (DELETE FROM entity_topics WHERE scope=$1 AND entity_id=$2 AND topic_id NOT IN (SELECT id FROM topics WHERE scope=$1 AND name=ANY($3))) INSERT INTO entity_topics SELECT $1,$2,id FROM topics WHERE scope=$1 AND name=ANY($3) ON CONFLICT DO NOTHING"#).bind(scope.as_str()).bind(id).bind(&change.topics).execute(&mut *tx).await.map_err(|_|Error::Storage)?;
        self.count(1);
        sqlx::query("UPDATE entities SET revision=revision+1 WHERE scope=$1 AND id=$2")
            .bind(scope.as_str())
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|_| Error::Storage)?;
        self.count(1);
        sqlx::query("INSERT INTO confirmation_history(scope,entity_id,revision,kind,previous,confirmed) VALUES($1,$2,$3,'classification',$4,$5)").bind(scope.as_str()).bind(id).bind(revision+1).bind(previous).bind(json!({"areas":change.areas,"topics":change.topics})).execute(&mut *tx).await.map_err(|_|Error::Storage)?;
        self.count(1);
        tx.commit().await.map_err(|_| Error::Storage)
    }
    pub async fn link(&self, scope: Scope, id: &str, change: LinkChange) -> Result<(), Error> {
        validate_id(id)?;
        validate_id(&change.target_id)?;
        if id == change.target_id || change.revision < 0 || change.revision == i64::MAX {
            return Err(Error::Invalid);
        }
        self.count(1);
        let mut tx = self.pool.begin().await.map_err(|_| Error::Storage)?;
        self.count(1);
        let rows = sqlx::query(
            "SELECT id,revision FROM entities WHERE scope=$1 AND id=ANY($2) ORDER BY id FOR UPDATE",
        )
        .bind(scope.as_str())
        .bind(vec![id, &change.target_id])
        .fetch_all(&mut *tx)
        .await
        .map_err(|_| Error::Storage)?;
        if rows.len() != 2 {
            self.count(1);
            return Err(Error::NotFound);
        }
        let old_revision = rows
            .iter()
            .find(|r| r.get::<String, _>("id") == id)
            .map(|r| r.get::<i64, _>("revision"))
            .ok_or(Error::NotFound)?;
        if old_revision != change.revision {
            self.count(1);
            return Err(Error::Conflict);
        }
        let (left, right) = if id < change.target_id.as_str() {
            (id, change.target_id.as_str())
        } else {
            (change.target_id.as_str(), id)
        };
        self.count(1);
        let counts: Vec<i64> = sqlx::query_scalar("SELECT count(*) FROM related_materials WHERE scope=$1 AND (left_id=ANY($2) OR right_id=ANY($2)) GROUP BY scope")
            .bind(scope.as_str()).bind(vec![id,change.target_id.as_str()]).fetch_all(&mut *tx).await.map_err(|_| Error::Storage)?;
        // Conservatively bound combined degree to keep every detail response complete.
        if !change.remove && counts.first().is_some_and(|n| *n >= 100) {
            self.count(1);
            return Err(Error::Limit);
        }
        self.count(1);
        let affected = if change.remove {
            sqlx::query("DELETE FROM related_materials WHERE scope=$1 AND left_id=$2 AND right_id=$3").bind(scope.as_str()).bind(left).bind(right).execute(&mut *tx).await
        } else {
            sqlx::query("INSERT INTO related_materials(scope,left_id,right_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(scope.as_str()).bind(left).bind(right).execute(&mut *tx).await
        }.map_err(|_| Error::Storage)?.rows_affected();
        if affected > 0 {
            self.count(1);
            sqlx::query("UPDATE entities SET revision=revision+1 WHERE scope=$1 AND id=$2")
                .bind(scope.as_str())
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|_| Error::Storage)?;
            self.count(1);
            sqlx::query("INSERT INTO confirmation_history(scope,entity_id,revision,kind,previous,confirmed) VALUES($1,$2,$3,$4,$5,$6)")
                .bind(scope.as_str()).bind(id).bind(old_revision+1).bind(if change.remove{"link-remove"}else{"link-add"})
                .bind(json!({"target_id":change.target_id,"linked":change.remove})).bind(json!({"target_id":change.target_id,"linked":!change.remove}))
                .execute(&mut *tx).await.map_err(|_| Error::Storage)?;
        }
        self.count(1);
        tx.commit().await.map_err(|_| Error::Storage)
    }
    // Hold this transaction from before provider observation through persistence. The shared
    // try-lock bounds overlap across manual commands, app polling, and other app processes.
    pub async fn lock_import(&self) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, Error> {
        self.count(1);
        let mut tx = self.pool.begin().await.map_err(|_| Error::Storage)?;
        self.count(1);
        let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(478310002)")
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| Error::Storage)?;
        if !acquired {
            self.count(1);
            tx.rollback().await.map_err(|_| Error::Storage)?;
            return Err(Error::Conflict);
        }
        Ok(tx)
    }
    pub async fn finish_import(
        &self,
        tx: sqlx::Transaction<'_, sqlx::Postgres>,
    ) -> Result<(), Error> {
        self.count(1);
        tx.commit().await.map_err(|_| Error::Storage)
    }
    pub async fn apply_import(&self, records: &[ImportedRecord]) -> Result<(), Error> {
        if records.is_empty() {
            return Ok(());
        }
        if records.len() > 100 {
            return Err(Error::Limit);
        }
        self.count(1);
        let mut tx = self.pool.begin().await.map_err(|_| Error::Storage)?;
        self.apply_import_in(&mut tx, records).await?;
        self.count(1);
        tx.commit().await.map_err(|_| Error::Storage)
    }
    pub(crate) async fn apply_import_in(
        &self,
        conn: &mut sqlx::PgConnection,
        records: &[ImportedRecord],
    ) -> Result<(), Error> {
        let observation = self.dependency_start("apply_import_in", &records)?;
        let result=async {
        if records.is_empty() {
            return Ok(());
        }
        if records.len() > 100 {
            return Err(Error::Limit);
        }
        let value = serde_json::to_value(records).map_err(|_| Error::Invalid)?;
        self.count(1);
        sqlx::query(r#"
WITH input AS MATERIALIZED (SELECT * FROM jsonb_to_recordset($1) AS x(source_id text,entity_id text,scope text,repository text,path text,kind text,source_revision text,digest text,content text)),
s AS (INSERT INTO sources(id,scope,repository,path,kind,status,last_success_at,verified_revision)
 SELECT source_id,scope,repository,path,kind,CASE WHEN content IS NULL THEN 'missing' ELSE 'ok' END,now(),source_revision FROM input
 ON CONFLICT(id) DO UPDATE SET generation=sources.generation+CASE WHEN sources.status IS DISTINCT FROM EXCLUDED.status OR sources.verified_revision IS DISTINCT FROM EXCLUDED.verified_revision OR ((SELECT content FROM input WHERE source_id=EXCLUDED.id) IS NOT NULL AND (SELECT content_digest FROM source_records WHERE entity_id=(SELECT entity_id FROM input WHERE source_id=EXCLUDED.id)) IS DISTINCT FROM (SELECT digest FROM input WHERE source_id=EXCLUDED.id)) THEN 1 ELSE 0 END,status=EXCLUDED.status,last_attempt_at=now(),last_success_at=now(),verified_revision=EXCLUDED.verified_revision,failure_code=NULL RETURNING id),
e AS (INSERT INTO entities(id,scope,source_id) SELECT i.entity_id,i.scope,i.source_id FROM input i JOIN s ON s.id=i.source_id
 ON CONFLICT(id) DO UPDATE SET source_id=EXCLUDED.source_id RETURNING id)
INSERT INTO source_records(entity_id,scope,content,content_digest,source_revision,present,absence_revision)
 SELECT i.entity_id,i.scope,i.content,i.digest,CASE WHEN i.content IS NULL THEN NULL ELSE i.source_revision END,i.content IS NOT NULL,CASE WHEN i.content IS NULL THEN i.source_revision ELSE NULL END FROM input i JOIN e ON e.id=i.entity_id
 ON CONFLICT(entity_id) DO UPDATE SET content=COALESCE(EXCLUDED.content,source_records.content),content_digest=COALESCE(EXCLUDED.content_digest,source_records.content_digest),source_revision=COALESCE(EXCLUDED.source_revision,source_records.source_revision),observed_at=now(),present=EXCLUDED.present,absence_revision=EXCLUDED.absence_revision
"#).bind(value).execute(&mut *conn).await.map_err(|_| Error::Storage)?;
        Ok(())

        }.await;
        self.dependency_finish(observation, &result);
        result
    }
    pub async fn mark_failed(&self, source_ids: &[String], kind: SourceKind) -> Result<(), Error> {
        if source_ids.is_empty() {
            return Ok(());
        }
        let mut conn = self.pool.acquire().await.map_err(|_| Error::Storage)?;
        self.mark_failed_in(&mut conn, source_ids, kind).await
    }
    pub(crate) async fn mark_failed_in(
        &self,
        conn: &mut sqlx::PgConnection,
        source_ids: &[String],
        kind: SourceKind,
    ) -> Result<(), Error> {
        let observation = self.dependency_start(
            "mark_failed_in",
            &(source_ids, kind.failure_code(), kind.as_str()),
        )?;
        let result=async {
        if source_ids.len() > 100 {
            return Err(Error::Limit);
        }
        if source_ids.is_empty() {
            return Ok(());
        }
        self.count(1);
        sqlx::query("UPDATE sources SET generation=generation+CASE WHEN status='failed' THEN 0 ELSE 1 END,status='failed',failure_code=$2,last_attempt_at=now() WHERE id=ANY($1) AND kind=$3")
            .bind(source_ids).bind(kind.failure_code()).bind(kind.as_str())
            .execute(conn).await.map_err(|_| Error::Storage)?;
        Ok(())

        }.await;
        self.dependency_finish(observation, &result);
        result
    }
}

pub fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
