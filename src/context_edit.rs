//! Explicit human edits of existing public Markdown originals; Core apply remains separate.
use crate::{
    context::{
        ContextScope, MAX_READ_BYTES, markdown_path, restricted, valid_digest, validate_path,
    },
    context_projection::{append_projections, projection},
    domain::Error,
    store::{Store, digest},
};
use serde::Serialize;
use serde_json::json;
use sqlx::Row;

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ContextEditResult {
    pub scope: ContextScope,
    pub path: String,
    pub revision: i64,
    pub content_digest: String,
    pub byte_len: usize,
    pub manual_edit_id: Option<String>,
    pub changed: bool,
}

impl Store {
    pub async fn edit_context(
        &self,
        scope: &ContextScope,
        path: &str,
        expected_revision: i64,
        expected_digest: &str,
        content: &str,
    ) -> Result<ContextEditResult, Error> {
        validate_path(path)?;
        if scope.as_str() != "personal" && !scope.as_str().starts_with("work/")
            || restricted(path)
            || !markdown_path(path)
            || expected_revision < 1
            || !valid_digest(expected_digest)
            || content.len() > MAX_READ_BYTES
        {
            return Err(Error::Invalid);
        }
        let bytes = content.as_bytes();
        let next_digest = digest(bytes);
        let mut tx = self.lock_context(true).await?;
        let result = async {
            self.count(1);
            let row = sqlx::query("SELECT material_id::text,revision,content_digest,byte_len,encode(sha256(content),'hex') AS actual_digest,octet_length(content) AS actual_len,origin_kind,source_digest,source_path FROM context_materials WHERE scope=$1 AND path=$2 AND NOT deleted AND NOT restricted FOR UPDATE")
                .bind(scope.as_str()).bind(path).fetch_optional(&mut *tx).await.map_err(|_| Error::Storage)?.ok_or(Error::NotFound)?;
            let current_revision: i64 = row.get("revision");
            let current_digest: String = row.get("content_digest");
            let material_id: String = row.get("material_id");
            let origin: String = row.get("origin_kind");
            let source_digest: Option<String> = row.get("source_digest");
            if row.get::<i64, _>("byte_len") != row.get::<i32, _>("actual_len") as i64
                || current_digest != row.get::<String, _>("actual_digest")
                || row.get::<String, _>("source_path") != format!("{}/{}", scope.as_str(), path)
                || !matches!((origin.as_str(), source_digest), ("native", None) | ("imported-file", Some(_)))
            {
                return Err(Error::Storage);
            }
            if current_revision != expected_revision || current_digest != expected_digest {
                return Err(Error::Conflict);
            }
            if current_digest == next_digest {
                return Ok(ContextEditResult { scope: scope.clone(), path: path.to_owned(), revision: current_revision, content_digest: current_digest, byte_len: bytes.len(), manual_edit_id: None, changed: false });
            }
            let payload = projection(scope, path, &next_digest, false, false, Some(bytes))?;
            let search_text = (!bytes.contains(&0)).then_some(content);
            self.count(1);
            let edit_id: String = sqlx::query_scalar("INSERT INTO context_manual_edits(store_id,material_id,expected_revision,expected_content_digest,resulting_content_digest) VALUES((SELECT store_id FROM context_store),$1::uuid,$2,$3,$4) RETURNING edit_id::text")
                .bind(&material_id).bind(current_revision).bind(expected_digest).bind(&next_digest)
                .fetch_one(&mut *tx).await.map_err(|_| Error::Storage)?;
            self.count(1);
            let updated: i64 = sqlx::query_scalar("UPDATE context_materials SET content=$1,content_digest=$2,byte_len=$3,search_text=$4,last_manual_edit_id=$5::uuid WHERE material_id=$6::uuid AND revision=$7 AND content_digest=$8 RETURNING revision")
                .bind(bytes).bind(&next_digest).bind(bytes.len() as i64).bind(search_text).bind(&edit_id).bind(&material_id).bind(current_revision).bind(expected_digest)
                .fetch_optional(&mut *tx).await.map_err(|_| Error::Storage)?.ok_or(Error::Conflict)?;
            if updated != current_revision + 1 { return Err(Error::Storage); }
            append_projections(self, &mut tx, &[json!({"material_id":material_id,"revision":updated,"payload":payload})]).await?;
            Ok(ContextEditResult { scope: scope.clone(), path: path.to_owned(), revision: updated, content_digest: next_digest, byte_len: bytes.len(), manual_edit_id: Some(edit_id), changed: true })
        }.await;
        self.finish_context(tx, result).await
    }
}
