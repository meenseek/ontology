//! PostgreSQL effects accept only the opaque contracts and proofs issued by Core.
use crate::{
    context::ContextScope,
    context_projection::{append_projections, projection},
    domain::Error,
    native_context::{NativeContextSession, source_error, stored_state},
    store::{Store, digest},
};
use context_core::harness::{
    ContextAbortProof, ContextApplyContract, ContextCommitReceipt, ContextCommitSession,
    ContextPreApplyCloseContract, ContextRecoveryContract, ContextRecoveryStatus,
    ContextTerminalProof, FinalizationOperation, HarnessApplyAttemptState, HarnessResult,
    SourceStoreIdentity, SourceVersion, StoredSourceState, VerifiedContextCommit,
};
use serde_json::{Value, json};
use sqlx::{Connection, PgConnection, Row, postgres::PgRow};
use std::collections::{BTreeMap, BTreeSet};

// Observe the PostgreSQL application payload at the encoder and returned-row
// boundaries. No original parameters or row contents are retained. These are not
// socket, TLS, transaction-control, or server-internal trigger byte counts.
#[derive(Clone, Debug)]
pub struct NativeSqlCall {
    pub statement: &'static str,
    pub request_digest: String,
    pub parameter_bytes: usize,
    pub row_bytes: usize,
    pub succeeded: bool,
    pub started: u64,
    pub finished: u64,
}
#[derive(Clone, Debug)]
pub struct NativeProjectionCall {
    pub records: usize,
    pub input_bytes: usize,
    // Shared Store counter delta during this awaited helper; use an isolated
    // Store workload when asserting it as a SQL-call count.
    pub store_call_delta: u64,
    pub succeeded: bool,
}
#[derive(Default)]
pub(crate) struct NativeSqlObservations {
    projections: Vec<NativeProjectionCall>,
    calls: Vec<NativeSqlCall>,
    sequence: u64,
    overflowed: bool,
}
type Observations = std::sync::Arc<std::sync::Mutex<NativeSqlObservations>>;
struct SqlObservation<'a> {
    store: &'a Store,
    observations: &'a Observations,
    statement: &'static str,
    parameters: std::sync::Arc<std::sync::Mutex<(sha2::Sha256, usize)>>,
}
struct SqlParameter<T> {
    value: T,
    parameters: std::sync::Arc<std::sync::Mutex<(sha2::Sha256, usize)>>,
}
impl<T: sqlx::Type<sqlx::Postgres>> sqlx::Type<sqlx::Postgres> for SqlParameter<T> {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        T::type_info()
    }
    fn compatible(ty: &sqlx::postgres::PgTypeInfo) -> bool {
        T::compatible(ty)
    }
}
impl<'q, T: sqlx::Encode<'q, sqlx::Postgres>> sqlx::Encode<'q, sqlx::Postgres> for SqlParameter<T> {
    fn encode_by_ref(
        &self,
        buffer: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        use sha2::Digest;
        let start = buffer.len();
        let result = self.value.encode_by_ref(buffer)?;
        let mut observed = self
            .parameters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let bytes = &buffer[start..];
        observed
            .0
            .update([u8::from(matches!(&result, sqlx::encode::IsNull::Yes))]);
        observed.0.update(bytes.len().to_be_bytes());
        observed.0.update(bytes);
        observed.1 = observed.1.saturating_add(bytes.len());
        Ok(result)
    }
    fn produces(&self) -> Option<sqlx::postgres::PgTypeInfo> {
        self.value.produces()
    }
    fn size_hint(&self) -> usize {
        self.value.size_hint()
    }
}
impl<'a> SqlObservation<'a> {
    fn new(store: &'a Store, observations: &'a Observations, statement: &'static str) -> Self {
        use sha2::Digest;
        Self {
            store,
            observations,
            statement,
            parameters: std::sync::Arc::new(std::sync::Mutex::new((
                sha2::Sha256::new_with_prefix(statement.as_bytes()),
                0,
            ))),
        }
    }
    fn bind<T>(&self, value: T) -> SqlParameter<T> {
        SqlParameter {
            value,
            parameters: self.parameters.clone(),
        }
    }
    fn start(&self) -> u64 {
        self.store.count(1);
        let mut state = self
            .observations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.sequence = state.sequence.saturating_add(1);
        state.sequence
    }
    fn finish(&self, started: u64, succeeded: bool, rows: &[PgRow]) {
        use sha2::Digest;
        use sqlx::ValueRef;
        let mut bytes = 0usize;
        let mut complete = true;
        for row in rows {
            for column in 0..row.len() {
                match row.try_get_raw(column) {
                    Ok(value) if value.is_null() => {}
                    Ok(_) => match row.try_get_unchecked::<&[u8], _>(column) {
                        Ok(value) => bytes = bytes.saturating_add(value.len()),
                        Err(_) => complete = false,
                    },
                    Err(_) => complete = false,
                }
            }
        }
        let parameters = self
            .parameters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut state = self
            .observations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.sequence = state.sequence.saturating_add(1);
        let finished = state.sequence;
        // This observer cannot make an otherwise valid effect fail or grow without
        // bound. An incomplete capture is explicit and cannot support a proof.
        if state.calls.len() == 512 || !complete {
            state.overflowed = true;
            return;
        }
        state.calls.push(NativeSqlCall {
            statement: self.statement,
            request_digest: digest(&parameters.0.clone().finalize()),
            parameter_bytes: parameters.1,
            row_bytes: bytes,
            succeeded,
            started,
            finished,
        });
    }
    async fn fetch_all(
        self,
        query: sqlx::query::Query<'_, sqlx::Postgres, sqlx::postgres::PgArguments>,
        connection: &mut PgConnection,
    ) -> HarnessResult<Vec<PgRow>> {
        let started = self.start();
        let result = query.fetch_all(connection).await;
        self.finish(
            started,
            result.is_ok(),
            result.as_ref().map(Vec::as_slice).unwrap_or(&[]),
        );
        result.map_err(storage)
    }
    async fn execute(
        self,
        query: sqlx::query::Query<'_, sqlx::Postgres, sqlx::postgres::PgArguments>,
        connection: &mut PgConnection,
    ) -> HarnessResult<sqlx::postgres::PgQueryResult> {
        let started = self.start();
        let result = query.execute(connection).await;
        self.finish(started, result.is_ok(), &[]);
        result.map_err(storage)
    }
}
impl NativeContextSession {
    /// Serialized input at the existing projection helper boundary. The SQL
    /// encoder observations above cover direct queries; this is a distinct scope.
    pub fn native_projection_observations(&self) -> Option<Vec<NativeProjectionCall>> {
        let state = self
            .sql_observations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (!state.overflowed).then(|| state.projections.clone())
    }
    pub fn native_sql_observations(&self) -> Option<Vec<NativeSqlCall>> {
        let state = self
            .sql_observations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (!state.overflowed).then(|| state.calls.clone())
    }
}

fn conflict<T>() -> HarnessResult<T> {
    Err(source_error(Error::Conflict))
}
fn storage(_: sqlx::Error) -> context_core::harness::HarnessError {
    source_error(Error::Storage)
}
fn encode(value: &impl serde::Serialize) -> HarnessResult<Vec<u8>> {
    serde_json::to_vec(value).map_err(|_| source_error(Error::Storage))
}
fn logical(path: &str) -> HarnessResult<(ContextScope, String, String)> {
    let source = path
        .strip_prefix("vault/")
        .ok_or_else(|| source_error(Error::Invalid))?;
    let mut parts = source.split('/');
    let first = parts.next().ok_or_else(|| source_error(Error::Invalid))?;
    let scope = if first == "work" {
        format!(
            "work/{}",
            parts.next().ok_or_else(|| source_error(Error::Invalid))?
        )
    } else {
        first.to_owned()
    };
    let path = parts.collect::<Vec<_>>().join("/");
    if path.is_empty()
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return conflict();
    }
    Ok((
        scope.parse().map_err(source_error)?,
        path,
        source.to_owned(),
    ))
}
fn previous(contract: &ContextApplyContract) -> Vec<SourceVersion> {
    contract
        .targets()
        .iter()
        .map(|t| t.previous().clone())
        .collect()
}
fn expected(contract: &ContextApplyContract) -> Value {
    json!({"targets":previous(contract),"non_targets":contract.non_target_versions()})
}
fn check_contract(
    session: &NativeContextSession,
    contract: &ContextApplyContract,
) -> HarnessResult<Vec<u8>> {
    if contract.store_identity() != Some(&session.identity) {
        return conflict();
    }
    let bytes = encode(contract)?;
    if bytes.is_empty()
        || bytes.len() > 1024 * 1024
        || contract.targets().len() > 10000
        || contract.non_target_versions().len() > 10000
    {
        return Err(source_error(Error::Limit));
    }
    let mut paths = BTreeSet::new();
    for v in contract
        .non_target_versions()
        .iter()
        .chain(contract.targets().iter().map(|t| t.previous()))
    {
        v.validate()?;
        logical(&v.logical_path)?;
        if !paths.insert(&v.logical_path) {
            return conflict();
        }
        match &v.state {
            StoredSourceState::Live { revision, .. }
            | StoredSourceState::Deleted { revision, .. } => {
                i64::try_from(*revision).map_err(|_| source_error(Error::Limit))?;
            }
            StoredSourceState::Missing => {}
        }
    }
    for t in contract.targets() {
        if let StoredSourceState::Live { revision, .. }
        | StoredSourceState::Deleted { revision, .. } = t.previous().state
            && revision >= i64::MAX as u64
        {
            return Err(source_error(Error::Limit));
        }
    }
    Ok(bytes)
}
async fn versions(
    store: &Store,
    observations: &Observations,
    connection: &mut PgConnection,
    wanted: &[SourceVersion],
) -> HarnessResult<Vec<SourceVersion>> {
    if wanted.is_empty() {
        return Ok(vec![]);
    }
    let paths = wanted
        .iter()
        .map(|v| logical(&v.logical_path).map(|(_, _, p)| p))
        .collect::<HarnessResult<Vec<_>>>()?;
    let rows = {
        let statement = "SELECT source_path,material_id::text,revision,content_digest,deleted FROM context_materials WHERE source_path=ANY($1) ORDER BY source_path COLLATE \"C\"";
        let observation = SqlObservation::new(store, observations, statement);
        let query = sqlx::query(statement).bind(observation.bind(&paths));
        observation.fetch_all(query, &mut *connection).await?
    };
    let states = rows
        .iter()
        .map(|r| Ok((r.get::<String, _>("source_path"), stored_state(r)?)))
        .collect::<HarnessResult<BTreeMap<_, _>>>()?;
    Ok(wanted
        .iter()
        .zip(paths)
        .map(|(v, p)| SourceVersion {
            logical_path: v.logical_path.clone(),
            state: states
                .get(&p)
                .cloned()
                .unwrap_or(StoredSourceState::Missing),
        })
        .collect())
}
async fn check_versions(
    store: &Store,
    observations: &Observations,
    connection: &mut PgConnection,
    wanted: &[SourceVersion],
) -> HarnessResult<()> {
    if versions(store, observations, connection, wanted).await? != wanted {
        return conflict();
    }
    Ok(())
}
async fn effect(
    store: &Store,
    observations: &Observations,
    connection: &mut PgConnection,
    contract: &ContextApplyContract,
    bytes: &[u8],
) -> HarnessResult<Option<PgRow>> {
    let a = contract.attempt();
    let mut rows = {
        let statement = "SELECT *,apply_id::text AS id,store_id::text AS store_identity FROM context_apply_batches WHERE state IN ('pending','committed') OR core_run_id=$1 OR core_apply_attempt_id=$2 OR expected_batch_id=$3 OR expected_journal_locator=$4";
        let observation = SqlObservation::new(store, observations, statement);
        let query = sqlx::query(statement)
            .bind(observation.bind(&a.run_identifier))
            .bind(observation.bind(&a.attempt_identifier))
            .bind(observation.bind(&a.expected_batch_identifier))
            .bind(observation.bind(&a.expected_journal_relative_path));
        observation.fetch_all(query, &mut *connection).await?
    };
    if rows.is_empty() {
        return Ok(None);
    }
    if rows.len() != 1 {
        return conflict();
    }
    let row = rows.remove(0);
    if row.get::<String, _>("store_identity")
        != contract
            .store_identity()
            .ok_or_else(|| source_error(Error::Conflict))?
            .store_id
        || row.get::<Option<Vec<u8>>, _>("core_contract").as_deref() != Some(bytes)
        || row
            .get::<Option<String>, _>("core_contract_digest")
            .as_deref()
            != Some(digest(bytes).as_str())
        || row.get::<String, _>("core_run_id") != a.run_identifier
        || row.get::<String, _>("core_apply_attempt_id") != a.attempt_identifier
        || row.get::<String, _>("prepared_run_digest") != a.prepared_run_digest
        || row.get::<String, _>("candidate_digest") != a.candidate_digest
        || row.get::<String, _>("expected_batch_id") != a.expected_batch_identifier
        || row.get::<String, _>("expected_journal_locator") != a.expected_journal_relative_path
        || row.get::<Value, _>("expected_source_versions") != expected(contract)
        || row.get::<Value, _>("context_targets")
            != serde_json::to_value(contract.targets()).map_err(|_| source_error(Error::Storage))?
    {
        return conflict();
    }
    Ok(Some(row))
}
async fn committed(
    store: &Store,
    observations: &Observations,
    connection: &mut PgConnection,
    contract: &ContextApplyContract,
    row: &PgRow,
) -> HarnessResult<ContextCommitReceipt> {
    let receipt: ContextCommitReceipt = serde_json::from_value(
        row.get::<Option<Value>, _>("commit_receipt")
            .ok_or_else(|| source_error(Error::Conflict))?,
    )
    .map_err(|_| source_error(Error::Conflict))?;
    receipt.validate_for(contract)?;
    if row.get::<Option<String>, _>("actual_batch_id").as_deref()
        != Some(receipt.batch_identifier())
        || row
            .get::<Option<String>, _>("actual_journal_locator")
            .as_deref()
            != Some(receipt.journal_relative_path())
    {
        return conflict();
    }
    check_versions(store, observations, connection, receipt.resulting()).await?;
    let (_, projection_digest) = committed_rows(
        store,
        observations,
        connection,
        &row.get::<String, _>("id"),
        receipt.resulting(),
    )
    .await?;
    if projection_digest != receipt.projection_metadata_digest() {
        return conflict();
    }
    Ok(receipt)
}
async fn committed_rows(
    store: &Store,
    observations: &Observations,
    connection: &mut PgConnection,
    apply_id: &str,
    wanted: &[SourceVersion],
) -> HarnessResult<(Vec<SourceVersion>, String)> {
    let paths = wanted
        .iter()
        .map(|v| logical(&v.logical_path).map(|(_, _, p)| p))
        .collect::<HarnessResult<Vec<_>>>()?;
    let rows = {
        let statement = "SELECT m.source_path,m.material_id::text,m.revision,m.content_digest,m.deleted,p.payload_digest FROM context_materials m JOIN context_material_versions v USING(material_id,revision) JOIN context_projection_versions p USING(material_id,revision) WHERE m.source_path=ANY($1) AND m.last_apply_id=$2::uuid AND v.apply_id=$2::uuid AND m.content=v.content AND m.content_digest=v.content_digest AND m.byte_len=v.byte_len AND m.deleted=v.deleted AND m.restricted=v.restricted AND m.search_text IS NOT DISTINCT FROM v.search_text AND p.payload_digest=encode(sha256(convert_to(p.payload::text,'UTF8')),'hex') AND p.payload->>'source_digest'=m.content_digest ORDER BY m.source_path COLLATE \"C\"";
        let observation = SqlObservation::new(store, observations, statement);
        let query = sqlx::query(statement)
            .bind(observation.bind(&paths))
            .bind(observation.bind(apply_id));
        observation.fetch_all(query, &mut *connection).await?
    };
    if rows.len() != wanted.len() {
        return conflict();
    }
    let mut resulting = Vec::new();
    let mut metadata = Vec::new();
    for row in rows {
        let logical_path = format!("vault/{}", row.get::<String, _>("source_path"));
        let state = stored_state(&row)?;
        metadata.push((
            logical_path.clone(),
            row.get::<String, _>("material_id"),
            row.get::<i64, _>("revision"),
            if row.get::<bool, _>("deleted") {
                "deleted"
            } else {
                "live"
            },
            row.get::<String, _>("content_digest"),
            row.get::<String, _>("payload_digest"),
        ));
        resulting.push(SourceVersion {
            logical_path,
            state,
        });
    }
    Ok((resulting, digest(&encode(&metadata)?)))
}
impl ContextCommitSession for NativeContextSession {
    fn verify_pre_apply_close(
        &mut self,
        contract: &ContextPreApplyCloseContract,
    ) -> HarnessResult<()> {
        if self.identity != *contract.store_identity() {
            return conflict();
        }
        contract.check_view(self.view_root())?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| source_error(Error::Storage))?;
        self.handle.block_on(async {
            let statement = "SELECT EXISTS(SELECT 1 FROM context_apply_batches WHERE state IN ('pending','committed') OR core_run_id=$1 OR prepared_run_digest=$2)";
            let observation = SqlObservation::new(&self.store, &self.sql_observations, statement);
            let query = sqlx::query(statement)
                .bind(observation.bind(contract.run_identifier()))
                .bind(observation.bind(contract.prepared_run_digest()));
            let rows = observation.fetch_all(query, &mut connection).await?;
            if rows.len() != 1 || rows[0].get::<bool, _>(0) { return conflict(); }
            Ok(())
        })
    }
    fn store_identity(&self) -> &SourceStoreIdentity {
        &self.identity
    }
    fn begin(&mut self, contract: &ContextApplyContract) -> HarnessResult<()> {
        let bytes = check_contract(self, contract)?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| source_error(Error::Storage))?;
        self.handle.block_on(async {
            let mut tx=connection.begin().await.map_err(storage)?;
            let row=effect(&self.store, &self.sql_observations, &mut tx,contract,&bytes).await?;
            check_versions(&self.store, &self.sql_observations, &mut tx,contract.non_target_versions()).await?;
            check_versions(&self.store, &self.sql_observations, &mut tx,&previous(contract)).await?;
            if let Some(row)=row {
                if row.get::<String,_>("state")!="pending" {return conflict();}
            } else if contract.requires_commit() {
                let a=contract.attempt();
                { let statement = "INSERT INTO context_apply_batches(apply_id,store_id,core_run_id,prepared_run_digest,candidate_digest,expected_source_versions,context_targets,core_apply_attempt_id,expected_batch_id,expected_journal_locator,state,core_contract,core_contract_digest) VALUES(gen_random_uuid(),$1::uuid,$2,$3,$4,$5,$6,$7,$8,$9,'pending',$10,$11)"; let observation = SqlObservation::new(&self.store, &self.sql_observations, statement); let query = sqlx::query(statement)
                    .bind(observation.bind(&self.identity.store_id)).bind(observation.bind(&a.run_identifier)).bind(observation.bind(&a.prepared_run_digest)).bind(observation.bind(&a.candidate_digest)).bind(observation.bind(expected(contract)))
                    .bind(observation.bind(serde_json::to_value(contract.targets()).map_err(|_|source_error(Error::Storage))?)).bind(observation.bind(&a.attempt_identifier)).bind(observation.bind(&a.expected_batch_identifier)).bind(observation.bind(&a.expected_journal_relative_path)).bind(observation.bind(&bytes)).bind(observation.bind(digest(&bytes)))
                    ; observation.execute(query, &mut tx).await? };
            }
            tx.commit().await.map_err(storage)
        })
    }
    fn recover(
        &mut self,
        recovery: &ContextRecoveryContract,
    ) -> HarnessResult<ContextRecoveryStatus> {
        let contract = recovery.apply();
        let bytes = check_contract(self, contract)?;
        let result = {
            let mut connection = self
                .connection
                .lock()
                .map_err(|_| source_error(Error::Storage))?;
            self.handle.block_on(async {
                let mut tx = connection.begin().await.map_err(storage)?;
                let row = effect(
                    &self.store,
                    &self.sql_observations,
                    &mut tx,
                    contract,
                    &bytes,
                )
                .await?;
                check_versions(
                    &self.store,
                    &self.sql_observations,
                    &mut tx,
                    contract.non_target_versions(),
                )
                .await?;
                let status = match row {
                    None => {
                        check_versions(
                            &self.store,
                            &self.sql_observations,
                            &mut tx,
                            &previous(contract),
                        )
                        .await?;
                        if contract.requires_commit() {
                            ContextRecoveryStatus::Absent
                        } else {
                            ContextRecoveryStatus::NotRequired
                        }
                    }
                    Some(row) => match row.get::<String, _>("state").as_str() {
                        "pending" => {
                            check_versions(
                                &self.store,
                                &self.sql_observations,
                                &mut tx,
                                &previous(contract),
                            )
                            .await?;
                            ContextRecoveryStatus::Pending
                        }
                        "aborted" => {
                            check_versions(
                                &self.store,
                                &self.sql_observations,
                                &mut tx,
                                &previous(contract),
                            )
                            .await?;
                            ContextRecoveryStatus::Aborted
                        }
                        state @ ("committed" | "finalized") => {
                            let receipt = committed(
                                &self.store,
                                &self.sql_observations,
                                &mut tx,
                                contract,
                                &row,
                            )
                            .await?;
                            if recovery.stored_receipt().is_some_and(|r| r != &receipt) {
                                return conflict();
                            }
                            if state == "finalized" {
                                ContextRecoveryStatus::Finalized(receipt)
                            } else {
                                ContextRecoveryStatus::Committed(receipt)
                            }
                        }
                        _ => return conflict(),
                    },
                };
                tx.commit().await.map_err(storage)?;
                Ok(status)
            })?
        };
        self.source_allowed = true;
        self.preserved_targets = contract
            .targets()
            .iter()
            .map(|t| t.previous().logical_path.clone())
            .collect();
        Ok(result)
    }
    fn commit(
        &mut self,
        verified: &VerifiedContextCommit<'_>,
    ) -> HarnessResult<ContextCommitReceipt> {
        let contract = verified.contract();
        let bytes = check_contract(self, contract)?;
        if !contract.requires_commit() {
            return conflict();
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| source_error(Error::Storage))?;
        self.handle.block_on(async {
            let mut tx=connection.begin().await.map_err(storage)?;
            let row=effect(&self.store, &self.sql_observations, &mut tx,contract,&bytes).await?.ok_or_else(||source_error(Error::Conflict))?;
            if row.get::<String,_>("state")!="pending" {return conflict();}
            check_versions(&self.store, &self.sql_observations, &mut tx,contract.non_target_versions()).await?;
            check_versions(&self.store, &self.sql_observations, &mut tx,&previous(contract)).await?;
            let apply_id=row.get::<String,_>("id");
            for page in contract.targets().chunks(100) {
                let mut records=Vec::new();let mut payloads=BTreeMap::new();
                for target in page {
                    let (scope,path,source_path)=logical(&target.previous().logical_path)?;
                    let deleted=target.operation()==FinalizationOperation::Delete;
                    let content=verified.target_content(&target.previous().logical_path);
                    if deleted!=content.is_none(){return conflict();}
                    let content=content.unwrap_or(&[]);let sha=digest(content);
                    if !deleted&&Some(sha.as_str())!=target.resulting_content_digest(){return conflict();}
                    let payload=projection(&scope,&path,&sha,false,deleted,(!deleted).then_some(content)).map_err(source_error)?;
                    let search_text=(payload["status"]=="searchable").then(||format!("{} {} {}",payload["title"].as_str().unwrap_or_default(),payload["body"].as_str().unwrap_or_default(),payload["aliases"]));
                    payloads.insert(source_path.clone(),payload);
                    let (id,revision)=match &target.previous().state {
                        StoredSourceState::Missing=>(None,None),
                        StoredSourceState::Live{material_id,revision,..}|StoredSourceState::Deleted{material_id,revision}=>(Some(material_id),Some(*revision)),
                    };
                    records.push(json!({"scope":scope.as_str(),"path":path,"source_path":source_path,"content":content,"sha":sha,"deleted":deleted,"id":id,"revision":revision,"search_text":search_text}));
                }
                let records=Value::Array(records);
                // JSON byte arrays decode inside PG to avoid per-target network calls.
                { let statement = "WITH input AS (SELECT x.*,decode((SELECT string_agg(lpad(to_hex(v::int),2,'0'),'') FROM jsonb_array_elements_text(x.content) AS b(v)),'hex') AS bytes FROM jsonb_to_recordset($1) AS x(scope text,path text,source_path text,content jsonb,sha text,deleted boolean,id uuid,revision bigint,search_text text)) INSERT INTO context_materials(scope,path,source_path,content_digest,content,byte_len,restricted,search_text,deleted,origin_kind,source_root,source_digest,imported_at,last_apply_id) SELECT scope,path,source_path,sha,COALESCE(bytes,''::bytea),jsonb_array_length(content),false,search_text,deleted,'native',NULL,NULL,NULL,$2::uuid FROM input WHERE id IS NULL"; let observation = SqlObservation::new(&self.store, &self.sql_observations, statement); let query = sqlx::query(statement)
                    .bind(observation.bind(&records)).bind(observation.bind(&apply_id)); observation.execute(query, &mut tx).await? };
                { let statement = "WITH input AS (SELECT x.*,decode((SELECT string_agg(lpad(to_hex(v::int),2,'0'),'') FROM jsonb_array_elements_text(x.content) AS b(v)),'hex') AS bytes FROM jsonb_to_recordset($1) AS x(scope text,path text,source_path text,content jsonb,sha text,deleted boolean,id uuid,revision bigint,search_text text)) UPDATE context_materials m SET content=COALESCE(i.bytes,''::bytea),content_digest=i.sha,byte_len=jsonb_array_length(i.content),search_text=i.search_text,deleted=i.deleted,last_apply_id=$2::uuid FROM input i WHERE i.id=m.material_id AND i.revision=m.revision AND i.source_path=m.source_path"; let observation = SqlObservation::new(&self.store, &self.sql_observations, statement); let query = sqlx::query(statement)
                    .bind(observation.bind(&records)).bind(observation.bind(&apply_id)); observation.execute(query, &mut tx).await? };
                let paths=payloads.keys().cloned().collect::<Vec<_>>();
                let rows={ let statement = "SELECT source_path,material_id::text,revision FROM context_materials WHERE source_path=ANY($1)"; let observation = SqlObservation::new(&self.store, &self.sql_observations, statement); let query = sqlx::query(statement).bind(observation.bind(&paths)); observation.fetch_all(query, &mut tx).await? };
                if rows.len()!=page.len(){return conflict();}
                let projections=rows.iter().map(|row|json!({"material_id":row.get::<String,_>("material_id"),"revision":row.get::<i64,_>("revision"),"payload":payloads[&row.get::<String,_>("source_path")]})).collect::<Vec<_>>();
                let input_bytes = serde_json::to_vec(&projections).map_or(usize::MAX, |bytes| bytes.len());
                let before = self.store.calls();
                let result = append_projections(&self.store,&mut tx,&projections).await;
                {
                    let mut state = self.sql_observations.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    if state.projections.len() == 512 { state.overflowed = true; }
                    else { state.projections.push(NativeProjectionCall { records: projections.len(), input_bytes, store_call_delta: self.store.calls().saturating_sub(before), succeeded: result.is_ok() }); }
                }
                result.map_err(source_error)?;
            }
            let (resulting,projection_digest)=committed_rows(&self.store, &self.sql_observations, &mut tx,&apply_id,&previous(contract)).await?;
            let receipt=ContextCommitReceipt::from_persisted(verified,previous(contract),resulting,projection_digest)?;
            { let statement = "UPDATE context_apply_batches SET state='committed',actual_batch_id=$2,actual_journal_locator=$3,commit_receipt=$4,updated_at=now() WHERE apply_id=$1::uuid AND state='pending'"; let observation = SqlObservation::new(&self.store, &self.sql_observations, statement); let query = sqlx::query(statement)
                .bind(observation.bind(&apply_id)).bind(observation.bind(receipt.batch_identifier())).bind(observation.bind(receipt.journal_relative_path())).bind(observation.bind(serde_json::to_value(&receipt).map_err(|_|source_error(Error::Storage))?)); observation.execute(query, &mut tx).await? };
            tx.commit().await.map_err(storage)?;
            Ok(receipt)
        })
    }
    fn finalize(&mut self, proof: &ContextTerminalProof) -> HarnessResult<()> {
        let contract = proof.contract();
        let bytes = check_contract(self, contract)?;
        let (HarnessApplyAttemptState::AppliedFinalized { apply_receipt, .. }
        | HarnessApplyAttemptState::RecoveredFinalized { apply_receipt, .. }) =
            &proof.persisted_attempt().attempt_state
        else {
            return conflict();
        };
        apply_receipt.validate()?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| source_error(Error::Storage))?;
        self.handle.block_on(async {
            let mut tx=connection.begin().await.map_err(storage)?;
            let row=effect(&self.store, &self.sql_observations, &mut tx,contract,&bytes).await?;
            check_versions(&self.store, &self.sql_observations, &mut tx,contract.non_target_versions()).await?;
            if contract.requires_commit(){
                let row=row.ok_or_else(||source_error(Error::Conflict))?;
                let receipt=committed(&self.store, &self.sql_observations, &mut tx,contract,&row).await?;
                if apply_receipt.context_commit_receipt.as_ref()!=Some(&receipt){return conflict();}
                let state=row.get::<String,_>("state");
                if state=="finalized" {
                    if row.get::<Option<String>,_>("final_core_receipt_digest").as_deref()!=Some(apply_receipt.apply_receipt_digest.as_str()){return conflict();}
                }else if state=="committed"{
                    { let statement = "UPDATE context_apply_batches SET state='finalized',final_core_receipt_digest=$2,updated_at=now() WHERE apply_id=$1::uuid"; let observation = SqlObservation::new(&self.store, &self.sql_observations, statement); let query = sqlx::query(statement).bind(observation.bind(row.get::<String,_>("id"))).bind(observation.bind(&apply_receipt.apply_receipt_digest)); observation.execute(query, &mut tx).await? };
                }else{return conflict();}
            }else if row.is_some()||apply_receipt.context_commit_receipt.is_some(){return conflict();}
            tx.commit().await.map_err(storage)
        })
    }
    fn abort(&mut self, proof: &ContextAbortProof) -> HarnessResult<()> {
        let contract = proof.contract();
        let bytes = check_contract(self, contract)?;
        if !matches!(
            proof.persisted_attempt().attempt_state,
            HarnessApplyAttemptState::AbortedBeforeMutation { .. }
                | HarnessApplyAttemptState::AbortedAfterRollback { .. }
        ) {
            return conflict();
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| source_error(Error::Storage))?;
        self.handle.block_on(async {
            let mut tx=connection.begin().await.map_err(storage)?;
            let row=effect(&self.store, &self.sql_observations, &mut tx,contract,&bytes).await?;
            check_versions(&self.store, &self.sql_observations, &mut tx,contract.non_target_versions()).await?;
            check_versions(&self.store, &self.sql_observations, &mut tx,&previous(contract)).await?;
            if let Some(row)=row {
                if !matches!(row.get::<String,_>("state").as_str(),"pending"|"aborted")||row.get::<Option<Value>,_>("commit_receipt").is_some(){return conflict();}
                { let statement = "UPDATE context_apply_batches SET state='aborted',updated_at=now() WHERE apply_id=$1::uuid"; let observation = SqlObservation::new(&self.store, &self.sql_observations, statement); let query = sqlx::query(statement).bind(observation.bind(row.get::<String,_>("id"))); observation.execute(query, &mut tx).await? };
            }
            tx.commit().await.map_err(storage)
        })
    }
}
