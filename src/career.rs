//! One current native snapshot for the declared career inventory, without a source-view copy.
use crate::{
    domain::Error,
    store::{Store, digest},
};
use context_core::{
    career::{
        CareerComparison, CareerDiscoveryRequest, CareerInventory, CareerOriginal,
        MAX_CAREER_SOURCE_BYTES, MAX_CAREER_TOTAL_BYTES, discover,
    },
    harness::{HarnessError, PolicyConfiguration, SourceVersion, StoredSourceState},
};
use sqlx::{PgConnection, Row};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(serde::Serialize)]
pub struct CareerQueryResult {
    pub inventory: CareerInventory,
    pub comparison_digest: Option<String>,
    pub finalization_structure_checked: bool,
    pub semantic_review_required: bool,
}

impl Store {
    pub async fn career_candidates(
        &self,
        policy_path: &Path,
        request: CareerDiscoveryRequest,
        comparison: Option<CareerComparison>,
        finalization: bool,
    ) -> Result<CareerQueryResult, Error> {
        request.validate().map_err(|_| Error::Invalid)?;
        let configuration = PolicyConfiguration::read(policy_path).map_err(|_| Error::Invalid)?;
        configuration
            .career_source_roots()
            .map_err(|_| Error::Invalid)?;
        let mut connection = self.dedicated_context_connection(false).await?;
        self.count(1);
        let store_id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
            .fetch_one(&mut connection)
            .await
            .map_err(|_| Error::Storage)?;
        let store = self.clone();
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let result = snapshot(
                &store,
                &handle,
                &mut connection,
                &configuration,
                store_id,
                request,
                comparison.as_ref(),
                finalization,
            );
            result.map_err(|e| {
                // Keep bounded diagnostic errors available at the native entrypoint.
                eprintln!("Career discovery failed: {e}");
                Error::Invalid
            })
        })
        .await
        .map_err(|_| Error::Storage)?
    }
}

/// Adapter-only discovery boundary. Raw multi-owner bytes never enter Harness role bundles.
#[allow(clippy::too_many_arguments)]
pub(crate) fn snapshot(
    store: &Store,
    handle: &tokio::runtime::Handle,
    connection: &mut PgConnection,
    configuration: &PolicyConfiguration,
    store_id: String,
    request: CareerDiscoveryRequest,
    comparison: Option<&CareerComparison>,
    finalization: bool,
) -> Result<CareerQueryResult, HarnessError> {
    if finalization && comparison.is_none() {
        return Err(HarnessError::InvalidRequest(
            "discovery without comparison is not finalization".into(),
        ));
    }
    request.validate()?;
    configuration.career_source_roots()?;
    let storage_error = |e: sqlx::Error| HarnessError::InvalidRepository(e.to_string());
    store.count(1);
    handle
        .block_on(
            sqlx::raw_sql("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
                .execute(&mut *connection),
        )
        .map_err(storage_error)?;
    let mut originals = BTreeMap::new();
    let result = (|| {
        let inventory = discover(configuration, Some(store_id), request, |paths| {
            let wave = handle
                .block_on(read_wave(store, connection, paths))
                .map_err(|e| HarnessError::InvalidRepository(e.to_string()))?;
            for original in &wave {
                originals.insert(original.path.clone(), original.clone());
            }
            Ok(wave)
        })?;
        if let Some(comparison) = comparison {
            comparison.validate_current(&inventory, &originals, finalization)?;
        }
        Ok(CareerQueryResult {
            inventory,
            comparison_digest: comparison.map(context_core::career::digest).transpose()?,
            finalization_structure_checked: finalization,
            semantic_review_required: true,
        })
    })();
    store.count(1);
    handle
        .block_on(sqlx::raw_sql("ROLLBACK").execute(connection))
        .map_err(storage_error)?;
    result
}

async fn read_wave(
    store: &Store,
    connection: &mut PgConnection,
    paths: &[String],
) -> Result<Vec<CareerOriginal>, Error> {
    let selected: Vec<_> = paths
        .iter()
        .map(|p| p.strip_prefix("vault/").ok_or(Error::Invalid))
        .collect::<Result<_, _>>()?;
    store.count(1);
    let rows = sqlx::query(r#"
        WITH selected AS MATERIALIZED (
            SELECT source_path,material_id,revision,origin_kind,source_digest,content_digest,byte_len,content
            FROM context_materials WHERE source_path=ANY($1) AND NOT deleted AND NOT restricted
        ), bounds AS (
            SELECT COALESCE(bool_and(byte_len BETWEEN 0 AND $2 AND octet_length(content) BETWEEN 0 AND $2
                AND byte_len=octet_length(content)),true) AS bounded,
                COALESCE(sum(byte_len),0)::bigint AS total FROM selected
        )
        SELECT s.source_path,s.material_id::text,s.revision,s.origin_kind,s.source_digest,s.content_digest,s.byte_len,
            CASE WHEN b.bounded AND b.total<=$3 THEN s.content END AS content
        FROM selected s CROSS JOIN bounds b ORDER BY s.source_path COLLATE "C"
    "#).bind(&selected).bind(MAX_CAREER_SOURCE_BYTES as i64).bind(MAX_CAREER_TOTAL_BYTES as i64)
        .fetch_all(connection).await.map_err(|_| Error::Storage)?;
    let mut originals = Vec::with_capacity(rows.len());
    for row in rows {
        let bytes: Vec<u8> = row
            .get::<Option<Vec<u8>>, _>("content")
            .ok_or(Error::Limit)?;
        let content_digest: String = row.get("content_digest");
        let source_digest: Option<String> = row.get("source_digest");
        let origin: String = row.get("origin_kind");
        if bytes.len() as i64 != row.get::<i64, _>("byte_len")
            || digest(&bytes) != content_digest
            || match origin.as_str() {
                "native" => source_digest.is_some(),
                "imported-file" => !source_digest
                    .as_deref()
                    .is_some_and(crate::context::valid_digest),
                _ => true,
            }
        {
            return Err(Error::Storage);
        }
        let path = format!("vault/{}", row.get::<String, _>("source_path"));
        let content = String::from_utf8(bytes).map_err(|_| Error::Invalid)?;
        let version = SourceVersion {
            logical_path: path.clone(),
            state: StoredSourceState::Live {
                material_id: row.get("material_id"),
                revision: row
                    .get::<i64, _>("revision")
                    .try_into()
                    .map_err(|_| Error::Storage)?,
                content_digest,
            },
        };
        originals.push(CareerOriginal {
            path,
            content,
            version: Some(version),
        });
    }
    Ok(originals)
}
